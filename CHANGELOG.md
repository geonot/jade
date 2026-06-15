# Changelog
- **[101]** (2026-06-15 14:05) driver: add explicit `jinn compile <file>` subcommand; rename toolchain CLI name to jinn
- **[100]** (2026-06-15 12:53) scope pillar foundation (task 2-32-3-1): src/pkgid.rs interned PkgId/ScopePath + side table per scope.md §1.1
- **[99]** (2026-06-15 05:52) scope pillar foundation (task 2-32-3-1): src/pkgid.rs — interned PkgId/ScopePath handles + side table per scope.md §1.1; PackageRecord{name,owner_scope,version,semantic_hash}; path-scoped `foo:baz:bar` rendering, version-distinct identities, hash-prefix mangling, single-package root fast path. 7 tests.
- **[98]** (2026-06-15 12:49) caps pillar (task 2-32-2): lattice + SSOT cap-sites table + SCC-fixpoint derivation pass + `needs` clause; resolves lamp prereq Q1/Q6/Q7
- **[98]** (2026-06-15 05:49) caps pillar landed (task 2-32-2): src/caps.rs lattice + src/cap_sites.rs SSOT table + src/typer/caps.rs SCC-fixpoint derivation pass + `needs` clause (fn/method); inferred-by-default, annotation = checked upper bound; derived⊑declared with call-path diagnostic; empty-row promotion rule. 21 tests, zero warnings.
- **[95]** (2026-06-15 11:31) lamp spec: deployments (§9) + two-axis output model (kind lib/bin/app/os/raw × substrate runtimeless/threadless/heapless)
- **[93]** (2026-06-15 11:10) lamp: decided top-to-bottom spec — separate cap pass, scope identity, interface v2 + hash ladder, config-block sugar (resolves Q1-Q9)
- **[88]** (2026-06-15 10:24) docs: lamp compiler prerequisites (lamp-prereqs.md) + task 2-32 tree
- **[87]** (2026-06-15 10:12) lamp: add §5.7 path-scoped identity, hash unification, and the compatibility ladder
- **[86]** (2026-06-15 08:38) lamp.md: specify transitive-dependency model for .jnb binaries (§5.6) — packed-by-reference + dedup, ABI-pinned MVS, used-surface hashing, requires-isolated escape hatch; reconcile §3.4/§5.2/§11/§12/§13
- **[85]** (2026-06-15 08:16) lamp spec rev2: project.lock, .jnb, multi-output + members, libjn dereferenced, TUF/FFI/MVS-lock fixes
- **[84]** (2026-06-15 05:19) docs: add idiomatic.md (anti-pattern catalog) + fmt.md (jinn fmt+lint spec)
- **[82]** (2026-06-15 05:04) docs(lamp): comprehensive technical spec for the Jinn package system (task 2-19 design)
- **[80]** (2026-06-14 03:31) 2-6-5-2: scope task records propagated error; together surfaces it (E1/E5) + conformance tests
- **[79]** (2026-06-14 02:42) 2-6-5-1: runtime scope error slot (first-error-wins CAS + take); decompose 2-6-5 into subtasks
- **[77]** (2026-06-14 02:24) 2-6-4: cooperative cancellation — stop <scope>, channel + back-edge cancellation points, defer-on-cancel cleanup, runtime wake, conformance tests + docs
- **[75]** (2026-06-13 22:42) task 2-6-3: together happy-path lowering (scheduler tasks, captures, scope-owned actor drain, join)
- **[73]** (2026-06-13 22:00) task 2-6-1/2-6-2: together scope surface + runtime jinn_scope_t layer
- **[70]** (2026-06-13 21:44) tasks 2-9/2-7/2-10: join primitive, stop-and-drain semantics, cooperative preemption yield injection
- **[65]** (2026-06-13 21:03) store: declarative decorator table shared by parser/typer/docs (task 2-31-12)
- **[65]** (2026-06-13 14:01) store: declarative decorator table (src/store_decorators.rs) shared by parser/typer/docs; rejects nonsensical combos (@kv+@vector, @mem+@versioned, @unique on f64, @increment on non-numeric) with precise diagnostics (task 2-31-12)
- **[64]** (2026-06-13 18:27) store: compact statement + @compact(threshold) auto-policy for tombstone reclamation (task 2-31-10)
- **[61]** (2026-06-13 18:06) store: persistent secondary indexes with fingerprint validation + rebuild-on-corruption (task 2-31-9)
- **[59]** (2026-06-13 17:44) Task 2-31-8: richer store where filters (in, between, contains/starts_with/ends_with, grouping)
- **[58]** (2026-06-13 17:16) store: add group-by aggregates (store.group(key).agg(val)) returning Vec<(key,val)> tuples; single-pass hashtable codegen, 6 tests (task 2-31-7)
- **[55]** (2026-06-13 16:58) 2-31-6: @kv schema-driven key/value types with optional get
- **[53]** (2026-06-13 16:45) Implement belongs-to relation traversal in stores (task 2-31-5): &owner as Owner becomes a stored I64 sid column, row.owner resolves the target row via the primary sid index; has-many emits a steering diagnostic; @cascade parses as a no-op. Adds tests/store_relations.rs conformance suite.
- **[52]** (2026-06-13 16:21) task 2-31-4: typed result sets for history/distinct/nearest/graph/search (replace opaque i64 handles with iterable Vecs)
- **[51]** (2026-06-13 09:20) task 2-31-4: typed result sets replace opaque i64 handles for store methods. `history(sid)` -> `[Row]` (Vec<Struct<__store_NAME>>, iterable + field access); `distinct(field)` -> `[FieldType]`; `vector.nearest(q,k)` -> `[(i64, f64)]` (sid + Euclidean distance score, new jinn_vec_nearest_scored runtime); `graph.from(n)`/`graph.to(n)` -> `[NeighborType]` (collected opposite-endpoint values, not a count); FTS `search(field,q)` -> `[i64]` doc-ids (new jinn_fts_search_ids_n runtime). ABI: iterable list = Type::Vec(elem) materialized as heap __vec_header{ptr,len,cap} pointer. Also fix latent tuple-index typing bug: `t[1]` on a heterogeneous tuple now resolves the correct element type for constant indices (was always element 0). Conformance: 8 new/updated tests in tests/integration.rs (history/distinct/nearest/graph/fts typed iteration). 1766 pass / 0 fail, zero new warnings.
- **[50]** (2026-06-13 15:06) task 2-31-3: schema fingerprint in store header + migration enforcement

