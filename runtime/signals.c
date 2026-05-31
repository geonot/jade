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

/* pthread_getattr_np / pthread_attr_getstack are GNU extensions; request
 * them before any system header is pulled in (directly or via jinn_rt.h). */
#ifndef _GNU_SOURCE
#define _GNU_SOURCE 1
#endif

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
/* Weak fallback definition so this translation unit can be linked into
 * programs that never pull in the scheduler (sched.o). When sched.o is
 * present its strong definition wins; otherwise tl_worker resolves to NULL
 * and coroutine-overflow detection degrades to the generic SIGSEGV path. */
__attribute__((weak)) _Thread_local jinn_worker_t *tl_worker = NULL;

/* Cached native-thread stack bounds for the trapping thread. Filled in
 * ordinary (async-signal-safe) context at handler-install time so the
 * signal handler only ever *reads* these scalars and never queries
 * pthread state itself (pthread_getattr_np is not async-signal-safe). */
static _Thread_local uintptr_t tl_stack_low = 0;   /* lowest usable byte */
static _Thread_local uintptr_t tl_stack_guard = 0; /* guard region size  */
static _Thread_local int tl_stack_cached = 0;      /* query attempted    */

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

/* Query and cache this thread's native stack bounds (idempotent).
 *
 * Runs in ordinary thread context (from the sigaltstack installer), so it
 * is safe to call the non-async-signal-safe GNU helpers here. glibc reports
 * `stackaddr` as the lowest usable byte of the stack mapping, with the guard
 * region sitting just below it; the stack itself grows downward toward
 * `stackaddr`. A stack overflow therefore faults at or just below
 * `tl_stack_low`. */
static void cache_thread_stack_bounds(void) {
    if (tl_stack_cached) {
        return;
    }
    tl_stack_cached = 1; /* mark attempted even on failure */
    pthread_attr_t attr;
    if (pthread_getattr_np(pthread_self(), &attr) != 0) {
        return;
    }
    void *addr = NULL;
    size_t size = 0;
    size_t guard = 0;
    if (pthread_attr_getstack(&attr, &addr, &size) == 0) {
        tl_stack_low = (uintptr_t)addr;
    }
    if (pthread_attr_getguardsize(&attr, &guard) == 0) {
        tl_stack_guard = (uintptr_t)guard;
    }
    pthread_attr_destroy(&attr);
}

/* Install a sigaltstack for the calling thread (idempotent). */
static void install_sigaltstack_for_thread(void) {
    cache_thread_stack_bounds();
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

/* Async-signal-safe write of a pointer as `0x…` hex (no newline). */
static void write_hex_addr(void *addr) {
    uintptr_t a = (uintptr_t)addr;
    char hex[2 + 2 * sizeof(uintptr_t)];
    hex[0] = '0';
    hex[1] = 'x';
    for (size_t i = 0; i < 2 * sizeof(uintptr_t); i++) {
        unsigned nib = (a >> (4 * (2 * sizeof(uintptr_t) - 1 - i))) & 0xF;
        hex[2 + i] = (char)(nib < 10 ? '0' + nib : 'a' + (nib - 10));
    }
    (void)write(STDERR_FILENO, hex, sizeof(hex));
}

/*
 * Async-signal-safe diagnostic for a coroutine stack overflow: the fault
 * address followed by the coroutine stack geometry and remediation advice.
 */
static void emit_overflow_msg(const char *prefix, void *addr) {
    (void)write(STDERR_FILENO, prefix, strlen(prefix));
    write_hex_addr(addr);
    (void)write(STDERR_FILENO, "\n", 1);
    char buf[80];
    (void)snprintf(buf, sizeof(buf),
                   "  coroutine stack size: %u bytes (guard page %u bytes)\n",
                   (unsigned)JINN_STACK_SIZE, (unsigned)JINN_GUARD_SIZE);
    (void)write(STDERR_FILENO, buf, strlen(buf));
    static const char advice[] =
        "  refactor deep recursion to use iteration, accumulator-style\n"
        "  tail calls, or `dispatch` blocks; or increase\n"
        "  JINN_STACK_SIZE in runtime/jinn_rt.h and recompile.\n";
    (void)write(STDERR_FILENO, advice, sizeof(advice) - 1);
}

/*
 * Async-signal-safe diagnostic for a native (main / worker) thread stack
 * overflow: the fault address followed by remediation advice.
 */
static void emit_thread_overflow_msg(void *addr) {
    static const char head[] =
        "jinn runtime: stack overflow (native thread) at fault address ";
    (void)write(STDERR_FILENO, head, sizeof(head) - 1);
    write_hex_addr(addr);
    (void)write(STDERR_FILENO, "\n", 1);
    static const char advice[] =
        "  the call stack exceeded the OS thread stack limit\n"
        "  (deep or unbounded recursion is the usual cause)\n"
        "  refactor to iteration or an explicit work stack, or raise the\n"
        "  limit with `ulimit -s` before running.\n";
    (void)write(STDERR_FILENO, advice, sizeof(advice) - 1);
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

    /* Native-thread stack overflow: the fault lands in (or just below)
     * this thread's stack guard region. Detected against bounds cached at
     * install time, keeping the handler async-signal-safe. The window is
     * the guard region plus 64 KiB of slop below (a large frame may step
     * past a single guard page) and one page above `tl_stack_low` (an
     * access near the very bottom of the stack). */
    if (tl_stack_low) {
        uintptr_t low = tl_stack_low;
        uintptr_t guard = tl_stack_guard ? tl_stack_guard : 4096;
        uintptr_t fa = (uintptr_t)fault;
        if (fa >= low - (guard + 65536) && fa < low + 4096) {
            emit_thread_overflow_msg(fault);
            _exit(134);
        }
    }

    /* Non-coroutine fault: produce a minimal diagnostic and re-raise
     * the default action so the OS dumps core / produces SIGSEGV. */
    const char *what = (sig == SIGBUS) ? "SIGBUS" : "SIGSEGV";
    (void)write(STDERR_FILENO, "jinn runtime: ", strlen("jinn runtime: "));
    (void)write(STDERR_FILENO, what, strlen(what));
    (void)write(STDERR_FILENO, " at ", 4);
    write_hex_addr(fault);
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
