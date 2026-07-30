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

static inline void waitq_push(jinn_waitq_node_t **head, jinn_waitq_node_t **tail,
                              jinn_waitq_node_t *node) {
    node->next = NULL;
    if (*tail) {
        (*tail)->next = node;
    } else {
        *head = node;
    }
    *tail = node;
}

/* Claim a popped node's coroutine for waking. Plain waiters always
 * succeed; a select node is won only by the first CAS on the selector's
 * claim word — losers discard the node (its selector is already awake or
 * being woken by another case). Must be called with the queue's channel
 * lock held; the node memory (selector stack / coroutine embed) stays
 * valid because the selector cannot leave jinn_select while its nodes are
 * reachable from any locked queue. */
static inline jinn_coro_t *waitq_node_claim(jinn_waitq_node_t *node) {
    if (node->select_claim) {
        int32_t expected = 0;
        if (!atomic_compare_exchange_strong_explicit(
                node->select_claim, &expected, 1,
                memory_order_acq_rel, memory_order_acquire)) {
            return NULL;
        }
    }
    return node->coro;
}

/* Pop until a wakeable waiter is found (skipping select nodes whose
 * selector was already claimed by another case). Returns the coroutine to
 * wake, or NULL if the queue drained. */
static inline jinn_coro_t *waitq_pop_wakeable(jinn_waitq_node_t **head,
                                              jinn_waitq_node_t **tail) {
    for (;;) {
        jinn_waitq_node_t *node = *head;
        if (!node) return NULL;
        *head = node->next;
        if (!*head) *tail = NULL;
        node->next = NULL;
        jinn_coro_t *c = waitq_node_claim(node);
        if (c) return c;
    }
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
    if (!ch) return;
    atomic_store(&ch->closed, 1);

    /* Detach and claim every waiter under ONE lock hold, then never touch
     * the channel again: the first coroutine we enqueue may run the
     * program's last use of this channel and free it, so a re-lock after
     * any enqueue is a use-after-free. One sweep suffices — `closed` was
     * stored above, and both park paths test it under this same lock, so
     * no waiter can arrive once the queues drain. Claimed coroutines are
     * chained through their intrusive `next` (unused while parked on a
     * channel: channel queues link through jinn_waitq_node_t, and a
     * claimed coroutine cannot run until we enqueue it). Select-node
     * memory (selector stacks) stays valid exactly as long as we hold the
     * lock — a selector woken by another case blocks on this lock in its
     * node-removal pass before it can return. */
    jinn_coro_t *wake_head = NULL, *wake_tail = NULL;
    chan_lock(ch);
    for (;;) {
        jinn_coro_t *c = waitq_pop_wakeable(&ch->recv_waitq, &ch->recv_waitq_tail);
        if (!c) c = waitq_pop_wakeable(&ch->send_waitq, &ch->send_waitq_tail);
        if (!c) break;
        c->next = NULL;
        if (wake_tail) wake_tail->next = c; else wake_head = c;
        wake_tail = c;
    }
    chan_unlock(ch);

    while (wake_head) {
        jinn_coro_t *c = wake_head;
        wake_head = c->next;
        c->next = NULL;
        c->wait_chan = NULL;
        c->state = JINN_CORO_READY;
        jinn_sched_enqueue(c);
    }
}

/* Remove the node whose coroutine is `target` from a wait queue. Returns
 * the node, or NULL. Used by scope cancellation to unpark a child blocked
 * on a channel so it reaches its next cancellation check and unwinds.
 * Safe when the coroutine is not queued (no-op). */
static jinn_waitq_node_t *waitq_remove_coro(jinn_waitq_node_t **head,
                                            jinn_waitq_node_t **tail,
                                            jinn_coro_t *target) {
    jinn_waitq_node_t *prev = NULL, *cur = *head;
    while (cur) {
        if (cur->coro == target) {
            if (prev) prev->next = cur->next; else *head = cur->next;
            if (*tail == cur) *tail = prev;
            cur->next = NULL;
            return cur;
        }
        prev = cur;
        cur = cur->next;
    }
    return NULL;
}

