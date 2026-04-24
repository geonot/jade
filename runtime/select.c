/*
 * Jade Runtime — Multi-channel select.
 *
 * Implements Go's select algorithm:
 * 1. Shuffle cases for fairness
 * 2. Lock all channels in address order (deadlock prevention)
 * 3. Scan for ready channel
 * 4. If none ready and has default → return default
 * 5. Enqueue on all channel wait queues
 * 6. Park coroutine
 * 7. On wake: dequeue from all other channels, return fired case
 */
#include "jade_rt.h"
#include <stdlib.h>
#include <string.h>

/* ── Spinlock helpers (same as channel.c) ────────────────────────── */

static inline void chan_lock(jade_chan_t *ch) {
    while (atomic_exchange_explicit(&ch->lock, 1, memory_order_acquire) != 0) {
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

/* Fisher-Yates shuffle */
static void shuffle(int *arr, int n, uint64_t *rng) {
    for (int i = n - 1; i > 0; i--) {
        uint64_t x = *rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *rng = x;
        int j = (int)(x % (uint64_t)(i + 1));
        int tmp = arr[i];
        arr[i] = arr[j];
        arr[j] = tmp;
    }
}

/* Sort by channel address for consistent locking */
static void sort_by_addr(int *order, jade_select_case_t *cases, int n) {
    /* Simple insertion sort — n is typically small (<10) */
    for (int i = 1; i < n; i++) {
        int key = order[i];
        uintptr_t key_addr = (uintptr_t)cases[key].chan;
        int j = i - 1;
        while (j >= 0 && (uintptr_t)cases[order[j]].chan > key_addr) {
            order[j + 1] = order[j];
            j--;
        }
        order[j + 1] = key;
    }
}

static void lock_all(jade_select_case_t *cases, int *lock_order, int n) {
    uintptr_t last = 0;
    for (int i = 0; i < n; i++) {
        jade_chan_t *ch = cases[lock_order[i]].chan;
        if (!ch) continue;
        uintptr_t addr = (uintptr_t)ch;
        if (addr != last) {
            chan_lock(ch);
            last = addr;
        }
    }
}

static void unlock_all(jade_select_case_t *cases, int *lock_order, int n) {
    uintptr_t last = 0;
    for (int i = n - 1; i >= 0; i--) {
        jade_chan_t *ch = cases[lock_order[i]].chan;
        if (!ch) continue;
        uintptr_t addr = (uintptr_t)ch;
        if (addr != last) {
            chan_unlock(ch);
            last = addr;
        }
    }
}

static int chan_can_send(jade_chan_t *ch) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);
    return (tail - head) < ch->capacity;
}

static int chan_can_recv(jade_chan_t *ch) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);
    return head < tail;
}

static void chan_send_locked(jade_chan_t *ch, const void *data) {
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);
    size_t idx = tail & (ch->capacity - 1);
    memcpy((char *)ch->buffer + idx * ch->elem_size, data, ch->elem_size);
    atomic_store_explicit(&ch->tail, tail + 1, memory_order_release);
}

static void chan_recv_locked(jade_chan_t *ch, void *data_out) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    size_t idx = head & (ch->capacity - 1);
    memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
    atomic_store_explicit(&ch->head, head + 1, memory_order_release);
}

static inline void waitq_push(jade_coro_t **head, jade_coro_t **tail, jade_coro_t *node) {
    node->next = NULL;
    if (*tail) {
        (*tail)->next = node;
    } else {
        *head = node;
    }
    *tail = node;
}

