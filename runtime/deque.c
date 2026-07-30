/*
 * Jinn Runtime — Chase-Lev work-stealing deque.
 *
 * Owning worker pushes/pops from bottom (LIFO — cache-friendly).
 * Thieves steal from top (FIFO — coarse-grained).
 *
 * Memory orderings follow Lê/Pop/Cohen/Nardelli, "Correct and Efficient
 * Work-Stealing for Weak Memory Models" (PPoPP 2013), in the same
 * fence-based style the file always used — the orderings were correct
 * before; the buffer *lifecycle* was not (task 8-14):
 *
 *   - grow() used to free() the old buffer inline while a thief could
 *     still be reading buffer[t & (capacity-1)] — a use-after-free — and
 *     published buffer and capacity as two separate non-atomic stores — a
 *     torn pair. Buffers are now immutable-once-retired objects carrying
 *     their own size, published by one atomic pointer, and reclaimed only
 *     at jinn_deque_destroy via the `prev` chain.
 *   - slots are now _Atomic: owner store vs. thief read is a formal data
 *     race on plain pointers.
 *   - allocation failure used to leave capacity = 0 (init) or silently
 *     overwrite a queued coroutine (grow full + OOM). Losing a queued
 *     coroutine is unrecoverable state corruption, so OOM aborts loudly,
 *     matching jinn_xmalloc's contract.
 */
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

/* Owner-only. Retires `old` (kept alive on the chain) and publishes the
 * doubled buffer. Returns the new buffer for the caller's store. */
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
    /* Release: a thief that acquires the new pointer sees the copied
     * slots. Thieves still holding the old pointer keep reading the old
     * buffer, which stays valid (retired, immutable, freed at destroy). */
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
    /* Owner's load: only the owner grows, so this is always the current
     * buffer from the owner's point of view. */
    jinn_deque_buf_t *buf = atomic_load_explicit(&dq->buf, memory_order_relaxed);
    atomic_store_explicit(&dq->bottom, b, memory_order_relaxed);
    atomic_thread_fence(memory_order_seq_cst);
    int64_t t = atomic_load_explicit(&dq->top, memory_order_relaxed);

    if (t <= b) {
        jinn_coro_t *c =
            atomic_load_explicit(&buf->slots[b & (buf->size - 1)], memory_order_relaxed);
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
        /* Acquire pairs with grow's release. Reading a buffer that is
         * retired a moment later is fine: the slot at index t was copied
         * into the successor and the retired buffer is never written
         * again nor freed before destroy, so the value read here is the
         * one the CAS below adjudicates. */
        jinn_deque_buf_t *buf = atomic_load_explicit(&dq->buf, memory_order_acquire);
        jinn_coro_t *c =
            atomic_load_explicit(&buf->slots[t & (buf->size - 1)], memory_order_relaxed);
        if (atomic_compare_exchange_strong_explicit(
                &dq->top, &t, t + 1,
                memory_order_seq_cst, memory_order_relaxed)) {
            return c;
        }
    }
    return NULL;  /* empty or contended */
}
