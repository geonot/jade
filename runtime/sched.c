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

static void jade_inject_push(jade_coro_t *c) {
    pthread_mutex_lock(&g_sched.inject_lock);
    c->next = NULL;
    if (g_sched.inject_tail) {
        g_sched.inject_tail->next = c;
    } else {
        g_sched.inject_head = c;
    }
    g_sched.inject_tail = c;
    pthread_mutex_unlock(&g_sched.inject_lock);
}

static jade_coro_t *jade_inject_pop(void) {
    pthread_mutex_lock(&g_sched.inject_lock);
    jade_coro_t *c = g_sched.inject_head;
    if (c) {
        g_sched.inject_head = c->next;
        if (!g_sched.inject_head) {
            g_sched.inject_tail = NULL;
        }
        c->next = NULL;
    }
    pthread_mutex_unlock(&g_sched.inject_lock);
    return c;
}

/* ── Worker parking ─────────────────────────────────────────────── */

static void jade_worker_park(jade_worker_t *w) {
    (void)w;
    atomic_fetch_add(&g_sched.idle_count, 1);
    pthread_mutex_lock(&g_sched.idle_lock);
    /* Double-check before sleeping */
    if (!atomic_load(&g_sched.shutdown)) {
        struct timespec ts;
        clock_gettime(CLOCK_REALTIME, &ts);
        ts.tv_nsec += 1000000; /* 1ms timeout to check timers */
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
    if (atomic_load(&g_sched.idle_count) > 0) {
        pthread_cond_signal(&g_sched.idle_cond);
    }
}

static void jade_sched_wake_all(void) {
    pthread_cond_broadcast(&g_sched.idle_cond);
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
        jade_context_swap(&w->sched_ctx, &c->ctx);
        /* Coroutine yielded or completed — back in scheduler */
        w->current = NULL;

        if (c->state == JADE_CORO_DONE) {
            if (!c->daemon) {
                atomic_fetch_sub(&g_sched.active_coros, 1);
            }
            jade_coro_destroy(c);
        } else if (c->state == JADE_CORO_SUSPENDED) {
            /* Parked on channel/actor wait queue; don't re-enqueue */
        } else {
            /* READY = yielded voluntarily, put back on run queue */
            jade_deque_push(&w->run_queue, c);
        }
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
    pthread_mutex_init(&g_sched.inject_lock, NULL);
    pthread_mutex_init(&g_sched.idle_lock, NULL);
    pthread_cond_init(&g_sched.idle_cond, NULL);

    for (int i = 0; i < num_workers; i++) {
        g_sched.workers[i].id = (uint32_t)i;
        g_sched.workers[i].rng_state = (uint64_t)i + 1; /* nonzero seed */
        g_sched.workers[i].current = NULL;
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

    pthread_mutex_destroy(&g_sched.inject_lock);
    pthread_mutex_destroy(&g_sched.idle_lock);
    pthread_cond_destroy(&g_sched.idle_cond);
    free(g_sched.workers);
    g_sched.workers = NULL;
}
