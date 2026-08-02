#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sched.h>
#include <time.h>
jinn_sched_t g_sched;
_Thread_local jinn_worker_t *tl_worker = NULL;
__attribute__((noinline)) jinn_worker_t *jinn_worker_self(void) {
    return tl_worker;
}

static uint32_t jinn_xorshift(uint64_t *state) {
    uint64_t x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    return (uint32_t)(x & 0xFFFFFFFF);
}

static _Atomic(int32_t) g_inject_lock = 0;
static inline void inject_lock_acquire(void) {
    int spins = 0;
    while (atomic_exchange_explicit(&g_inject_lock, 1, memory_order_acquire) != 0) {
        if (spins < 16) {
#if defined(__x86_64__)
            __builtin_ia32_pause();
#elif defined(__aarch64__)
            __asm__ volatile("yield");
#endif
        } else {
            sched_yield();
        }
        spins++;
    }
}
static inline void inject_lock_release(void) {
    atomic_store_explicit(&g_inject_lock, 0, memory_order_release);
}
static void jinn_inject_push(jinn_coro_t *c) {
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
static jinn_coro_t *jinn_inject_pop(void) {
    inject_lock_acquire();
    jinn_coro_t *c = g_sched.inject_head;
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
static void jinn_worker_park(jinn_worker_t *w) {
    for (int spin = 0; spin < 40; spin++) {
        if (spin % 10 == 0) {
            jinn_coro_t *c = jinn_inject_pop();
            if (c) {
                jinn_deque_push(&w->run_queue, c);
                return;
            }
        }
        int n = g_sched.num_workers;
        if (n > 1) {
            uint32_t victim = jinn_xorshift(&w->rng_state) % (uint32_t)n;
            if (victim != w->id) {
                jinn_coro_t *c = jinn_deque_steal(&g_sched.workers[victim].run_queue);
                if (c) {
                    jinn_deque_push(&w->run_queue, c);
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
    atomic_fetch_add(&g_sched.idle_count, 1);
    pthread_mutex_lock(&g_sched.idle_lock);
    if (!atomic_load(&g_sched.shutdown)) {
        struct timespec ts;
        timespec_get(&ts, TIME_UTC);
        ts.tv_nsec += 100000;
        if (ts.tv_nsec >= 1000000000) {
            ts.tv_sec += 1;
            ts.tv_nsec -= 1000000000;
        }
        pthread_cond_timedwait(&g_sched.idle_cond, &g_sched.idle_lock, &ts);
    }
    pthread_mutex_unlock(&g_sched.idle_lock);
    atomic_fetch_sub(&g_sched.idle_count, 1);
}
static void jinn_sched_wake_one(void) {
    if (atomic_load_explicit(&g_sched.idle_count, memory_order_relaxed) > 0) {
        pthread_mutex_lock(&g_sched.idle_lock);
        pthread_cond_signal(&g_sched.idle_cond);
        pthread_mutex_unlock(&g_sched.idle_lock);
    }
}
static void jinn_sched_wake_all(void) {
    pthread_mutex_lock(&g_sched.idle_lock);
    pthread_cond_broadcast(&g_sched.idle_cond);
    pthread_mutex_unlock(&g_sched.idle_lock);
}
static jinn_coro_t *jinn_find_work(jinn_worker_t *w) {

    jinn_coro_t *c = jinn_deque_pop(&w->run_queue);
    if (c) return c;
    c = jinn_inject_pop();
    if (c) return c;

    int n = g_sched.num_workers;
    if (n <= 1) return NULL;
    uint32_t start = jinn_xorshift(&w->rng_state) % (uint32_t)n;
    for (int i = 0; i < n; i++) {
        uint32_t victim = (start + (uint32_t)i) % (uint32_t)n;
        if (victim == w->id) continue;
        c = jinn_deque_steal(&g_sched.workers[victim].run_queue);
        if (c) return c;
    }
    return NULL;
}
static void *jinn_worker_loop(void *arg) {
    jinn_worker_t *w = (jinn_worker_t *)arg;
    tl_worker = w;
    jinn_install_worker_sigaltstack();

    while (!atomic_load_explicit(&g_sched.shutdown, memory_order_acquire)) {
        jinn_coro_t *c = jinn_find_work(w);
        if (!c) {
            jinn_worker_park(w);
            continue;
        }
        c->state = JINN_CORO_RUNNING;
        w->current = c;
        w->held_lock = NULL;
        w->held_locks = NULL;
        w->held_locks_n = 0;
        jinn_scope_set_current((jinn_scope_t *)c->scope);
        jinn_context_swap(&w->sched_ctx, &c->ctx);

        w->current = NULL;
        jinn_scope_set_current(NULL);
        if (w->held_lock) {
            atomic_store_explicit(w->held_lock, 0, memory_order_release);
            w->held_lock = NULL;
        }
        if (w->held_locks) {
            for (int li = w->held_locks_n - 1; li >= 0; li--) {
                atomic_store_explicit(w->held_locks[li], 0, memory_order_release);
            }
            w->held_locks = NULL;
            w->held_locks_n = 0;
        }
        if (w->last_action == SCHED_ACTION_DESTROY) {
            if (c->scope) {
                jinn_scope_unregister_child((jinn_scope_t *)c->scope, c);
                jinn_scope_child_done((jinn_scope_t *)c->scope);
            }
            if (!c->daemon) {
                int64_t remaining = atomic_fetch_sub(&g_sched.active_coros, 1) - 1;
                if (remaining <= 0) {
                    pthread_mutex_lock(&g_sched.done_lock);
                    pthread_cond_signal(&g_sched.done_cond);
                    pthread_mutex_unlock(&g_sched.done_lock);
                }
            }
            jinn_coro_destroy(c);
        } else if (w->last_action == SCHED_ACTION_REQUEUE) {
            jinn_deque_push(&w->run_queue, c);
        }
    }
    return NULL;
}
void jinn_sched_init(int num_workers) {
    jinn_install_crash_handlers();
    if (num_workers <= 0) {
        num_workers = (int)sysconf(_SC_NPROCESSORS_ONLN);
        if (num_workers <= 0) num_workers = 4;
        if (num_workers > 8) num_workers = 8;
    }
    memset(&g_sched, 0, sizeof(g_sched));
    g_sched.num_workers = num_workers;
    g_sched.workers = (jinn_worker_t *)calloc((size_t)num_workers, sizeof(jinn_worker_t));
    atomic_store(&g_sched.active_coros, 0);
    atomic_store(&g_sched.shutdown, 0);
    atomic_store(&g_sched.idle_count, 0);
    atomic_store(&g_sched.started, 0);
    g_sched.inject_head = NULL;
    g_sched.inject_tail = NULL;
    atomic_store(&g_inject_lock, 0);
    pthread_mutex_init(&g_sched.idle_lock, NULL);
    pthread_cond_init(&g_sched.idle_cond, NULL);
    pthread_mutex_init(&g_sched.done_lock, NULL);
    pthread_cond_init(&g_sched.done_cond, NULL);
    for (int i = 0; i < num_workers; i++) {
        g_sched.workers[i].id = (uint32_t)i;
        g_sched.workers[i].rng_state = (uint64_t)i + 1;
        g_sched.workers[i].current = NULL;
        g_sched.workers[i].held_lock = NULL;
        g_sched.workers[i].held_locks = NULL;
        g_sched.workers[i].held_locks_n = 0;
        g_sched.workers[i].last_action = 0;
        jinn_deque_init(&g_sched.workers[i].run_queue);
    }
}
static void jinn_sched_start_workers(void) {
    int expected = 0;
    if (atomic_compare_exchange_strong(&g_sched.started, &expected, 1)) {
        for (int i = 0; i < g_sched.num_workers; i++) {
            pthread_create(&g_sched.workers[i].thread, NULL,
                           jinn_worker_loop, &g_sched.workers[i]);
        }
    }
}
void jinn_sched_spawn(jinn_coro_t *c) {
    if (!c->daemon) {
        atomic_fetch_add(&g_sched.active_coros, 1);
    }
    jinn_sched_start_workers();
    jinn_worker_t *w = tl_worker;
    if (w) {
        jinn_deque_push(&w->run_queue, c);
    } else {
        jinn_inject_push(c);
    }
    jinn_sched_wake_one();
}
void jinn_sched_enqueue(jinn_coro_t *c) {
    jinn_worker_t *w = tl_worker;
    if (w) {
        jinn_deque_push(&w->run_queue, c);
    } else {
        jinn_inject_push(c);
    }
    jinn_sched_wake_one();
}

void jinn_sched_run(void) {
    if (!atomic_load(&g_sched.started)) { return; }
    pthread_mutex_lock(&g_sched.done_lock);
    while (atomic_load(&g_sched.active_coros) > 0) {
        pthread_cond_wait(&g_sched.done_cond, &g_sched.done_lock);
    }
    pthread_mutex_unlock(&g_sched.done_lock);
}

void jinn_sched_shutdown(void) {
    atomic_store(&g_sched.shutdown, 1);
    jinn_sched_wake_all();
    if (atomic_load(&g_sched.started)) {
        for (int i = 0; i < g_sched.num_workers; i++) {
            pthread_join(g_sched.workers[i].thread, NULL);
        }
        for (int i = 0; i < g_sched.num_workers; i++) {
            jinn_deque_destroy(&g_sched.workers[i].run_queue);
        }
        jinn_actor_retire_flush();
    }
    pthread_mutex_destroy(&g_sched.idle_lock);
    pthread_cond_destroy(&g_sched.idle_cond);
    pthread_mutex_destroy(&g_sched.done_lock);
    pthread_cond_destroy(&g_sched.done_cond);
    free(g_sched.workers);
    g_sched.workers = NULL;
}

void jinn_sched_yield(void) {
    jinn_worker_t *w = tl_worker;
    if (w && w->current) {
        jinn_coro_yield();
    } else {
        struct timespec ns = {0, 10000};
        nanosleep(&ns, NULL);
    }
}
void jinn_sched_park(void) {
    jinn_worker_t *w = tl_worker;
    if (!w || !w->current) {
        struct timespec ns = {0, 10000};
        nanosleep(&ns, NULL);
        return;
    }
    jinn_coro_t *c = w->current;
    c->state = JINN_CORO_SUSPENDED;
    w->held_lock = NULL;
    w->last_action = SCHED_ACTION_PARK;
    jinn_context_swap(&c->ctx, &w->sched_ctx);
}
void jinn_sched_unpark(jinn_coro_t *c) {
    if (!c) return;
    c->state = JINN_CORO_READY;
    jinn_sched_enqueue(c);
    jinn_sched_wake_one();
}
