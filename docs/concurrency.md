# Concurrency — tasks, channels, actors, scopes, and shutdown

The canonical contract for how Jinn programs run concurrently and — more
importantly — how they *stop*. Shutdown is the part of a concurrency model that
is easiest to get subtly wrong, so the rules below are stated precisely and each
one is exercised by a passing test in
[`../tests/concurrency_shutdown.rs`](../tests/concurrency_shutdown.rs).

Everything here is derived from the runtime and codegen as they exist, not from
aspiration:

| Concern | Source |
| --- | --- |
| Scheduler | [`../runtime/sched.c`](../runtime/sched.c) |
| Channels | [`../runtime/channel.c`](../runtime/channel.c) |
| Actors | [`../runtime/actor.c`](../runtime/actor.c), [`../src/codegen/actors.rs`](../src/codegen/actors.rs) |
| Scopes | [`../runtime/scope.c`](../runtime/scope.c) |
| `close` / `stop` lowering | [`../src/codegen/mir_codegen/magic.rs`](../src/codegen/mir_codegen/magic.rs) |
| `*main` epilogue | [`../src/codegen/mir_codegen/mod.rs`](../src/codegen/mir_codegen/mod.rs) |

Data that crosses a task boundary obeys the ownership rules in
[`memory-model.md`](memory-model.md) §5: aggregates **move** into a task or a
message, and a second capture is a compile error.

## Execution model

Jinn runs on an **M:N work-stealing scheduler**: a small pool of OS worker
threads (one per CPU, capped at 8) multiplexes an unbounded number of
lightweight **coroutines**. A coroutine is a stackful green thread; it runs
until it returns, yields, or *parks* (blocks on a channel), at which point
control swaps back to the worker, which finds other work.

```
*main thread ──► jinn_sched_init ──► run *main body ──► jinn_sched_run ──► jinn_sched_shutdown
                                          │
                                          ├─ dispatch ──► coroutine ──► worker pool ◄─► work-stealing deques
                                          └─ spawn    ──► actor coroutine + mailbox channel
```

Two kinds of work land on the scheduler:

| Construct | Coroutine kind | Counts toward `active_coros`? |
| --- | --- | --- |
| `dispatch` inside a `together` scope | **non-daemon** (scope-owned) | **yes** |
| `spawn Actor` inside a `together` scope | **non-daemon** (scope-owned) | **yes** |
| `spawn Actor` outside any scope | **daemon** | **no** |

> A `dispatch` *outside* any `together` scope is a compile error — a task
> needs a scope to run in, so the compiler rejects the silent-no-op instead
> of creating a coroutine nothing ever schedules. (A *bound* dispatch is a
> generator and is legal anywhere.)

This distinction is the single most important fact about Jinn shutdown.

### Coroutine lifecycle and `active_coros`

The scheduler keeps a global count, `active_coros`, of *non-daemon* coroutines
that have been spawned but not yet finished. Non-daemon coroutines increment it
on spawn and decrement it on completion; `jinn_sched_run()` blocks `*main` until
the count reaches zero.

Daemon coroutines never touch the count. They are fire-and-forget in the
sense that `jinn_sched_run()` does not wait for them — but before the
scheduler runs, the `*main` epilogue calls `jinn_actor_stop_all`, which
closes every live daemon mailbox and waits (bounded, a few seconds at most)
for already-enqueued messages to drain. So queued sends to a top-level actor
are normally delivered at exit; the drain is best-effort and bounded, not a
guarantee — `stop`/`join` explicitly when delivery matters.

## Channels

A channel is a **bounded MPMC FIFO**. Capacity is rounded up to a power of two
(a capacity of `0` becomes the default, 64). All buffer access happens under a
small atomic spinlock, so channels are safe to drive both from coroutines (which
park when full or empty) and from raw OS threads (which spin).

```jinn
ch is channel(4)            # untyped, capacity 4
ch is channel of i64(1024)  # typed element, capacity 1024

send ch, value              # parks while the buffer is full
v is receive ch             # parks while the buffer is empty
close ch                    # mark the channel closed
```

