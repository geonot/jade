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

struct jinn_scope {
    _Atomic(int64_t)  live_children;
    _Atomic(int32_t)  cancelled;
    _Atomic(int32_t)  lock;
    _Atomic(int32_t)  has_error;       /* first-error-wins latch (0/1) */
    _Atomic(int64_t)  error_val;       /* the recorded error (one machine word) */
    jinn_coro_t      *parent;          /* parked parent coroutine, or NULL */
    jinn_scope_t     *prev;            /* enclosing scope (nesting stack) */
    /* Live children / scope-owned mailboxes. Growable — a fixed cap used to
     * silently drop registrations past 64, so cancellation missed them
     * (task 8-12). Entries are guaranteed valid only while `lock` is held:
     * exit paths unregister under the lock before freeing, so every reader
     * (cancel, wake, stop) iterates under the lock rather than snapshotting.
     */
    jinn_coro_t     **children;
    int               child_count;
    int               child_cap;
    void            **actors;
    int               actor_count;
    int               actor_cap;
};

/* realloc-or-die, matching jinn_xmalloc's contract. */
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

/*
 * Both accessors are noinline on purpose, for the same reason as
 * jinn_worker_self (see jinn_rt.h): callers that resume after a
 * `jinn_context_swap` may be running on a different thread than the one that
 * parked them, so the TLS block address must be re-derived by the callee
 * rather than reused from a callee-saved register. Reading or writing
 * `tl_scope` directly from a function that spans a park would touch the
 * *parking* thread's slot.
 */
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
    /* Inherit cancellation if the scope is already cancelled. */
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

/*
 * Remove an exiting child from its scope's registry, BEFORE the child's
 * live_children decrement and destruction. Ordering matters twice over:
 * until the decrement, the parent's join cannot return, so the scope
 * itself is guaranteed alive here; and once removed under the lock, no
 * cancel/wake iteration can ever see the freed coroutine (they iterate
 * under the same lock). Fixes the freed-coroutine-in-run-queue crash the
 * 2026-07 review found (§7: cancel after siblings completed).
 */
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

/* Same discipline for scope-owned actor mailboxes: the actor's exit block
 * frees its mailbox (jinn_actor_destroy), so it must leave the registry
 * first or a later cancel would jinn_actor_stop() freed memory. */
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
    /* Mark every live child cancelled, and wake any blocked on a channel so
     * they resume, observe cancellation, and unwind at their next check.
     * Iterate UNDER the scope lock: registry entries are valid only while
     * the lock is held (an exiting child unregisters under the same lock,
     * so nothing here can touch a freed coroutine). The lock is a spinlock,
     * but the work per entry is a couple of atomic stores and at most a
     * channel-lock wake; the ordering scope-lock → channel-lock is used
     * nowhere in reverse. */
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
    /* Cancellation closes scope-owned actor mailboxes too: a cancelled actor
     * unwinds at its next receive rather than draining. Closing wakes any
     * parked receive so the unwind can proceed. */
    for (int i = 0; i < s->actor_count; i++) {
        jinn_actor_stop(s->actors[i]);
    }
    scope_unlock(s);
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
    /* Under the lock: see jinn_scope_cancel for the validity invariant. */
    scope_lock(s);
    for (int i = 0; i < s->actor_count; i++) {
        jinn_actor_stop(s->actors[i]);
    }
    scope_unlock(s);
}

/* Wake any cancelled child currently parked on a channel so it resumes,
 * observes cancellation, and unwinds. Idempotent and cheap; called from the
 * join loop to close the race where a child parks just after cancellation
 * marked it but before it published its `wait_chan`. */
static void scope_wake_cancelled(jinn_scope_t *s) {
    if (!atomic_load_explicit(&s->cancelled, memory_order_acquire)) return;
    /* Under the lock: see jinn_scope_cancel for the validity invariant. */
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
            break;
        }
        scope_wake_cancelled(s);
        /* Re-derived per iteration: parking below can resume us on another
         * worker, and `w->sched_ctx` must be *this* thread's scheduler. */
        jinn_worker_t *w = jinn_worker_self();
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
        /* Hand the scope lock to the scheduler: released only after this
         * context is saved, so jinn_scope_child_done cannot read `parent`
         * and enqueue us while the old context is still live (task 8-10;
         * the old code scope_unlock()ed here, before the swap). */
        w->held_lock = &s->lock;
        w->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &w->sched_ctx);
        /* Resumed — re-check live_children. */
    }

    return atomic_load_explicit(&s->has_error, memory_order_acquire) ? 1 : 0;
}

void jinn_scope_join(jinn_scope_t *s) {
    if (!s) return;
    jinn_scope_join_no_free(s);
    /* Pop scope, restore the enclosing one, and free. Via the setter, not
     * `tl_scope` directly: the join above may have parked us and resumed us on
     * a different thread. */
    jinn_scope_set_current(s->prev);
    free(s->children);
    free(s->actors);
    free(s);
}

/* Join, then take any recorded error before freeing the scope. Returns the
 * recorded error word, or INT64_MIN if no child propagated an error. */
int64_t jinn_scope_join_take_error(jinn_scope_t *s) {
    if (!s) return INT64_MIN;
    int had = jinn_scope_join_no_free(s);
    int64_t word = had ? atomic_load_explicit(&s->error_val, memory_order_acquire)
                       : INT64_MIN;
    /* Setter, not `tl_scope` directly — the join may have migrated us. */
    jinn_scope_set_current(s->prev);
    free(s->children);
    free(s->actors);
    free(s);
    return word;
}
