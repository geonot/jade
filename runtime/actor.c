/*
 * Jade Runtime — Actor helpers.
 *
 * Actors are coroutines that receive messages via a typed channel.
 * The mailbox layout is: { ptr channel, i32 alive, ... state fields }.
 * The compiler generates actor loop, spawn, and send inline.
 * These helpers handle park/wake for the (unused) legacy path.
 */
#include "jade_rt.h"

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
