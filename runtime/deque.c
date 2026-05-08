/*
 * Jinn Runtime — Chase-Lev work-stealing deque.
 *
 * Owning worker pushes/pops from bottom (LIFO — cache-friendly).
 * Thieves steal from top (FIFO — coarse-grained).
 */
#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>

void jinn_deque_init(jinn_deque_t *dq) {
    dq->capacity = JINN_DEQUE_INIT_CAP;
    dq->buffer = (jinn_coro_t **)calloc((size_t)dq->capacity, sizeof(jinn_coro_t *));
    if (!dq->buffer) { dq->capacity = 0; return; }
    atomic_store_explicit(&dq->top, 0, memory_order_relaxed);
    atomic_store_explicit(&dq->bottom, 0, memory_order_relaxed);
}

void jinn_deque_destroy(jinn_deque_t *dq) {
    free(dq->buffer);
    dq->buffer = NULL;
}

static void jinn_deque_grow(jinn_deque_t *dq) {
    int64_t old_cap = dq->capacity;
    int64_t new_cap = old_cap * 2;
    jinn_coro_t **new_buf = (jinn_coro_t **)calloc((size_t)new_cap, sizeof(jinn_coro_t *));
    if (!new_buf) return;
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed);
    for (int64_t i = t; i < b; i++) {
        new_buf[i & (new_cap - 1)] = dq->buffer[i & (old_cap - 1)];
    }
    free(dq->buffer);
    dq->buffer = new_buf;
    dq->capacity = new_cap;
}

void jinn_deque_push(jinn_deque_t *dq, jinn_coro_t *c) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    if (b - t >= dq->capacity) {
        jinn_deque_grow(dq);
    }
    dq->buffer[b & (dq->capacity - 1)] = c;
    atomic_thread_fence(memory_order_release);
    atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
}

jinn_coro_t *jinn_deque_pop(jinn_deque_t *dq) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed) - 1;
    atomic_store_explicit(&dq->bottom, b, memory_order_relaxed);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);

    if (t <= b) {
        jinn_coro_t *c = dq->buffer[b & (dq->capacity - 1)];
        if (t == b) {
            /* Last element — race with stealers */
            if (!atomic_compare_exchange_strong_explicit(
                    &dq->top, &t, t + 1,
                    memory_order_seq_cst, memory_order_relaxed)) {
                c = NULL;  /* lost the race */
            }
            atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
        }
        return c;
    }
    /* Empty */
    atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
    return NULL;
}

jinn_coro_t *jinn_deque_steal(jinn_deque_t *dq) {
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_acquire);

    if (t < b) {
        jinn_coro_t *c = dq->buffer[t & (dq->capacity - 1)];
        if (atomic_compare_exchange_strong_explicit(
                &dq->top, &t, t + 1,
                memory_order_seq_cst, memory_order_relaxed)) {
            return c;
        }
    }
    return NULL;  /* empty or contended */
}
