# Persistent Store: Review & 12 Improvements

A review of Jinn's first-class persistent store feature — the `store` declaration,
its decorators (`@simple`, `@mem`, `@kv`, `@graph`, `@vector(n)`, `@timeseries`,
`@versioned`, `@column`, hooks), field attributes (`@index`, `@unique`, `@bloom`,
`@search`, `@cascade`, ...), the statement surface (`insert` / `set` / `delete` /
`destroy` / `restore` / `save` / `transaction`), query blocks
(`where` / `sort` / `take` / `skip` / `limit` / `select` / `delete`), `view`
definitions, and `migration` blocks.

Implementation footprint: ~7.9k LOC of compiler support
(`src/parser/decl/actor_store.rs`, `src/parser/stmt/store.rs`,
`src/typer/call/store_methods.rs`, `src/mir/lower/store_{expr,stmt}.rs`,
`src/codegen/stores/*`, `src/codegen/mir_codegen/store{,_ext}/*`) plus ~1.7k LOC
of C runtime (`runtime/wal.c`, `kv.c`, `column.c`, `index.c`, `bloom.c`,
`migrate.c`, `vector.c`, `sqlite.c`).

The overall shape is strong and very Jinn: declarative schemas, the compiler
doing the heavy lifting, zero ORM ceremony. The WAL has a real durability model
(fdatasync-by-default, documented sync policies). The weaknesses are mostly in
semantic depth: transactions that don't roll back, untyped `i64` result handles,
and a query surface that stops short of relations and grouping.

---

## 1. Make `transaction` actually transactional

**Today:** `transaction` blocks lower to `__txn_begin` / `__txn_commit`, and in
`src/codegen/mir_codegen/magic.rs` both are literal no-ops (`const_int(0)`).
Inserts inside a transaction are individually durable; a panic or early return
mid-block leaves a partial commit. The keyword currently promises atomicity it
does not deliver.

**Improve:** Implement real begin/commit/rollback on top of the existing WAL.
`__txn_begin` records the WAL offset and switches the store to group-commit
mode; `__txn_commit` calls `jinn_wal_commit_group()` and applies buffered
mutations; unwinding (or a `fail` propagating out of the block) truncates the
WAL back to the saved offset and discards in-memory effects. This composes with
the error model: `transaction` becomes an effect boundary that rolls back on
any escaping error.

## 2. Typed result sets instead of opaque `i64` handles

