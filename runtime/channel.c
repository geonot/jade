#include "jinn_rt.h"
#include <stdlib.h>
#include <string.h>
#include <sched.h>
#include <unistd.h>
#include <stdio.h>
#include <stdio.h>
#ifndef CHAN_DEBUG
#define CHAN_DEBUG 0
#endif
#define CHAN_TRACE(...) do { if (CHAN_DEBUG) fprintf(stderr, __VA_ARGS__); } while(0)
static inline void chan_lock(jinn_chan_t *ch) {
    while (atomic_exchange_explicit(&ch->lock, 1, memory_order_acquire) != 0) {

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
    atomic_store(&ch->refs, 1);
    return ch;
}
void jinn_chan_destroy(jinn_chan_t *ch) {
    if (!ch) return;
    jinn_chan_close(ch);
    free(ch->buffer);
    free(ch);
}
void jinn_chan_retain(jinn_chan_t *ch) {
    if (!ch) return;
    atomic_fetch_add_explicit(&ch->refs, 1, memory_order_relaxed);
}
void jinn_chan_release(jinn_chan_t *ch) {
    if (!ch) return;
    int64_t prev = atomic_fetch_sub_explicit(&ch->refs, 1, memory_order_acq_rel);
    if (prev == 1) {
        jinn_chan_destroy(ch);
    }
}
void jinn_chan_close(jinn_chan_t *ch) {
    if (!ch) return;
    atomic_store(&ch->closed, 1);
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
        jinn_worker_t *wc = jinn_worker_self();
        if (wc && wc->current
            && atomic_load_explicit(&wc->current->cancelled, memory_order_acquire)) {
            return 0;
        }
        chan_lock(ch);
        if (atomic_load(&ch->closed)) {
            chan_unlock(ch);
            return 0;
        }
        uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
        uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);
        if (tail - head < ch->capacity) {
            size_t idx = tail & (ch->capacity - 1);
            memcpy((char *)ch->buffer + idx * ch->elem_size, data, ch->elem_size);
            atomic_store_explicit(&ch->tail, tail + 1, memory_order_release);

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

        jinn_worker_t *w = jinn_worker_self();
        if (!w || !w->current) {
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
        if (atomic_load_explicit(&self->cancelled, memory_order_acquire)) {
            chan_unlock(ch);
            return 0;
        }
        self->state = JINN_CORO_SUSPENDED;
        self->wait_chan = ch;
        self->wq_node.coro = self;
        self->wq_node.select_claim = NULL;

        waitq_push(&ch->send_waitq, &ch->send_waitq_tail, &self->wq_node);
        w->held_lock = &ch->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_coro_swap_out(self, &w->sched_ctx);
    }
}
int64_t jinn_chan_pending(jinn_chan_t *ch) {
    if (!ch) return 0;
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);
    return tail > head ? (int64_t)(tail - head) : 0;
}
int jinn_chan_recv(jinn_chan_t *ch, void *data_out) {
    if (!ch) return 0;
    for (;;) {

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
            return 0;
        }

        jinn_worker_t *w = jinn_worker_self();
        if (!w || !w->current) {
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
        waitq_push(&ch->recv_waitq, &ch->recv_waitq_tail, &self->wq_node);

        w->held_lock = &ch->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_coro_swap_out(self, &w->sched_ctx);
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
