# Jinn Concurrency & Shutdown Semantics

This document is the canonical contract for how Jinn programs run
concurrently and — more importantly — how they *stop*. Shutdown is the
part of a concurrency model that is easiest to get subtly wrong, so the
rules below are stated precisely and each one is exercised by a passing
test in [tests/concurrency_shutdown.rs](tests/concurrency_shutdown.rs).

Everything here is derived from the runtime and codegen as they actually
exist today, not from aspiration:

- Scheduler — [runtime/sched.c](runtime/sched.c)
- Channels — [runtime/channel.c](runtime/channel.c)
- Actor helpers — [runtime/actor.c](runtime/actor.c)
- Actor lowering — [src/codegen/actors.rs](src/codegen/actors.rs)
- `close` / `stop` lowering — [src/codegen/mir_codegen/magic.rs](src/codegen/mir_codegen/magic.rs)
- `*main` epilogue — [src/codegen/mir_codegen/mod.rs](src/codegen/mir_codegen/mod.rs)

## Execution model

Jinn runs on an **M:N work-stealing scheduler**: a small pool of OS
worker threads (one per CPU, capped at 8) multiplexes an unbounded number
of lightweight **coroutines**. A coroutine is a stackful green thread; it
runs until it returns, yields, or *parks* (blocks on a channel), at which
point control swaps back to the worker, which finds other work.

```
*main thread ──► jinn_sched_init ──► run *main body ──► jinn_sched_run ──► jinn_sched_shutdown
                                          │
                                          ├─ spawn ──► coroutine ──► worker pool ◄─► work-stealing deques
                                          └─ actor  ──► daemon coroutine + mailbox channel
```

Two kinds of work land on the scheduler:

| Surface construct        | Coroutine kind | Counts toward `active_coros`? |
| ------------------------ | -------------- | ----------------------------- |
| `dispatch` / parallel block | **non-daemon** | **yes** |
| `spawn Actor`            | **daemon**     | **no** |

This distinction is the single most important fact about Jinn shutdown,
and it is covered next.

## Coroutine lifecycle and `active_coros`

The scheduler keeps a global count, `active_coros`, of *non-daemon*
coroutines that have been spawned but not yet finished
(see [runtime/sched.c](runtime/sched.c)). When a non-daemon coroutine
completes, the count is decremented; when it reaches zero the scheduler
signals that the program's concurrent work is done.

- **Non-daemon** coroutines (ordinary `spawn` of a dispatch block /
  parallel work) increment `active_coros` on spawn and decrement it on
  completion. `jinn_sched_run()` blocks `*main` until this count hits
  zero — i.e. it **waits for all non-daemon work to finish**.
- **Daemon** coroutines do *not* touch `active_coros`. They are
  fire-and-forget: the program may exit while they are still running or
  parked. **Every actor is spawned as a daemon**
  (`jinn_coro_set_daemon` is called on the actor coroutine in
  [src/codegen/actors.rs](src/codegen/actors.rs)).

The practical consequence: `jinn_sched_run()` does **not** wait for
actors. An actor that is parked waiting for its next message never blocks
program exit, and conversely the program will *not* automatically linger
to let an actor finish draining its mailbox.

## Channels

A channel is a **bounded multi-producer / multi-consumer FIFO**
([runtime/channel.c](runtime/channel.c)). Capacity is rounded up to a
power of two (a capacity of `0` becomes the default, 64). All buffer
access happens under a small atomic spinlock, so channels are safe to
drive both from coroutines (which park when full/empty) and from raw OS
threads (which spin).

```jinn
ch is channel(4)            # untyped, capacity 4
ch is channel of i64(1024)  # typed element, capacity 1024

send ch, value              # blocks/parks while the buffer is full
v is receive ch             # blocks/parks while the buffer is empty
close ch                    # mark the channel closed
```

| Operation        | Runtime symbol        | Blocking behaviour                                              |
| ---------------- | --------------------- | -------------------------------------------------------------- |
| `send ch, v`     | `jinn_chan_send`      | Parks while full. **Silently drops `v` if the channel is closed.** |
| `receive ch`     | `jinn_chan_recv`      | Parks while empty. Drains buffered values even after close.     |
| `close ch`       | `jinn_chan_close`     | Idempotent; sets the closed flag and wakes all parked waiters.  |

The two closed-channel behaviours are deliberate and load-bearing for
clean shutdown:

1. **Send after close is a silent no-op.** Once a channel is closed,
   `jinn_chan_send` returns without enqueuing. No panic, no error — the
   value is dropped. Producers therefore do not need to coordinate with
   the close; they simply stop having effect.
2. **Receive drains, then signals end-of-stream.** `jinn_chan_recv`
   keeps returning buffered values until the buffer is empty, and only
   *then* — when the channel is both empty and closed — does it report
   end-of-stream (a `0` return at the C ABI; the non-blocking
   `jinn_chan_try_recv` reports the same condition with a `-1`/`u32::MAX`
   sentinel). No message that was enqueued before the close is lost.

## Actors

An actor is a daemon coroutine plus a **typed mailbox channel**. The
channel pointer lives at offset 0 of the actor's mailbox struct
([runtime/actor.c](runtime/actor.c)), so "the actor" and "its mailbox
channel" are interchangeable for shutdown purposes.

```jinn
actor Worker
    sum                     # state field

    @work n                 # message handler
        sum is sum + n

*main
    w is spawn Worker       # daemon coroutine + mailbox channel
    w.work(10)              # enqueue a `work` message (async send)
    w.work(32)
    stop w                  # close the mailbox channel
```