Extend store header 24->40B: append 8B FNV-1a schema fingerprint
(field names/types/order/decorators + store decorators) and 8B schema
version; count@8/rec_size@16 offsets preserved. On reopen, emit
jinn_store_check_schema: match -> proceed, legacy 0 -> stamp+proceed,
mismatch w/o bridging migration -> abort with precise diagnostic.
Migrations stamp the new fingerprint (jinn_store_stamp_schema) and run
under jinn_migration_enter/leave so they bypass the open-time check.
Update all header-rewriting paths (create, hard-delete, migrate
add/drop). Fix latent cross-binary migration segfault: gen_migration
opens store via real __store_<name>_ensure_open. Conformance:
tests/store_schema.rs (3 tests). 1762 pass / 0 fail.
- **[47]** (2026-06-13 14:51) remediation task 2-14: adversarial memory-model soundness fuzzer (tests/ownership_fuzz.rs) — generates random ownership-stressing programs (nested take, field/heap moves under control flow, container-read aliasing, copy, rebind-after-move); asserts clean rejection OR no abort/segfault/ICE; integrates with ASan sweep to catch UAF/double-free in Perceus+escape+tombstones. 1759 tests green.
- **[45]** (2026-06-13 14:48) remediation task 2-13: numeric-coercion property suite (tests/coercion_property.rs) — 6 properties compile at -O0 AND -O3 and assert runtime output == Rust reference, pinning the inference->lowering coercion boundary (int var/literal -> f64 param, width coercion, chained float coercion)
- **[42]** (2026-06-13 14:44) remediation: fix 5 crash-safety soundness bugs (task 2-30) + complete StoreError @unique/@required error-model integration (task 2-31-2). String slice/char_at bounds checks, oversized-shift trap, Result-with-err-enum monomorphization (Param->Enum annotation resolution), take-in-loop double-free now a compile error. Stale s247 snippet de-keyworded. All 1752 tests green, zero warnings.
- **[41]** (2026-06-11 12:05) task 2-31-1: Real WAL-backed store transactions — begin snapshots data files + WAL offsets, commit group-fsyncs as one durable batch, escaping errors/traps roll back data files, WAL, and index/column/fts state; nested txns join outermost; +9-test conformance suite tests/store_transactions.rs; documented in jinn.md
- **[39]** (2026-06-10 11:35) Add tests/crash_safety.rs: crash/trap/invalid-access/overwrite/unsafe-edge-case conformance suite (32 passing tests + 5 ignored tests pinning soundness bugs filed as task 2-30)
- **[38]** (2026-06-10 11:03) Design structured concurrency: docs/structured-concurrency.md (together scopes, scope-owned actors, cancellation, error propagation) + cross-links (task 2-5)
- **[36]** (2026-06-10 10:56) Document all remaining P1 tasks (2-5..2-22) against review findings with verified current-state probes, gap analysis, and definitions of done; complete P0 sweep tasks 2-23..2-29
- **[28]** (2026-06-10 10:45) Pin P0 safety-floor conformance (tasks 2-23..2-29): div/mod-by-zero + INT_MIN/-1 traps, vec OOB diagnostic, generator IR, take-in-arg, bare-toplevel/missing-main rejection, contextual keywords after Dot; add 11 hand-crafted broken-MIR unit tests to mir::verify
- **[27]** (2026-06-08 03:12) Task 2-8: observable send-after-close (send yields bool; docs+tests)
- **[22]** (2026-06-08 03:04) Complete checked error model: err-raise/quaternary/propagation end-to-end; fix fallibility-inference infinite loop, Ok/Err mono collision (task 4), R3 non-fallible propagation; migrate corpus off prefix-! and ?>
- **[19]** (2026-06-06 07:29) task 2-4-4: typer quaternary lowering + fallibility inference