| Operation | Runtime symbol | Blocking behaviour |
| --- | --- | --- |
| `send ch, v` | `jinn_chan_send` | Parks while full. Yields `false` and discards `v` if the channel is closed (`N-11`: a heap payload is currently leaked, not dropped); `true` once delivered. |
| `receive ch` | `jinn_chan_recv` | Parks while empty. Drains buffered values even after close. |
| `close ch` | `jinn_chan_close` | Idempotent; sets the closed flag and wakes all parked waiters. |

The two closed-channel behaviours are load-bearing for clean shutdown:

1. **Send after close is observable, never fatal.** `send` is an expression
   yielding a `bool`: `true` when the value was enqueued, `false` when the
   channel was already closed and the value dropped. No panic, no lost-value
   ambiguity — a producer can branch on the result to learn the consumer is
   gone. Used as a bare statement the boolean is ignored, so fire-and-forget
   producers need no ceremony.
2. **Receive drains, then signals end-of-stream.** `receive` keeps returning
   buffered values until the buffer is empty, and only then — empty *and* closed
   — reports end-of-stream. No message enqueued before the close is lost.

## Actors

An actor is a coroutine plus a **typed mailbox channel**. The channel pointer
lives at offset 0 of the mailbox struct, so "the actor" and "its mailbox" are
interchangeable for shutdown purposes.

```jinn
actor Worker
    sum                     # state field

    @work n                 # message handler
        sum is sum + n

*main
    w is spawn Worker       # coroutine + mailbox channel
    w.work(10)              # enqueue a `work` message (async send)
    w.work(32)
    stop w                  # close the mailbox; the actor drains, then exits
    join w                  # wait for the handler loop to finish
    0
```

- `spawn Actor(...)` allocates the mailbox, binds initial state fields, creates
  the coroutine, and enqueues it. Outside a scope it is marked **daemon**;
  inside a `together` scope it is a scope-owned non-daemon child.
- `handle.method(args)` is an **asynchronous send**: it packs the arguments into
  a message and enqueues it. It does not wait for the handler to run.
- `stop handle` closes the mailbox. For a message actor this is
  **stop-and-drain**: the receive loop delivers every message enqueued before
  the `stop`, *then* exits. `stop` is graceful, not a hard kill — no enqueued
  message is dropped.
- `join handle` parks the caller until the mailbox is closed **and** the handler
  loop has fully exited, waiting on a one-shot completion latch at the tail of
  the mailbox struct. It is idempotent: once the actor is done the latch stays
  set. The usual graceful shutdown is `stop w` then `join w`.

  > **Footgun:** never `join` an actor from inside one of *its own* handlers —
  > the handler is part of the loop `join` waits to exit, so it deadlocks. This
  > is not statically detected; keep `join` on the owning side.

### Message actors versus loop actors

The actor loop is generated in one of two shapes:

- **Message actor** (no `*loop` handler): a **blocking** receive loop. Each
  iteration receives, dispatches on the message tag, and exits on end-of-stream.
  Because receive drains first, a message actor processes every message enqueued
  before `stop`, then exits cleanly.
- **Loop actor** (`*loop` handler, optionally `*loop <ms>`): a **polling** loop.
  Each iteration runs the `*loop` body, yields or sleeps, then does a
  non-blocking receive — got a message, dispatch; empty but open, loop again;
  closed, exit. A loop actor does periodic work *and* services messages.

Both shapes converge on the same exit path: signal the completion latch so any
pending `join` wakes, then close and *retire* the mailbox channel — the
actual free is deferred to scheduler shutdown, so a racing waiter never
touches freed memory — and return.

## Structured concurrency — the `together` scope

`together` opens a lexical scope that owns the concurrent work started inside
it. It layers on top of everything above without changing any of it: code that
never writes a scope behaves byte-for-byte as before.

```jinn
together
    dispatch
        index(files)
    dispatch
        fetch(remote)
log('both done')            # statically true: the scope joined
```

The indented body runs on the current coroutine. Every `dispatch` and `spawn`
executed while the scope is the innermost enclosing scope — dynamically,
including inside functions called from the body — registers its coroutine as a
**child**. When the body falls off the end, the scope **joins**: the parent
parks until every child has completed.

A scope may be named, and the name's only operation is `stop`:

