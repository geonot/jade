/*
 * Jade Runtime — Chase-Lev work-stealing deque.
 *
 * Owning worker pushes/pops from bottom (LIFO — cache-friendly).
 * Thieves steal from top (FIFO — coarse-grained).
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>

void jade_deque_init(jade_deque_t *dq) {
    dq->capacity = JADE_DEQUE_INIT_CAP;
    dq->buffer = (jade_coro_t **)calloc((size_t)dq->capacity, sizeof(jade_coro_t *));
    atomic_store_explicit(&dq->top, 0, memory_order_relaxed);
    atomic_store_explicit(&dq->bottom, 0, memory_order_relaxed);
}

void jade_deque_destroy(jade_deque_t *dq) {
    free(dq->buffer);
    dq->buffer = NULL;
}

static void jade_deque_grow(jade_deque_t *dq) {
    int64_t old_cap = dq->capacity;
    int64_t new_cap = old_cap * 2;
    jade_coro_t **new_buf = (jade_coro_t **)calloc((size_t)new_cap, sizeof(jade_coro_t *));
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed);
    for (int64_t i = t; i < b; i++) {
        new_buf[i & (new_cap - 1)] = dq->buffer[i & (old_cap - 1)];
    }
    free(dq->buffer);
    dq->buffer = new_buf;
    dq->capacity = new_cap;
}

void jade_deque_push(jade_deque_t *dq, jade_coro_t *c) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    if (b - t >= dq->capacity) {
        jade_deque_grow(dq);
    }
    dq->buffer[b & (dq->capacity - 1)] = c;
    atomic_thread_fence(memory_order_release);
    atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
}

jade_coro_t *jade_deque_pop(jade_deque_t *dq) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed) - 1;
    atomic_store_explicit(&dq->bottom, b, memory_order_relaxed);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);

    if (t <= b) {
        jade_coro_t *c = dq->buffer[b & (dq->capacity - 1)];
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

jade_coro_t *jade_deque_steal(jade_deque_t *dq) {
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_acquire);

    if (t < b) {
        jade_coro_t *c = dq->buffer[t & (dq->capacity - 1)];
        if (atomic_compare_exchange_strong_explicit(
                &dq->top, &t, t + 1,
                memory_order_seq_cst, memory_order_relaxed)) {
            return c;
        }
    }
    return NULL;  /* empty or contended */
}
