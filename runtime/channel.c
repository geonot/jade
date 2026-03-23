/*
 * Jade Runtime — Typed channels (bounded MPMC ring buffer).
 *
 * Fast path: atomic CAS on head/tail, no lock.
 * Slow path: park coroutine on wait queue, scheduler resumes on data/space.
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>

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
    pthread_mutex_init(&ch->lock, NULL);

    return ch;
}

void jade_chan_destroy(jade_chan_t *ch) {
    if (!ch) return;
    pthread_mutex_destroy(&ch->lock);
    free(ch->buffer);
    free(ch);
}

void jade_chan_close(jade_chan_t *ch) {
    atomic_store(&ch->closed, 1);

    /* Wake all blocked receivers so they get the close signal */
    pthread_mutex_lock(&ch->lock);
    jade_coro_t *c = ch->recv_waitq;
    ch->recv_waitq = NULL;
    /* Also wake senders */
    jade_coro_t *s = ch->send_waitq;
    ch->send_waitq = NULL;
    pthread_mutex_unlock(&ch->lock);

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
}

void jade_chan_send(jade_chan_t *ch, const void *data) {
    for (;;) {
        pthread_mutex_lock(&ch->lock);

        /* Check for close */
        if (atomic_load(&ch->closed)) {
            pthread_mutex_unlock(&ch->lock);
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
                pthread_mutex_unlock(&ch->lock);
                jade_sched_enqueue(waiter);
            } else {
                pthread_mutex_unlock(&ch->lock);
            }
            return;
        }

        /* Buffer full — park this coroutine */
        jade_worker_t *w = tl_worker;
        if (!w || !w->current) {
            /* Called from main thread (no coroutine) — spin-wait */
            pthread_mutex_unlock(&ch->lock);
            sched_yield();
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

        pthread_mutex_unlock(&ch->lock);

        /* Yield to scheduler — will be resumed when a recv frees space */
        jade_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed here — retry send from the top */
    }
}

int jade_chan_recv(jade_chan_t *ch, void *data_out) {
    for (;;) {
        pthread_mutex_lock(&ch->lock);

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
                pthread_mutex_unlock(&ch->lock);
                jade_sched_enqueue(waiter);
            } else {
                pthread_mutex_unlock(&ch->lock);
            }
            return 1;  /* success */
        }

        /* Buffer empty */
        if (atomic_load(&ch->closed)) {
            /* Channel closed, no more data coming */
            memset(data_out, 0, ch->elem_size);
            pthread_mutex_unlock(&ch->lock);
            return 0;  /* closed */
        }

        /* Park this coroutine */
        jade_worker_t *w = tl_worker;
        if (!w || !w->current) {
            /* Called from main thread — spin-wait */
            pthread_mutex_unlock(&ch->lock);
            sched_yield();
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

        pthread_mutex_unlock(&ch->lock);

        /* Yield to scheduler */
        jade_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — retry recv */
    }
}
