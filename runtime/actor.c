/*
 * Jade Runtime — Actor helpers for scheduler integration.
 *
 * Actors run as coroutines. The mailbox uses scheduler-aware parking
 * instead of pthread condvars.
 */
#include "jade_rt.h"
#include <stdlib.h>

/*
 * jade_actor_park: park the current coroutine on a mailbox.
 * Called from actor loop when mailbox is empty.
 * The mailbox has a pointer to the actor coroutine at a known offset.
 * We just suspend the current coroutine.
 */
void jade_actor_park(void *mailbox_ptr) {
    (void)mailbox_ptr;
    jade_worker_t *w = tl_worker;
    if (!w || !w->current) return;
    jade_coro_t *self = w->current;
    self->state = JADE_CORO_SUSPENDED;
    self->wait_chan = mailbox_ptr;
    jade_context_swap(&self->ctx, &w->sched_ctx);
    /* Resumed when a message is sent to this actor */
}

/*
 * jade_actor_wake: wake a coroutine parked on a mailbox.
 * Called from the send path after enqueuing a message.
 * The coroutine pointer is stored in the mailbox struct.
 */
void jade_actor_wake(void *mailbox_ptr) {
    (void)mailbox_ptr;
    /* The actual wake logic is in the generated code since it knows
     * the mailbox layout. This function is a hook for the runtime
     * if needed. For now, the compiler emits the wake inline. */
}
