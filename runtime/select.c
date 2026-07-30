/*
 * Jinn Runtime — Multi-channel select (task 8-11 redesign).
 *
 * Go's algorithm, with real multi-queue membership:
 *  1. Shuffle cases for fairness; lock all channels in address order.
 *  2. Scan for a ready case (a closed channel counts: receive fires with
 *     the zero value, exactly like jinn_chan_recv's end-of-stream; a send
 *     on a closed channel fires without sending, matching statement-send's
 *     silent-false — note the frontend does not generate send cases yet,
 *     and when it does the case struct needs a result slot to surface the
 *     failure, see .ryu/tasks/8-11.notes).
 *  3. Nothing ready + default → return -1 (default). -1 is returned ONLY
 *     for the default case — the old 256-retry cap that reported
 *     exhaustion as -1 (default masquerade) is gone; a selector with no
 *     ready case and no default parks for as long as it takes.
 *  4. Otherwise park with one waiter node on EVERY case's wait queue
 *     (jinn_waitq_node_t, stack-allocated — the old single intrusive
 *     pointer could only wait on one channel per attempt, so a selector
 *     parked on channel A was never woken by traffic on channel B).
 *     Wakers CAS the shared claim word, so exactly one case wins.
 *  5. Park holding ALL case locks; the scheduler releases them in reverse
 *     acquisition order after the context is saved (multi-lock variant of
 *     the 8-10 handoff). Because release is reverse and re-acquisition is
 *     forward (sorted), a woken selector cannot pass lock_all — and so
 *     cannot touch this frame's arrays — until the release loop finished.
 *  6. On wake: re-lock everything, remove the remaining nodes, re-scan.
 *     A cancelled selector (scope cancellation) returns -1 so the
 *     generated post-suspension cancellation check can unwind.
 */
#include "jinn_rt.h"
#include <alloca.h>
#include <stdlib.h>
#include <string.h>

/* ── Spinlock helpers (same as channel.c) ────────────────────────── */

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
static void sort_by_addr(int *order, jinn_select_case_t *cases, int n) {
    /* Simple insertion sort — n is typically small */
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

static void lock_all(jinn_select_case_t *cases, int *lock_order, int n) {
    uintptr_t last = 0;
    for (int i = 0; i < n; i++) {
        jinn_chan_t *ch = cases[lock_order[i]].chan;
        if (!ch) continue;
        uintptr_t addr = (uintptr_t)ch;
        if (addr != last) {
            chan_lock(ch);
            last = addr;
        }
    }
}

static void unlock_all(jinn_select_case_t *cases, int *lock_order, int n) {
    uintptr_t last = 0;
    for (int i = n - 1; i >= 0; i--) {
        jinn_chan_t *ch = cases[lock_order[i]].chan;
        if (!ch) continue;
        uintptr_t addr = (uintptr_t)ch;
        if (addr != last) {
            chan_unlock(ch);
            last = addr;
        }
    }
}

static int chan_can_send(jinn_chan_t *ch) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_acquire);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);
    return (tail - head) < ch->capacity;
}

static int chan_can_recv(jinn_chan_t *ch) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_acquire);
    return head < tail;
}

static void chan_send_locked(jinn_chan_t *ch, const void *data) {
    uint64_t tail = atomic_load_explicit(&ch->tail, memory_order_relaxed);
    size_t idx = tail & (ch->capacity - 1);
    memcpy((char *)ch->buffer + idx * ch->elem_size, data, ch->elem_size);
    atomic_store_explicit(&ch->tail, tail + 1, memory_order_release);
}

static void chan_recv_locked(jinn_chan_t *ch, void *data_out) {
    uint64_t head = atomic_load_explicit(&ch->head, memory_order_relaxed);
    size_t idx = head & (ch->capacity - 1);
    memcpy(data_out, (char *)ch->buffer + idx * ch->elem_size, ch->elem_size);
    atomic_store_explicit(&ch->head, head + 1, memory_order_release);
}

/* ── Node queue helpers (channel lock held) ──────────────────────── */

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

