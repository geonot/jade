// Coroutine spawn benchmark — C comparison using ucontext
// 100K coroutine spawns, each yields a value then returns.
//
// Comparability: tagged cross-paradigm, and the reason matters. ucontext's
// swapcontext() calls sigprocmask on every switch; Jinn's context switch is
// the assembly in runtime/sched.c with no syscall at all. The ratio therefore
// measures "syscall per switch vs none", not code quality on either side.
// Iteration counts were realigned to 100K on 2026-08-06 — the C side had been
// running 1,000,000, which made the published ratio wrong by 10x on top of
// the paradigm difference.

#include <stdio.h>
#include <stdlib.h>
#include <ucontext.h>

#define STACK_SIZE (64 * 1024)

static ucontext_t coro_ctx, main_ctx;
static long yielded_value;

static void coro_body(void) {
    yielded_value = 1;
    swapcontext(&coro_ctx, &main_ctx);
}

int main() {
    long n = 100000;
    long total = 0;
    char *stack = malloc(STACK_SIZE);
    for (long i = 0; i < n; i++) {
        getcontext(&coro_ctx);
        coro_ctx.uc_stack.ss_sp = stack;
        coro_ctx.uc_stack.ss_size = STACK_SIZE;
        coro_ctx.uc_link = &main_ctx;
        makecontext(&coro_ctx, coro_body, 0);
        swapcontext(&main_ctx, &coro_ctx);
        total += yielded_value;
    }
    printf("%ld\n", total);
    free(stack);
    return 0;
}
