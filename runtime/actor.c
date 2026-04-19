/*
 * Jade Runtime — Actor helpers.
 *
 * Actors are coroutines that receive messages via a typed channel.
 * The mailbox layout is: { ptr channel, i32 alive, ... state fields }.
 * The compiler generates actor loop, spawn, and send inline.
 * These helpers handle stop/destroy lifecycle.
 */
#include "jade_rt.h"
#include <stdlib.h>

/*
 * jade_actor_park: park the current coroutine (legacy, unused by channel path).
 * Kept for ABI stability.
 */
void jade_actor_park(void *mailbox_ptr) {
    (void)mailbox_ptr;
    jade_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jade_coro_t *self = w->current;
    self->state = JADE_CORO_SUSPENDED;
    self->wait_chan = mailbox_ptr;
    jade_context_swap(&self->ctx, &w->sched_ctx);
}

/*
 * jade_actor_wake: wake a coroutine parked on a mailbox (legacy, unused).
 */
void jade_actor_wake(void *mailbox_ptr) {
    (void)mailbox_ptr;
}

/*
 * jade_actor_stop: stop an actor by closing its channel.
 * The channel pointer is at offset 0 of the mailbox struct.
 */
void jade_actor_stop(void *mailbox_ptr) {
    jade_chan_t *ch = *(jade_chan_t **)mailbox_ptr;
    if (ch) {
        jade_chan_close(ch);
    }
}

/*
 * jade_actor_destroy: fully clean up an actor's resources.
 * Called after the actor loop has exited (channel drained/closed).
 * Closes the channel (if not already closed), destroys it, and frees the mailbox.
 */
void jade_actor_destroy(void *mailbox_ptr) {
    if (!mailbox_ptr) return;
    jade_chan_t *ch = *(jade_chan_t **)mailbox_ptr;
    if (ch) {
        /* Close if not already closed, then destroy */
        jade_chan_close(ch);
        jade_chan_destroy(ch);
        *(jade_chan_t **)mailbox_ptr = NULL;
    }
    free(mailbox_ptr);
}
