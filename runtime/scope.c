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
    _Atomic(int32_t)  has_error;       /* first-error-wins latch (0/1) */
    _Atomic(int64_t)  error_val;       /* the recorded error (one machine word) */
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
    atomic_store(&s->has_error, 0);
    atomic_store(&s->error_val, 0);
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

/*
 * jinn_actor_spawn_scoped: called at actor spawn, before sched_spawn.
 * If a `together` scope is current, the actor becomes a *scope-owned*
 * non-daemon child: it is registered with the scope (so the scope join waits
 * for it) and its mailbox is tracked (so scope exit closes it = stop-and-drain).
 * Outside any scope, the actor is a daemon, fire-and-forget, exactly as before.
 */
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

    /* Mark every live child cancelled, and wake any blocked on a channel so
     * they resume, observe cancellation, and unwind at their next check. */
    for (int i = 0; i < n; i++) {
        if (snapshot[i]) {
            atomic_store_explicit(&snapshot[i]->cancelled, 1, memory_order_release);
            void *wc = snapshot[i]->wait_chan;
            if (wc) {
                jinn_chan_wake_coro((jinn_chan_t *)wc, snapshot[i]);
            }
        }
    }
    /* Cancellation closes scope-owned actor mailboxes too: a cancelled actor
     * unwinds at its next receive rather than draining. Closing wakes any
     * parked receive so the unwind can proceed. */
    for (int i = 0; i < na; i++) {
        jinn_actor_stop(actors[i]);
    }
}

/* Record a child's propagated error on the scope. First-error-wins (E2): a
 * CAS on `has_error` ensures only the first caller stores `error_val`. The
 * recording child then cancels the scope (E1) so siblings unwind. */
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

/* Record an error on the scope owning the *currently running* coroutine.
 * Called from a scope task's top frame when an error propagates out of it. */
void jinn_scope_record_current_error(int64_t errval) {
    jinn_worker_t *w = tl_worker;
    if (w && w->current && w->current->scope) {
        jinn_scope_record_error((jinn_scope_t *)w->current->scope, errval);
    }
}

/* If an error was recorded on this scope, write it to *out and return 1. */
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
    int na = s->actor_count;
    void *actors[JINN_SCOPE_MAX_ACTORS];
    for (int i = 0; i < na; i++) actors[i] = s->actors[i];
    scope_unlock(s);
    for (int i = 0; i < na; i++) {
        jinn_actor_stop(actors[i]);
    }
}

/* Wake any cancelled child currently parked on a channel so it resumes,
 * observes cancellation, and unwinds. Idempotent and cheap; called from the
 * join loop to close the race where a child parks just after cancellation
 * marked it but before it published its `wait_chan`. */
static void scope_wake_cancelled(jinn_scope_t *s) {
    if (!atomic_load_explicit(&s->cancelled, memory_order_acquire)) return;
    scope_lock(s);
    int n = s->child_count;
    jinn_coro_t *snapshot[JINN_SCOPE_MAX_ACTORS];
    for (int i = 0; i < n; i++) snapshot[i] = s->children[i];
    scope_unlock(s);
    for (int i = 0; i < n; i++) {
        jinn_coro_t *c = snapshot[i];
        if (c && c->state == JINN_CORO_SUSPENDED) {
            void *wc = c->wait_chan;
            if (wc) jinn_chan_wake_coro((jinn_chan_t *)wc, c);
        }
    }
}

static int jinn_scope_join_no_free(jinn_scope_t *s) {
    for (;;) {
        if (atomic_load_explicit(&s->live_children, memory_order_acquire) <= 0) {
            break;
        }
        scope_wake_cancelled(s);
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

    return atomic_load_explicit(&s->has_error, memory_order_acquire) ? 1 : 0;
}

void jinn_scope_join(jinn_scope_t *s) {
    if (!s) return;
    jinn_scope_join_no_free(s);
    /* Pop scope, restore the enclosing one, and free. */
    tl_scope = s->prev;
    free(s);
}

/* Join, then take any recorded error before freeing the scope. Returns the
 * recorded error word, or INT64_MIN if no child propagated an error. */
int64_t jinn_scope_join_take_error(jinn_scope_t *s) {
    if (!s) return INT64_MIN;
    int had = jinn_scope_join_no_free(s);
    int64_t word = had ? atomic_load_explicit(&s->error_val, memory_order_acquire)
                       : INT64_MIN;
    tl_scope = s->prev;
    free(s);
    return word;
}