- `spawn Actor(...)` allocates the mailbox, binds initial state fields,
  creates the coroutine, marks it **daemon**, and enqueues it on the
  scheduler ([src/codegen/actors.rs](src/codegen/actors.rs)).
- `handle.method(args)` is an **asynchronous send**: it packs the
  arguments into a message and enqueues it on the mailbox channel. It
  does not wait for the handler to run.
- `stop handle` lowers to `jinn_chan_close` on the mailbox channel
  (`__stop` in [src/codegen/mir_codegen/magic.rs](src/codegen/mir_codegen/magic.rs)).
  It is exactly "close the actor's mailbox".

### Message actors vs loop actors

The actor loop is generated in
[src/codegen/actors.rs](src/codegen/actors.rs) in one of two shapes:

- **Message actor** (no `*loop` handler): a **blocking** receive loop.
  Each iteration calls `jinn_chan_recv`; a received message is dispatched
  on its tag; an end-of-stream result (channel closed *and* drained)
  exits the loop. Because receive drains first, **a message actor
  processes every message that was enqueued before `stop`, then exits
  cleanly.**
- **Loop actor** (`*loop` handler, with optional `*loop <ms>` sleep): a
  **polling** loop. Each iteration runs the `*loop` body, yields (or
  sleeps `ms`), then does a **non-blocking** `jinn_chan_try_recv`:
  `got` → dispatch, `empty-but-open` → loop again, `closed` → exit. A
  loop actor performs periodic work *and* services messages, and exits
  when its mailbox is closed.

Both shapes converge on the same exit path: when the loop ends they call
`jinn_actor_destroy(mailbox)` (close + destroy the channel + free the
mailbox), and the coroutine returns.

## Program termination

`*main` is compiled with a fixed epilogue
([src/codegen/mir_codegen/mod.rs](src/codegen/mir_codegen/mod.rs)):

1. `jinn_sched_init` — start the scheduler (lazily; workers spin up on the
   first spawn).
2. Run the user `*main` body.
3. `jinn_sched_run()` — **block until `active_coros == 0`**, i.e. until
   every *non-daemon* coroutine has finished. Daemon actors are not
   awaited here.
4. `jinn_sched_shutdown()` — set the shutdown flag, wake all workers, and
   `pthread_join` them.
5. Return `*main`'s exit code.

Each worker loop checks the shutdown flag at the top of every iteration
([runtime/sched.c](runtime/sched.c)). When a daemon actor is parked on
its mailbox at shutdown, control has already swapped back to the worker;
the worker observes the flag and exits, abandoning the parked actor
(process teardown reclaims it). This is correct and intended: a parked
actor blocked on `receive` does **not** prevent the program from exiting.

## Sharp edges (read this before shipping a concurrent program)

These follow directly from the rules above. They are sharp, not bugs —
but they are the things that bite.

1. **Actors are not awaited.** Because actors are daemon coroutines,
   `*main` returning will tear the program down *without* waiting for an
   actor to finish processing its mailbox. If you need an actor's work to
   be observable, you must synchronize — typically by having the actor
   send results back on a channel that `*main` `receive`s, or by closing
   the mailbox (`stop`) and giving the daemon a chance to drain before
   `*main` returns. Sprinkling `usleep` to "let the actor catch up" (as
   several example apps do) is a smell, not a contract.
2. **A non-yielding handler wedges its worker.** A coroutine only yields
   control at a yield/park point. A handler (or `*loop` body) that spins
   forever without yielding never swaps back to the worker, so the worker
   never observes the shutdown flag, so `jinn_sched_shutdown`'s
   `pthread_join` hangs. Every long-running handler must yield.
3. **The all-parked deadlock.** `jinn_sched_run` only returns when
   `active_coros` reaches zero. If every *non-daemon* coroutine parks on a
   channel that will never receive a value (e.g. mutually waiting
   producers/consumers), `active_coros` never decrements and `*main`
   hangs at `jinn_sched_run`. Close channels you are done with, and make
   sure some non-daemon coroutine can always make progress.
4. **Send after close is silent.** Sending to a closed channel (or a
   stopped actor) drops the value with no diagnostic. Treat `close`/`stop`
   as a one-way valve: once shut, producers have no effect.

## Testing

Every rule above is backed by an end-to-end test that compiles and runs a
real Jinn program through `jinnc`, in
[tests/concurrency_shutdown.rs](tests/concurrency_shutdown.rs):

| Test                                | Asserts                                                                 |
| ----------------------------------- | ----------------------------------------------------------------------- |
| `channel_fifo_roundtrip`            | `send`/`receive` preserve FIFO order and lose nothing.                  |
| `channel_capacity_one_interleaved`  | A capacity-1 channel forces strict ping-pong without loss.              |
| `channel_receive_drains_buffer`     | Every value enqueued before `close` is still delivered by `receive`.    |
| `actor_processes_then_stops`        | After `stop`, the program joins its workers and exits 0 — no hang.      |
| `actor_without_stop_still_exits`    | A daemon actor parked on `receive` does not block program exit.         |

The multithreaded MPMC stress, crash-consistency, and tail-latency
characterization for channels lives separately in
[tests/channel_stress.rs](tests/channel_stress.rs); under the `sanitize`
CI job the C runtime is TSan-instrumented, so those tests double as the
channel's data-race check.
