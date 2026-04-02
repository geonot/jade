/*
 * Jade Runtime — Coroutine create/destroy/trampoline.
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

static _Atomic(uint32_t) g_coro_id_counter = 0;

/* ── Stack cache (avoids repeated mmap/munmap) ───────────────────── */

#define STACK_CACHE_MAX 64

typedef struct {
    void   *base;
    size_t  size;
} cached_stack_t;

static cached_stack_t g_stack_cache[STACK_CACHE_MAX];
static _Atomic(int32_t) g_stack_cache_count = 0;
static _Atomic(int32_t) g_stack_cache_lock = 0;

static inline void stack_cache_acquire(void) {
    while (atomic_exchange_explicit(&g_stack_cache_lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}

static inline void stack_cache_release(void) {
    atomic_store_explicit(&g_stack_cache_lock, 0, memory_order_release);
}

static void *stack_cache_pop(size_t size) {
    stack_cache_acquire();
    int count = atomic_load_explicit(&g_stack_cache_count, memory_order_relaxed);
    for (int i = count - 1; i >= 0; i--) {
        if (g_stack_cache[i].size == size) {
            void *base = g_stack_cache[i].base;
            /* Swap with last element */
            g_stack_cache[i] = g_stack_cache[count - 1];
            atomic_store_explicit(&g_stack_cache_count, count - 1, memory_order_relaxed);
            stack_cache_release();
            return base;
        }
    }
    stack_cache_release();
    return NULL;
}

static int stack_cache_push(void *base, size_t size) {
    stack_cache_acquire();
    int count = atomic_load_explicit(&g_stack_cache_count, memory_order_relaxed);
    if (count >= STACK_CACHE_MAX) {
        stack_cache_release();
        return 0; /* cache full */
    }
    g_stack_cache[count].base = base;
    g_stack_cache[count].size = size;
    atomic_store_explicit(&g_stack_cache_count, count + 1, memory_order_relaxed);
    stack_cache_release();
    return 1;
}

/* Forward declarations */
static void jade_coro_trampoline(void);
static void jade_coro_exit(void);

jade_coro_t *jade_coro_create(void (*entry)(void*), void *arg) {
    jade_coro_t *c = (jade_coro_t *)calloc(1, sizeof(jade_coro_t));
    if (!c) return NULL;

    size_t total = JADE_STACK_SIZE;

    /* Try to reuse a cached stack */
    void *base = stack_cache_pop(total);
    if (!base) {
        /* Allocate stack with mmap for guard page support */
        base = mmap(NULL, total, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (base == MAP_FAILED) {
            free(c);
            return NULL;
        }
        /* Guard page at the bottom (stack grows down) */
        mprotect(base, JADE_GUARD_SIZE, PROT_NONE);
    }

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
        /* Try to cache the stack for reuse */
        if (!stack_cache_push(c->stack_base, c->stack_size)) {
            munmap(c->stack_base, c->stack_size);
        }
    }
    free(c);
}

/*
 * Thread-local for generator coroutine (direct context-swap, no scheduler).
 * Set by jade_gen_resume before swapping to the generator.
 */
_Thread_local jade_coro_t *tl_gen_coro = NULL;

/*
 * Trampoline: first function called when a coroutine starts.
 * Reads entry and arg from callee-saved registers set during create.
 */
static void jade_coro_trampoline(void) {
    jade_coro_t *self;
    jade_coro_t *gen = tl_gen_coro;
    if (gen) {
        /* Generator coroutine — runs via direct context swap, no scheduler */
        tl_gen_coro = NULL;
        self = gen;
        self->entry(self->arg);
        /* Generator entry should not return (codegen emits jade_gen_suspend + unreachable).
         * If it somehow does, just spin forever to avoid stack corruption. */
        for (;;) {}
    }

    /* Scheduler-spawned coroutine */
    jade_worker_t *w = tl_worker;
    self = w ? w->current : NULL;
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
    w->held_chan_lock = NULL;
    w->last_action = SCHED_ACTION_DESTROY;
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
    w->held_chan_lock = NULL;
    w->last_action = SCHED_ACTION_REQUEUE;
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

/* ── Generator direct context-swap API ─────────────────────────── */

/*
 * Generator control block layout (32 bytes):
 *   offset  0: coro_ptr       (*jade_coro_t)        — 8 bytes
 *   offset  8: value          (i64)                  — 8 bytes
 *   offset 16: has_value      (u8)                   — 1 byte
 *   offset 17: done           (u8)                   — 1 byte
 *   offset 24: caller_ctx_ptr (*jade_context_t)      — 8 bytes
 */
#define GEN_CORO_OFF       0
#define GEN_CALLER_CTX_OFF 24
#define GEN_DONE_OFF       17

/*
 * jade_gen_resume: Direct context swap from caller to generator.
 * Saves caller context on caller's stack, stores its pointer in the gen block,
 * and swaps to the generator coroutine.  Returns when the generator yields
 * or finishes.
 */
void jade_gen_resume(void *gen_blk) {
    /* Don't resume a finished generator */
    uint8_t done = *((uint8_t *)gen_blk + GEN_DONE_OFF);
    if (done) return;

    jade_coro_t *c = *(jade_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    jade_context_t caller_ctx;
    /* Store pointer to our stack-local context into the gen block */
    *(jade_context_t **)((char *)gen_blk + GEN_CALLER_CTX_OFF) = &caller_ctx;
    tl_gen_coro = c;
    jade_context_swap(&caller_ctx, &c->ctx);
    /* Returned here: generator has yielded or finished */
}

/*
 * jade_gen_suspend: Direct context swap from generator back to caller.
 * Reads the caller_ctx_ptr from the gen block and swaps back.
 */
void jade_gen_suspend(void *gen_blk) {
    jade_coro_t *c = *(jade_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    jade_context_t *caller_ctx = *(jade_context_t **)((char *)gen_blk + GEN_CALLER_CTX_OFF);
    jade_context_swap(&c->ctx, caller_ctx);
    /* Returned here: caller called .next() again (jade_gen_resume) */
}

/*
 * jade_gen_destroy: Free a generator's coroutine and control block.
 */
void jade_gen_destroy(void *gen_blk) {
    jade_coro_t *c = *(jade_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    if (c) jade_coro_destroy(c);
    free(gen_blk);
}