```jinn
together workers
    for shard in shards
        dispatch
            crunch(shard)
    if overload()
        stop workers       # cancel
```

Scopes nest; children belong to the innermost scope.

### Actors inside a scope

```jinn
together
    w is spawn Worker      # scope-owned: NOT a daemon
    w.work(10)
    w.work(32)
# scope exit: mailbox auto-closed, actor drains, then joins
```

A scope-owned actor is a non-daemon child whose mailbox the scope closes at body
end. Because a message actor drains before exiting, scope exit *is* a
first-class stop-and-drain: every message sent before the end of the block is
processed before control passes the block. An explicit `stop w` inside the body
still means "close the mailbox now"; the scope's auto-stop is idempotent.

### Child lifetime rules

- **L1 — Registration.** A `dispatch` lexically inside the `together` body
  becomes a child of that scope (nested dispatches inside a child body
  register into the same scope); `spawn` registers dynamically (the runtime
  reads the current scope), so helper functions register their *spawns* but
  **not** their dispatches — a `dispatch` in a helper called from the body
  is rejected at compile time; give the helper its own `together`.
- **L2 — Join on exit.** Control leaves a `together` block only after every
  child has completed — normally, with a propagated error, or by cancellation
  unwind. An explicit `return` inside the body is a compile error (it would
  skip the join).
- **L3 — No escape.** A child cannot be moved out of its scope. Ordinary
  lexical scoping plus the `return` rejection enforce this; returning a
  *bound* dispatch (a generator value) is not specifically rejected.
- **L4 — Actor teardown.** At normal body exit the scope first `stop`s every
  scope-owned actor, then joins all children, so actors drain in parallel with
  other children finishing.
- **L5 — Nesting.** A child that opens its own scope joins its own children
  before it completes; lifetimes form a tree, and the outer scope never observes
  a half-finished inner scope.
- **L6 — `*main`.** Unchanged. The `*main` epilogue is the implicit root join
  for non-daemon work; the scope join merely happens earlier and lexically.

### Cancellation

