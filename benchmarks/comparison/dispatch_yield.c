/* Dispatch yield benchmark — C comparison
 * Simulates generator yielding 1M values via ucontext coroutine.
 */
#include <stdio.h>
#include <stdlib.h>
#include <ucontext.h>

#define STACK_SIZE (64 * 1024)
#define N 30000000

static ucontext_t gen_ctx, main_ctx;
static long yielded_value;
static int gen_done;

static void generator(void) {
    for (long i = 0; i < N; i++) {
        yielded_value = i;
        swapcontext(&gen_ctx, &main_ctx);
    }
    gen_done = 1;
    swapcontext(&gen_ctx, &main_ctx);
}

int main(void) {
    char *stack = malloc(STACK_SIZE);
    getcontext(&gen_ctx);
    gen_ctx.uc_stack.ss_sp = stack;
    gen_ctx.uc_stack.ss_size = STACK_SIZE;
    gen_ctx.uc_link = &main_ctx;
    gen_done = 0;
    makecontext(&gen_ctx, generator, 0);

    long total = 0;
    while (1) {
        swapcontext(&main_ctx, &gen_ctx);
        if (gen_done) break;
        total += yielded_value;
    }
    printf("%ld\n", total);
    free(stack);
    return 0;
}
