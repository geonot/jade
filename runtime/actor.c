/*
 * Jinn Runtime — Actor helpers.
 *
 * Actors are coroutines that receive messages via a typed channel.
 * The mailbox layout is: { ptr channel, i32 alive, ... state fields }.
 * The compiler generates actor loop, spawn, and send inline.
 * These helpers handle stop/destroy lifecycle.
 */
#include "jinn_rt.h"
#include <stdlib.h>
#include <stdio.h>

/*
 * jinn_actor_park: park the current coroutine (legacy, unused by channel path).
 * Kept for ABI stability.
 */
void jinn_actor_park(void *mailbox_ptr) {
    (void)mailbox_ptr;
    jinn_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jinn_coro_t *self = w->current;
    self->state = JINN_CORO_SUSPENDED;
    self->wait_chan = mailbox_ptr;
    jinn_context_swap(&self->ctx, &w->sched_ctx);
}

/*
 * jinn_actor_wake: wake a coroutine parked on a mailbox (legacy, unused).
 */
void jinn_actor_wake(void *mailbox_ptr) {
    (void)mailbox_ptr;
}

/*
 * jinn_actor_stop: stop an actor by closing its channel.
 * The channel pointer is at offset 0 of the mailbox struct.
 */
void jinn_actor_stop(void *mailbox_ptr) {
    jinn_chan_t *ch = *(jinn_chan_t **)mailbox_ptr;
    if (ch) {
        jinn_chan_close(ch);
    }
}

/*
 * Join primitive: a one-shot completion latch with a waiter queue.
 * Created lazily on first join; signalled once when the actor loop exits.
 */
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

/*
 * jinn_join_get: return the join latch stored at mailbox offset 3,
 * creating it on first access. mailbox layout:
 *   { ptr channel @0, i32 alive @1, <state> @2, ptr join @last }.
 * The compiler passes the address of the join slot.
 *
 * Lazy creation races: the joiner (e.g. *main) and the exiting actor's
 * signal path can both observe an empty slot concurrently. Publish via CAS
 * so both sides converge on a single latch — otherwise the signaller can
 * mark a latch the joiner never sees, and the joiner waits forever.
 */
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

/* Signal completion: mark done and wake all waiters. */
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

/* Park the caller until the actor's loop has exited (done latch set). */
void jinn_actor_join(void *join_slot_ptr) {
    jinn_join_t *j = jinn_join_get(join_slot_ptr);
    for (;;) {
        if (atomic_load_explicit(&j->done, memory_order_acquire)) {
            return;
        }
        /* Re-derived per iteration: parking below can resume us on another
         * worker, and `wk->sched_ctx` must be *this* thread's scheduler. */
        jinn_worker_t *wk = jinn_worker_self();
        if (!wk || !wk->current) {
            /* Non-coroutine context (e.g. *main): spin-yield to scheduler. */
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
        join_unlock(j);
        wk->last_action = SCHED_ACTION_PARK;
        jinn_context_swap(&self->ctx, &wk->sched_ctx);
        /* Resumed — re-check done. */
    }
}



/*
 * jinn_actor_destroy: fully clean up an actor's resources.
 * Called after the actor loop has exited (channel drained/closed).
 * Closes the channel (if not already closed), destroys it, and frees the mailbox.
 */
void jinn_actor_destroy(void *mailbox_ptr) {
    if (!mailbox_ptr) return;
    /* A scope-owned actor's mailbox is tracked in its scope's registry;
     * leave it before freeing or a later cancel/stop would close freed
     * memory (task 8-12). Runs on the actor's own coroutine (the exit
     * block), so the owning scope is reachable via the current coroutine
     * and — because our live-children slot is still counted — guaranteed
     * alive. */
    jinn_worker_t *w = jinn_worker_self();
    if (w && w->current && w->current->scope) {
        jinn_scope_unregister_actor((jinn_scope_t *)w->current->scope, mailbox_ptr);
    }
    jinn_chan_t *ch = *(jinn_chan_t **)mailbox_ptr;
    if (ch) {
        /* Close if not already closed, then destroy */
        jinn_chan_close(ch);
        jinn_chan_destroy(ch);
        *(jinn_chan_t **)mailbox_ptr = NULL;
    }
    free(mailbox_ptr);
}
