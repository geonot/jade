# Structured concurrency: the `together` scope

**Status: implemented.** The runtime lives in `runtime/scope.c`; the
`together` construct, cancellation, and error propagation all work. This
document is the specification they are checked against.

This document specifies Jinn's structured-concurrency model: a lexical
**scope** construct that owns the concurrent work started inside it, joins
that work on exit, propagates errors from children to the scope owner, and
gives cancellation a precise, cooperative meaning. It replaces
fire-and-forget actors and unblocks the actor-supervision integration
deferred from the error model (`docs/error-effects.md` §8).

It is layered *on top of* — and changes nothing in — the canonical shutdown
contract in [docs/concurrency.md](concurrency.md). Every rule there
(`active_coros`, daemon actors, drain-then-close channels, the `*main`
epilogue) remains exactly as tested by `tests/concurrency_shutdown.rs`.
Structured concurrency is opt-in: code that never writes a scope behaves
byte-for-byte as today.

---

## 1. Principles

1. **Concurrency has a lexical home.** Work started inside a scope cannot
   outlive the scope. When control leaves the block, every child has
   finished — normally, by error, or by cancellation. No leaked tasks, no
   `usleep` synchronization.
2. **The common case is silent.** "Do these things concurrently, wait for
   all of them" is one keyword and indentation. No handles, no join calls,
   no ceremony — the intelligent-compiler convention applies.
3. **Errors flow up, cancellation flows down.** A child that fails cancels
   its siblings and re-raises at the scope, through the ordinary error
   machinery (implicit propagation, `From` conversion, quaternary
   handling). There is no second error system.
4. **Cancellation is cooperative and value-shaped.** A cancelled child
   unwinds at its next suspension point via the existing early-return path
   (defers run, Perceus drops are emitted). No signals, no unwinding
   exceptions, no preemption.
5. **`stop` and `close` keep their meanings.** `close ch` closes a channel;
   `stop handle` closes an actor mailbox; `stop scope` cancels a scope.
   One family, one mental model: a one-way "no more input" valve.

---

## 2. Surface syntax

### 2.1 The scope block

```jinn
together
    dispatch
        index(files)
    dispatch
        fetch(remote)
log "both done"            # statically true: the scope joined
```

`together` opens a scope. The indented body runs on the current coroutine.
Every `dispatch` and `spawn` executed *while the scope is the innermost
enclosing scope* (dynamically — including inside functions called from the
body) registers its coroutine as a **child** of the scope. When the body
falls off the end, the scope **joins**: the parent parks until every child
has completed. Only then does execution continue past the block.

A scope may be named, like a named `dispatch`:

```jinn
together workers
    for shard in shards
        dispatch
            crunch(shard)
    if overload()
        stop workers       # cancel: see §4
```

The name is a binding (`is`-style, scope-local) whose only operation is
`stop`. Scopes nest; children belong to the innermost scope.

### 2.2 Actors inside a scope

```jinn
together
    w is spawn Worker      # scope-owned: NOT a daemon
    w.work(10)
    w.work(32)
# scope exit: mailbox auto-closed, actor drains, then joins
```

`spawn` inside a scope produces a **scope-owned actor**: a *non-daemon*
child whose mailbox the scope automatically closes (`stop`) when the body
ends. Because a message actor drains its mailbox before exiting
(`docs/concurrency.md`, "Message actors"), scope exit *is* the
first-class **stop-and-drain** the review asked for: every message sent
before the end of the block is processed before control passes the block.

An explicit `stop w` inside the body is still allowed and still means
"close the mailbox now"; the scope's auto-`stop` at exit is idempotent
(`jinn_chan_close` already is).

`spawn` *outside* any scope is unchanged: a daemon, fire-and-forget,
never awaited — full backward compatibility.

### 2.3 Grammar

```
scope_stmt := "together" [ident] NEWLINE INDENT block DEDENT [quat_arms]
```

`together` is a new keyword token. The optional trailing `quat_arms` are
the standard multiline quaternary arms (§5). `stop` extends from actor
handles to scope names; no other grammar changes.

---

## 3. Child lifetime rules

L1. **Registration.** A coroutine created by `dispatch`/`spawn` while a
    scope is current becomes a child of the innermost current scope. The
    "current scope" is carried on the coroutine (a runtime field), so
    helper functions called from the body register their dispatches too.

L2. **Join on exit.** Control leaves a `together` block only after every
    child has completed. Completion means the child coroutine returned —
    normally, with a propagated error, or by cancellation unwind.

