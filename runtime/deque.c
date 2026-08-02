#include "jinn_rt.h"
#include <stdio.h>
#include <stdlib.h>
static jinn_deque_buf_t *deque_buf_alloc(int64_t size, jinn_deque_buf_t *prev) {
    jinn_deque_buf_t *b = (jinn_deque_buf_t *)calloc(
        1, sizeof(jinn_deque_buf_t) + (size_t)size * sizeof(_Atomic(jinn_coro_t *)));
    if (!b) {
        fprintf(stderr, "jinn: out of memory (scheduler run queue, %lld slots)\n",
                (long long)size);
        abort();
    }
    b->size = size;
    b->prev = prev;
    return b;
}
void jinn_deque_init(jinn_deque_t *dq) {
    atomic_store_explicit(&dq->buf, deque_buf_alloc(JINN_DEQUE_INIT_CAP, NULL),
                          memory_order_relaxed);
    atomic_store_explicit(&dq->top, 0, memory_order_relaxed);
    atomic_store_explicit(&dq->bottom, 0, memory_order_relaxed);
}
void jinn_deque_destroy(jinn_deque_t *dq) {
    jinn_deque_buf_t *b = atomic_load_explicit(&dq->buf, memory_order_relaxed);
    atomic_store_explicit(&dq->buf, NULL, memory_order_relaxed);
    while (b) {
        jinn_deque_buf_t *prev = b->prev;
        free(b);
        b = prev;
    }
}
static jinn_deque_buf_t *deque_grow(jinn_deque_t *dq, jinn_deque_buf_t *old,
                                    int64_t t, int64_t b) {
    jinn_deque_buf_t *nb = deque_buf_alloc(old->size * 2, old);
    for (int64_t i = t; i < b; i++) {
        atomic_store_explicit(
            &nb->slots[i & (nb->size - 1)],
            atomic_load_explicit(&old->slots[i & (old->size - 1)],
                                 memory_order_relaxed),
            memory_order_relaxed);
    }
    atomic_store_explicit(&dq->buf, nb, memory_order_release);
    return nb;
}
void jinn_deque_push(jinn_deque_t *dq, jinn_coro_t *c) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    jinn_deque_buf_t *buf = atomic_load_explicit(&dq->buf, memory_order_relaxed);
    if (b - t >= buf->size) {
        buf = deque_grow(dq, buf, t, b);
    }
    atomic_store_explicit(&buf->slots[b & (buf->size - 1)], c, memory_order_relaxed);
    atomic_thread_fence(memory_order_release);
    atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
}
jinn_coro_t *jinn_deque_pop(jinn_deque_t *dq) {
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_relaxed) - 1;
    jinn_deque_buf_t *buf = atomic_load_explicit(&dq->buf, memory_order_relaxed);
    atomic_store_explicit(&dq->bottom, b, memory_order_relaxed);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);
    if (t <= b) {
        jinn_coro_t *c =
            atomic_load_explicit(&buf->slots[b & (buf->size - 1)], memory_order_relaxed);
        if (t == b) {
            if (!atomic_compare_exchange_strong_explicit(
                    &dq->top, &t, t + 1,
                    memory_order_seq_cst, memory_order_relaxed)) {
                c = NULL;
            }
            atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
        }
        return c;
    }
    atomic_store_explicit(&dq->bottom, b + 1, memory_order_relaxed);
    return NULL;
}
jinn_coro_t *jinn_deque_steal(jinn_deque_t *dq) {
    int64_t t = atomic_load_explicit(&dq->top, memory_order_acquire);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t b = atomic_load_explicit(&dq->bottom, memory_order_acquire);
    if (t < b) {
        jinn_deque_buf_t *buf = atomic_load_explicit(&dq->buf, memory_order_acquire);
        jinn_coro_t *c =
            atomic_load_explicit(&buf->slots[t & (buf->size - 1)], memory_order_relaxed);
        if (atomic_compare_exchange_strong_explicit(
                &dq->top, &t, t + 1,
                memory_order_seq_cst, memory_order_relaxed)) {
            return c;
        }
    }
    return NULL;
}
