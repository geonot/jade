/*
 * Jinn Runtime — Structured concurrency scopes (`together`).
 *
 * A scope owns the concurrent work started inside it. Children
 * (dispatched coroutines, scope-owned actors) register on creation; the
 * scope join parks the parent until every child has completed. A scope can
 * be cancelled (by `stop <scope>` or a failing child): cancellation marks
 * live children, which unwind at their next suspension point.
 *
 * The "current scope" is thread-local. A `together` block pushes its scope,
 * runs its body on the current coroutine/thread, then joins and pops. The
 * worker loop restores a resumed coroutine's scope so that dispatches made
 * from helper functions called inside the body still register correctly.
 */
#ifndef _POSIX_C_SOURCE
#define _POSIX_C_SOURCE 200809L
#endif
#include "jinn_rt.h"
#include <stdlib.h>
#include <time.h>

#define JINN_SCOPE_MAX_ACTORS 64

struct jinn_scope {
    _Atomic(int64_t)  live_children;
    _Atomic(int32_t)  cancelled;
    _Atomic(int32_t)  lock;
    jinn_coro_t      *parent;          /* parked parent coroutine, or NULL */
    jinn_scope_t     *prev;            /* enclosing scope (nesting stack) */
    jinn_coro_t      *children[JINN_SCOPE_MAX_ACTORS]; /* live child coros */
    int               child_count;
    void             *actors[JINN_SCOPE_MAX_ACTORS];   /* scope-owned mailboxes */
    int               actor_count;
};

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

jinn_scope_t *jinn_scope_current(void) {
    return tl_scope;
}

void jinn_scope_set_current(jinn_scope_t *s) {
    tl_scope = s;
}

jinn_scope_t *jinn_scope_create(void) {
    jinn_scope_t *s = (jinn_scope_t *)jinn_xmalloc(sizeof(jinn_scope_t));
    atomic_store(&s->live_children, 0);
    atomic_store(&s->cancelled, 0);
    atomic_store(&s->lock, 0);
    s->parent = NULL;
    s->prev = tl_scope;
    s->child_count = 0;
    s->actor_count = 0;
    tl_scope = s;
    return s;
}

void jinn_scope_register_child(jinn_coro_t *child) {
    jinn_scope_t *s = tl_scope;
    if (!s || !child) return;
    child->scope = s;
    atomic_fetch_add(&s->live_children, 1);
    scope_lock(s);
    if (s->child_count < JINN_SCOPE_MAX_ACTORS) {
        s->children[s->child_count++] = child;
    }
    /* Inherit cancellation if the scope is already cancelled. */
    if (atomic_load_explicit(&s->cancelled, memory_order_acquire)) {
        atomic_store_explicit(&child->cancelled, 1, memory_order_release);
    }
    scope_unlock(s);
}

void jinn_scope_add_actor(jinn_scope_t *s, void *mailbox_ptr) {
    if (!s || !mailbox_ptr) return;
    scope_lock(s);
    if (s->actor_count < JINN_SCOPE_MAX_ACTORS) {
        s->actors[s->actor_count++] = mailbox_ptr;
    }
    scope_unlock(s);
}

void jinn_scope_child_done(jinn_scope_t *s) {
    if (!s) return;
    int64_t remaining = atomic_fetch_sub(&s->live_children, 1) - 1;
    if (remaining <= 0) {
        scope_lock(s);
        jinn_coro_t *p = s->parent;
        s->parent = NULL;
        scope_unlock(s);
        if (p) {
            p->state = JINN_CORO_READY;
            jinn_sched_enqueue(p);
        }
    }
}

void jinn_scope_cancel(jinn_scope_t *s) {
    if (!s) return;
    if (atomic_exchange_explicit(&s->cancelled, 1, memory_order_acq_rel)) {
        return; /* already cancelled — idempotent */
    }
    scope_lock(s);
    int n = s->child_count;
    jinn_coro_t *snapshot[JINN_SCOPE_MAX_ACTORS];
    for (int i = 0; i < n; i++) snapshot[i] = s->children[i];
    int na = s->actor_count;
    void *actors[JINN_SCOPE_MAX_ACTORS];
    for (int i = 0; i < na; i++) actors[i] = s->actors[i];
    scope_unlock(s);

    /* Mark every live child cancelled. */
    for (int i = 0; i < n; i++) {
        if (snapshot[i]) {
            atomic_store_explicit(&snapshot[i]->cancelled, 1, memory_order_release);
        }
    }
    /* Cancellation closes scope-owned actor mailboxes too: a cancelled actor
     * unwinds at its next receive rather than draining. Closing wakes any
     * parked receive so the unwind can proceed. */
    for (int i = 0; i < na; i++) {
        jinn_actor_stop(actors[i]);
    }
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
    int na = s->actor_count;
    void *actors[JINN_SCOPE_MAX_ACTORS];
    for (int i = 0; i < na; i++) actors[i] = s->actors[i];
    scope_unlock(s);
    for (int i = 0; i < na; i++) {
        jinn_actor_stop(actors[i]);
    }
}

void jinn_scope_join(jinn_scope_t *s) {
    if (!s) return;

    for (;;) {
        if (atomic_load_explicit(&s->live_children, memory_order_acquire) <= 0) {
            break;
        }
        jinn_worker_t *w = tl_worker;
        if (!w || !w->current) {
            /* Parent is *main / a non-coroutine thread: spin-yield so the
             * scheduler can run children to completion. */
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
        scope_unlock(s);
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — re-check live_children. */
    }

    /* Pop scope, restore the enclosing one, and free. */
    tl_scope = s->prev;
    free(s);
}