static void waitq_remove_node(jinn_waitq_node_t **head, jinn_waitq_node_t **tail,
                              jinn_waitq_node_t *node) {
    jinn_waitq_node_t *prev = NULL, *cur = *head;
    while (cur) {
        if (cur == node) {
            if (prev) prev->next = cur->next; else *head = cur->next;
            if (*tail == cur) *tail = prev;
            cur->next = NULL;
            return;
        }
        prev = cur;
        cur = cur->next;
    }
}

/* Claim-aware pop, mirroring channel.c: skip select nodes whose selector
 * was already won by another case. */
static jinn_coro_t *waitq_pop_wakeable(jinn_waitq_node_t **head,
                                       jinn_waitq_node_t **tail) {
    for (;;) {
        jinn_waitq_node_t *node = *head;
        if (!node) return NULL;
        *head = node->next;
        if (!*head) *tail = NULL;
        node->next = NULL;
        if (node->select_claim) {
            int32_t expected = 0;
            if (!atomic_compare_exchange_strong_explicit(
                    node->select_claim, &expected, 1,
                    memory_order_acq_rel, memory_order_acquire)) {
                continue;
            }
        }
        return node->coro;
    }
}

/* Execute a ready case with all locks held. Returns the woken counterpart
 * waiter (to enqueue after unlock) or NULL. */
static jinn_coro_t *fire_case_locked(jinn_select_case_t *c) {
    if (c->is_send) {
        if (atomic_load(&c->chan->closed)) {
            return NULL; /* fires without sending — see header comment */
        }
        chan_send_locked(c->chan, c->data);
        return waitq_pop_wakeable(&c->chan->recv_waitq, &c->chan->recv_waitq_tail);
    }
    if (chan_can_recv(c->chan)) {
        chan_recv_locked(c->chan, c->data);
        return waitq_pop_wakeable(&c->chan->send_waitq, &c->chan->send_waitq_tail);
    }
    /* closed and empty: end-of-stream — zero the destination, like
     * jinn_chan_recv's closed return */
    memset(c->data, 0, c->chan->elem_size);
    return NULL;
}

static int case_ready(jinn_select_case_t *c) {
    if (!c->chan) return 0;
    if (c->is_send) {
        return atomic_load(&c->chan->closed) || chan_can_send(c->chan);
    }
    return chan_can_recv(c->chan) || atomic_load(&c->chan->closed);
}