L3. **No escape.** A child cannot be moved out of its scope. Binding a
    dispatch (`gen is dispatch ...`) inside a scope and returning `gen`
    from the enclosing function is a compile error ("coroutine cannot
    outlive its `together` scope") — the same escape analysis used for
    borrowed views applies.

L4. **Scope-owned actor teardown.** At normal body exit the scope first
    `stop`s every scope-owned actor (close mailbox), then joins all
    children. Actors therefore drain in parallel with other children
    finishing.

L5. **Nesting.** A child that opens its own `together` joins its own
    children before it completes; lifetimes form a tree. The outer scope
    never observes a half-finished inner scope.

L6. **`*main`.** Unchanged. The `*main` epilogue (`jinn_sched_run` waiting
    on `active_coros`) is the implicit root join for non-daemon work, as
    today. Scope children are non-daemon coroutines, so they are counted
    by `active_coros` exactly like plain dispatches; the scope join merely
    happens earlier and lexically.

---

## 4. Cancellation

A scope becomes **cancelled** when either:

- a child completes with an error (§5), or
- `stop <scope-name>` executes (from the body or from any child).

Cancellation is **cooperative** and propagates **down** the scope tree:
cancelling a scope marks every live child (and, transitively, the children
of their nested scopes) as cancelled.

C1. **Cancellation points.** A cancelled coroutine unwinds at its next
    *suspension point*: channel park (`send` on full, `receive` on empty),
    `yield`, scheduler yield in an actor `*loop` tick, or sleep. Already
    *parked* coroutines are woken immediately (the same wake-all used by
    `jinn_chan_close`).

C2. **Unwind semantics.** Unwinding reuses the error model's early-return
    path: `defer`s run, Perceus drops are emitted, the coroutine returns.
    It behaves as if the built-in error `Cancelled` were raised at the
    suspension point. `Cancelled` is not catchable and never enters a
    function's inferred error union — it is the runtime tearing the child
    down, not a value the program handles. Cleanup belongs in `defer`.

C3. **Scope-owned actors under cancellation.** Cancellation closes the
    mailbox *and* marks the actor cancelled: unlike a plain `stop`, a
    cancelled message actor does **not** drain remaining messages — it
    unwinds at its next receive. (Plain `stop` + normal exit still
    drains; cancellation is the fast path, `stop` is the graceful path.)

C4. **Channels are not closed by cancellation.** A channel is an ordinary
    value with its own `close`; cancellation does not reach into user
    data. Parked operations on it are woken with their existing
    closed-style results only for the unwinding coroutine (the park
    returns control so the unwind can proceed), other users of the channel
    are unaffected.

C5. **The non-yielding child, restated.** Cancellation cannot interrupt a
    child that never reaches a suspension point. This is the same sharp
    edge as `docs/concurrency.md` §"Sharp edges" item 2, and the same fix:
    long-running loops must yield. A cancelled scope whose child spins
    forever joins forever.

C6. **`stop` is idempotent and monotone.** Cancelling an already-cancelled
    scope, or `stop`ping an already-stopped actor, is a no-op.

---

## 5. Error propagation from children

This section composes with the locked error model
([docs/error-effects.md](error-effects.md)); none of its rules change.

E1. **A failing child cancels its siblings and re-raises at the scope.**
    If a child's body propagates an error out of its top frame, the scope
    records it, cancels (§4), joins the remaining children, and then the
    `together` block itself produces that error.

E2. **First error wins.** The first recorded error is the scope's result;
    errors from siblings that fail *while unwinding from cancellation* are
    dropped. (Deterministic aggregation is a possible future extension;
    first-wins matches Trio/Kotlin and keeps the error type a single
    union, not a list.)

E3. **The scope is a fallible expression.** The error union of a
    `together` block is the union of the error unions of all lexically
    reachable child bodies (computed with the same SCC least-fixpoint
    machinery as function inference, `docs/error-effects.md` §7). The
    block participates in implicit propagation: with no handler, a failing
    scope makes the enclosing function fallible and re-raises, with `From`
    conversion applied as usual (R1/R3 checked at the scope site).

E4. **Local handling is the standard quaternary**, in its multiline form
    with the block as subject:

```jinn
together
    dispatch
        fetch(a)           # ! NetError
    dispatch
        save(b)            # ! FileError
    !! log(err)            # err as union; siblings already cancelled+joined
```

    The `!!` arm runs after the join, with `err` bound to the first error.
    A `?` arm is permitted (`$` is Unit) but rarely useful; the `!` arm is
    meaningless for scopes and rejected.

E5. **Actor handlers propagate to their scope.** A fallible message or
    `*loop` handler in a *scope-owned* actor that propagates an error
    completes the actor child with that error — triggering E1. This is the
    supervision boundary promised in `docs/error-effects.md` §8. A
    *daemon* actor (spawned outside any scope) with a fallible handler
    keeps today's behavior: the error terminates that actor coroutine
    silently (it has no parent to inform) — the compiler emits a warning
    suggesting a scope.

E6. **Checking.** All existing rules apply unchanged: the scope's
    produced union must convert into the enclosing function's declared
    `! E` (R1), `err`-raises inside child bodies check against the child
    body's own context (R6), and `match`/quaternary exhaustiveness is
    untouched.

---

## 6. How `stop` and `close` compose with scopes

| Statement        | Target          | Meaning                                          |
| ---------------- | --------------- | ------------------------------------------------ |
| `close ch`       | channel         | unchanged: close, wake waiters, drain-then-EOS   |
| `stop w`         | actor handle    | unchanged: close mailbox; actor drains and exits |
| `stop workers`   | scope name      | **new**: cancel the scope (§4)                   |

Composition rules:

S1. `stop` on a scope-owned actor inside its scope is graceful early
    shutdown: drain semantics, the scope join then sees it finish.
S2. Scope cancellation (`stop workers` / a failing child) overrides drain:
    cancelled actors exit at the next receive without draining (C3).
S3. Closing a channel that scope children are parked on is the normal way
    to let children finish: receive drains then reports end-of-stream,
    the child's loop ends, the child completes, the join proceeds. The
    all-parked deadlock of `docs/concurrency.md` becomes a *scope* hang —
    same cause, same cure (close what you are done with), but now the hang
    is lexically attributable to one block.
S4. `stop` of an outer scope from inside an inner one cancels the outer,
    which transitively cancels the inner (§4); the inner join then
    completes by unwind.

---

## 7. Worked example

```jinn
err FetchError
    Timeout

*mirror(urls as [String]) returns i64 ! FetchError
    done is channel of i64(64)
    together pool
        agg is spawn Counter            # scope-owned, auto stop+drain
        for u in urls
            dispatch
                b is fetch(u)           # ! FetchError: failure cancels pool
                agg.add(b.len())
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

One block expresses: bounded parallel fetches, a scope-owned aggregating
actor with guaranteed drain, early cancellation on quota, and typed error
propagation (`FetchError` flows out of `mirror` implicitly).

---

## 8. Runtime and lowering sketch

Minimal additions, reusing existing machinery:

- `jinn_scope_t` in the runtime: `{ atomic live_children; atomic cancelled;
  first_error slot (CAS); parked-parent wake; owned-actor list }`.
- `jinn_coro_t` gains two words: `scope` (current scope pointer, inherited
  at spawn) and `cancelled` flag. Park loops in `runtime/channel.c` and the
  yield path in `runtime/sched.c` check `cancelled` and return a distinct
  "unwind" status.
- `together` lowers to `scope_create`; each child spawn registers
  (`live_children++`, child.scope set); child epilogue does
  `scope_child_done(scope, maybe_error)` which decrements, CASes the first
  error, cancels on error, and wakes the parent at zero; block end lowers
  to `stop`-owned-actors, `scope_join`, then either fall-through or the
  error early-return path already generated for quaternaries.
- The child top frame is compiled like a fallible function whose
  propagation target is the scope slot — the same auto-wrap/early-return
  codegen as `docs/error-effects.md`, retargeted.
- Escape analysis (L3) extends the existing borrowed-view escape checker.

No changes to `active_coros` accounting, the worker loop, the `*main`
epilogue, or any channel/actor C ABI symbol. All seven
`tests/concurrency_shutdown.rs` tests must pass unchanged.

## 9. Relation to `supervisor`

The parser already accepts a `supervisor` declaration (name, strategy
`one_for_one | one_for_all | rest_for_one`, children); it is dormant.
Supervisors are the *restart* layer and will be specified as sugar over
scopes: a supervisor is a long-lived `together` whose error handler
restarts children per strategy instead of re-raising. That design follows
implementation of this document and is out of scope here.

## 10. Conformance plan

To be added to `tests/concurrency_shutdown.rs` (or a sibling
`tests/structured_concurrency.rs`) when implemented:

| Test                                  | Asserts                                                        |
| ------------------------------------- | -------------------------------------------------------------- |
| `scope_joins_all_dispatches`          | Code after `together` observes all child effects.              |
| `scope_owned_actor_drains_on_exit`    | Messages sent before block end are processed; no sleep needed. |
| `failing_child_cancels_siblings`      | Sibling parked on a channel unwinds; scope re-raises the error. |
| `scope_error_propagates_with_from`    | Child error converts via `From` into the enclosing `! E`.       |
| `stop_scope_cancels_without_drain`    | Cancelled actor skips remaining mailbox messages.              |
| `defer_runs_on_cancellation`          | A cancelled child's `defer` executes.                          |
| `daemon_spawn_outside_scope_unchanged`| `spawn` at top level still daemon; program exits as today.     |
| `coroutine_escape_is_compile_error`   | Returning a scope-bound dispatch fails to compile (L3).        |
