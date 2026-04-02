/*
 * Jade Runtime — Typed channels (bounded MPMC ring buffer).
 *
 * Uses an atomic spinlock instead of pthread_mutex to avoid glibc 2.42+
 * __owner assertions when coroutines migrate between worker threads.
 */
#include "jade_rt.h"
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

static inline void chan_lock(jade_chan_t *ch) {
    while (atomic_exchange_explicit(&ch->lock, 1, memory_order_acquire) != 0) {
        /* Spin with a pause hint for better performance under contention */
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}

static inline void chan_unlock(jade_chan_t *ch) {
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

jade_chan_t *jade_chan_create(size_t elem_size, size_t capacity) {
    jade_chan_t *ch = (jade_chan_t *)calloc(1, sizeof(jade_chan_t));
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
    ch->recv_waitq = NULL;
    atomic_store(&ch->lock, 0);

    return ch;
}

void jade_chan_destroy(jade_chan_t *ch) {
    if (!ch) return;
    free(ch->buffer);
    free(ch);
}

void jade_chan_close(jade_chan_t *ch) {
    atomic_store(&ch->closed, 1);

    /* Wake all blocked receivers so they get the close signal */
    chan_lock(ch);
    jade_coro_t *c = ch->recv_waitq;
    ch->recv_waitq = NULL;
    /* Also wake senders */
    jade_coro_t *s = ch->send_waitq;
    ch->send_waitq = NULL;
    chan_unlock(ch);

    while (c) {
        jade_coro_t *next = c->next;
        c->next = NULL;
        c->wait_chan = NULL;
        c->state = JADE_CORO_READY;
        jade_sched_enqueue(c);
        c = next;
    }
    while (s) {
        jade_coro_t *next = s->next;
        s->next = NULL;
        s->wait_chan = NULL;
        s->state = JADE_CORO_READY;
        jade_sched_enqueue(s);
        s = next;
    }
    /* Wake non-coroutine thread waiters */
}

void jade_chan_send(jade_chan_t *ch, const void *data) {
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
            jade_coro_t *waiter = ch->recv_waitq;
            if (waiter) {
                ch->recv_waitq = waiter->next;
                waiter->next = NULL;
                waiter->wait_chan = NULL;
                waiter->state = JADE_CORO_READY;
                chan_unlock(ch);
                jade_sched_enqueue(waiter);
            } else {
                chan_unlock(ch);
            }
            return;
        }

        /* Buffer full — park this coroutine */
        jade_worker_t *w = tl_worker;
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

        jade_coro_t *self = w->current;
        self->state = JADE_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->next = NULL;

        /* Append to send wait queue */
        if (!ch->send_waitq) {
            ch->send_waitq = self;
        } else {
            jade_coro_t *t = ch->send_waitq;
            while (t->next) t = t->next;
            t->next = self;
        }

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler — will be resumed when a recv frees space */
        w->held_chan_lock = ch;
        w->last_action = SCHED_ACTION_PARK;
        jade_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed here — retry send from the top */
    }
}

int jade_chan_recv(jade_chan_t *ch, void *data_out) {
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
            jade_coro_t *waiter = ch->send_waitq;
            if (waiter) {
                ch->send_waitq = waiter->next;
                waiter->next = NULL;
                waiter->wait_chan = NULL;
                waiter->state = JADE_CORO_READY;
                chan_unlock(ch);
                jade_sched_enqueue(waiter);
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
        jade_worker_t *w = tl_worker;
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

        jade_coro_t *self = w->current;
        self->state = JADE_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->next = NULL;

        /* Append to recv wait queue */
        if (!ch->recv_waitq) {
            ch->recv_waitq = self;
        } else {
            jade_coro_t *t = ch->recv_waitq;
            while (t->next) t = t->next;
            t->next = self;
        }

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler */
        w->held_chan_lock = ch;
        w->last_action = SCHED_ACTION_PARK;
        jade_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — retry recv */
    }
}