int jinn_select(jinn_select_case_t *cases, int n, int has_default) {
    if (n <= 0) return -1;

    jinn_worker_t *w = jinn_worker_self();
    /* Off-worker callers (e.g. *main on the process thread) have no
     * per-worker rng; a constant seed here would make every call shuffle
     * identically and starve all but one always-ready case. */
    static _Atomic(uint64_t) g_select_seed = 0x9E3779B97F4A7C15ULL;
    uint64_t rng = w ? w->rng_state
                     : atomic_fetch_add_explicit(&g_select_seed,
                                                 0x9E3779B97F4A7C15ULL,
                                                 memory_order_relaxed) |
                           1u;

    /* No case cap: order/node arrays are stack VLAs sized by the (static,
     * codegen-emitted) case count. The old fixed poll_order[16] silently
     * never polled cases 17+. */
    int *poll_order = (int *)alloca((size_t)n * sizeof(int));
    int *lock_order = (int *)alloca((size_t)n * sizeof(int));
    jinn_waitq_node_t *nodes =
        (jinn_waitq_node_t *)alloca((size_t)n * sizeof(jinn_waitq_node_t));
    _Atomic(int32_t) **locks =
        (_Atomic(int32_t) **)alloca((size_t)n * sizeof(_Atomic(int32_t) *));

    for (int i = 0; i < n; i++) {
        poll_order[i] = i;
        lock_order[i] = i;
    }
    shuffle(poll_order, n, &rng);
    sort_by_addr(lock_order, cases, n);
    if (w) w->rng_state = rng;

    /* Deduplicated lock list in acquisition order, for the park handoff.
     * first_chan is the lowest-addressed live channel — NULL-chan cases
     * sort first in lock_order, so lock_order[0] cannot be used here. */
    int n_locks = 0;
    jinn_chan_t *first_chan = NULL;
    {
        uintptr_t last = 0;
        for (int i = 0; i < n; i++) {
            jinn_chan_t *ch = cases[lock_order[i]].chan;
            if (!ch || (uintptr_t)ch == last) continue;
            if (!first_chan) first_chan = ch;
            locks[n_locks++] = &ch->lock;
            last = (uintptr_t)ch;
        }
    }

    for (;;) {
        /* A cancelled selector must not park again; let the generated
         * cancellation check after the suspension point unwind. */
        jinn_worker_t *wc = jinn_worker_self();
        if (wc && wc->current
            && atomic_load_explicit(&wc->current->cancelled, memory_order_acquire)) {
            return -1;
        }

        lock_all(cases, lock_order, n);

        /* Scan for a ready case in shuffled order (fairness). */
        for (int i = 0; i < n; i++) {
            int idx = poll_order[i];
            jinn_select_case_t *c = &cases[idx];
            if (!case_ready(c)) continue;
            jinn_coro_t *counterpart = fire_case_locked(c);
            unlock_all(cases, lock_order, n);
            if (counterpart) {
                counterpart->wait_chan = NULL;
                counterpart->state = JINN_CORO_READY;
                jinn_sched_enqueue(counterpart);
            }
            return idx;
        }

        if (has_default || n_locks == 0) {
            unlock_all(cases, lock_order, n);
            return -1; /* default fired (or no live channels at all) */
        }

        jinn_worker_t *pw = jinn_worker_self();
        if (!pw || !pw->current) {
            /* Non-coroutine context: back off and re-scan. */
            unlock_all(cases, lock_order, n);
            for (int _spin = 0; _spin < 128; _spin++) {
#if defined(__x86_64__)
                __builtin_ia32_pause();
#elif defined(__aarch64__)
                __asm__ volatile("yield");
#endif
            }
            continue;
        }

        /* Publish one waiter node on every case's queue, sharing one claim
         * word so exactly one waker wins. */
        jinn_coro_t *self = pw->current;
        _Atomic(int32_t) claim;
        atomic_store_explicit(&claim, 0, memory_order_relaxed);
        for (int i = 0; i < n; i++) {
            jinn_select_case_t *c = &cases[i];
            if (!c->chan) continue;
            nodes[i].coro = self;
            nodes[i].select_claim = &claim;
            if (c->is_send) {
                waitq_push(&c->chan->send_waitq, &c->chan->send_waitq_tail, &nodes[i]);
            } else {
                waitq_push(&c->chan->recv_waitq, &c->chan->recv_waitq_tail, &nodes[i]);
            }
        }
        self->state = JINN_CORO_SUSPENDED;
        /* For scope cancellation's targeted wake (jinn_chan_wake_coro via
         * c->wait_chan): any one case channel suffices — the wake only
         * needs to make the selector re-check its cancelled flag. */
        self->wait_chan = first_chan;

        /* Multi-lock handoff (8-10): the scheduler releases every case
         * lock in reverse acquisition order after the context save. */
        pw->held_locks = locks;
        pw->held_locks_n = n_locks;
        pw->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &pw->sched_ctx);

        /* Woken: by a claiming waker, by close, or by cancellation.
         * Remove the remaining nodes under the locks, then re-scan. */
        lock_all(cases, lock_order, n);
        for (int i = 0; i < n; i++) {
            jinn_select_case_t *c = &cases[i];
            if (!c->chan) continue;
            if (c->is_send) {
                waitq_remove_node(&c->chan->send_waitq, &c->chan->send_waitq_tail, &nodes[i]);
            } else {
                waitq_remove_node(&c->chan->recv_waitq, &c->chan->recv_waitq_tail, &nodes[i]);
            }
        }
        unlock_all(cases, lock_order, n);
        jinn_worker_t *ww = jinn_worker_self();
        if (ww && ww->current) {
            ww->current->wait_chan = NULL;
        }
        /* Loop: re-scan with fresh shuffle order for fairness. */
        shuffle(poll_order, n, &rng);
        if (ww) ww->rng_state = rng;
    }
}
