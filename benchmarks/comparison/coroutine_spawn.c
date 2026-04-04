// Coroutine spawn benchmark — C comparison using ucontext
// 100K coroutine spawns, each yields a value then returns

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
    long n = 1000000;
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
