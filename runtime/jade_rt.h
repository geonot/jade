/*
 * Jade Runtime — Concurrency primitives for the Jade language.
 *
 * Stackful coroutines, typed channels, M:N work-stealing scheduler,
 * actor support, select, timers.
 *
 * All functions prefixed with jade_ to avoid symbol collisions.
 */
#pragma once

#include <stdint.h>
#include <stddef.h>
#include <stdatomic.h>
#include <pthread.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ── Forward declarations ────────────────────────────────────────── */

typedef struct jade_coro    jade_coro_t;
typedef struct jade_sched   jade_sched_t;
typedef struct jade_chan     jade_chan_t;
typedef struct jade_worker  jade_worker_t;
typedef struct jade_deque   jade_deque_t;
typedef struct jade_timer   jade_timer_t;

/* ── Context (platform-specific) ─────────────────────────────────── */

#if defined(__x86_64__) || defined(_M_X64)
typedef struct {
    void *rsp;
    void *rbp;
    void *rbx;
    void *r12;
    void *r13;
    void *r14;
    void *r15;
} jade_context_t;  /* 56 bytes */
#elif defined(__aarch64__) || defined(_M_ARM64)
typedef struct {
    void *sp;
    void *lr;
    void *fp;
    void *x19_x28[10];
    double d8_d15[8];
} jade_context_t;  /* 168 bytes */
#else
#include <setjmp.h>
typedef struct {
    jmp_buf env;
} jade_context_t;
#endif

/* ── Coroutine ───────────────────────────────────────────────────── */

typedef enum {
    JADE_CORO_READY,
    JADE_CORO_RUNNING,
    JADE_CORO_SUSPENDED,
    JADE_CORO_DONE
} jade_coro_state_t;

struct jade_coro {
    jade_context_t     ctx;
    void              *stack_base;
    uint32_t           stack_size;
    jade_coro_state_t  state;
    void             (*entry)(void*);
    void              *arg;
    jade_coro_t       *next;          /* intrusive list for wait queues */
    void              *wait_chan;      /* channel blocked on, or NULL */
    uint32_t           id;
    int                select_ready;  /* which select case fired (-1 = none) */
    uint8_t            daemon;        /* 1 = daemon coro (actor), doesn't block sched_run */
};

#define JADE_STACK_SIZE  (64 * 1024)   /* 64KB per coroutine */
#define JADE_GUARD_SIZE  4096          /* 1 page guard */

jade_coro_t *jade_coro_create(void (*entry)(void*), void *arg);
void         jade_coro_destroy(jade_coro_t *c);
void         jade_coro_yield(void);
void         jade_coro_set_daemon(jade_coro_t *c);

/* ── Generator direct context-swap API ───────────────────────────── */

void jade_gen_resume(void *gen_blk);
void jade_gen_suspend(void *gen_blk);
void jade_gen_destroy(void *gen_blk);

extern _Thread_local jade_coro_t *tl_gen_coro;

/* ── Context switch (defined in assembly or fallback) ────────────── */

void jade_context_swap(jade_context_t *from, jade_context_t *to);

/* ── Work-stealing deque ─────────────────────────────────────────── */

#define JADE_DEQUE_INIT_CAP 1024

struct jade_deque {
    jade_coro_t       **buffer;
    _Atomic(int64_t)    top;
    _Atomic(int64_t)    bottom;
    int64_t             capacity;
};

void         jade_deque_init(jade_deque_t *dq);
void         jade_deque_destroy(jade_deque_t *dq);
void         jade_deque_push(jade_deque_t *dq, jade_coro_t *c);
jade_coro_t *jade_deque_pop(jade_deque_t *dq);
jade_coro_t *jade_deque_steal(jade_deque_t *dq);

/* ── Scheduler ───────────────────────────────────────────────────── */

