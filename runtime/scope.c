#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#include "jinn_rt.h"
#include <stdlib.h>
#include <time.h>
struct jinn_scope {
    _Atomic(int64_t)  live_children;
    _Atomic(int32_t)  cancelled;
    _Atomic(int32_t)  lock;
    _Atomic(int32_t)  has_error;
    _Atomic(int64_t)  error_val;
    jinn_coro_t      *parent;
    jinn_scope_t     *prev;
    jinn_coro_t     **children;
    int               child_count;
    int               child_cap;
    void            **actors;
    int               actor_count;
    int               actor_cap;
};
static void *scope_xrealloc(void *p, size_t size) {
    void *q = realloc(p, size);
    if (!q) {
        fprintf(stderr, "jinn: out of memory (scope registry)\n");
        abort();
    }
    return q;
}
_Thread_local jinn_scope_t *tl_scope = NULL;
static inline void scope_lock(jinn_scope_t *s) {
    while (atomic_exchange_explicit(&s->lock, 1, memory_order_acquire) != 0) {
#if defined(__x86_64__)
        __builtin_ia32_pause();
#elif defined(__aarch64__)
        __asm__ volatile("yield");
#endif
    }
}
static inline void scope_unlock(jinn_scope_t *s) {
    atomic_store_explicit(&s->lock, 0, memory_order_release);
}
__attribute__((noinline)) jinn_scope_t *jinn_scope_current(void) {
    return tl_scope;
}
__attribute__((noinline)) void jinn_scope_set_current(jinn_scope_t *s) {
    tl_scope = s;
}
jinn_scope_t *jinn_scope_create(void) {
    jinn_scope_t *s = (jinn_scope_t *)jinn_xmalloc(sizeof(jinn_scope_t));
    atomic_store(&s->live_children, 0);
    atomic_store(&s->cancelled, 0);
    atomic_store(&s->lock, 0);
    atomic_store(&s->has_error, 0);
    atomic_store(&s->error_val, 0);
    s->parent = NULL;
    s->prev = tl_scope;
    s->children = NULL;
    s->child_count = 0;
    s->child_cap = 0;
    s->actors = NULL;
    s->actor_count = 0;
    s->actor_cap = 0;
    tl_scope = s;
    return s;
}
void jinn_scope_register_child(jinn_coro_t *child) {
    jinn_scope_t *s = tl_scope;
    if (!s || !child) return;
    child->scope = s;
    atomic_fetch_add(&s->live_children, 1);
    scope_lock(s);
    if (s->child_count == s->child_cap) {
        s->child_cap = s->child_cap ? s->child_cap * 2 : 16;
        s->children = (jinn_coro_t **)scope_xrealloc(
            s->children, (size_t)s->child_cap * sizeof(*s->children));
    }
    s->children[s->child_count++] = child;
    if (atomic_load_explicit(&s->cancelled, memory_order_acquire)) {
        atomic_store_explicit(&child->cancelled, 1, memory_order_release);
    }
    scope_unlock(s);
}
void jinn_scope_add_actor(jinn_scope_t *s, void *mailbox_ptr) {
    if (!s || !mailbox_ptr) return;
    scope_lock(s);
    if (s->actor_count == s->actor_cap) {
        s->actor_cap = s->actor_cap ? s->actor_cap * 2 : 16;
        s->actors = (void **)scope_xrealloc(s->actors,
                                            (size_t)s->actor_cap * sizeof(*s->actors));
    }
    s->actors[s->actor_count++] = mailbox_ptr;
    scope_unlock(s);
}
void jinn_scope_unregister_child(jinn_scope_t *s, jinn_coro_t *child) {
    if (!s || !child) return;
    scope_lock(s);
    for (int i = 0; i < s->child_count; i++) {
        if (s->children[i] == child) {
            s->children[i] = s->children[--s->child_count];
            break;
        }
    }
    scope_unlock(s);
}

void jinn_scope_unregister_actor(jinn_scope_t *s, void *mailbox_ptr) {
    if (!s || !mailbox_ptr) return;
    scope_lock(s);
    for (int i = 0; i < s->actor_count; i++) {
        if (s->actors[i] == mailbox_ptr) {
            s->actors[i] = s->actors[--s->actor_count];
            break;
        }
    }
    scope_unlock(s);
}