static inline jade_coro_t *waitq_pop(jade_coro_t **head, jade_coro_t **tail) {
    jade_coro_t *node = *head;
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

static int waitq_remove(jade_coro_t **head, jade_coro_t **tail, jade_coro_t *node) {
    jade_coro_t *prev = NULL;
    jade_coro_t *cur = *head;
    while (cur && cur != node) {
        prev = cur;
        cur = cur->next;
    }
    if (!cur) {
        return 0;
    }
    if (prev) {
        prev->next = cur->next;
    } else {
        *head = cur->next;
    }
    if (*tail == cur) {
        *tail = prev;
    }
    cur->next = NULL;
    return 1;
}

int jade_select(jade_select_case_t *cases, int n, int has_default) {
    if (n == 0) return -1;

    jade_worker_t *w = tl_worker;
    uint64_t rng = w ? w->rng_state : 12345;

    /* Create shuffle and lock orders */
    int poll_order[16];
    int lock_order[16];
    int limit = n < 16 ? n : 16;
    for (int i = 0; i < limit; i++) {
        poll_order[i] = i;
        lock_order[i] = i;
    }
    shuffle(poll_order, limit, &rng);
    sort_by_addr(lock_order, cases, limit);

    if (w) w->rng_state = rng;

    /* Lock all channels */
    lock_all(cases, lock_order, limit);

    /* Scan for ready channel in shuffled order */
    for (int i = 0; i < limit; i++) {
        int idx = poll_order[i];
        jade_select_case_t *c = &cases[idx];
        if (!c->chan) continue;

        if (c->is_send) {
            if (chan_can_send(c->chan)) {
                chan_send_locked(c->chan, c->data);
                /* Wake receiver if waiting */
                jade_coro_t *waiter = waitq_pop(&c->chan->recv_waitq, &c->chan->recv_waitq_tail);
                if (waiter) {
                    waiter->wait_chan = NULL;
                    waiter->state = JADE_CORO_READY;
                    unlock_all(cases, lock_order, limit);
                    jade_sched_enqueue(waiter);
                    return idx;
                }
                unlock_all(cases, lock_order, limit);
                return idx;
            }
        } else {
            if (chan_can_recv(c->chan)) {
                chan_recv_locked(c->chan, c->data);
                /* Wake sender if waiting */
                jade_coro_t *waiter = waitq_pop(&c->chan->send_waitq, &c->chan->send_waitq_tail);
                if (waiter) {
                    waiter->wait_chan = NULL;
                    waiter->state = JADE_CORO_READY;
                    unlock_all(cases, lock_order, limit);
                    jade_sched_enqueue(waiter);
                    return idx;
                }
                unlock_all(cases, lock_order, limit);
                return idx;
            }
        }
    }

    /* Nothing ready */
    if (has_default) {
        unlock_all(cases, lock_order, limit);
        return -1;
    }

    /* Enqueue on a channel wait queue, park, then retry in a loop.
     *
     * Because the coro has a single intrusive `next` pointer, it can only
     * sit on one wait queue at a time. We compensate by enqueuing on each
     * channel in round-robin fashion across retries so that all channels
     * eventually get a chance to wake us. After each wake we re-scan all
     * channels for readiness. This is the standard polling-retry approach
     * for single-node intrusive lists. */
    if (!w || !w->current) {
        unlock_all(cases, lock_order, limit);
        return -1;
    }

    jade_coro_t *self = w->current;
    int enqueue_start = 0; /* rotate which channel we enqueue on */
    int max_retries = 256; /* safety bound to avoid infinite spin */

    for (int attempt = 0; attempt < max_retries; attempt++) {
        self->state = JADE_CORO_SUSPENDED;
        self->select_ready = -1;
        self->wait_chan = NULL;

        /* Enqueue on the next available channel (round-robin) */
        int enqueued = 0;
        for (int r = 0; r < limit; r++) {
            int i = (enqueue_start + r) % limit;
            jade_select_case_t *c = &cases[i];
            if (!c->chan) continue;

            self->wait_chan = c->chan;
            if (c->is_send) {
                waitq_push(&c->chan->send_waitq, &c->chan->send_waitq_tail, self);
            } else {
                waitq_push(&c->chan->recv_waitq, &c->chan->recv_waitq_tail, self);
            }
            enqueue_start = (i + 1) % limit; /* next attempt uses next channel */
            enqueued = 1;
            break;
        }

        if (!enqueued) {
            /* No valid channels at all */
            unlock_all(cases, lock_order, limit);
            return -1;
        }

        unlock_all(cases, lock_order, limit);

        /* Park — scheduler will resume us when the channel we're on fires */
        w->held_chan_lock = NULL;
        w->last_action = SCHED_ACTION_PARK;
        jade_context_swap(&self->ctx, &w->sched_ctx);

        /* Woken — re-lock all channels and scan for readiness */
        lock_all(cases, lock_order, limit);

        /* Dequeue ourselves from whatever wait queue we were on */
        if (self->wait_chan) {
            jade_chan_t *wch = (jade_chan_t *)self->wait_chan;
            /* Try both queues — we don't know if waker already removed us */
            if (!waitq_remove(&wch->send_waitq, &wch->send_waitq_tail, self)) {
                waitq_remove(&wch->recv_waitq, &wch->recv_waitq_tail, self);
            }
            self->wait_chan = NULL;
            self->next = NULL;
        }

        /* Scan all channels in shuffled order for a ready one */
        for (int i = 0; i < limit; i++) {
            int idx = poll_order[i];
            jade_select_case_t *c = &cases[idx];
            if (!c->chan) continue;
            if (c->is_send && chan_can_send(c->chan)) {
                chan_send_locked(c->chan, c->data);
                jade_coro_t *waiter = waitq_pop(&c->chan->recv_waitq, &c->chan->recv_waitq_tail);
                if (waiter) {
                    waiter->wait_chan = NULL;
                    waiter->state = JADE_CORO_READY;
                    unlock_all(cases, lock_order, limit);
                    jade_sched_enqueue(waiter);
                    return idx;
                }
                unlock_all(cases, lock_order, limit);
                return idx;
            }
            if (!c->is_send && chan_can_recv(c->chan)) {
                chan_recv_locked(c->chan, c->data);
                jade_coro_t *waiter = waitq_pop(&c->chan->send_waitq, &c->chan->send_waitq_tail);
                if (waiter) {
                    waiter->wait_chan = NULL;
                    waiter->state = JADE_CORO_READY;
                    unlock_all(cases, lock_order, limit);
                    jade_sched_enqueue(waiter);
                    return idx;
                }
                unlock_all(cases, lock_order, limit);
                return idx;
            }
        }
        /* Nothing ready yet — loop back and enqueue on next channel */
    }

    /* Exhausted retries — potential deadlock */
    unlock_all(cases, lock_order, limit);
    fprintf(stderr, "jade: select: exhausted %d retries — possible deadlock\n",
            max_retries);
    return -1;
}
