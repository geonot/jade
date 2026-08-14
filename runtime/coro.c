#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
static _Atomic(uint32_t) g_coro_id_counter = 0;


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
        return 0;
    }
    g_stack_cache[count].base = base;
    g_stack_cache[count].size = size;
    atomic_store_explicit(&g_stack_cache_count, count + 1, memory_order_relaxed);
    stack_cache_release();
    return 1;
}
static void jinn_coro_trampoline(void);
static void jinn_coro_exit(void);
jinn_coro_t *jinn_coro_create(void (*entry)(void*), void *arg) {
    jinn_coro_t *c = (jinn_coro_t *)calloc(1, sizeof(jinn_coro_t));
    if (!c) return NULL;
    size_t total = JINN_STACK_SIZE;

    void *base = stack_cache_pop(total);
    if (!base) {
        base = mmap(NULL, total, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (base == MAP_FAILED) {
            free(c);
            return NULL;
        }
        mprotect(base, JINN_GUARD_SIZE, PROT_NONE);
    }
    c->stack_base  = base;
    c->stack_size  = (uint32_t)total;
    c->entry       = entry;
    c->arg         = arg;
    c->state       = JINN_CORO_READY;
    c->id          = atomic_fetch_add(&g_coro_id_counter, 1);
    c->next        = NULL;
    c->wait_chan   = NULL;
    c->daemon      = 0;
    c->on_exit_cb  = NULL;
    c->on_exit_arg = NULL;
    c->scope       = NULL;
    c->txn_state   = NULL;
#ifdef JINN_SAN_TSAN_FIBERS
    c->san_fiber = __tsan_create_fiber(0);
#endif
    atomic_store(&c->cancelled, 0);
    uintptr_t stack_top = (uintptr_t)base + total;
    stack_top &= ~(uintptr_t)15;
    stack_top -= 8;
    *(void **)stack_top = (void *)jinn_coro_exit;
    stack_top -= 8;
    *(void **)stack_top = (void *)jinn_coro_trampoline;
#if defined(__x86_64__) || defined(_M_X64)
    c->ctx.rsp = (void *)stack_top;
    c->ctx.rbp = (void *)stack_top;
    c->ctx.r12 = (void *)entry;
    c->ctx.r13 = arg;
#elif defined(__aarch64__) || defined(_M_ARM64)
    c->ctx.sp  = (void *)stack_top;
    c->ctx.fp  = (void *)stack_top;
    c->ctx.lr  = (void *)jinn_coro_trampoline;
    c->ctx.x19_x28[0] = (void *)entry;
    c->ctx.x19_x28[1] = arg;
#else
    (void)stack_top;
#endif
    return c;
}
void jinn_coro_destroy(jinn_coro_t *c) {
    if (!c) return;
#ifdef JINN_SAN_TSAN_FIBERS
    if (c->san_fiber) __tsan_destroy_fiber(c->san_fiber);
#endif
    if (c->stack_base) {
        if (!stack_cache_push(c->stack_base, c->stack_size)) {
            munmap(c->stack_base, c->stack_size);
        }
    }
    free(c);
}

_Thread_local jinn_coro_t *tl_gen_coro = NULL;
static void jinn_coro_trampoline(void) {
#ifdef JINN_SAN_ASAN_FIBERS
    const void *san_old_bottom = NULL;
    size_t san_old_size = 0;
    __sanitizer_finish_switch_fiber(NULL, &san_old_bottom, &san_old_size);
#endif
    jinn_coro_t *self;
    jinn_coro_t *gen = tl_gen_coro;
    if (gen) {
        tl_gen_coro = NULL;
        self = gen;
#ifdef JINN_SAN_ASAN_FIBERS
        self->san_ret_bottom = san_old_bottom;
        self->san_ret_size = san_old_size;
#endif
        self->entry(self->arg);
        for (;;) {}
    }

    jinn_worker_t *w = tl_worker;
    self = w ? w->current : NULL;
    if (!self) return;
#ifdef JINN_SAN_ASAN_FIBERS
    self->san_ret_bottom = san_old_bottom;
    self->san_ret_size = san_old_size;
#endif
    self->entry(self->arg);
    jinn_coro_exit();
}
static void jinn_coro_exit(void) {
    jinn_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jinn_coro_t *self = w->current;
    if (self->on_exit_cb) {
        void (*cb)(void *) = self->on_exit_cb;
        void *arg = self->on_exit_arg;
        self->on_exit_cb = NULL;
        cb(arg);
    }
    self->state = JINN_CORO_DONE;
    w->held_lock = NULL;
    w->last_action = SCHED_ACTION_DESTROY;
    jinn_coro_swap_out_final(self, &w->sched_ctx);
    __builtin_unreachable();
}
void jinn_coro_yield(void) {
    jinn_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jinn_coro_t *c = w->current;
    c->state = JINN_CORO_READY;
    w->held_lock = NULL;
    w->last_action = SCHED_ACTION_REQUEUE;
    jinn_coro_swap_out(c, &w->sched_ctx);
}
jinn_coro_t *jinn_current_coro(void) {
    jinn_worker_t *w = tl_worker;
    return w ? w->current : NULL;
}
jinn_worker_t *jinn_current_worker(void) {
    return tl_worker;
}
void jinn_coro_set_daemon(jinn_coro_t *c) {
    if (c) c->daemon = 1;
}
void jinn_coro_set_on_exit(jinn_coro_t *c, void (*cb)(void *), void *arg) {
    if (!c) return;
    c->on_exit_cb = cb;
    c->on_exit_arg = arg;
}

#define GEN_CORO_OFF       0
#define GEN_CALLER_CTX_OFF 24
#define GEN_DONE_OFF       17

void jinn_gen_resume(void *gen_blk) {
    uint8_t done = *((uint8_t *)gen_blk + GEN_DONE_OFF);
    if (done) return;
    jinn_coro_t *c = *(jinn_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    jinn_context_t caller_ctx;
    *(jinn_context_t **)((char *)gen_blk + GEN_CALLER_CTX_OFF) = &caller_ctx;
    tl_gen_coro = c;
    jinn_coro_swap_in(&caller_ctx, c);
    tl_gen_coro = NULL;
}

void jinn_gen_suspend(void *gen_blk) {
    jinn_coro_t *c = *(jinn_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    jinn_context_t *caller_ctx = *(jinn_context_t **)((char *)gen_blk + GEN_CALLER_CTX_OFF);
    jinn_coro_swap_out(c, caller_ctx);
}
void jinn_gen_destroy(void *gen_blk) {
    jinn_coro_t *c = *(jinn_coro_t **)((char *)gen_blk + GEN_CORO_OFF);
    if (c) jinn_coro_destroy(c);
    free(gen_blk);
}