/* Scheduler actions communicated from coroutine to scheduler across swap */
#define SCHED_ACTION_PARK    0  /* parked on wait queue — don't touch coroutine */
#define SCHED_ACTION_REQUEUE 1  /* voluntary yield — re-enqueue */
#define SCHED_ACTION_DESTROY 2  /* coroutine exited — destroy it */

struct jade_worker {
    pthread_t          thread;
    uint32_t           id;
    jade_deque_t       run_queue;
    jade_coro_t       *current;
    jade_context_t     sched_ctx;
    uint64_t           rng_state;
    void              *held_chan_lock;  /* channel lock held across context swap */
    int                last_action;     /* SCHED_ACTION_* set before swap */
};

struct jade_sched {
    jade_worker_t     *workers;
    int                num_workers;
    _Atomic(int64_t)   active_coros;
    _Atomic(int32_t)   shutdown;
    /* Global inject queue */
    jade_coro_t       *inject_head;
    jade_coro_t       *inject_tail;
    /* Idle parking */
    pthread_mutex_t    idle_lock;
    pthread_cond_t     idle_cond;
    _Atomic(int32_t)   idle_count;
    /* Started flag */
    _Atomic(int32_t)   started;
};

void jade_sched_init(int num_workers);
void jade_sched_spawn(jade_coro_t *c);
void jade_sched_run(void);
void jade_sched_shutdown(void);
void jade_sched_enqueue(jade_coro_t *c);
void jade_sched_yield(void);
void jade_sched_park(void);
void jade_sched_unpark(jade_coro_t *c);

/* Get current coroutine (thread-local) */
jade_coro_t  *jade_current_coro(void);
jade_worker_t *jade_current_worker(void);

/* ── Channels ────────────────────────────────────────────────────── */

struct jade_chan {
    _Atomic(uint64_t)  head;
    _Atomic(uint64_t)  tail;
    uint64_t           capacity;
    size_t             elem_size;
    void              *buffer;
    _Atomic(int32_t)   closed;
    jade_coro_t       *send_waitq;
    jade_coro_t       *recv_waitq;
    _Atomic(int32_t)   lock;         /* spinlock (no thread-ownership tracking) */
};

jade_chan_t *jade_chan_create(size_t elem_size, size_t capacity);
void        jade_chan_send(jade_chan_t *ch, const void *data);
int         jade_chan_recv(jade_chan_t *ch, void *data_out);
void        jade_chan_close(jade_chan_t *ch);
void        jade_chan_destroy(jade_chan_t *ch);

/* ── Select ──────────────────────────────────────────────────────── */

typedef struct jade_select_case {
    jade_chan_t   *chan;
    void          *data;
    int            is_send;
} jade_select_case_t;

int jade_select(jade_select_case_t *cases, int n, int has_default);

/* ── Timers ──────────────────────────────────────────────────────── */

struct jade_timer {
    uint64_t deadline_ns;
    int      fired;
};

void     jade_timer_set(jade_timer_t *t, uint64_t deadline_ns);
int      jade_timer_check(jade_timer_t *t);
uint64_t jade_time_now_ns(void);

/* ── Pool allocator ──────────────────────────────────────────────── */

typedef struct jade_pool jade_pool_t;

jade_pool_t *jade_pool_create(size_t obj_size, size_t count);
void        *jade_pool_alloc(jade_pool_t *pool);
void         jade_pool_free(jade_pool_t *pool, void *ptr);
void         jade_pool_reset(jade_pool_t *pool);
void         jade_pool_destroy(jade_pool_t *pool);
size_t       jade_pool_count(jade_pool_t *pool);
size_t       jade_pool_capacity(jade_pool_t *pool);

/* ── Actor helpers ───────────────────────────────────────────────── */

void jade_actor_park(void *mailbox_ptr);
void jade_actor_wake(void *mailbox_ptr);

/* ── Global scheduler instance ───────────────────────────────────── */

extern jade_sched_t g_sched;
extern _Thread_local jade_worker_t *tl_worker;

#ifdef __cplusplus
}
#endif
