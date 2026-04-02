/*
 * Jade Runtime — M:N work-stealing scheduler.
 *
 * N worker threads, each with a Chase-Lev deque.
 * Idle workers steal from others or park on a condvar.
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sched.h>

/* Global scheduler */
jade_sched_t g_sched;

/* Thread-local: current worker */
_Thread_local jade_worker_t *tl_worker = NULL;

/* ── RNG for steal target selection ─────────────────────────────── */

static uint32_t jade_xorshift(uint64_t *state) {
    uint64_t x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    return (uint32_t)(x & 0xFFFFFFFF);
}

/* ── Inject queue (global, for cross-worker spawns) ──────────────── */

/* Spinlock for inject queue — avoids futex overhead of pthread_mutex */
static _Atomic(int32_t) g_inject_lock = 0;

static inline void inject_lock_acquire(void) {
    while (atomic_exchange_explicit(&g_inject_lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}

static inline void inject_lock_release(void) {
    atomic_store_explicit(&g_inject_lock, 0, memory_order_release);
}

static void jade_inject_push(jade_coro_t *c) {
    inject_lock_acquire();
    c->next = NULL;
    if (g_sched.inject_tail) {
        g_sched.inject_tail->next = c;
    } else {
        g_sched.inject_head = c;
    }
    g_sched.inject_tail = c;
    inject_lock_release();
}

static jade_coro_t *jade_inject_pop(void) {
    inject_lock_acquire();
    jade_coro_t *c = g_sched.inject_head;
    if (c) {
        g_sched.inject_head = c->next;
        if (!g_sched.inject_head) {
            g_sched.inject_tail = NULL;
        }
        c->next = NULL;
    }
    inject_lock_release();
    return c;
}

/* ── Worker parking ─────────────────────────────────────────────── */

static void jade_worker_park(jade_worker_t *w) {
    /* Phase 1: brief spin — avoids costly futex for short idle periods */
    for (int spin = 0; spin < 40; spin++) {
        /* Check inject queue every 10 spins to avoid lock contention */
        if (spin % 10 == 0) {
            jade_coro_t *c = jade_inject_pop();
            if (c) {
                jade_deque_push(&w->run_queue, c);
                return;
            }
        }
        /* Try stealing once per spin */
        int n = g_sched.num_workers;
        if (n > 1) {
            uint32_t victim = jade_xorshift(&w->rng_state) % (uint32_t)n;
            if (victim != w->id) {
                jade_coro_t *c = jade_deque_steal(&g_sched.workers[victim].run_queue);
                if (c) {
                    jade_deque_push(&w->run_queue, c);
                    return;
                }
            }
        }
#if defined(__x86_64__)
        __builtin_ia32_pause();
        __builtin_ia32_pause();
        __builtin_ia32_pause();
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }

    /* Phase 2: park on condvar with timeout */
    atomic_fetch_add(&g_sched.idle_count, 1);
    pthread_mutex_lock(&g_sched.idle_lock);
    if (!atomic_load(&g_sched.shutdown)) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_nsec += 100000; /* 100μs timeout */
        if (ts.tv_nsec >= 1000000000) {
            ts.tv_sec += 1;
            ts.tv_nsec -= 1000000000;
        }
        pthread_cond_timedwait(&g_sched.idle_cond, &g_sched.idle_lock, &ts);
    }
    pthread_mutex_unlock(&g_sched.idle_lock);
    atomic_fetch_sub(&g_sched.idle_count, 1);
}

static void jade_sched_wake_one(void) {
    /* Only signal if there might be idle workers */
    if (atomic_load_explicit(&g_sched.idle_count, memory_order_relaxed) > 0) {
        pthread_mutex_lock(&g_sched.idle_lock);
        pthread_cond_signal(&g_sched.idle_cond);
        pthread_mutex_unlock(&g_sched.idle_lock);
    }
}

static void jade_sched_wake_all(void) {
    pthread_mutex_lock(&g_sched.idle_lock);
    pthread_cond_broadcast(&g_sched.idle_cond);
    pthread_mutex_unlock(&g_sched.idle_lock);
}

/* ── Finding work ───────────────────────────────────────────────── */

static jade_coro_t *jade_find_work(jade_worker_t *w) {
    /* 1. Pop from own deque (LIFO — hot cache) */
    jade_coro_t *c = jade_deque_pop(&w->run_queue);
    if (c) return c;

    /* 2. Check global inject queue */
    c = jade_inject_pop();
    if (c) return c;

    /* 3. Steal from a random other worker (FIFO) */
    int n = g_sched.num_workers;
    if (n <= 1) return NULL;
    uint32_t start = jade_xorshift(&w->rng_state) % (uint32_t)n;
    for (int i = 0; i < n; i++) {
        uint32_t victim = (start + (uint32_t)i) % (uint32_t)n;
        if (victim == w->id) continue;
        c = jade_deque_steal(&g_sched.workers[victim].run_queue);
        if (c) return c;
    }

    return NULL;
}

/* ── Worker loop ────────────────────────────────────────────────── */

static void *jade_worker_loop(void *arg) {
    jade_worker_t *w = (jade_worker_t *)arg;
    tl_worker = w;

    while (!atomic_load_explicit(&g_sched.shutdown, memory_order_acquire)) {
        jade_coro_t *c = jade_find_work(w);
        if (!c) {
            jade_worker_park(w);
            continue;
        }

        /* Run the coroutine */
        c->state = JADE_CORO_RUNNING;
        w->current = c;
        w->held_chan_lock = NULL;
        jade_context_swap(&w->sched_ctx, &c->ctx);
        /* Coroutine yielded or completed — back in scheduler */
        w->current = NULL;

        /*
         * Release any channel lock held across the context swap.
         * This ensures the coroutine's context is fully saved before
         * any waker can dequeue it from a wait queue.
         */
        if (w->held_chan_lock) {
            jade_chan_t *ch = (jade_chan_t *)w->held_chan_lock;
            atomic_store_explicit(&ch->lock, 0, memory_order_release);
            w->held_chan_lock = NULL;
        }

        if (w->last_action == SCHED_ACTION_DESTROY) {
            if (!c->daemon) {
                atomic_fetch_sub(&g_sched.active_coros, 1);
            }
            jade_coro_destroy(c);
        } else if (w->last_action == SCHED_ACTION_REQUEUE) {
            jade_deque_push(&w->run_queue, c);
        }
        /* PARK: coroutine is on a wait queue — don't touch it */
    }
    return NULL;
}

/* ── Public API ─────────────────────────────────────────────────── */

void jade_sched_init(int num_workers) {
    if (num_workers <= 0) {
        num_workers = (int)sysconf(_SC_NPROCESSORS_ONLN);
        if (num_workers <= 0) num_workers = 4;
        /* Cap at 8 for sanity in small workloads */
        if (num_workers > 8) num_workers = 8;
    }
    memset(&g_sched, 0, sizeof(g_sched));
    g_sched.num_workers = num_workers;
    g_sched.workers = (jade_worker_t *)calloc((size_t)num_workers, sizeof(jade_worker_t));
    atomic_store(&g_sched.active_coros, 0);
    atomic_store(&g_sched.shutdown, 0);
    atomic_store(&g_sched.idle_count, 0);
    atomic_store(&g_sched.started, 0);
    g_sched.inject_head = NULL;
    g_sched.inject_tail = NULL;
    atomic_store(&g_inject_lock, 0);
    pthread_mutex_init(&g_sched.idle_lock, NULL);
    pthread_cond_init(&g_sched.idle_cond, NULL);

    for (int i = 0; i < num_workers; i++) {
        g_sched.workers[i].id = (uint32_t)i;
        g_sched.workers[i].rng_state = (uint64_t)i + 1; /* nonzero seed */
        g_sched.workers[i].current = NULL;
        g_sched.workers[i].held_chan_lock = NULL;
        g_sched.workers[i].last_action = 0;
        jade_deque_init(&g_sched.workers[i].run_queue);
    }
}

static void jade_sched_start_workers(void) {
    int expected = 0;
    if (atomic_compare_exchange_strong(&g_sched.started, &expected, 1)) {
        for (int i = 0; i < g_sched.num_workers; i++) {
            pthread_create(&g_sched.workers[i].thread, NULL,
                           jade_worker_loop, &g_sched.workers[i]);
        }
    }
}

void jade_sched_spawn(jade_coro_t *c) {
    if (!c->daemon) {
        atomic_fetch_add(&g_sched.active_coros, 1);
    }

    /* Start worker threads lazily on first spawn */
    jade_sched_start_workers();

    /* Push onto local deque if on a worker, otherwise inject globally */
    jade_worker_t *w = tl_worker;
    if (w) {
        jade_deque_push(&w->run_queue, c);
    } else {
        jade_inject_push(c);
    }
    jade_sched_wake_one();
}

void jade_sched_enqueue(jade_coro_t *c) {
    jade_worker_t *w = tl_worker;
    if (w) {
        jade_deque_push(&w->run_queue, c);
    } else {
        jade_inject_push(c);
    }
    jade_sched_wake_one();
}

void jade_sched_run(void) {
    /* Block until all coroutines are done */
    while (atomic_load(&g_sched.active_coros) > 0) {
        /* If no workers started yet, nothing to wait for */
        if (!atomic_load(&g_sched.started)) break;
        usleep(100);  /* 100μs poll interval */
    }
}

void jade_sched_shutdown(void) {
    atomic_store(&g_sched.shutdown, 1);
    jade_sched_wake_all();

    if (atomic_load(&g_sched.started)) {
        for (int i = 0; i < g_sched.num_workers; i++) {
            pthread_join(g_sched.workers[i].thread, NULL);
            jade_deque_destroy(&g_sched.workers[i].run_queue);
        }
    }

    pthread_mutex_destroy(&g_sched.idle_lock);
    pthread_cond_destroy(&g_sched.idle_cond);
    free(g_sched.workers);
    g_sched.workers = NULL;
}

/* Aliases matching the codegen-declared symbols */
void jade_sched_yield(void) {
    jade_worker_t *w = tl_worker;
    if (w && w->current) {
        jade_coro_yield();
    } else {
        /* Called from main thread (not a coroutine) — brief sleep to avoid busy-spin */
        usleep(10);
    }
}

void jade_sched_park(void) {
    jade_worker_t *w = tl_worker;
    if (!w || !w->current) {
        /* Called from main thread — can't truly park, just brief sleep */
        usleep(10);
        return;
    }
    jade_coro_t *c = w->current;
    c->state = JADE_CORO_SUSPENDED;
    w->held_chan_lock = NULL;
    w->last_action = SCHED_ACTION_PARK;
    jade_context_swap(&c->ctx, &w->sched_ctx);
    /* Resumed here when unparked */
}

void jade_sched_unpark(jade_coro_t *c) {
    if (!c) return;
    c->state = JADE_CORO_READY;
    jade_sched_enqueue(c);
    jade_sched_wake_one();
}
