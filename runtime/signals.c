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

#define JINN_SIGSTACK_SIZE (64 * 1024)
static _Atomic int g_handlers_installed = 0;
__attribute__((weak)) _Thread_local jinn_worker_t *tl_worker = NULL;
static _Thread_local uintptr_t tl_stack_low = 0;
static _Thread_local uintptr_t tl_stack_guard = 0;
static _Thread_local int tl_stack_cached = 0;

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
static void cache_thread_stack_bounds(void) {
    if (tl_stack_cached) {
        return;
    }
    tl_stack_cached = 1;
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
static void install_sigaltstack_for_thread(void) {
    cache_thread_stack_bounds();
    pthread_once(&g_altstack_key_once, altstack_key_init);
    if (pthread_getspecific(g_altstack_key)) {
        return;
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
static struct sigaction g_prev_segv_action;
static struct sigaction g_prev_bus_action;
static void crash_handler(int sig, siginfo_t *si, void *uctx) {
    void *fault = si ? si->si_addr : NULL;
    jinn_worker_t *w = tl_worker;
    jinn_coro_t *c = w ? w->current : NULL;
    if (c && c->stack_base) {
        uintptr_t base = (uintptr_t)c->stack_base;
        uintptr_t guard_end = base + JINN_GUARD_SIZE;
        uintptr_t fa = (uintptr_t)fault;
        if (fa >= base - 4096 && fa < guard_end + 4096) {
            (void)write(STDERR_FILENO,
                        "jinn runtime: coroutine stack overflow at fault address ",
                        strlen("jinn runtime: coroutine stack overflow at fault address "));
            emit_overflow_msg("", fault);
            _exit(134);
        }
    }
    if (tl_stack_low) {
        uintptr_t low = tl_stack_low;
        uintptr_t guard = tl_stack_guard ? tl_stack_guard : 4096;
        uintptr_t fa = (uintptr_t)fault;
        if (fa >= low - (guard + 65536) && fa < low + 4096) {
            emit_thread_overflow_msg(fault);
            _exit(134);
        }
    }

    const char *what = (sig == SIGBUS) ? "SIGBUS" : "SIGSEGV";
    (void)write(STDERR_FILENO, "jinn runtime: ", strlen("jinn runtime: "));
    (void)write(STDERR_FILENO, what, strlen(what));
    (void)write(STDERR_FILENO, " at ", 4);
    write_hex_addr(fault);
    (void)write(STDERR_FILENO, "\n", 1);
    struct sigaction *prev = (sig == SIGBUS) ? &g_prev_bus_action : &g_prev_segv_action;
    if ((prev->sa_flags & SA_SIGINFO) && prev->sa_sigaction) {
        prev->sa_sigaction(sig, si, uctx);
        return;
    }
    if (!(prev->sa_flags & SA_SIGINFO) && prev->sa_handler != SIG_DFL &&
        prev->sa_handler != SIG_IGN && prev->sa_handler) {
        prev->sa_handler(sig);
        return;
    }
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
        return;
    }
    struct sigaction sa = {0};
    sa.sa_sigaction = crash_handler;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK | SA_RESTART;
    sigaction(SIGSEGV, &sa, &g_prev_segv_action);
    sigaction(SIGBUS, &sa, &g_prev_bus_action);
}
void jinn_install_worker_sigaltstack(void) {
    install_sigaltstack_for_thread();
}