void jinn_chan_wake_coro(jinn_chan_t *ch, jinn_coro_t *c) {
    if (!ch || !c) return;
    chan_lock(ch);
    jinn_waitq_node_t *node = waitq_remove_coro(&ch->send_waitq, &ch->send_waitq_tail, c);
    if (!node) node = waitq_remove_coro(&ch->recv_waitq, &ch->recv_waitq_tail, c);
    jinn_coro_t *wake = node ? waitq_node_claim(node) : NULL;
    chan_unlock(ch);
    if (wake) {
        wake->wait_chan = NULL;
        wake->state = JINN_CORO_READY;
        jinn_sched_enqueue(wake);
    }
}

int jinn_chan_send(jinn_chan_t *ch, const void *data) {
    if (!ch) return 0;
    for (;;) {
        /* Re-derived every iteration, never cached across the park below: a
         * resumed coroutine may be running on a different worker now. */
        jinn_worker_t *wc = jinn_worker_self();
        if (wc && wc->current
            && atomic_load_explicit(&wc->current->cancelled, memory_order_acquire)) {
            return 0;
        }
        chan_lock(ch);

        /* Check for close */
        if (atomic_load(&ch->closed)) {
            chan_unlock(ch);
            return 0;
        }

        uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
        uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);

        if (tail - head < ch->capacity) {
            /* Buffer has space — write element */
            size_t idx = tail & (ch->capacity - 1);
            memcpy((char *)ch->buffer + idx * ch->elem_size, data, ch->elem_size);
            atomic_store_explicit(&ch->tail, tail + 1, memory_order_release);

            /* Wake one blocked receiver if any */
            jinn_coro_t *waiter = waitq_pop_wakeable(&ch->recv_waitq, &ch->recv_waitq_tail);
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

        /* Buffer full — park this coroutine */
        jinn_worker_t *w = jinn_worker_self();
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
        /* Re-check cancellation under the lock to avoid a lost wakeup: if the
         * scope was cancelled after our top-of-loop check, bail instead of
         * parking forever. */
        if (atomic_load_explicit(&self->cancelled, memory_order_acquire)) {
            chan_unlock(ch);
            return 0;
        }
        self->state = JINN_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->wq_node.coro = self;
        self->wq_node.select_claim = NULL;

        /* Append to send wait queue in O(1) */
        waitq_push(&ch->send_waitq, &ch->send_waitq_tail, &self->wq_node);

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler — will be resumed when a recv frees space */
        w->held_lock = &ch->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed here — retry send from the top */
    }
}

int jinn_chan_recv(jinn_chan_t *ch, void *data_out) {
    if (!ch) return 0;
    for (;;) {
        /* Re-derived every iteration — see jinn_chan_send. */
        jinn_worker_t *wc = jinn_worker_self();
        if (wc && wc->current
            && atomic_load_explicit(&wc->current->cancelled, memory_order_acquire)) {
            memset(data_out, 0, ch->elem_size);
            return 0;
        }
        chan_lock(ch);

        uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
        uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);

        if (head < tail) {
            /* Buffer has data — read element */
            size_t idx = head & (ch->capacity - 1);
            memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
            atomic_store_explicit(&ch->head, head + 1, memory_order_release);

            /* Wake one blocked sender if any */
            jinn_coro_t *waiter = waitq_pop_wakeable(&ch->send_waitq, &ch->send_waitq_tail);
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
        jinn_worker_t *w = jinn_worker_self();
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
        if (atomic_load_explicit(&self->cancelled, memory_order_acquire)) {
            memset(data_out, 0, ch->elem_size);
            chan_unlock(ch);
            return 0;
        }
        self->state = JINN_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->wq_node.coro = self;
        self->wq_node.select_claim = NULL;

        /* Append to recv wait queue in O(1) */
        waitq_push(&ch->recv_waitq, &ch->recv_waitq_tail, &self->wq_node);

        /* Don't unlock — scheduler will release after context is saved */

        /* Yield to scheduler */
        w->held_lock = &ch->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — retry recv */
    }
}

int jinn_chan_try_recv(jinn_chan_t *ch, void *data_out) {
    if (!ch) return -1;
    chan_lock(ch);

    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);

    if (head < tail) {
        size_t idx = head & (ch->capacity - 1);
        memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
        atomic_store_explicit(&ch->head, head + 1, memory_order_release);

        jinn_coro_t *waiter = waitq_pop_wakeable(&ch->send_waitq, &ch->send_waitq_tail);
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
