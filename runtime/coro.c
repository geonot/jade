/*
 * Jade Runtime — Coroutine create/destroy/trampoline.
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

static _Atomic(uint32_t) g_coro_id_counter = 0;

/* Forward declarations */
static void jade_coro_trampoline(void);
static void jade_coro_exit(void);

jade_coro_t *jade_coro_create(void (*entry)(void*), void *arg) {
    jade_coro_t *c = (jade_coro_t *)calloc(1, sizeof(jade_coro_t));
    if (!c) return NULL;

    /* Allocate stack with mmap for guard page support */
    size_t total = JADE_STACK_SIZE;
    void *base = mmap(NULL, total, PROT_READ | PROT_WRITE,
                      MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (base == MAP_FAILED) {
        free(c);
        return NULL;
    }

    /* Guard page at the bottom (stack grows down) */
    mprotect(base, JADE_GUARD_SIZE, PROT_NONE);

    c->stack_base  = base;
    c->stack_size  = (uint32_t)total;
    c->entry       = entry;
    c->arg         = arg;
    c->state       = JADE_CORO_READY;
    c->id          = atomic_fetch_add(&g_coro_id_counter, 1);
    c->next        = NULL;
    c->wait_chan   = NULL;
    c->select_ready = -1;
    c->daemon      = 0;

    /*
     * Set up the initial stack so that jade_context_swap's `ret`
     * jumps to jade_coro_trampoline.
     *
     * Stack grows down. We set up:
     *   [top - 8]  = &jade_coro_exit  (fake return address for trampoline)
     *   [top - 16] = &jade_coro_trampoline (the "return address" for context_swap)
     *
     * We stash entry in r12 and arg in r13 (callee-saved, preserved across swap).
     */
    uintptr_t stack_top = (uintptr_t)base + total;
    /* Align to 16 bytes (ABI) */
    stack_top &= ~(uintptr_t)15;
    /* Push fake return for trampoline -> jade_coro_exit */
    stack_top -= 8;
    *(void **)stack_top = (void *)jade_coro_exit;
    /* Push trampoline as the address context_swap's `ret` will go to */
    stack_top -= 8;
    *(void **)stack_top = (void *)jade_coro_trampoline;

    /* Set up context */
#if defined(__x86_64__) || defined(_M_X64)
    c->ctx.rsp = (void *)stack_top;
    c->ctx.rbp = (void *)stack_top;
    c->ctx.r12 = (void *)entry;  /* stash entry */
    c->ctx.r13 = arg;             /* stash arg */
#elif defined(__aarch64__) || defined(_M_ARM64)
    c->ctx.sp  = (void *)stack_top;
    c->ctx.fp  = (void *)stack_top;
    c->ctx.lr  = (void *)jade_coro_trampoline;
    c->ctx.x19_x28[0] = (void *)entry;
    c->ctx.x19_x28[1] = arg;
#else
    /* Portable fallback: can't pre-set context, will use setjmp in trampoline */
    (void)stack_top;
#endif

    return c;
}

void jade_coro_destroy(jade_coro_t *c) {
    if (!c) return;
    if (c->stack_base) {
        munmap(c->stack_base, c->stack_size);
    }
    free(c);
}

/*
 * Trampoline: first function called when a coroutine starts.
 * Reads entry and arg from callee-saved registers set during create.
 */
static void jade_coro_trampoline(void) {
    jade_worker_t *w = tl_worker;
    jade_coro_t *self = w ? w->current : NULL;
    if (!self) return;

    /* Call the actual coroutine entry function */
    self->entry(self->arg);

    /* Entry returned — mark done and yield back to scheduler */
    jade_coro_exit();
}

static void jade_coro_exit(void) {
    jade_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jade_coro_t *self = w->current;
    self->state = JADE_CORO_DONE;
    /* Swap back to the scheduler; this coroutine is never resumed */
    jade_context_swap(&self->ctx, &w->sched_ctx);
    /* unreachable */
    __builtin_unreachable();
}

/*
 * jade_coro_yield: voluntary yield back to the scheduler.
 * The scheduler will re-enqueue this coroutine.
 */
void jade_coro_yield(void) {
    jade_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jade_coro_t *c = w->current;
    c->state = JADE_CORO_READY;
    jade_context_swap(&c->ctx, &w->sched_ctx);
    /* Resumed here when re-scheduled */
}

jade_coro_t *jade_current_coro(void) {
    jade_worker_t *w = tl_worker;
    return w ? w->current : NULL;
}

jade_worker_t *jade_current_worker(void) {
    return tl_worker;
}

void jade_coro_set_daemon(jade_coro_t *c) {
    if (c) c->daemon = 1;
}