A scope becomes **cancelled** when a child completes with an error, or when
`stop <scope-name>` executes from the body *or from inside a child task*
(the scope pointer rides the child's capture block). Cancellation recurses
into nested scopes: cancelling a scope marks its direct children and cancels
every child scope, so a grandchild parked in an inner join is released when
its own children unwind.

- **C1 — Cancellation points.** A cancelled coroutine unwinds at its next
  channel or select suspension point (`send` on full, `receive` on empty).
  Already-parked coroutines are woken immediately. Channel `send`/`receive`
  check the cancelled flag at the top of their park loop *and* just before
  committing to park, which closes the lost-wakeup race under the channel
  lock. Loop back-edges are cancellation points in every scheduler task.
  **Gap (`N-6r`):** sleeps (currently a raw `nanosleep` on the worker
  thread), IO parks, and joins are *not* cancellation points.
- **C2 — Unwind semantics.** Unwinding reuses the error model's early-return
  path: `defer`s run, drops are emitted, the coroutine returns. It behaves as if
  a built-in `Cancelled` error were raised at the suspension point. `Cancelled`
  is not catchable and never enters a function's inferred error union — it is
  the runtime tearing the child down, not a value the program handles. Cleanup
  belongs in `defer`.
- **C3 — Actors under cancellation.** Cancellation closes the mailbox *and*
  marks the actor cancelled: unlike a plain `stop`, a cancelled message actor
  does **not** drain — it unwinds at its next receive.
- **C4 — Channels are untouched.** A channel is ordinary user data with its own
  `close`; cancellation does not reach into it. Parked operations are woken with
  closed-style results only for the unwinding coroutine.
- **C5 — The non-yielding child.** Cancellation cannot interrupt a child that
  never reaches a suspension point. A cancelled scope whose child spins forever
  joins forever.
- **C6 — Idempotent and monotone.** Cancelling an already-cancelled scope, or
  stopping an already-stopped actor, is a no-op.

Two limitations are documented rather than silently mishandled: `defer`s whose
cleanup references values defined inside a loop are not materialised at the
synthesized cleanup block, and a coroutine parent parked in the scope join
relies on the direct wake from cancel (the join-loop re-wake only runs when the
parent is `*main`). Neither affects the common
`together`/`dispatch`/`stop` shape; both are tracked as `N-1` and `N-2` in
[`roadmap.md`](roadmap.md#concurrency).

### Error propagation from children

This composes with the error model in [`error-effects.md`](error-effects.md);
none of its rules change.

- **E1 — A failing child cancels its siblings and re-raises at the scope.** The
  scope records the error, cancels, joins the remaining children, and then the
  `together` block itself produces that error.
- **E2 — First error wins.** Errors from siblings that fail *while unwinding
  from cancellation* are dropped. This matches Trio and Kotlin and keeps the
  error type a single union rather than a list.
- **E3 — The scope is a fallible expression.** Its error union is the union of
  the error unions of all lexically reachable child bodies, computed with the
  same SCC least-fixpoint machinery as function inference. With no handler, a
  failing scope makes the enclosing function fallible and re-raises, with `From`
  conversion applied as usual.
- **E4 — Local handling is the standard quaternary**, in its multiline form with
  the block as subject:

  ```jinn
  together
      dispatch
          fetch(a)           # ! NetError
      dispatch
          save(b)            # ! FileError
      !! log(err)            # siblings already cancelled and joined
  ```

  The `!!` arm runs after the join with `err` bound to the first error. A `?`
  arm is permitted (`$` is Unit) but rarely useful; the `!` arm is
  meaningless for scopes and is rejected with a diagnostic.
- **E5 — Actor handlers must handle their errors locally.** Handler bodies
  are synthesized as non-fallible functions: an unhandled fallible call in a
  handler is a hard compile error, in both daemon and scope-owned actors.
  The intended design — a scope-owned actor's propagating handler completes
  the child with the error (triggering E1), a daemon's terminates silently
  with a warning — is not implemented (`N-9`).

### How `stop` and `close` compose

| Statement | Target | Meaning |
| --- | --- | --- |
| `close ch` | channel | Close, wake waiters, drain then end-of-stream. |
| `stop w` | actor handle | Close the mailbox; the actor drains and exits. |
| `stop workers` | scope name | Cancel the scope. |

One family, one mental model: a one-way "no more input" valve. `stop` on a
scope-owned actor inside its scope is graceful early shutdown with drain
semantics. Scope cancellation overrides drain. Closing a channel that scope
children are parked on is the normal way to let children finish. `stop` of an
outer scope from inside an inner one cancels the outer and, transitively,
the inner — both halves work: `stop` resolves from child tasks, and
cancellation recurses into nested scopes.

`supervisor` is parsed but dormant — it is the *restart* layer, and will be
specified as sugar over scopes (`N-4`).

### Worked example

```jinn
err FetchError
    Timeout

*mirror(urls as Vec of String) returns i64 ! FetchError
    done is channel of i64(64)
    together pool
        agg is spawn Counter            # scope-owned, auto stop-and-drain
        for u in urls
            dispatch
                b is fetch(u)           # failure cancels the pool
                agg.add(b.length)
                send done, 1
        dispatch
            n is 0
            for _ in urls
                receive done
                n is n + 1
                if n equals limit
                    stop pool           # enough: cancel the rest
    total()                             # statically: all work settled
```

One block expresses bounded parallel fetches, a scope-owned aggregating actor
with a guaranteed drain, early cancellation on quota, and typed error
propagation.

## Program termination

`*main` is compiled with a fixed epilogue:

1. `jinn_sched_init` — start the scheduler lazily; workers spin up on the first
   spawn.
2. Run the user `*main` body.
3. `jinn_actor_stop_all` — close every live daemon mailbox and wait (bounded,
   a few seconds at most) for pending message counts to reach zero: a
   best-effort drain, not a guarantee.
4. `jinn_sched_run()` — block until `active_coros == 0`, i.e. until every
   non-daemon coroutine has finished.
5. `jinn_sched_shutdown()` — set the shutdown flag, wake all workers,
   `pthread_join` them, then free retired mailboxes.
6. Return `*main`'s exit code.

A parked daemon actor is woken by its mailbox close, exits its loop cleanly,
and is reclaimed at shutdown. It does not prevent the program from exiting.

## Sharp edges

These follow from the rules above. They are sharp, not bugs — but they bite.

1. **Top-level actors get a bounded, best-effort drain, not a guarantee.**
   The epilogue's `jinn_actor_stop_all` normally delivers already-enqueued
   messages, but the wait is bounded and delivery is not a contract.
   Synchronize deliberately when it matters: put the actor in a `together`
   scope, or `stop` then `join` it, or have it send results back on a channel
   that `*main` receives. Sprinkling `usleep` to "let the actor catch up" is
   a smell, not a contract.
2. **Long loops yield automatically.** A coroutine swaps back to its worker only
   at a yield or park point, so the compiler inserts a scheduler yield at every
   loop **back-edge** inside scope tasks and actor loops (the `inject_yields`
   MIR pass; generators are not covered). This is a *correctness* pass — it runs at every optimization
   level, including none — and is justified at MIR level because LLVM has no
   notion of the scheduler. Injection only covers back-edges; it cannot rescue a
   genuinely infinite loop, which is a logic bug. Opt a hot kernel out with
   `@no_yield`:

   ```jinn
   @no_yield
   *crunch xs
       # tight numeric kernel; no scheduler yields inserted
   ```
3. **The all-parked deadlock.** `jinn_sched_run` returns only when
   `active_coros` reaches zero. If every non-daemon coroutine parks on a channel
   that will never receive, `*main` hangs. Close channels you are done with, and
   make sure some non-daemon coroutine can always make progress. Inside a scope
   the same hang becomes lexically attributable to one block.
4. **Send after close discards the value — but tells you.** Treat
   `close`/`stop` as a one-way valve: once shut, producers have no effect.
   Bind the result (`delivered is send ch, v`) if you need to know whether
   your value landed. (`N-11`: the discarded heap payload is currently
   leaked, not dropped, and select's send-arm on a closed channel reports
   success.)

## Conformance

Every rule above is backed by an end-to-end test that compiles and runs a real
program through `jinnc`, in
[`../tests/concurrency_shutdown.rs`](../tests/concurrency_shutdown.rs):

| Test | Asserts |
| --- | --- |
| `channel_fifo_roundtrip` | `send`/`receive` preserve FIFO order and lose nothing. |
| `channel_capacity_one_interleaved` | A capacity-1 channel forces strict ping-pong without loss. |
| `channel_receive_drains_buffer` | Every value enqueued before `close` is still delivered. |
| `send_after_close_is_observable` | `send` yields `true` open, `false` after `close`. |
| `bare_send_ignores_result` | A bare `send` statement stays fire-and-forget. |
| `actor_processes_then_stops` | After `stop`, the program joins its workers and exits 0. |
| `loop_actor_stop_drains_all_messages` | `stop` is stop-and-drain for a loop actor. |
| `join_after_stop_completes` | `join` parks until the handler loop has fully exited. |
| `join_twice_is_idempotent` | A second `join` on a finished actor returns immediately. |
| `tight_loop_actor_does_not_starve_siblings` | Injected back-edge yields prevent starvation. |
| `actor_without_stop_still_exits` | A parked daemon actor does not block program exit. |
| `scope_joins_all_dispatches` | `together` joins every child before control leaves the block. |
| `scope_owned_actor_drains_on_exit` | A `spawn` inside `together` drains at block exit. |
| `daemon_spawn_outside_scope_unchanged` | `spawn` outside any scope stays a daemon. |
| `stop_scope_cancels_without_drain` | `stop <scope>` cancels a child at its next park. |
| `defer_runs_on_cancellation` | A `defer` inside a cancelled task still runs. |

Multithreaded MPMC stress, crash consistency, and tail-latency characterization
live in [`../tests/channel_stress.rs`](../tests/channel_stress.rs). Under the
sanitize job the C runtime is TSan-instrumented, so those tests double as the
channel's data-race check. Every context switch carries
`__tsan_switch_to_fiber` and `__sanitizer_start_switch_fiber` annotations
(the `jinn_coro_swap_*` helpers in `runtime/jinn_rt.h` are the only way the
runtime switches), so whole-program TSan and ASan results are meaningful
across coroutine migrations.
