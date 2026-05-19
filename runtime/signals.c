/*
 * Jinn Runtime — Crash handlers (P0-6).
 *
 * Installs a SIGSEGV / SIGBUS handler that detects coroutine stack
 * overflow by checking the faulting address against the guard page of
 * the active coroutine on the trapping thread. Produces a diagnostic
 * and exits with rc=134 (SIGABRT-equivalent) instead of a silent
 * segfault, garbage value, or rc=0.
 *
 * The handler runs on a per-thread sigaltstack so that a faulted
 * stack does not prevent it from executing.
 */

#include "jinn_rt.h"

#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

/* sigaltstack must be at least SIGSTKSZ, but we want something generous
 * so the handler can call into snprintf/write safely. */
#define JINN_SIGSTACK_SIZE (64 * 1024)

static _Atomic int g_handlers_installed = 0;
static pthread_key_t g_altstack_key;
static pthread_once_t g_altstack_key_once = PTHREAD_ONCE_INIT;

static void altstack_free(void *p) {
    if (p) {
        munmap(p, JINN_SIGSTACK_SIZE);
    }
}

static void altstack_key_init(void) {
    pthread_key_create(&g_altstack_key, altstack_free);
}

/* Install a sigaltstack for the calling thread (idempotent). */
static void install_sigaltstack_for_thread(void) {
    pthread_once(&g_altstack_key_once, altstack_key_init);
    if (pthread_getspecific(g_altstack_key)) {
        return; /* already installed */
    }
    void *mem = mmap(NULL, JINN_SIGSTACK_SIZE, PROT_READ | PROT_WRITE,
                     MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (mem == MAP_FAILED) {
        return;
    }
    stack_t ss = {0};
    ss.ss_sp = mem;
    ss.ss_size = JINN_SIGSTACK_SIZE;
    ss.ss_flags = 0;
    if (sigaltstack(&ss, NULL) == 0) {
        pthread_setspecific(g_altstack_key, mem);
    } else {
        munmap(mem, JINN_SIGSTACK_SIZE);
    }
}

/*
 * Async-signal-safe write of a short message followed by a decimal
 * integer and a newline. Avoids stdio buffering.
 */
static void emit_overflow_msg(const char *prefix, void *addr) {
    (void)write(STDERR_FILENO, prefix, strlen(prefix));
    char buf[64];
    /* Format address as hex manually (avoid printf in signal handler). */
    uintptr_t a = (uintptr_t)addr;
    char hex[2 + 2 * sizeof(uintptr_t) + 1];
    hex[0] = '0';
    hex[1] = 'x';
    for (size_t i = 0; i < 2 * sizeof(uintptr_t); i++) {
        unsigned nib = (a >> (4 * (2 * sizeof(uintptr_t) - 1 - i))) & 0xF;
        hex[2 + i] = (char)(nib < 10 ? '0' + nib : 'a' + (nib - 10));
    }
    hex[sizeof(hex) - 1] = '\0';
    (void)write(STDERR_FILENO, hex, sizeof(hex) - 1);
    (void)write(STDERR_FILENO, "\n", 1);
    (void)snprintf(buf, sizeof(buf),
                   "  coroutine stack size: %u bytes (guard page %u bytes)\n",
                   (unsigned)JINN_STACK_SIZE, (unsigned)JINN_GUARD_SIZE);
    (void)write(STDERR_FILENO, buf, strlen(buf));
    (void)write(STDERR_FILENO,
                "  refactor deep recursion to use iteration, accumulator-style\n"
                "  tail calls, or `dispatch` blocks; or increase\n"
                "  JINN_STACK_SIZE in runtime/jinn_rt.h and recompile.\n",
                strlen("  refactor deep recursion to use iteration, accumulator-style\n"
                       "  tail calls, or `dispatch` blocks; or increase\n"
                       "  JINN_STACK_SIZE in runtime/jinn_rt.h and recompile.\n"));
}

static void crash_handler(int sig, siginfo_t *si, void *uctx) {
    (void)uctx;
    void *fault = si ? si->si_addr : NULL;

    /* If the current thread is running a coroutine and the fault is in
     * (or below) its guard page, this is a coroutine stack overflow. */
    jinn_worker_t *w = tl_worker;
    jinn_coro_t *c = w ? w->current : NULL;
    if (c && c->stack_base) {
        uintptr_t base = (uintptr_t)c->stack_base;
        uintptr_t guard_end = base + JINN_GUARD_SIZE;
        uintptr_t fa = (uintptr_t)fault;
        /* Accept anywhere in the guard page, and a small slop above it
         * (a frame that crossed the guard by a few bytes). */
        if (fa >= base - 4096 && fa < guard_end + 4096) {
            (void)write(STDERR_FILENO,
                        "jinn runtime: coroutine stack overflow at fault address ",
                        strlen("jinn runtime: coroutine stack overflow at fault address "));
            emit_overflow_msg("", fault);
            _exit(134);
        }
    }

    /* Non-coroutine fault: produce a minimal diagnostic and re-raise
     * the default action so the OS dumps core / produces SIGSEGV. */
    const char *what = (sig == SIGBUS) ? "SIGBUS" : "SIGSEGV";
    (void)write(STDERR_FILENO, "jinn runtime: ", strlen("jinn runtime: "));
    (void)write(STDERR_FILENO, what, strlen(what));
    (void)write(STDERR_FILENO, " at ", 4);
    {
        uintptr_t a = (uintptr_t)fault;
        char hex[2 + 2 * sizeof(uintptr_t) + 1];
        hex[0] = '0';
        hex[1] = 'x';
        for (size_t i = 0; i < 2 * sizeof(uintptr_t); i++) {
            unsigned nib = (a >> (4 * (2 * sizeof(uintptr_t) - 1 - i))) & 0xF;
            hex[2 + i] = (char)(nib < 10 ? '0' + nib : 'a' + (nib - 10));
        }
        hex[sizeof(hex) - 1] = '\0';
        (void)write(STDERR_FILENO, hex, sizeof(hex) - 1);
    }
    (void)write(STDERR_FILENO, "\n", 1);

    /* Restore default handler and re-raise so the OS produces a core
     * dump and the process exits with the standard signal status. */
    struct sigaction dfl = {0};
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);
    sigaction(sig, &dfl, NULL);
    raise(sig);
}

void jinn_install_crash_handlers(void) {
    install_sigaltstack_for_thread();

    int expected = 0;
    if (!atomic_compare_exchange_strong(&g_handlers_installed, &expected, 1)) {
        return; /* already installed globally */
    }

    struct sigaction sa = {0};
    sa.sa_sigaction = crash_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_RESTART;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
}

/* Worker threads call this from their startup so their sigaltstack is
 * installed before they run any user coroutine. */
void jinn_install_worker_sigaltstack(void) {
    install_sigaltstack_for_thread();
}
