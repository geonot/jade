/*
 * Jinn Runtime — Typed channels (bounded MPMC ring buffer).
 *
 * Uses an atomic spinlock instead of pthread_mutex to avoid glibc 2.42+
 * __owner assertions when coroutines migrate between worker threads.
 */
#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <sched.h>
#include <unistd.h>
#include <stdio.h>

/* Debug: set to 1 to enable channel tracing */
#ifndef CHAN_DEBUG
#define CHAN_DEBUG 0
#endif
#define CHAN_TRACE(...) do { if (CHAN_DEBUG) fprintf(stderr, __VA_ARGS__); } while(0)

/* ── Spinlock helpers ────────────────────────────────────────────── */

static inline void chan_lock(jinn_chan_t *ch) {
    while (atomic_exchange_explicit(&ch->lock, 1, memory_order_acquire) != 0) {
        /* Spin with a pause hint for better performance under contention */
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}

static inline void chan_unlock(jinn_chan_t *ch) {
    atomic_store_explicit(&ch->lock, 0, memory_order_release);
}

/* Round up to next power of 2 */
static uint64_t next_pow2(uint64_t v) {
    if (v == 0) return 1;
    v--;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v |= v >> 32;
    return v + 1;
}

static inline void waitq_push(jinn_coro_t **head, jinn_coro_t **tail, jinn_coro_t *node) {
    node->next = NULL;
    if (*tail) {
        (*tail)->next = node;
    } else {
        *head = node;
    }
    *tail = node;
}

static inline jinn_coro_t *waitq_pop(jinn_coro_t **head, jinn_coro_t **tail) {
    jinn_coro_t *node = *head;
    if (!node) {
        return NULL;
    }
    *head = node->next;
    if (!*head) {
        *tail = NULL;
    }
    node->next = NULL;
    return node;
}

jinn_chan_t *jinn_chan_create(size_t elem_size, size_t capacity) {
    jinn_chan_t *ch = (jinn_chan_t *)calloc(1, sizeof(jinn_chan_t));
    if (!ch) return NULL;

    if (capacity == 0) capacity = 64;
    capacity = (size_t)next_pow2(capacity);

    ch->capacity  = capacity;
    ch->elem_size = elem_size;
    ch->buffer    = calloc(capacity, elem_size);
    atomic_store(&ch->head, 0);
    atomic_store(&ch->tail, 0);
    atomic_store(&ch->closed, 0);
    ch->send_waitq = NULL;
    ch->send_waitq_tail = NULL;
    ch->recv_waitq = NULL;
    ch->recv_waitq_tail = NULL;
    atomic_store(&ch->lock, 0);

    return ch;
}

void jinn_chan_destroy(jinn_chan_t *ch) {
    if (!ch) return;
    jinn_chan_close(ch);
    free(ch->buffer);
    free(ch);
}

void jinn_chan_close(jinn_chan_t *ch) {
    atomic_store(&ch->closed, 1);

    /* Wake all blocked receivers so they get the close signal */
    chan_lock(ch);
    jinn_coro_t *c = ch->recv_waitq;
    ch->recv_waitq = NULL;
    ch->recv_waitq_tail = NULL;
    /* Also wake senders */
    jinn_coro_t *s = ch->send_waitq;
    ch->send_waitq = NULL;
    ch->send_waitq_tail = NULL;
    chan_unlock(ch);

    while (c) {
        jinn_coro_t *next = c->next;
        c->next = NULL;
        c->wait_chan = NULL;
        c->state = JINN_CORO_READY;
        jinn_sched_enqueue(c);
        c = next;
    }
    while (s) {
        jinn_coro_t *next = s->next;
        s->next = NULL;
        s->wait_chan = NULL;
        s->state = JINN_CORO_READY;
        jinn_sched_enqueue(s);
        s = next;
    }
    /* Wake non-coroutine thread waiters */
}

void jinn_chan_send(jinn_chan_t *ch, const void *data) {
    for (;;) {
        chan_lock(ch);

        /* Check for close */
        if (atomic_load(&ch->closed)) {
            chan_unlock(ch);
            return;
        }

        uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
        uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);

        if (tail - head < ch->capacity) {
            /* Buffer has space — write element */
            size_t idx = tail & (ch->capacity - 1);
            memcpy((char *)ch->buffer + idx * ch->elem_size, data, ch->elem_size);
            atomic_store_explicit(&ch->tail, tail + 1, memory_order_release);

            /* Wake one blocked receiver if any */
            jinn_coro_t *waiter = waitq_pop(&ch->recv_waitq, &ch->recv_waitq_tail);
            if (waiter) {
                waiter->wait_chan = NULL;
                waiter->state = JINN_CORO_READY;
                chan_unlock(ch);
                jinn_sched_enqueue(waiter);
            } else {
                chan_unlock(ch);
            }
            return;
        }

        /* Buffer full — park this coroutine */
        jinn_worker_t *w = tl_worker;
        if (!w || !w->current) {
            /* Called from non-coroutine context — spin-wait then retry */
            chan_unlock(ch);
            CHAN_TRACE("send: full ch=%p (h=%lu t=%lu cap=%lu), backoff\n",
                       (void*)ch, (unsigned long)head, (unsigned long)tail, (unsigned long)ch->capacity);
            for (int _spin = 0; _spin < 128; _spin++) {
#if defined(__x86_64__)
                __builtin_ia32_pause();
#elif defined(__aarch64__)
                __asm__ volatile("yield");
#endif
            }
            continue;
        }

        jinn_coro_t *self = w->current;
        self->state = JINN_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->next = NULL;

        /* Append to send wait queue in O(1) */
        waitq_push(&ch->send_waitq, &ch->send_waitq_tail, self);

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler — will be resumed when a recv frees space */
        w->held_chan_lock = ch;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed here — retry send from the top */
    }
}