void jinn_actor_spawn_scoped(jinn_coro_t *coro, void *mailbox_ptr) {
    jinn_scope_t *s = tl_scope;
    if (s) {
        jinn_scope_register_child(coro);
        jinn_scope_add_actor(s, mailbox_ptr);
    } else {
        jinn_coro_set_daemon(coro);
    }
}
void jinn_scope_child_done(jinn_scope_t *s) {
    if (!s) return;
    scope_lock(s);
    int64_t remaining = atomic_fetch_sub(&s->live_children, 1) - 1;
    jinn_coro_t *p = NULL;
    if (remaining <= 0) {
        p = s->parent;
        s->parent = NULL;
    }
    scope_unlock(s);
    if (p) {
        p->state = JINN_CORO_READY;
        jinn_sched_enqueue(p);
    }
}
void jinn_scope_cancel(jinn_scope_t *s) {
    if (!s) return;
    if (atomic_exchange_explicit(&s->cancelled, 1, memory_order_acq_rel)) {
        return;
    }
    scope_lock(s);
    for (int i = 0; i < s->child_count; i++) {
        jinn_coro_t *c = s->children[i];
        if (c) {
            atomic_store_explicit(&c->cancelled, 1, memory_order_release);
            void *wc = c->wait_chan;
            if (wc) {
                jinn_chan_wake_coro((jinn_chan_t *)wc, c);
            }
        }
    }
    for (int i = 0; i < s->actor_count; i++) {
        jinn_actor_stop(s->actors[i]);
    }
    scope_unlock(s);
}

void jinn_scope_record_error(jinn_scope_t *s, int64_t errval) {
    if (!s) return;
    int32_t expected = 0;
    if (atomic_compare_exchange_strong_explicit(
            &s->has_error, &expected, 1,
            memory_order_acq_rel, memory_order_acquire)) {
        atomic_store_explicit(&s->error_val, errval, memory_order_release);
    }
    jinn_scope_cancel(s);
}
void jinn_scope_record_current_error(int64_t errval) {
    jinn_worker_t *w = tl_worker;
    if (w && w->current && w->current->scope) {
        jinn_scope_record_error((jinn_scope_t *)w->current->scope, errval);
    }
}
int jinn_scope_take_error(jinn_scope_t *s, int64_t *out) {
    if (!s) return 0;
    if (atomic_load_explicit(&s->has_error, memory_order_acquire)) {
        if (out) *out = atomic_load_explicit(&s->error_val, memory_order_acquire);
        return 1;
    }
    return 0;
}
int jinn_scope_check_cancelled(void) {
    jinn_worker_t *w = tl_worker;
    if (w && w->current) {
        return atomic_load_explicit(&w->current->cancelled, memory_order_acquire);
    }
    return 0;
}
void jinn_scope_stop_actors(jinn_scope_t *s) {
    if (!s) return;
    scope_lock(s);
    for (int i = 0; i < s->actor_count; i++) {
        jinn_actor_stop(s->actors[i]);
    }
    scope_unlock(s);
}
static void scope_wake_cancelled(jinn_scope_t *s) {
    if (!atomic_load_explicit(&s->cancelled, memory_order_acquire)) return;
    scope_lock(s);
    for (int i = 0; i < s->child_count; i++) {
        jinn_coro_t *c = s->children[i];
        if (c && c->state == JINN_CORO_SUSPENDED) {
            void *wc = c->wait_chan;
            if (wc) jinn_chan_wake_coro((jinn_chan_t *)wc, c);
        }
    }
    scope_unlock(s);
}
static int jinn_scope_join_no_free(jinn_scope_t *s) {
    for (;;) {
        if (atomic_load_explicit(&s->live_children, memory_order_acquire) <= 0) {
            scope_lock(s);
            scope_unlock(s);
            break;
        }
        scope_wake_cancelled(s);
        jinn_worker_t *w = jinn_worker_self();
        if (!w || !w->current) {
            jinn_sched_yield();
            continue;
        }
        jinn_coro_t *self = w->current;
        scope_lock(s);
        if (atomic_load_explicit(&s->live_children, memory_order_acquire) <= 0) {
            scope_unlock(s);
            break;
        }
        self->state = JINN_CORO_SUSPENDED;
        s->parent = self;
        w->held_lock = &s->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
    }
    return atomic_load_explicit(&s->has_error, memory_order_acquire) ? 1 : 0;
}
void jinn_scope_join(jinn_scope_t *s) {
    if (!s) return;
    jinn_scope_join_no_free(s);
    jinn_scope_set_current(s->prev);
    free(s->children);
    free(s->actors);
    free(s);
}
int64_t jinn_scope_join_take_error(jinn_scope_t *s) {
    if (!s) return INT64_MIN;
    int had = jinn_scope_join_no_free(s);
    int64_t word = had ? atomic_load_explicit(&s->error_val, memory_order_acquire)
                       : INT64_MIN;
    jinn_scope_set_current(s->prev);
    free(s->children);
    free(s->actors);
    free(s);
    return word;
}