Lower Expr::Quaternary (and !!-less Ternary over Result/Option) to a Block
evaluating the subject once then an EnumIs-gated ternary. Bind $ to the
unwrapped success value (Placeholder intercepted via dollar_stack; suppress
placeholder-currying) and err to the unwrapped error. !! err and the bare
default propagate via propagate_err_value with From X->E conversion into the
enclosing Result (R1/R3 diagnostics). Implicit propagation: a bare fallible
bind in a fallible-capable fn desugars to f() ? $ !! err, gated off variant
ctors, annotated binds, and non-fallible fns (R4: main).

Migrate statement/bind ?/!/!! parsing to the pratt quaternary; delete legacy
finish_bare_* and bind handler-chains; fix nested boolean ternary in arms.
Add 11 quaternary conformance tests in tests/error_effects.rs; mark the
deprecated prefix-! raise corpus #[ignore] (task 2-4-7). Discovered and filed
task 4: Ok/Err ctor monomorphization-collision with multiple Result
instantiations (pre-existing, independent of this work).
- **[19]** (2026-06-06 00:40) task 2-4-4: typer quaternary lowering + fallibility. Lower `Expr::Quaternary` (and `!!`-less Ternary over Result/Option) to a `Block` evaluating the subject once then an `EnumIs`-gated ternary; bind `$` to the unwrapped success value (intercept `Placeholder` via `dollar_stack`, suppress placeholder-currying) and `err` to the unwrapped error. `!! err` / bare `!!`-less default propagates via `propagate_err_value` (From-converts `X -> E` into the enclosing Result, R1/R3 diagnostics). Implicit propagation: a bare fallible bind in a fallible-capable fn (`v is f()`) desugars to `f() ? $ !! err`; gated off variant ctors, annotated binds, and non-fallible fns (R4: `main`). Migrate statement/bind `?`/`!`/`!!` parsing to the pratt quaternary (delete legacy `finish_bare_*` + bind handler-chains); fix nested boolean ternary in arms. Add 11 quaternary conformance tests; mark deprecated prefix-`!`-raise corpus `#[ignore]` (task 2-4-7).
- **[18]** (2026-06-06 06:50) task 2-4-3: parser quaternary `e ? ok ! nothing !! err` — add Expr::Quaternary AST node, parse inline + multiline arms (Quaternary iff `!!` present, else Ternary), structural arms in fmt/resolve/scc/implicit/decl, typer stub for 2-4-4, 5 parser tests
- **[15]** (2026-06-06 05:34) task 2-4-2: remove ?> propagation; err-raise + !! lex/parse intact
- **[11]** (2026-06-04 12:45) Implement Option/Result prelude combinator surfaces (task 2-2)

Add full combinator method surfaces shared by typer+codegen for the canonical
prelude Option of T and Result of T,E per docs/error-effects.md §2:
Option{is_some,is_none,unwrap,unwrap_or,map,and_then,ok_or},
Result{is_ok,is_err,unwrap,unwrap_or,map,map_err,and_then,ok,err}.

Combinators desugar to EnumIs/EnumUnwrap/VariantCtor/IndirectCall via Ternary,
monomorphizing target enums through the existing enum mono scheme. Extend the
type-annotation parser to accept multi-arg generics (Result of T, E) in
returns/bind positions via a context flag, avoiding the param-list comma
ambiguity. Add tests/error_effects.rs conformance suite.
- **[9]** (2026-06-04 12:34) docs: checked error-effect system design spec (task 2-1)
- **[8]** (2026-06-04 12:30) Pin String UTF-8 semantics: scalar .length, byte .byte_count, spec + conformance tests; fix raw-string lexer double-encoding
- **[6]** (2026-06-04 12:15) Shared builtin-method registry: single source of truth for typer+codegen (Vec/Map/String); fix chain/flatten/enumerate/keys/values drift
- **[3]** (2026-06-04 11:56) traits: synthesize default trait method bodies into impls; add tests/traits.rs conformance suite
- **[1]** (2026-06-04 11:50) P0(1-1): clippy hard gate in CI (-D warnings), fix all 784 clippy warnings + approx_constant error, clean store/wal artifacts from tree