**Today:** Many surfaces in `src/typer/call/store_methods.rs` type their result
as raw `Type::I64`: `history(sid)`, `search(field, q)`, `vector.nearest(...)`,
`graph.from(...)` / `graph.to(...)`, `distinct(field)`. The programmer receives
a number that is secretly a handle or count, losing all type safety and
composability (can't iterate, can't `.name` it, can't pass it to generic code).

**Improve:** Introduce a proper `Rows<StoreName>` result type (the typer already
synthesizes per-store record types for `where` results). `history` returns
`[UserRecord]`, `nearest` returns `[(sid, score as f64)]`, `search` returns
rows. This is the single highest-leverage change for making stores feel like a
real part of the language rather than an FFI veneer.

## 3. Surface `@unique` / `@required` violations through the error model

**Today:** Constraint failure behavior is implicit and undocumented — inserts
that violate `@unique` either silently succeed, silently skip, or abort,
depending on the codegen path. Nothing flows through Jinn's `err` /
quaternary-handling error model.

**Improve:** Declare a built-in `err StoreError` (`Duplicate`, `Missing`,
`Constraint`, `Io`) and make `insert` / `set` fallible expressions that
participate in the standard quaternary / `else` handling. Idiomatic Jinn means
the compiler checks that constraint failures are either handled or explicitly
propagated — silent data loss is never an option.

## 4. Relations are parsed but not queryable — implement traversal

**Today:** `parse_store_field` accepts `&owner as Owner` (relations) and
`&items as [Item]` (has-many), and `@cascade` exists, but there is no join or
traversal surface: you cannot write `order.customer.name` or query across
stores.

**Improve:** Implement relation traversal in query blocks and field access on
result rows: `o.customer` resolves the foreign sid through the target store's
primary index; has-many fields yield typed row sets. `@cascade` then gets real
semantics on `delete`/`destroy`. Without this, the relation syntax is a trap —
either make it work or remove it from the grammar.

## 5. Group-by and aggregate combinations in query blocks

**Today:** Aggregations are single-shot store methods (`sum`, `avg`, `min`,
`max`, `count`, `distinct`) over whole stores; query blocks support filtering
and sorting but no grouping. Computing "average age per city" requires manual
loops.

**Improve:** Add `group <field>` as a query clause, with aggregate expressions
in `select`:

```
r is users query
    where age > 18
    group city
    select city, avg(age), count
```

The columnar runtime (`runtime/column.c`) is already positioned to make this
fast for `@column` stores.

## 6. Vacuum / compaction for soft-deleted rows

**Today:** `delete` is a soft delete (tombstone via the built-in `deleted`
field) and `restore` resurrects rows, but nothing ever reclaims space. Files
and scans grow monotonically; `destroy` removes logically but the WAL and
record file retain history forever.

**Improve:** Add `compact StoreName` (statement or auto-policy decorator
`@compact(threshold)`) that rewrites the record file dropping tombstones older
than any live `@versioned` reference, then checkpoints and truncates the WAL.
This is the missing half of the soft-delete lifecycle.

## 7. Compile-time schema fingerprint + migration enforcement

**Today:** `migration "name" version N` blocks with `up`/`down` and
`add`/`drop`/`rename` ops exist, and `runtime/migrate.c` rewrites files. But
nothing ties the compiled schema to the on-disk file: change a field type and
rerun, and the binary will happily reinterpret old records with the new layout.

**Improve:** Embed a schema fingerprint (hash of field names, types, order,
decorators) in the store file header at creation. On open, compare; on
mismatch, look for a migration chain that bridges the versions and apply it,
otherwise fail with a precise diagnostic ("store Users on disk is v2
(name,age); binary expects v3 (+email) — no migration found"). This turns
"compile-time-checked persistent stores" from aspiration into guarantee.

## 8. Declarative durability instead of a global env var

**Today:** WAL sync policy is process-global, chosen by `JINN_WAL_SYNC` at
first open (`runtime/wal.c`). A program mixing a critical ledger store with a
throwaway cache store gets one policy for both, controlled outside the source.

**Improve:** Per-store decorators — `@durable` (fsync per entry, the default),
`@relaxed` (group commit), `@volatile` (no sync) — compiled into the store's
runtime registration. Keep the env var as an override for testing only.
Declaring durability where the data is declared is exactly the
intent-not-syntax philosophy.

## 9. Richer `where` filters: grouping, `in`, ranges, string operators

**Today:** `parse_store_filter` supports flat `field op value` chains joined by
`and`/`or` with no parenthesized grouping (so `a and (b or c)` is
inexpressible), and only the six comparison operators. No `in [..]`,
`between`, `contains`, `starts_with`, or case-insensitive match — despite
strings being first-class in stores and `@search`/FTS existing.

**Improve:** Reuse the full expression grammar inside query-block `where`
(already an `Expr` there) and extend the statement-form filter to parse a
proper boolean expression over fields, then compile it through the existing
`store_filter.rs` machinery with short-circuit index selection. Add `in`,
range, and string predicates that lower to index/bloom probes when available.

## 10. Persistent secondary indexes

**Today:** `@index`, `@sorted`, and `@bloom` structures live in
`runtime/index.c` / `bloom.c` but are rebuilt by scanning the record file at
startup. For the "millions of rows" use cases the decorators advertise, open
latency becomes O(data).

**Improve:** Persist index pages alongside the store file
(`name.store.idx`), updated through the same WAL entries so crash recovery
replays index mutations too. Validate against the schema fingerprint (see #7)
and fall back to a rebuild only on corruption. Opens become O(1).

## 11. Generalize `@kv` beyond `String -> i64`

**Today:** The `@kv` dispatch in `store_methods.rs` hard-codes keys as
`Type::String` and values as `Type::I64`. `kv.get` on a missing key returns an
indistinguishable `0`.

**Improve:** Let the store's declared fields define key/value types
(`key as String`, `val as f64` or `val as String`), infer the method
signatures from the schema, and type `get` as the standard optional/fallible
shape so missing keys are handled through normal `else` flow rather than a
sentinel zero. Single source of truth: schema drives the method surface for
typer and codegen alike.

**Done.** `store_methods.rs` derives `(key_ty, val_ty)` from the schema's
`key`/`val` fields (defaulting `String`/`i64`). `kv.get` now returns
`Option of <val_ty>`, so a missing key flows through `? $ ! ...`, `else`, or
`.unwrap_or(...)` rather than returning a sentinel `0`. Numeric value types are
supported: `i64` is stored directly and `f64` is bit-cast through the 8-byte
slot in codegen (`emit_kv_set`/`emit_kv_get`). Keys must be `String`;
non-numeric value types are rejected at compile time with a precise diagnostic
(string values await a runtime value-blob format). `incr`/`decr` are gated to
integer value types. The schema is the single source of truth shared by the
typer (method surface) and codegen (storage representation).

## 12. Consolidate decorator parsing into a declarative table

**Today:** `parse_store_def` and `parse_store_field` are long `if attr == "..."`
chains (~200 lines) duplicating knowledge that the typer and codegen also
encode (which decorators exist, their argument shapes, which combine legally).
Nothing rejects nonsensical combinations (`@kv` + `@vector(n)`,
`@mem` + `@versioned`, `@unique` on an `f64`).

**Improve:** Define one table — name, argument arity/type, applicability
(store vs field), mutual exclusions — consumed by the parser for syntax, the
typer for validation, and docs generation. Emit precise diagnostics for invalid
combinations at compile time. This follows the project convention that built-in
surfaces have a single source of truth shared by typer and codegen, and makes
adding decorator #13 a one-line change.

---

## Suggested priority

| Priority | Items | Rationale |
|---|---|---|
| Correctness first | 1, 3, 7 | Current behavior can silently lose or corrupt data |
| Language integrity | 2, 4, 11 | Untyped handles and dead syntax undermine the type system |
| Capability | 5, 9, 10 | Make the query layer earn the "first-class" claim |
| Operations | 6, 8 | Production lifecycle: space reclamation, declared durability |
| Hygiene | 12 | Pays down maintenance cost for everything above |