int jinn_chan_recv(jinn_chan_t *ch, void *data_out) {
    for (;;) {
        chan_lock(ch);

        uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
        uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);

        if (head < tail) {
            /* Buffer has data — read element */
            size_t idx = head & (ch->capacity - 1);
            memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
            atomic_store_explicit(&ch->head, head + 1, memory_order_release);

            /* Wake one blocked sender if any */
            jinn_coro_t *waiter = waitq_pop(&ch->send_waitq, &ch->send_waitq_tail);
            if (waiter) {
                waiter->wait_chan = NULL;
                waiter->state = JINN_CORO_READY;
                chan_unlock(ch);
                jinn_sched_enqueue(waiter);
            } else {
                chan_unlock(ch);
            }
            return 1;  /* success */
        }

        /* Buffer empty */
        if (atomic_load(&ch->closed)) {
            /* Channel closed, no more data coming */
            memset(data_out, 0, ch->elem_size);
            chan_unlock(ch);
            return 0;  /* closed */
        }

        /* Park this coroutine */
        jinn_worker_t *w = tl_worker;
        if (!w || !w->current) {
            /* Called from non-coroutine context — spin-wait then retry */
            chan_unlock(ch);
            for (int _spin = 0; _spin < 128; _spin++) {
#if defined(__x86_64__)
                __builtin_ia32_pause();
#elif defined(__aarch64__)
                __asm__ volatile("yield");
#endif
            }
            continue;
        }

        jinn_coro_t *self = w->current;
        self->state = JINN_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->next = NULL;

        /* Append to recv wait queue in O(1) */
        waitq_push(&ch->recv_waitq, &ch->recv_waitq_tail, self);

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler */
        w->held_chan_lock = ch;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — retry recv */
    }
}

int jinn_chan_try_recv(jinn_chan_t *ch, void *data_out) {
    chan_lock(ch);

    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);

    if (head < tail) {
        size_t idx = head & (ch->capacity - 1);
        memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
        atomic_store_explicit(&ch->head, head + 1, memory_order_release);

        jinn_coro_t *waiter = waitq_pop(&ch->send_waitq, &ch->send_waitq_tail);
        if (waiter) {
            waiter->wait_chan = NULL;
            waiter->state = JINN_CORO_READY;
            chan_unlock(ch);
            jinn_sched_enqueue(waiter);
        } else {
            chan_unlock(ch);
        }
        return 1;
    }

    if (atomic_load(&ch->closed)) {
        memset(data_out, 0, ch->elem_size);
        chan_unlock(ch);
        return -1;
    }

    chan_unlock(ch);
    return 0;
}
