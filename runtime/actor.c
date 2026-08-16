#include <time.h>
#include "jinn_rt.h"
#include <stdlib.h>
#include <stdio.h>
void jinn_actor_stop(void *mailbox_ptr) {
    jinn_chan_t *ch = *(jinn_chan_t **)mailbox_ptr;
    if (ch) {
        jinn_chan_close(ch);
    }
}
struct jinn_join {
    _Atomic(int32_t) done;
    _Atomic(int32_t) lock;
    jinn_coro_t     *waitq;
};
static inline void join_lock(jinn_join_t *j) {
    while (atomic_exchange_explicit(&j->lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}
static inline void join_unlock(jinn_join_t *j) {
    atomic_store_explicit(&j->lock, 0, memory_order_release);
}
jinn_join_t *jinn_join_create(void) {
    jinn_join_t *j = (jinn_join_t *)jinn_xmalloc(sizeof(jinn_join_t));
    atomic_store(&j->done, 0);
    atomic_store(&j->lock, 0);
    j->waitq = NULL;
    return j;
}
jinn_join_t *jinn_join_get(void *join_slot_ptr) {
    _Atomic(jinn_join_t *) *slot = (_Atomic(jinn_join_t *) *)join_slot_ptr;
    jinn_join_t *j = atomic_load_explicit(slot, memory_order_acquire);
    if (!j) {
        jinn_join_t *fresh = jinn_join_create();
        jinn_join_t *expected = NULL;
        if (atomic_compare_exchange_strong_explicit(
                slot, &expected, fresh,
                memory_order_acq_rel, memory_order_acquire)) {
            j = fresh;
        } else {
            free(fresh);
            j = expected;
        }
    }
    return j;
}
void jinn_join_signal(void *join_slot_ptr) {
    jinn_join_t *j = jinn_join_get(join_slot_ptr);
    join_lock(j);
    atomic_store_explicit(&j->done, 1, memory_order_release);
    jinn_coro_t *w = j->waitq;
    j->waitq = NULL;
    join_unlock(j);
    while (w) {
        jinn_coro_t *next = w->next;
        w->next = NULL;
        w->state = JINN_CORO_READY;
        jinn_sched_enqueue(w);
        w = next;
    }
}
void jinn_actor_join(void *join_slot_ptr) {
    jinn_join_t *j = jinn_join_get(join_slot_ptr);
    for (;;) {
        if (atomic_load_explicit(&j->done, memory_order_acquire)) {
            return;
        }
        jinn_worker_t *wk = jinn_worker_self();
        if (!wk || !wk->current) {

            jinn_sched_yield();
            continue;
        }
        jinn_coro_t *self = wk->current;
        join_lock(j);
        if (atomic_load_explicit(&j->done, memory_order_acquire)) {
            join_unlock(j);
            return;
        }
        self->state = JINN_CORO_SUSPENDED;
        self->next = j->waitq;
        j->waitq = self;
        wk->held_lock = &j->lock;
        wk->last_action = SCHED_ACTION_PARK;
        jinn_coro_swap_out(self, &wk->sched_ctx);

    }
}
typedef struct jinn_live_mb {
    void                *mailbox;
    struct jinn_live_mb *next;
} jinn_live_mb_t;
static jinn_live_mb_t  *g_live_mailboxes = NULL;
static pthread_mutex_t  g_live_mb_lock = PTHREAD_MUTEX_INITIALIZER;

void jinn_actor_register_live(void *mailbox_ptr) {
    if (!mailbox_ptr) return;
    jinn_live_mb_t *n = (jinn_live_mb_t *)jinn_xmalloc(sizeof(jinn_live_mb_t));
    n->mailbox = mailbox_ptr;
    pthread_mutex_lock(&g_live_mb_lock);
    n->next = g_live_mailboxes;
    g_live_mailboxes = n;
    pthread_mutex_unlock(&g_live_mb_lock);
}
void jinn_actor_unregister_live(void *mailbox_ptr) {
    if (!mailbox_ptr) return;
    pthread_mutex_lock(&g_live_mb_lock);
    jinn_live_mb_t **cur = &g_live_mailboxes;
    while (*cur) {
        if ((*cur)->mailbox == mailbox_ptr) {
            jinn_live_mb_t *dead = *cur;
            *cur = dead->next;
            free(dead);
            break;
        }
        cur = &(*cur)->next;
    }
    pthread_mutex_unlock(&g_live_mb_lock);
}
void jinn_actor_stop_all(void) {
    pthread_mutex_lock(&g_live_mb_lock);
    jinn_live_mb_t *list = g_live_mailboxes;
    g_live_mailboxes = NULL;
    int n_chans = 0;
    for (jinn_live_mb_t *n = list; n; n = n->next) n_chans++;
    jinn_chan_t **chans = NULL;
    if (n_chans > 0) {
        chans = (jinn_chan_t **)jinn_xmalloc((size_t)n_chans * sizeof(*chans));
        int i = 0;
        for (jinn_live_mb_t *n = list; n; n = n->next) {
            chans[i++] = *(jinn_chan_t **)n->mailbox;
        }
    }
    pthread_mutex_unlock(&g_live_mb_lock);

    while (list) {
        jinn_live_mb_t *next = list->next;
        free(list);
        list = next;
    }

    for (int i = 0; i < n_chans; i++) {
        if (chans[i]) jinn_chan_close(chans[i]);
    }

    const int64_t max_spins = 500000;
    for (int64_t spin = 0; spin < max_spins; spin++) {
        int64_t pending = 0;
        for (int i = 0; i < n_chans; i++) {
            pending += jinn_chan_pending(chans[i]);
        }
        if (pending == 0) break;
        struct timespec ns = {0, 10000};
        nanosleep(&ns, NULL);
    }
    free(chans);
}

typedef struct jinn_retired_mb {
    void                   *mailbox;
    void                   *chan;
    struct jinn_retired_mb *next;
} jinn_retired_mb_t;
static _Atomic(jinn_retired_mb_t *) g_retired_mailboxes = NULL;
static void jinn_actor_retire_mailbox(void *mailbox_ptr, void *chan) {
    jinn_retired_mb_t *node =
        (jinn_retired_mb_t *)jinn_xmalloc(sizeof(jinn_retired_mb_t));
    node->mailbox = mailbox_ptr;
    node->chan = chan;
    node->next = atomic_load_explicit(&g_retired_mailboxes, memory_order_relaxed);
    while (!atomic_compare_exchange_weak_explicit(
        &g_retired_mailboxes, &node->next, node,
        memory_order_release, memory_order_relaxed)) {
    }
}
void jinn_actor_retire_flush(void) {
    jinn_retired_mb_t *n =
        atomic_exchange_explicit(&g_retired_mailboxes, NULL, memory_order_acquire);
    while (n) {
        jinn_retired_mb_t *next = n->next;
        if (n->chan) jinn_chan_destroy((jinn_chan_t *)n->chan);
        free(n->mailbox);
        free(n);
        n = next;
    }
}
void jinn_actor_destroy(void *mailbox_ptr) {
    if (!mailbox_ptr) return;
    jinn_actor_unregister_live(mailbox_ptr);
    jinn_worker_t *w = jinn_worker_self();
    if (w && w->current && w->current->scope) {
        jinn_scope_unregister_actor((jinn_scope_t *)w->current->scope, mailbox_ptr);
    }
    pthread_mutex_lock(&g_live_mb_lock);
    jinn_chan_t *ch = *(jinn_chan_t **)mailbox_ptr;
    if (ch) {
        *(jinn_chan_t **)mailbox_ptr = NULL;
    }
    pthread_mutex_unlock(&g_live_mb_lock);
    if (ch) {
        jinn_chan_close(ch);
    }
    *(int32_t *)((char *)mailbox_ptr + sizeof(void *)) = 0;
    jinn_actor_retire_mailbox(mailbox_ptr, ch);
}
