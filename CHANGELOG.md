# Changelog
- **[154]** (2026-08-14) types pass, part 2: generic types get one spelling — constructors stop baking unresolved inference variables into mono names, unification sees through monomorphized names, and `Pair<A, B>` finally has an annotation

Second slice of the Types section (T-1r's core closed, residue refiled as
T-1r2). Full suite is 2232 tests; fmt and clippy clean; the whole-corpus
ASan+LSan sweep stays at zero memory corruption.

- **A generic struct had at least three mutually non-unifying spellings.**
  `Box(value is 42)` minted its mono name *while the argument type was still
  an inference variable*, producing a struct literally named `Box_?0`; the
  instantiated `*unwrap(b as Box of T)` spelled the same type structurally
  as `Struct(Box, [i64])`; and an explicit annotation spelled it `Box_i64`.
  Codegen knew only one of the three — field reads through the others hit
  the *silent* `(i64, index 0)` fallback in field lookup and read garbage
  that was only correct when the field happened to be a leading i64. Now:
  constructors whose type arguments are still unresolved defer instantiation
  (typed structurally, field lookup falls back to the generic template) and
  the end-of-lowering canonicalization pass — which already did exactly this
  for generic *enums* — mints the mono name once the arguments resolve;
  every mint records its origin `(base, args)` in a table the unifier and
  generic-call type maps consult, so `Box_string` unifies with `Box of T`
  and maps `T := String` (previously `build_type_map` recognized only *bare*
  `T` parameters and silently defaulted everything else to i64 — a
  `Box of String` argument reached `unwrap__G_i64`). `collect_type_mapping`
  also learned the Struct/Tuple/Map/Array/View/Frozen/Channel arms it never
  had.
- **Multi-parameter generics had no expressible annotation.** In a parameter
  list, `p as Pair of A, B` parses as *two parameters* (`p as Pair of A`
  and an untyped `B`) — the comma is claimed by the list, so the two-argument
  form documented in jinn.ebnf's `generic_args` (`"<" type_expr {"," ...}
  ">"`) simply did not exist in the parser. It does now: `Pair<A, B>` in any
  type position, nested (`Pair<i64, Pair<i64, string>>` — a `>>` closer is
  split in place), with `Vec<T>`/`Map<K, V>`/`View<T>`/`Frozen<T>` accepted
  as the bracket spelling of the built-in sugar, and `jinn fmt` printing
  multi-argument generics back in bracket form (it used to print the
  unparseable `of` form; single-argument types keep `of`). `of` remains the
  single-argument style throughout.
- **Eagerly-instantiated methods no longer abort compiles they don't
  belong to.** Instantiating `Pair<i64, Pair<i64, string>>` also instantiated
  `sum_len` (which calls `.second.length`) and hard-failed the whole program
  even though nothing calls `sum_len` on that instantiation. Method-lowering
  failures now surface only for methods some call site actually names —
  C++-style: ill-typed instantiations of *unused* methods are not errors.
- Pinned in `tests/semantics_regression.rs` (spelling unification round-trip
  with a heap string, angle-bracket annotations incl. the `>>` split, generic
  enums through function boundaries) and `tests/programs/generic_containers.jn`
  joins the snapshot/differential/sanitizer corpora. The tour's Generics
  section documents the bracket form with a compiled example. Residue —
  phantom params still default silently, no expression-position `<...>` for
  return-only function generics, undefined generic bases in uncalled
  signatures pass silently, `Map<K, V>` is grammar-only until non-String
  keys exist — is T-1r2 in the roadmap.

- **[153]** (2026-08-14) types pass, part 1: tuples get one canonical layout — the phi-shape merge error and a silent heterogeneous-tuple corruption die together, unannotated signatures resolve before callers read them, and a type named `E` unifies with itself

First slice of the Types/inference/diagnostics section (T-10 closed, T-15
closed, T-1r reduced). Includes the compiler changes checkpointed in the
untagged `updates` commit (39c1a65). Full suite is 2229 tests; the
whole-corpus ASan+LSan sweep is now 1020 runs — **every corpus program
compiles for the first time** — with zero memory corruption; fib/spectral/csv
benchmarks are unchanged (checksums identical).

- **T-10's diagnostic was the visible edge of a silent wrong-code bug.**
  Codegen's `ArrayInit` built *every* tuple as an LLVM `[N x T]` array typed
  by the first element. For a heterogeneous tuple like `(Tok, i64)` that
  stores the i64 at the enum's unpadded size (offset 12) while the canonical
  `{ %Tok, i64 }` layout — used by returns, calls, and phis — reads it at
  offset 16, so every lexer-style `(token, position)` return came back as
  `position >> 32`, i.e. 0. Homogeneous tuples happened to have identical
  layouts, which is why the corpus never caught it: the only witness was
  `compiler_pipeline.jn`, which failed to *compile* on the downstream
  phi-shape mismatch (the T-10 diagnostic), and once the merge was made
  representable the wrong position surfaced as a non-advancing parser
  recursing to stack overflow. Tuples now lower to their canonical
  `llvm_ty` struct at construction, tuple field reads handle anonymous
  structs positionally, and a new `coerce_aggregate_value` helper coerces
  mismatched aggregate shapes *element-wise* (recursing through nested
  aggregates, byte-reinterpreting only same-size leaves) — it replaces both
  the phi-shape diagnostic and the byte-reinterpret return coercion, which
  was this same layout bug in a second costume. `compiler_pipeline.jn`
  compiles, runs, and joins the snapshot + differential harnesses (the
  `UNSUPPORTED` lists are empty); the layout and merge behaviors are pinned
  in `tests/semantics_regression.rs`.
- **T-15: unsolved exported signatures resolved after callers had already
  read them.** `build_fn_scheme` ran only for functions in `inferable_fns`
  (unannotated *parameters*), so a function with annotated parameters and an
  unannotated return kept its raw inference variable in the `fns` registry —
  callers in other modules read it before resolution and saw it defaulted to
  i64 ([151] watched `csv.parse` callers read garbage this way). The scheme
  pass now covers every non-generic function and writes the
  canonicalized/resolved parameter and return types back to the registry, so
  the annotation-free path is authoritative across module boundaries. Pinned
  two-file in `tests/module_resolution.rs` and same-module in
  `tests/semantics_regression.rs`.
- **T-1r, the harshest spelling: a user type actually named `E` never
  unified with its own annotation.** The parser reads any single uppercase
  letter in type position as a type parameter, so `items as Vec of E`
  produced `Param("E")` while the value had `Enum("E")` — rejected with
  `expected E, found E`. Unification now identifies a `Param` with a
  same-named declared enum or struct, and the equal-Display fallback
  diagnostic names the *kinds* (`the type parameter E` vs `the enum E`) so
  the residue of T-1r can't hide behind identical spellings again. Pinned in
  `tests/semantics_regression.rs`; phantom/return-only parameters and
  turbofish remain open in the roadmap entry.

- **[152]** (2026-08-13) the std adoption sweep lands: csv parses 315x faster, strings 30x, json 17x — span-based byte-indexed rewrites of the four M-13r modules, whole-String view coercion, and `toml.parse_frozen`

M-13r step 4 and M-14r step 3, gated by [151]'s behavior pins and benchmarks.
Every checksum is identical before and after; the full suite is 2224 tests,
and the whole-corpus ASan+LSan sweep stays at **zero memory corruption** with
the leak tail unchanged.

- **What the copying idioms actually cost.** The sweep's scouting corrected
  the roadmap's own account of the byte/scalar seam: `slice`, `char_at`, and
  `view` were always byte-indexed (docs/strings.md had it right) — the real
  defect was `.length`, a *scalar count that scans the whole string on every
  call*, used as the bound of nearly every std byte-loop. `while i <
  s.length` made csv/json/strings parsing O(n²) in the input *and*
  under-scanned non-ASCII text (`strings.trim` corrupted multi-byte strings
  by slicing to a scalar bound). Per-byte accumulation (`out + s.slice(i,
  i+1)`) stacked a second quadratic on top.
- **The rewrite.** All four modules now bound byte loops with `.byte_count`
  (O(1)) and build results from *spans* — one slice per token, segment, or
  unchanged run instead of one per byte: `csv.parse`/`Reader.next_row`
  (quoted fields concatenate only around escaped quotes),
  `json.parse_string`/`__escape`, `strings.replace`/`split_lines`/
  `split_whitespace`/`title_case`/`snake_to_camel`/`camel_to_snake`/
  `trim*`/`count`. `strings.to_lower`/`to_upper` delegate to the builtin
  single-pass methods (same ASCII-fold semantics), and `strings.reverse` is
  now scalar-aware (`reverse('héllo')` is `'olléh'`, not byte salad).
  Measured at `--opt 3` against [151]'s `pre-adoption` tag, checksums
  identical: csv_parse 1.26s -> 4.0ms (315x), json_parse 1.14s -> 66ms
  (17x), std_string_ops 193ms -> 6.5ms (30x), sort_strings 42ms -> 36ms.
- **Views where lending fits.** A whole `String` now coerces into a
  `View of u8` parameter (the byte-window sibling of the existing
  `Vec`/array coercion, pinned in `tests/views.rs`), `strings.__contains_byte`
  takes `View of u8`, and `sort.is_sorted`/`binary_search` take
  `View of i64` — callers pass vectors, arrays, views, or (for u8) strings
  unchanged. `sort`'s string ordering dropped its scalar-bounded manual
  compare for the builtin byte-lexicographic `<` (memcmp; also *correct* on
  multi-byte scalars where the old loop mis-ordered).
- **`toml.parse_frozen` (M-14r step 3).** The config-loader pattern from
  `design/freeze.md`: parse once, return `Frozen of TomlTable`, read through
  the ordinary accessors, share across a `together` without copies. Pinned in
  `tests/stdlib/toml_tests.jn` (new, with parse/section pins); the tour's
  Frozen section names the pattern.
- Non-ASCII behavior pins joined `strings_tests.jn` (trim/split/replace/case
  over multi-byte text, scalar-aware reverse); sort gained view-parameter and
  byte-lex pins. Remaining adoption residue (url/toml/http/regex/date still
  carry `.length`-bounded byte loops) stays in `M-13r`'s note in the roadmap.

- **[151]** (2026-08-13) the adoption sweep's gates find six live compiler bugs: match arms lost their values to trailing drops, match-payload rewraps double-freed, String captures aliased, nested-loop method mutations vanished, float `neq` ignored NaN, and stdlib behavior tests join the gates

Preparing M-13r's std adoption sweep demanded behavior pins for `csv` and
`json` (which had none) and benchmarks for the four target modules (which had
none). Writing those pins and benchmarks — before touching a line of std —
surfaced six distinct compiler defects and a string of std defects, every one
a silent wrong answer or a memory error on plausible code. All are fixed and
pinned; the full suite is 2222 tests across 53 binaries, and the post-fix
whole-corpus ASan+LSan sweep reports **zero memory corruption** with the leak
tail unchanged in character.

- **A value-position `match` lost every arm to one trailing drop (MIR
  lowering).** When an arm block's tail expression only *read* a local
  (`result + "]"`), the typer's scope-end drop landed after the tail, and
  `lower_block_expr` took the last statement's value — the drop's void — as
  the arm value, poisoning the merge phi so *every* arm returned `""`.
  `json.pretty` returned an empty string for any array or object; any
  String-valued match with a block arm was affected. Block-expression values
  now come from the last non-drop statement (`tests/semantics_regression.rs`).
- **Consuming a match-payload bind was invisible to the subject
  (typer).** `match obj ... JObj(o) ? ... JObj(o)` — the `json.set` idiom —
  re-wrapped the payload while `obj`'s drop stayed live: a double free at
  scope exit, SIGSEGV in four lines of user code. Pattern binds now link to
  their subject's place; consuming the bind consumes the subject through the
  same funnel (use-after is an ordinary moved-value diagnostic naming the
  ctor site), and the scope-drop collector expands bind links the same way.
  Residue (subjects that are temporaries are unlinked; moving one bind of a
  multi-field payload leaks the others) is filed as `M-17`.
- **String captures aliased instead of cloning (typer).** A `String` variable
  captured by a constructor, variant, `vector()`/array/tuple literal, or
  channel send copied the 24-byte header — two owners of one heap buffer, a
  double free the corpus never saw because its strings fit SSO inline. Strings
  are contractually non-consumable parameters, so the sound semantics is a
  clone: capture sites now wrap bare String vars in an internal `__clone`
  method (temporaries still move; `Map.set`/`push` were already sound).
- **Mutating methods in nested loops lost scalar-field writes (codegen).** The
  by-value receiver spill emitted its store at the first call site's insertion
  point — inside the inner loop — so every iteration re-stored the stale
  pre-loop struct: `sb.write_byte(..)` in a `while` inside a `while` kept one
  byte; heap-indirect writes (vec pushes) survived, which is why it read as
  flaky rather than broken. The store now lands right after the receiver's
  defining instruction. `tests/programs/expected/ecs.out` was re-recorded: its
  snapshot had baked the bug in (the fifth loop rebind was lost).
- **Float `neq` compiled to ordered ONE (codegen).** `NaN neq NaN` was false,
  so `math.is_nan` — `x neq x` — never returned true. `neq` on floats is now
  IEEE-754 UNE in both codegen and store filters.
- **Int literals only coerced to float on the left (typer, T-14 residue).**
  `sign(0) equals 0` and `x equals -1` against an f64 both rejected; the
  promotion now applies to either side and through unary minus.
- **std defects the new pins caught:** `csv.get_column`/`to_records` did not
  compile when called (aggregate-element binds, invisible to the `--lib` gate
  because the functions were unannotated — the general hazard is filed as
  `T-15`); `csv` and `json` internals carried unsolved element types that
  callers saw defaulted to i64, reading garbage through every method (both
  modules are now fully annotated); `random.next_f64` used an arithmetic
  shift, going negative half the time; `path.with_ext`/`with_name` prefixed
  bare names with `./`; `std/math` lacked `NAN`; `json.keys` now returns a
  copy instead of moving the object's key vector.
- **Gates and benchmarks.** `tests/stdlib/` was never wired into any gate and
  three of its suites had rotted (math, path, random — all green now);
  `tests/stdlib_behavior.rs` runs all twelve suites via `par_map` in ~2s.
  New `csv_tests.jn`/`json_tests.jn` pin parser, writer, accessor, and
  `pretty` behavior. New benchmarks `csv_parse`, `json_parse`, `std_string_ops`,
  `sort_strings` measure the copying idioms; the `pre-adoption` history tag
  is the sweep's before/after gate (csv_parse 1.26s, json_parse 1.14s,
  std_string_ops 193ms, sort_strings 42ms under the harness).

- **[150]** (2026-08-13) M-4r closes: read-only method calls through element views operate on the original

The last read shape the view surface did not cover. A user method called
through an element view (`pts.at_view(0).norm2()`, `p.norm2()` inside
`for p in pts.views()`) now dispatches against the element type in the typer —
rejecting methods whose inferred receiver bits say mutating or consuming
("a view is a read-only borrowed window", with the owning-element and copy
alternatives named) — and in codegen the receiver passes the view's element
pointer directly (methods already take `self` by pointer), after a non-empty
check. The call therefore operates on the *original* element, exactly as
`design/second-class-refs.md` specified, with no element copy. With field
reads ([149]) and method calls ([150]) both reading through the pointer,
`M-4r` is closed: expression-position `get` keeps copy semantics by contract,
and `at_view`/`views()` are the zero-copy spellings. Frozen-argument peeling
and view coercion apply to the method's arguments like every other call path.
Pinned in `tests/views.rs`; full suite 2211 tests, whole-corpus ASan sweep
still zero corruption. The std adoption sweep stays open at `M-13r` — it
walks the `T-13r` byte/scalar seam (`slice` is scalar-indexed, `view` is
byte-indexed across ~160 std sites) and needs its own benchmark-gated pass.

- **[149]** (2026-08-13) the ownership surfaces finish their designs: views bind and lend with root-locking, one frozen value feeds every task in a `together`, closure calls reach the capability row, and generators stop aliasing their arguments

The remaining sequenced steps of [148]'s three features, plus the residues its
probing left open. Full suite 2210 tests across 52 binaries; the whole-corpus
ASan+LSan sweep stays at zero memory corruption (929 clean runs, and the leak
tail is one program smaller than [148]'s — the closure-temp drops below).

- **Views bind, lock their root, and lend (M-13 steps 2–3).** `v is
  xs.view(1, 3)` is legal and registers a borrow of `xs` (or `xs.items`, or a
  frozen root) in the same lattice stack iteration borrows use — generalized
  with a `BorrowKind` so every conflict site (moves, mutating methods,
  assignments, call arguments, reassignment of the root) names what holds the
  borrow: a `for` loop, a bound view, or a frozen share. The borrow dies with
  the view's block; rebinding the view releases and re-registers it; a bind
  from another view inherits its root; views of temporaries reject ("the data
  it points into dies with this statement"); view-typed parameters have no
  local root, the call-level borrow covering them. `for p in pts.views()` is
  lending iteration — the binder is a per-element view, iteration borrows the
  vector, and `views()` anywhere else is rejected as an iteration form. Field
  reads through an element view (`pts.at_view(i).x`, `p.x` in the loop) GEP
  through the view pointer with a non-empty check — no element copy, closing
  the field half of `M-4r`. Partial moves out of a view reject at the move
  chokepoint ("a borrowed window that owns nothing"); tuple binds and
  constructor inits (including *inferred* struct fields, a hole [148] missed)
  reject views; `Type::View` joined the consuming-inference whitelist.
- **One frozen value feeds every task in a `together` (M-14 step 2).** Frozen
  captures inside a `together` no longer move: each `dispatch` captures the
  same value by pointer — sound because tasks join before the owner's scope
  ends and there is no state to race on — and a `FrozenShare` borrow rejects
  any move of the shared value until the join, with the owner readable
  afterwards and dropping exactly once. The exception requires the frozen
  value to be bound *outside* the `together` (a body-local's drop would race
  the join; those still move into one task). Pipes peel frozen arguments like
  direct calls — `fz ~ total` works, `fz ~ grow` rejects with the frozen
  wording — and actor sends of frozen payloads reject with guidance
  ("declare the handler parameter `Frozen of ...`, or send a copy") instead
  of a bare unification mismatch.
- **Closure calls reach the capability row (M-16 step 2).** Calling through a
  function-typed parameter inside a `needs`-annotated function is now a
  capability error: the scanner marks calls whose callee is a parameter, the
  row derives a conservative `indirect-call` pseudo-capability that no
  annotation can declare, the fixpoint propagates it like any capability, and
  the diagnostic explains ("the callee's capabilities cannot be classified
  yet; call a named function instead") with the introduction path. Lambda
  bodies were already scanned at their definition site, so a closure's own
  effects were never lost — the taint closes the false *acceptance* on the
  caller side.
- **Generators stop aliasing their arguments (M-16 step 3, creation half).**
  The same probe that exposed closure aliasing reproduced on generators:
  `g is emit(xs)` left `xs` usable (pushes were visible through the frame)
  and consuming `xs` then resuming SIGSEGVed. A generator's aggregate
  arguments are now inferred consuming — the frame outlives the call, so the
  frame owns them — making both shapes ordinary use-after-move errors with
  the generator named as the consumer. Suspended-frame drops (freeing what a
  half-run generator still holds) remain open at `M-16r`.
- **Closure temporaries joined the drop-obligation set.** `ClosureCreate`
  with captures now seeds `M-9r2`'s must-hold dataflow, so an
  expression-position capturing closure (`apply(|x| x + n, 7)`) frees its
  environment at function exit instead of leaking it (the env-drop call is
  visible in the caller's IR). Per-iteration closure temps in loops remain
  the documented loop-reallocation residue — measured by address-space-limited
  runs (20M loop-iteration capturing closures still exceed a 200 MB cap,
  while the capture-free loop runs clean).
- Docs: the tour's Views and Frozen sections show bind-position views,
  lending iteration, and the shared-`together` example (compiled);
  the Generators section states the move-in rule; `memory-model.md` §12,
  M11, and M12 updated; the three design docs' status headers record steps
  2–3; roadmap entries M-13r/M-14r/M-16r rewritten to what actually remains.

- **[148]** (2026-08-13) the three designed ownership surfaces become real: `freeze` makes deep immutability a type, `View of T` makes zero-copy windows second-class, closures stop aliasing the frame and own their environments — and probing found ctor captures double-freeing, closure captures use-after-freeing, and task capture slots truncating anything wider than a word

Step 1 of all three [147] designs, implemented in one pass with the
probe-first method. Every feature landed with its own suite; the full suite is
2195 tests across 52 binaries, and the post-pass `ci/sanitize-corpus.sh`
sweep (926 clean runs at `--opt 0` and `--opt 3`) reports **zero memory
corruption**, with the leak tail unchanged in size from [147]'s baseline
(M-9r2's known residue).

- **`freeze` / `Frozen of T` (M-14 step 1, `tests/freeze.rs`, 20 tests).**
  `freeze x` is a new hard keyword and unary expression: it consumes an
  aggregate operand through the same place lattice as every other move
  (`MoveReason::Freeze`, first-wins so the freeze site survives the bind's
  own assign-move record) and produces `Frozen of T` — a real `Type` variant
  that unifies congruently, delegates every ownership predicate to its
  payload, and is erased to `T` in the end-of-function HIR canonicalization,
  so MIR and codegen never see it and the runtime representation is
  unchanged. Freezability is structural and recursive with the offending
  field path named (`@resource` types, channels, actors, coroutines,
  generators, functions, and views refuse). Reads auto-deref everywhere —
  field and element reads, iteration, read-only methods and parameters — and
  every write rejects at its natural chokepoint: builtin mutating methods and
  user methods with inferred mutating/consuming receivers at method dispatch,
  assignment through any frozen component at the assign statement, partial
  moves at the `mark_place_moved_checked` funnel, and mutating or consuming
  parameters at call-argument peeling (consuming rejects because moving into
  a mutable owner would thaw; read-only parameters accept a frozen argument
  by silently peeling the wrapper). `freeze v` of an unannotated parameter
  makes the parameter consuming through the existing inference; freezing a
  borrow (loop binder, borrowed param) is rejected with the clone-first fix.
  Step 2 — every `dispatch` in a `together` sharing one frozen value — is
  tracked at M-14r; until then frozen values move into one task like any
  aggregate.
- **`View of T` (M-13 step 1, `tests/views.rs`, 13 tests).** A view is
  `{ptr, len}` by value: trivially droppable, bounds-checked, created by
  `xs.view(a, b)`, `xs.at_view(i)`, and `s.view(a, b)` over string bytes
  (heap or SSO — inline strings spill to an entry alloca first), or by
  passing a whole `Vec` or array to a `View of T` parameter (automatic
  `view_full` coercion at call boundaries). Views support `.length`,
  `.get(i)` (value-category elements clone out), and `for` iteration — the
  MIR loop lowering needed no change because `VecLen`/`IndexUnchecked` gained
  view arms in codegen. Second-classness is enforced as a compile error in
  every escaping position: binds ("a view lives only within its statement"),
  returns (declared or inferred), struct/enum/store fields, nested
  annotations (`Vec of View` anywhere), task captures, closure captures,
  channel/actor sends, and yields. Out-of-range windows trap with their own
  message. Because step 1 has no bind-position views, the existing
  statement-scoped borrow rules are the entire soundness argument — no
  lattice work was needed, exactly as the design sequenced it.
- **Closure captures (M-16 step 1, `tests/closure_captures.rs`, 17 tests).**
  The design's premise was stale: capturing lambdas already parsed and ran by
  aliasing the enclosing frame — `total is || xs.sum()` then `xs.push(100)`
  read the push through the "capture", and consuming `xs` then calling
  `total()` SIGSEGVed. Captures now classify by category at lambda lowering:
  scalars copy, `String`s and value structs clone into the environment,
  aggregates move (`MoveReason::ClosureCapture`, the use-after diagnostic
  names the capture site and the clone-first fix), and views, `@resource`
  values, and borrowed parameters reject. The closure value is an aggregate:
  `Type::Fn` moved from the scalar to the aggregate category (assignment
  moves it, one task at most, function-typed parameters borrow), and the
  environment is a heap block whose slot 0 is a synthesized
  `lambda.N.env_drop` — dropping a closure drops every capture and frees the
  env through that pointer, uniformly and type-erased, with plain function
  references keeping a null env. Returning a closure over a local aggregate
  is now *sound* (the env owns it), so the [143] "captures the local" ban is
  deleted and its regression test now pins the accepted behavior. Calling a
  moved closure is rejected on the named-var call path, which had no
  use-after-move check at all. Caps edges for indirect calls, generator
  frames, and expression-position closure temps (which leak their env) are
  tracked at M-16r.
- **Three live memory bugs found by probing, none pinned by any test.**
  (1) A constructor or container literal capturing a bound aggregate —
  `app is App(cfg is xs)`, `nested is vector(xs)` — recorded the move for
  diagnostics but never told drop emission: both the struct and `xs` dropped
  at scope end, a silent double-free on pristine [147] (`free(): invalid
  pointer` at exit). The consumed-set now mirrors `record_ctor_capture`.
  (2) The HIR var-id collector had no `IndirectCall` arm, so a closure used
  only as a callee (`g is || f() + 1`) was invisible to capture analysis and
  to drop exclusion — nested closures double-freed. (3) Scope-task capture
  slots were hardcoded to 8 bytes on both the writer (`emit_scope_spawn`,
  `emit_coro_create`) and reader sides; any capture wider than a word — a
  24-byte `String`, a 16-byte closure — overflowed its malloc and corrupted
  the heap. Slots are now sized and offset by actual store size on both
  sides (`gen_capture_offsets`), and a task capturing a spilled-SSO string
  is pinned.
- **Smaller fixes riding along.** MIR closure captures are sorted by name
  (env layout was `HashSet`-ordered, nondeterministic across runs); the MIR
  free-var walk no longer treats a nested lambda's parameters as captures of
  the outer closure; `jinn fmt` prints lambda parameter annotations and
  return types instead of dropping them, and no longer collapses a
  multi-statement lambda body to the literal `0` (the unrepresentable case
  now degrades to a reparse refusal, per the [144] posture); the fictional
  `lambda_expr = "(" params ")" "=>" expr` EBNF production — a token that
  never existed — is replaced with the real `|params|` grammar, and
  `unary_expr` gains the `not`/`~`/`@`/`freeze` prefixes it was missing;
  `.jni` interfaces bump to version 3 for the `View`/`Frozen` type variants.
  `docs/jinn.md` documents all three features with compiled examples
  (closure captures under Lambdas; new Frozen values and Views subsections),
  `docs/memory-model.md` gains rule M12 (closures capture exactly like
  tasks) and §12 (frozen values and views), the category table moves
  function values to the aggregate row, and the three design docs' status
  headers record what shipped and what remains (M-13r, M-14r, M-16r in the
  roadmap).

- **[147]** (2026-08-13) the memory model gets its verification teeth: MIR repairs and proves its own drop placement, the whole corpus runs under ASan with zero corruption, idiomatic field writes stop being invisible to inference, and every slice of a vector was wrong

The remaining memory-model items, worked as one batch (closed M-9r, M-10,
M-15; reduced M-6r, M-8, M-11, M-12; designs written for M-13, M-14, M-16).
Probing before porting — the [146] method — again found live unsoundness that
no test pinned:

- **Idiomatic field writes were invisible to both inference scans.** The
  documented method style drops `self.` (`data is x`, `total is total + x`),
  and both the consuming and the mutating scan only recognized the
  `self.data is x` spelling. A method storing a parameter the idiomatic way
  double-freed at runtime (`free(): invalid pointer`, probe p6); a method
  mutating a field the idiomatic way was not marked receiver-mutating, so
  calling it on a nested place mutated a copy and silently lost the update —
  the exact class [146] made a compile error, escaped through the house
  style (probe p7: `h.c.bump(5)` left `h.c.total` at 0). Both scans now treat
  a bind to a field of the method's self type as a store into `self` /
  receiver mutation, with parameter names excluded from field shadowing.
- **Early returns leaked every live container.** The typer emits scope-end
  drops only at block ends; a `return` inside an `if` exited through no drop
  at all (MIR for the `path.normalize` shape: the fall-through block dropped
  both vectors, the early-return block dropped neither). Fixed at the layer
  that owns drop placement: `src/drops/mir_drops.rs` now runs a
  must-hold dataflow over container allocations (`vec_new`, `map_init`,
  container-typed `clone`/known-call results) with ownership-precise transfer
  edges — per-slot consuming information passed down from the typer's
  `fn_param_access`, container-insert builtin methods, stores, sends,
  captures, phi edges, returns — and inserts a drop before any `return` a
  must-held allocation reaches. Must-held (intersection at joins) is the
  safety argument: a value moved on *any* path never gets an unconditional
  drop, so the repair can only free, never double-free. Inserted drops land
  after inlined `defer` bodies, preserving defer-before-drop on early exits.
  `src/drops/verify.rs` then re-runs the same analysis as a post-condition
  and fails the compile ("this is a compiler bug") if anything must-held
  still reaches a `return` — closing M-9r's "leak side" for the container
  obligation set. Conditional consumption still leaks its untaken branch
  (documented at M-7r; an unconditional drop there would double-free), and
  method-call results/String temps are outside the obligation set (M-9r2).
- **Quaternary arms never recorded moves; consuming pipes segfaulted.** The
  post-lowering move-recording walk had no `Block` arm, and `? !!` arms lower
  to blocks — `o ? eat(v).length !! 0` then `v.length` compiled and read
  freed memory. `x ~ eat` (pipe into a consuming function) compiled and
  SIGSEGVed. The walk now descends into block statements (bind values only —
  quaternary subject/`$`/`err` binds and desugared-iterator binds are borrows
  at runtime and must not tombstone), records ternary arms with
  snapshot-and-union semantics (consuming the same value in both `? !` arms
  is one move, matching if/else), and handles `Pipe` exactly like `Call`
  including call-site aliasing checks. The ternary-as-quaternary path also
  stopped lowering its subject twice. Probing the last M-6r entry showed
  `defer` observes variable state at scope exit (2 000 reallocating pushes
  after registration, defer printed 2001), so mutation during a pending defer
  is sound and stays legal; the item asked for a restriction that would have
  been wrong.
- **Receiver-typed inference (M-8).** The AST-level scans no longer join the
  builtin name-bucket when the receiver's type is knowable from single-static
  facts (parameter annotations, constructor binds, `self` and its fields —
  one binding event, or the fact dies): `g.set(x)` on a `Gauge` whose `set`
  stores nothing no longer forces the caller's argument into a move. Facts
  are single-static so branch-local rebinds cannot resurrect a stale type.
- **M-10, the whole-corpus sweep.** `ci/sanitize-corpus.sh` compiles
  `tests/programs` (87), every `apps/` entry (21), and every snippet (~400)
  against an ASan-instrumented runtime at `--opt 0` and `--opt 3` and runs
  each in an isolated scratch dir: 919 clean runs, **zero memory
  corruption**, 99 leak runs (49 programs, 76 B–49 KB — the M-9r2 tail,
  reported but non-gating; `JINN_SAN_STRICT=1` gates), 2 compile failures
  (T-10's pinned program). Its very first run caught the batch's biggest
  surprise: **every vec slice was silently wrong** — codegen declared
  `__jinn_vec_slice` with three arguments but the runtime takes four, so
  `elem_size` was whatever sat in the register (0 in release builds:
  `v from 1 to 4` printed `[0, 0, 0]`; under ASan: terabyte allocation
  requests). No test ever ran a vec slice. Fixed, pinned in
  `tests/semantics_regression.rs`. `ci/fuzz-ownership.py` (seeded,
  deterministic) mutates ownership-relevant syntax — duplicated args,
  inserted/swapped `take`/`copy`, late uses, rebinds — over a 60-file sample:
  197 mutants, 128 compiled and ran clean under ASan, 69 cleanly rejected,
  zero ICEs, zero memory errors. Actor-heavy programs can SEGV spuriously
  under ASan (no report) because the runtime lacks
  `__sanitizer_start_switch_fiber` annotations — classified `segv?`,
  non-gating, filed with N-5.
- **M-11 assertions.** `type Point @value` / `type Bag @aggregate` pin a
  struct's ownership category; a contradicting definition is a compile error
  naming the flipping field. Parser, `fmt` printer, typer check
  (`check_category_assertion`), docs, and pins.
- **M-12 mechanics.** `.jni` interface files are version 2: every parameter
  carries `consumes`/`mutates` populated from the inferred tables after
  typing, so inferred consumingness is in the interface as the item required.
  `--lib` compiles warn per exported function whose parameter consumes by
  inference, naming the escape site and the explicit `v as take ...`
  spelling. Required-explicitness policy and `fmt` insertion wait on the
  package surface (M-12r).
- **M-15, `std/arena`.** A generational `Arena of T` (insert/get/set/remove/
  contains/size/handles over parallel slot/generation/liveness vectors with a
  free list) ships as the blessed replacement for pointer-linked structures;
  stale handles are detected (`contains` false, `get` traps), slot reuse
  bumps generations. Building it surfaced the practical T-1r wall: generic
  constructor functions break method resolution on the result, `Option of T`
  returns from generic methods don't resolve at call sites, and bind
  annotations don't unify with generic constructors — the API avoids all
  three (trap-checked `get`, constructor-literal idiom) and the tour
  documents the pattern. 50th alpha-stable module, auto-gated.
- **Designs for the three remaining majors.** `design/second-class-refs.md`
  (M-13: never-storable views — the stack is the lifetime; reuses the [146]
  place lattice for root-locking), `design/freeze.md` (M-14: one-way deep
  immutability, scope-bounded sharing via `together`, no refcounts, `freeze`
  is a runtime no-op), `design/closure-captures.md` (M-16: closures capture
  like tasks — values copy, aggregates move, nothing by reference; the
  closure is itself an aggregate; spec-before-implementation as the item
  demanded).

2145 tests green across 49 binaries (21 new: 13 place-ownership pins for
every fix above, arena end-to-end, category assertions, the boundary warning,
5 drop-verifier unit tests including "conditionally-moved values must not get
an unconditional drop", the vec-slice pin); `cargo fmt --check` and
`cargo clippy --release -- -D warnings` clean; corpus sweep and fuzzer as
above. Roadmap: M-9r, M-10, M-15 closed; M-6r shrunk to dynamic element
indices and `sim for`/`together` capture; M-8→M-8r, M-11→M-11r, M-12→M-12r,
M-9r→M-9r2, M-10→M-10r residues filed; M-13/M-14/M-16 marked design-complete.

- **[146]** (2026-08-12) ownership goes place-based: one lattice for moves and borrows of `root.field.elem…`, four double-frees fixed, and the corpus gave up two silently-broken components

M-6 — the last blocker — plus M-5r. The refactor replaced `moved_vars` +
`moved_fields` (variable-keyed, single-level) with one place lattice
(`src/typer/place.rs`): a place is a root binding plus a projection path of
fields and elements, with overlap and disjointness queries (`s` covers `s.a`
covers `s.a.b`; `s.a` and `s.b` are disjoint; `v[0]` and `v[1]` are disjoint,
any dynamic index conservatively overlaps). Every recording, clearing,
snapshot/merge, and read-check site was ported; the flow-sensitivity
machinery, loop-repeat check, and all pinned diagnostics are byte-compatible
at variable granularity. Iteration borrows became a stack of places: `for x in
s.items`, `for k, v in m` (the map desugar), and `Iter`-trait desugared loops
now register the iterated place, and moves, mutating calls,
mutation-through-calls, and reassignment of any *overlapping* place in the
body are compile errors — while sibling places stay free, which the corpus
depends on (`loop self.def_names` mutating `self.def_required` in std/args is
the canonical shape and still compiles). Call-site exclusivity now checks
argument places: overlapping bare-place arguments where a parameter mutates or
consumes one of them are rejected; computed arguments (`qsort(a, 0, a.length -
1)`) evaluate before the call and stay legal, as does the container idiom
`v.set(i, v.get(j))` (builtin container methods keep two-phase semantics).

Probing each seam before porting it found that most field reads deep-copy at
runtime (so `f(s.a, s.a)` and constructor field captures were benign copies,
not the moves the roadmap feared — the checks now match that reality), but
four shapes were live memory unsafety, all now fixed:

- **Moving out of a borrowed parameter double-freed.** `*peek(v)\n    s is v`
  minted an owning binding of the caller's value; callee and caller both
  freed. Now a bind whose source is a borrowed binding borrows too
  (`Ownership::Borrowed` propagates; no tombstone, no drop), and `s is take v`
  from a borrow is a compile error that suggests declaring the parameter
  `take`.
- **Constructor capture of a borrowed parameter double-freed.** `Box2(v is v)`
  aliased the parameter into an owning struct. Struct-literal, tuple, array,
  and `vec(...)` captures now count as escapes in consuming-parameter
  inference, so such parameters are consuming and call sites move (enum
  variant payloads already were).
- **The user-method ownership checks were inert.** The typer double-mangled
  the method key (`Sink_swallow` became `Sink_swallow_swallow`), so a method
  that stored its argument — [143]'s M-8 fix — never marked the move and
  double-freed at runtime; the element-temp-mutation, iterate-while-mutating,
  and call-aliasing checks for user methods were dead for the same reason. One
  key fix revived all four.
- **`take o.inner.data` compiled and SIGSEGVed.** Nested-place `take` was
  silently unrecorded and miscompiled; it is now a compile error naming the
  place ("take the outer field first").

Two more shapes were silent lost updates, now compile errors: passing a field
or element read to a parameter that mutates it (`app(s.a)` mutated a copy),
and calling a mutating method on a nested receiver (`h.c.bump()` bumped a
copy). Enforcing those found real corpus bugs: `std/collections`' `Heap` and
`PriorityQueue` passed `self.data` to their sift helpers — every heap ever
constructed was an insertion-ordered vector, and no test or program had ever
noticed; the sifts are now in-place methods and a heap finally pops in sorted
order. And `apps/physics_engine` ran `integrator.step(world.get($), dt)` —
sixty frames of physics integrated copies and threw them away; the app now
binds the body out, steps it, and writes it back.

`tests/place_ownership.rs` (21 tests) pins the four unsafety fixes, the two
lost-update rules, place-granular iteration borrows for fields/maps/`Iter`,
sibling-disjointness acceptance, the swap idiom, deep-place reads under moved
prefixes, and — for the first time — the [143] diagnostics themselves
("is iterating it", "passed twice"), which had been pinned only by corpus
compile gates. Residue is filed as M-6r (literal-only element indices,
ternary arms not snapshotted, pipe move-marking, defer mutation checks).
- **[145]** (2026-08-12) roadmap remediation: `needs` is checked for real, drops are verified after every Perceus transform, trait impls must conform, `chr` finally means a character, and store filters learn `in`-in-query-blocks and case folding

Six roadmap items closed (C-1, M-7, M-9, T-5r, T-13, S-7), each leaving its
honest residue filed (C-1r, M-7r, M-9r, T-5r2, T-13r, S-7r).

**Capability checking is live (C-1).** The old table keyed every entry on
`std.net.connect`-style symbols that are not callable names, so nothing ever
matched and `needs pure` was a comment. Rather than re-keying onto the std
API surface wholesale, effects are now classified where they actually happen:
at the extern leaves. `src/cap_sites.rs` classifies every extern the std
library declares (fs/net/process/env/clock/random; `fopen` is mode-dependent;
`setenv` is `state 'env'`), a call to an *unclassified* extern — or a
`syscall` or `asm` block — derives `ffi.unsafe`, and benign externs
(malloc/memcmp/sin/crypto/fd-level I/O on already-held descriptors) derive
nothing, on the capability-security reading that the descriptor is the
authority and the aperture is where it was acquired. On top of the leaves,
`io.*`/`fs.*` calls carry vetted path-scoped signatures (`io.write_file(lit,
..)` derives `fs.write 'lit'`), trusted as row replacements only when the
callee resolved to a module loaded from a std directory (the driver records
provenance during module resolution) — a user module named `io` keeps its
derived row. The fixpoint now scans every expression form (lambda bodies,
store filters, `together` blocks and select arms were all invisible before),
covers generic functions and type/impl methods (method calls join a name
bucket — over-approximate, but only ever toward false rejection), and the
whole thing stays opt-in: no `needs`, no check. The roadmap's sneaky canary
now fails to compile with the introduction path named, and `tests/caps.rs`
pins eight behaviours including scoped-path bounding and the
method-body-cannot-hide-an-effect case.

**Drop placement is verified (M-9).** `src/drops/verify.rs` runs by default in
release (`JINN_MIR_VERIFY=0` opts out): after each of sinking, elision, reuse
pairing, and fusion, the per-block drop multiset must be preserved (elision
may remove only trivially-droppable entries), and a final path-sensitive
dataflow — dropped-set forward analysis with kills at SSA re-definitions, so
loop-local owners do not false-positive — rejects any use-after-drop, any
value dropped twice on one path (including through DropMany and phi edges),
and reuse metadata whose save/consume sites no longer exist. Failures print
per-function diagnostics and die as compiler bugs. The whole corpus compiles
clean under it, which is now a regression fence rather than an assumption;
the leak side (every owner dropped at least once) needs typer-owned
obligations threaded into MIR and is filed as M-9r.

**Impls must conform to their traits (T-5r).** Conformance previously checked
method presence only, so an impl could silently widen or narrow the declared
`! E` row — unsound for callers dispatching through the bound. The check now
compares declared error rows exactly (neither widening nor narrowing, after
`Self` and trait-type-argument substitution), parameter counts, and annotated
parameter/return types, naming both the impl site and the trait declaration
site in the diagnostic. Unannotated impl signatures still conform by
adoption (T-5r2).

**`chr` encodes, `byte` does not (T-13).** The decision: `chr(code)` UTF-8-
encodes a Unicode scalar (surrogates and codes past 0x10FFFF encode U+FFFD),
and a new `byte(code)` builtin emits the raw single byte, documented in
docs/strings.md as outside the UTF-8 contract exactly like mid-scalar slices.
Both now reject non-integer arguments (the old lowering type-checked
`chr("x")`). Sweeping std's byte-assembly callers onto `byte` exposed that
the String-as-byte-buffer idiom was already half-broken — every one of these
loops bounded byte offsets with the scalar `.length`: `uuid.v7()` always
returned `""` (byte 8 is `0x80`–`0xBF`, invisible to the scalar count),
`url.percent_encode("é")` returned `"%C3"`, `codec.to_hex(from_hex("c3a9"))`
returned `"c3"`, `strings.to_lower` truncated multi-byte input, and
`StringBuilder` undercounted its malloc/memcpy sizes. All fixed with
`byte_count` bounds and pinned by new conformance tests in
tests/string_unicode.rs; the surviving scalar-bound `.slice(i, s.length)`
idiom on ASCII-expected parse paths is T-13r.

**Store filters (S-7).** Query blocks accept `field in [v1, v2]` — the parser
already desugars `in` to `[..].contains(field)`, and the typer now recognises
that shape, expands it to an equality chain, and *hoists it to the head of
the filter*, because codegen folds conditions left-to-right with no
precedence: an Or-chain is only correct at the head, so `where tag equals 'c'
and val in [10, 30]` is reordered to `(v10 ∨ v30) ∧ tag` rather than
miscompiled to `(tag ∧ v10) ∨ v30`. Combining `in` with `or`, a second `in`,
or an empty list is a diagnostic. Query blocks also gained method-form text
predicates (`name.starts_with('x')`), which never existed on that path. And
both filter paths gained ASCII case-insensitive matching — `iequals`,
`icontains`, `istarts_with`, `iends_with` — compiled against a new
`jinn_ascii_imemcmp` runtime helper, the same folding `@search` and
`to_lower` already use. The statement parser's parse-time mixed-connector
rejection still blocks `and`+`in` combinations there (S-7r).

**The consuming-call diagnostic names the path (M-7).** Consuming-parameter
inference now records the span of the consuming site per inferred slot and
whether it sits on a conditional path (if/match arms, loop bodies, ternary
arms), the tables propagate through monomorphization, and the use-after-move
diagnostic says `it is consumed at file:line:col` — with an explicit note
when that consumption is conditional, since the analysis is path-insensitive
and treats may-consume as always-consume. Callee names are demangled before
printing. The analysis itself is unchanged (M-7r).
- **[144]** (2026-08-10) `jinn fmt` no longer destroys code: printer rebuilt against the real grammar, gated by compile-after-format

X-1 measured 164 of 688 corpus files that compiled before `jinnc fmt` and not
after, applied silently by `--write`. The damage was printer grammar drift —
the printer had never been held to the grammar the parser actually accepts.
Measuring by damage class and fixing each:

- **Invented grammar.** Extern printed without `*`, parens, or `as`
  (`extern usleep us i32`); store statements printed SQL (`insert into`,
  `delete from`) with every filter dropped; store headers lost their
  decorators and fields lost theirs (`@search`, `@index`, `@default`,
  relations, store methods — all silently deleted, i.e. reformatting a store
  changed its schema); actors lost their state fields entirely and printed
  handlers in a form the parser rejects.
- **Dropped semantics.** Bind access modifiers (`row is copy t.get(i)` →
  `row is t.get(i)`) and bind type annotations vanished — reformatting changed
  ownership semantics. Generic `of T` clauses disappeared from types, enums,
  and functions; `! E` error rows disappeared from function signatures; layout
  attributes (`@packed`, `@align`) disappeared from types; the implicit `self`
  parameter was printed explicitly.
- **Wrong surface forms.** Float literals printed `0.0` as `0` (silently
  changing arithmetic types), `&`/`*` for the `%`/`@` pointer operators,
  `use std.time` for `use std/time`, `Tree<T>` for `Tree of T`,
  `Map of String, String` in positions where only the value-sugar
  `Map of String` parses, `err` variants with `of` instead of parens, and
  `not equals` for `neq`.
- **Structure loss.** Multi-clause functions desugared to statement-position
  if-expressions printed as `cond ? x ! ...` — now printed as real `if`
  blocks. Query blocks, `select`, and `dispatch` printed as `...` — now
  printed in their block forms. `for` loops lost their `to`/`by` bounds and
  index binders; desugared `if x is pat` matches printed empty arms — now
  `nop`. Precedence was never parenthesized (`(n & (n-1)) equals 0`
  reformatted into a parse error); binary operands, postfix receivers, and
  cast operands now get parens when compound.

Two enforcement changes make the class stay dead.
`fmt_output_still_frontend_checks_over_corpus` formats all 574 files of
`snippets/`, `tests/programs/`, `benchmarks/`, and `std/` and fails if any
file that frontend-checked before formatting stops afterward — the gate X-1
said was missing by construction (the old gate checked idempotence only, and
only over snippets/). And `fmt --write` now re-parses its own output before
writing, refusing with a formatter-bug message instead of damaging the file —
so the next printer regression is a refusal, not silent corruption. X-1 drops
to minor: `apps/` stay ungated (per-file checks need project context) and a
few expression-position fallbacks (`asm`, block expressions) survive behind
the guard.

- **[143]** (2026-08-10) roadmap remediation: 21 items closed — the ownership seams, the verified type-system defects, embed gating, and the store residue

The five memory-model blockers all lived at the same seam — moves of
*variables* were tracked exactly while moves and borrows through *expressions*
were not — and they close together at variable granularity (place granularity
stays open as M-6):

- **Constructor expressions move their sources (M-1).** Struct literals, enum
  payloads, `vec(v1, v2)`, tuples, and arrays now tombstone a bare aggregate
  variable in value position exactly as `b is a` does, with a dedicated
  `CtorCapture` diagnostic. This immediately caught a latent double-free
  pattern; move recording is suppressed inside `return`/`break`/`err` payloads
  because those paths diverge and the conservative loop-repeat check was
  rejecting `return JObject(o)` inside a `loop` (seen in `std/json.jn`).
- **One call site, one owner (M-2).** A per-call alias check rejects the same
  variable in two consuming positions, or in one consuming and any other
  position, of a single call — `combine(v, v)` was minting two owners of one
  buffer and exiting 0.
- **Call-site exclusivity (M-3).** A new parameter-mutation inference
  (`src/typer/mutate_infer.rs`, a fixpoint structurally identical to
  consuming-parameter inference, covering free functions and methods and
  propagated to monomorphized names) rejects passing one variable twice to a
  call that mutates it through either parameter. `app(v, v)` no longer chases
  its own append. The roadmap's `noalias` miscompile half was already stale —
  codegen no longer emits those attributes.
- **Mutating a temporary copy is an error (M-4).** Container element reads
  still copy (M-4r/M-13 record the cost), but `grid.get(0).push(3)` — and any
  builtin mutating method, or user method whose receiver-mutation was
  inferred, on an element-read receiver — is now a compile error instead of a
  silently discarded update. This found two real bugs in std:
  `dataframe.from_csv` built every column into a discarded temporary (all
  frames came back empty), and `add_row` appended every cell into one. Both
  rewritten column-first.
- **Iteration borrows (M-5).** `for x in v` registers an iteration borrow of
  `v` for the body: mutating calls on `v`, moves of `v`, and passing `v` to a
  parameter the callee mutates are compile errors, so the append-while-iterate
  hang is unrepresentable in the direct-loop form. Map/`Iter`-desugar and
  field-place loops remain open as M-5r.
- **Consuming methods by body, not by name (M-8, reduced).** User methods now
  run through the same escape scan as free functions, so a storing method is
  consuming whatever its name; the builtin name list remains only for
  runtime-implemented methods and for receivers whose types are unknown at
  AST-scan time (the residual imprecision).

Type system and diagnostics: generic-type methods are monomorphized *with
bodies* — instantiation queues the substituted method, a post-lowering drain
lowers it through the same path as plain type methods, so `Box of i64(42)`
followed by `b.get()` works (old T-1; annotation-driven instantiation also
declares methods now). Undefined names are rejected in the typer with a span
instead of surviving to a span-less codegen error, which also makes
`--emit-hir` exit non-zero on broken code — the gap that made the frontend
gates vacuous (old T-3; the fix immediately exposed that `transaction` blocks
were typer-scoped while their bindings are function-scoped at runtime, now
lowered scope-free to match pinned behavior). A bare `! E` means
`Result of Unit, E` for real: the ok type is `Void`, a valued tail is a typer
diagnostic, and the old inkwell panic is unreachable (T-4). Trait methods
parse `! E` (T-5; conformance checking still open as T-5r). Duplicate
catch-all clauses are a parse error naming both sites (T-6). Unsolved type
variables warn by default — container elements included, which was the silent
path — and `--strict-types` escalates *unconstrained* unsolved variables to
errors while leaving ordinary literal defaulting alone, so the flag's name is
true without rejecting `x is 42` (T-7). The M-4 check also found
`snippets/101-200/s172.jn` transposing a matrix into discarded row copies —
the differential gate had been pinning agreement on silently wrong output.
Out-of-range literals against an int annotation warn with the wrapped
value (T-8). ALL_CAPS constants cannot be shadowed (T-9). The whole compile
runs on a 256 MiB thread, so flat operator chains bounded only by memory
replace a stack overflow at ~5000 terms (T-11). A UTF-8 BOM is stripped and
non-ASCII lex errors print the decoded character, not mojibake (T-12).
Int→float coercion works in bind annotations and for integer literals against
float operands in binary expressions, and constraint-mismatch messages name
expected/found in the caller's order (T-14).

Effects and store: `embed` rejects absolute paths and canonicalized escapes
from the source directory (C-2). The comptime purity classifier is a least
fixpoint that consults callees instead of deciding purity from argument shapes
(C-3). `search` returns rows — the typer types it as the store's row struct
and codegen resolves the posting ids against the store file, skipping deleted
records, in id order (S-4). `JINN_WAL_SYNC=group` outside a transaction
degrades to per-record `fdatasync` instead of never syncing (S-6). A WAL with
bad magic exits 2 with the existing message instead of `abort()` (S-9,
reduced). `jinn init NAME` scaffolds into `NAME/` (X-2). The stale half of S-7
(`in [..]` "missing" — it exists in the statement filter path) is corrected in
the roadmap.

- **[142]** (2026-08-07) the std gate now means what its name says; apps/ enters the gates and two of them were broken

[141] argued that the gates measuring a narrower surface than their names
implied is what let everything else drift, and then did not change the
definition that caused it. `docs/std.md` still defined **alpha-stable** as
`jinnc std/<m>.jn --lib --emit-hir` exiting 0, and
`tests/std_stable_subset.rs` still ran only that. All 49 modules do import and
link today — that was verified by hand while fixing AR-F1 — but nothing pinned
it, so the exact drift the entry diagnosed could recur silently.

The tier is now defined by two bars and both are enforced.
`std_stable_subset_imports_and_links` writes a five-line `use std/<m>` program
per module, compiles it through codegen, links it against the runtime and runs
it. The frontend check stays, because it covers functions no importer reaches;
the link check catches what lives entirely past the frontend, which is where
AR-F1's ten failures were — missing runtime C symbols and codegen ICEs. Both
gates were checked against a deliberately broken `std/math.jn` to confirm they
are not vacuous.

**`apps/` was in no test at all, and two of the 21 had stopped compiling.**
Only `alpha_release_demo` was ever built, by the smoke script. Sweeping the
rest by hand found `blockchain_node` reading a `Block` after pushing it into a
`Vec`, and `lattice_crypto` reading an `Sk` whole after moving out its `pk`
field. Both are *correct* rejections by the use-after-free diagnostics [141]
added under AR-F3 — the corpus was never updated to match the language it
documents. Fixed where the defect is: read `b.hash` before the push rather
than after it, and `copy sk_a.pk` out instead of moving it. Both apps are
self-checking and now report `invalid_blocks 0` and `key.match 1` /
`roundtrip.ok 1` / `mac.ok 1`. `tests/apps_build.rs` builds and runs all 21 in
1.5s, fanned out through `par_map`, so this cannot recur quietly. Recorded as
AR-F33.

The 36 `benchmarks/` programs were swept the same way and all compile at
`--opt 3`; they are left out of the gates because they are timing harnesses,
not assertions.

- **[141]** (2026-08-06) alpha review remediation: 24 findings fixed, three more found by the new gates

The 2026-08-06 alpha readiness review is written up in
docs/alpha-review-findings.md; per-finding remediation state is in
docs/alpha-review-status.md. This entry records what was decided, because
several of the fixes turned on which of two documents was right rather than
on which code was wrong.

**The gates measured a narrower surface than their names implied, and that
is what let everything else drift.** `docs/std.md` defined "alpha-stable" as
`--emit-hir` exiting 0 — type-checks, nothing more. All 49 modules passed it
while 10 could not be imported at all by a five-line program. Of 546 corpus
programs only the 87 in tests/programs were ever compiled and run; snippets/
was parsed and formatted, apps/ and benchmarks/ were referenced by no test.
MIR verify was behind `#[cfg(debug_assertions)]`, so it was absent from the
compiler users run, and it did not check phi/edge type agreement anyway. A
`KNOWN_ICE` whitelist asserted that a program still crashed.

So the gates came first. `tests/corpus_differential.rs` compiles **and runs**
snippets/ and tests/programs/ at `--opt 0` and `--opt 3` and diffs stdout and
exit code. MIR verify checks phi types and runs in release (`JINN_MIR_VERIFY=0`
opts out). The whitelist is replaced by a list that asserts the *specific*
diagnostic, so it fails both if the program regresses to a panic and if it
starts compiling. `scripts/alpha_release_smoke.sh` pointed at a directory that
moved to apps/, which had been aborting preflight at step 5 of 7 — so steps 6
and 7 had not run in the gate at all.

That paid for itself immediately. The differential harness found a miscompile
in snippets/201-300/s298.jn that was flaky at *both* optimisation levels: an
actor's queued messages were dropped at process exit. Actor coroutines are
daemons, so `jinn_sched_run` returned without waiting and nothing closed the
mailboxes. Actors now register in a live list, and exit closes and drains
them. Two further defects the review had not found came out of neighbouring
fixes: passing a struct that owns a Vec double-freed it (parameter ownership
was decided before inference resolved the type, so an unannotated parameter
defaulted to Owned and the callee emitted drop glue for a value the caller
still owned), and Map values were stored in an 8-byte slot, which made
`Map of String` structurally impossible and was the real cause of the
`logging` and `url` LLVM-verification failures.

**Where the spec and the implementation disagreed, the spec won.** Two
findings were framed backwards in the review and are worth recording as
decisions:

- `q is p` on a struct of scalars aliased. docs/memory-model.md §1 puts such a
  struct in category Value — "deep copy; both live, independent" — so the
  binding now copies. `tests/crash_safety.rs` had a passing test asserting the
  aliasing; it now pins value semantics.
- Writes through a struct *parameter* were reported as "lost inside a loop".
  docs/access-semantics.md §4.1 makes a POD-struct parameter `Owned`, i.e. an
  independent copy, so the mutation *reaching* the caller was the defect. The
  callee now copies POD struct parameters on entry and borrows drop-needing
  ones, which is what the document says and what makes the loop and non-loop
  cases agree.

**Double-quoted strings now take escapes and interpolation.** They were raw,
which docs/jinn.md never said and which quietly broke 16 std modules that
write `"\n"`, `"\r\n"` or `"\x1b["`. Sixteen call sites across terminal,
signal and the tour were already written assuming interpolation, so making it
work fixed more than it changed; one snippet wanting a literal brace now
escapes it. `jinn fmt` was emitting string contents unescaped, which made this
non-idempotent — it now escapes properly.

**Other P0s, each verified by re-running the original reproduction:** join
points are unified in the typer with a diagnostic naming both branches
(this one defect was behind eight of the ten unimportable std modules);
MIR DCE no longer deletes trapping instructions, so `as strict` aborts at
every optimisation level; enum→int reads only the discriminant instead of
four bytes past the object, float→int saturates instead of emitting poison,
and explicit enum discriminants are carried through instead of being parsed
and discarded; `destroy` builds survivors in a temp file and renames instead
of truncating the live data file, and invalidates indexes that its offset
shift would otherwise leave pointing at the wrong row; the store has a real
intra-process writer lock (flock on a shared fd is a no-op between
coroutines); `together` inside a coroutine joins its children, by threading
the scope pointer through spawn rather than reading TLS that a context swap
has already invalidated; `%String` passes the data pointer rather than the
24-byte header, which was silently corrupting every hash and file path over
23 bytes; a constant in pattern position compares instead of binding, which
is why `json.parse` could not parse anything; `main` binds the success value
of a fallible call and exits non-zero on an unhandled error; four
use-after-free constructions in safe code are compile errors; dependency URLs
cannot escape the package cache, and a corrupt store header cannot overflow
the heap.

**ci/sanitize.sh is usable again.** It ran `cargo clean` and then the whole
suite twice, destroying the shared release build and taking about an hour, so
nobody ran it — which is why the review's memory findings rested on crash
signatures rather than an instrumented sweep. It now builds an instrumented
runtime into its own target directory and sweeps nine targeted programs under
ASan+UBSan and TSan in about two minutes. On its first run it caught a data
race in the actor-drain code added earlier in this same change.

**Benchmarks.** `coroutine_spawn` ran 100,000 iterations in Jinn against
1,000,000 in C, so the published ratio was wrong by 10x before the paradigm
difference was even considered. Counts realigned; the remaining gap is
ucontext's per-switch sigprocmask against a syscall-free context switch, and
says so at the top of the file. `run_benchmarks.py` now captures stdout for
every language, not just Jinn, and names any benchmark whose languages
disagree — which would have caught this for free.

**Tooling.** `jinn bind` emitted `//` comments, inlined C block comments into
signatures and bound macros as functions, so its output was rejected at line
1; it now produces parseable Jinn from real system headers and honestly skips
what it cannot represent, with a reason. The LSP treated UTF-16 positions as
byte offsets, so hover and goto-definition stopped working to the right of any
emoji. `jinn run` hashes dependency sources instead of trusting the entry
file alone; `.jni` interface reuse is off by default, since `to_decls()` never
reconstructed the functions list.

Regression coverage is in tests/alpha_review_regressions.rs and the corpus
differential harness. Still open, and recorded as such in the status document:
capabilities remain inert, generics remain narrow, transaction atomicity is
unchanged, and the fmt printer's grammar drift is untouched — the standing
recommendation for capabilities, `fmt --write`, `jinn bind` and `.jni` is
demotion rather than repair.

- **[140]** (2026-08-03 01:23) test suite 65s -> 40.5s: parallel test runner, fan-out corpus harnesses, sharded proptests
- **[140]** (2026-08-02) test suite 65s -> 40.5s

The edit-test loop was the bottleneck, not the build (an incremental
`cargo build --release` is 1.8s). Measured first: nearly every test forks
jinnc, and a two-line program costs ~90ms — ~16-22ms of process startup
(mostly mapping the 157MB libLLVM.so), ~8ms of actual compilation, and
35-50ms of `cc`/`ld`. The suite is process cost, not assertion logic.

Four changes, each measured:

- `scripts/test.sh` runs test *binaries* concurrently. `cargo test` walks
  targets strictly one at a time, so any suite that cannot saturate the box
  leaves cores idle — that was the single biggest waste. Same binaries, same
  assertions, same exit semantics; failing suites' logs are printed.
- Corpus harnesses that were one `#[test]` looping serially now fan out
  through `tests/support/parallel.rs` (`std::thread::scope`, no new
  dependency, input-order-preserving so failure output stays deterministic):
  programs_harness 7.15s -> 1.85s, doc_examples 3.58s -> 1.31s. doc_examples
  needed its sequential `doctest:file` state resolved into per-item state
  before the map, not inside it.
- ownership_fuzz's single 200-case proptest is now 8 shards of 25 so libtest
  can schedule them: 9.38s -> 2.56s. Same total cases.
- Compile-and-run property suites read their case count from
  `tests/support/cases.rs` (`JINN_PROPTEST_CASES`). Local default is small;
  CI and preflight set 64, so release gating keeps the full coverage and
  `.proptest-regressions` seeds replay at any count.

Rejected after measuring: swapping linkers (mold/lld absent, ld.gold is
*slower* than default ld) and raising concurrency past nproc — the reference
box is 4 real cores, and 8x8 oversubscription inflated CPU 207s -> 321s and
made wall time *worse*.

Fixed a bug in the new runner before trusting it: naming logs by basename
collapsed the two binaries both named `jinnc` (from src/lib.rs and
src/main.rs) into one file, silently dropping 309 lib tests. Test-count
parity with `cargo test` is now exact (2019), and failure detection was
verified with deliberate canaries in both an integration suite and
src/lib.rs. Rationale and the cost model are written up in docs/testing.md.
- **[139]** (2026-08-02 19:11) strip all code comments; reformat

Removed every comment from Rust (src, tests, fuzz, build.rs), C (runtime,
test harnesses), and Jinn (std, libjn, apps, examples, benchmarks,
snippets, test programs) sources. ~8100 lines of comment removed.

Used the existing scripts/strip_rust_comments.py for Rust — it is a real
lexer (raw strings with arbitrary hash counts, byte strings, char literals
vs lifetimes, nested block comments) and needed no changes. Added a
companion scripts/strip_c_comments.py for C and Jinn, which also drops
lines that held only a comment rather than leaving blank gaps. Verified
both preserve string and char literal contents: `"https://"`, `b'#'`, and
the printable-ASCII tables in std/binary|bytes|hex are all intact, and the
one surviving `/*` is inside a format string in src/bind.rs.

Reformatting then surfaced a collapsible_if in typer/lower/decl.rs that
the comments had been padding out; folded it into the let-chain. Fixed a
doubled-space run in the adjacent "declares returns X but its body
produces a different type" diagnostic while there.

Zero warnings, clippy clean, fmt clean, 2013/2013 tests, apps 21/21,
benchmarks 36/36, std gate green.
- **[138]** (2026-08-02 18:59) tests run in their own cwd; add gitignored .data/ for ad-hoc runs

A store resolves its .store/.wal relative to the process cwd, so any test
that compiled into a tempdir but ran the binary with the harness's cwd
wrote its state into the repo root. 18 run-sites across 11 test files did
exactly that; tests/error_effects.rs was the live offender, leaving
items.* and jobs.* behind on every `cargo test`.

Every compiled-binary run site now passes .current_dir(dir.path()) (or the
harness's scratch dir). A full suite run now leaves the root clean.

Added .data/ — a gitignored scratch cwd for running programs with
persistent state by hand, with a tracked README explaining why it exists.
Benchmarks already isolate into benchmarks/_build/cwd_*; unchanged.

Removed 30 stale .store/.wal files from the root (untracked, already
covered by .gitignore).
- **[137]** (2026-08-02 18:25) std gate green: all 49 alpha-stable modules type-check

The last failing test suite is now passing. Full suite 2013/2013, zero
failures, clippy clean, fmt clean, apps 21/21, benchmarks 36/36.

Two compiler defects found by working the gate:

1. extern opaque-pointer rule. A declared `%i8`/`%u8`/`%void` parameter is
   C's opaque-handle convention (`char *`/`void *`). The typer accepted a
   String there but rejected every other raw pointer, so passing a vec
   header to `jinn_spawn_exec(const void *)` was impossible — std/process
   could not bind its own runtime. At an extern boundary the C signature
   is the user's assertion; any raw pointer now satisfies an opaque
   pointee. Non-opaque pointees still unify strictly.

2. `copy` is a binding modifier, not an expression (access-semantics.md
   §2), so `copy self` in tail position fails. std/dataframe leaned on
   `return self` from a by-ptr method instead, handing out a borrow where
   the signature promised an owned DataFrame. Added `__clone_df` and
   routed all 7 "no matching column" paths through it.

std source fixes: bangle/http/event/url/terminal/dataframe. Types are
global and unqualified (std.md), so error unions are written `! NetError`,
not `! net.NetError`. http's PoolEntry.fd was i64 while every socket API
is i32.

New regression tests: method + ptr-method `! E` unions, no-Debug-spans in
error diagnostics, extern opaque-pointer FFI round-trip through a real
subprocess.
- **[136]** (2026-08-02 18:12) std gate: remove sqlite; fix `! E` on methods; clean error diagnostics

Removed std/sqlite.jn — the wrapper never type-checked and shipping a
broken binding is worse than shipping none. runtime/sqlite.c stays: it is
live FFI with a passing integration test, usable via `extern
*jinn_sqlite_*`. build.rs/jinn_rt.h/std.md updated.

Compiler bug found while fixing std/net: `! E` was desugared to
`Result of T, E` for free functions (resolve.rs:140) but BOTH method
signature paths dropped f.error_types, so `err X` inside any method was
rejected with "this function returns T". Methods now honor `! E`
end-to-end through codegen.

Six user-facing diagnostics printed raw `Span { start, end, line, .. }`
Debug output. All now use span.loc() -> file:line:col.

std sources fixed:
- argon/rational/date/logging: `"" + n` is not concatenation; the typer
  deliberately rejects it. Use to_string(n), the idiom the rest of std
  already uses.
- net: replaced the `return false` C-ism sentinel with a real `err
  NetError` union. This is what the error-effect system is for, and it
  unblocked bangle/http/event which consumed the sentinel API.

tests/programs/actors_multi.jn was racy — two independent actors have no
cross-mailbox ordering guarantee, so the snapshot flaked ~40% of runs.
Sequenced the drains; 20/20 deterministic.

std gate 13 -> 7 failing modules. clippy clean, fmt clean.
- **[135]** (2026-08-02 17:37) review eval: P1-6 — mir-drops summary is behind --debug-drops only

The driver printed the mir-drops summary on any compile where the pass
did useful work, so a clean user build leaked internal optimizer
telemetry to stderr. The roadmap's B.3 requires internal info to sit
behind an explicit flag. Gate solely on cli.debug_drops; tests/drops_debug
already passes the flag explicitly, so coverage is unchanged.

Tests 2008 passing, clippy clean, fmt clean.
- **[134]** (2026-08-02 17:35) review eval: reject unknown constructors; restore fmt gate

P0-class soundness hole found while auditing the std gate: the typer's
struct-literal path fell through to Type::Struct(name) for ANY unresolved
name, so `x is TotallyUndefinedThing()` compiled, linked, and ran. Every
typo in a constructor position was silently accepted, and the resulting
fabricated type is what produced the misleading "expected `None`, found
`Option__G_i64`" cascade in std/collections.jn.

- Reject constructor calls that name no declared type/actor/variant.
- Levenshtein-based "did you mean" over known types and variants.
- Name the Jinn spelling directly for foreign prelude words
  (None/Null/Nil -> Nothing, Just -> Some, Error -> Err).
- Fix spec divergence: jinn.md said `None`, implementation and
  error-effects.md/fmt.md say `Nothing`. `Nothing` wins; jinn.md and
  std/collections.jn corrected.
- cargo fmt --all: 58 files were drifted at HEAD, so CI's fmt gate was
  already red. Now clean.

std gate 14 -> 13 failing modules. Tests 2004 -> 2008. clippy clean,
fmt clean, apps 21/21, benchmarks 36/36.
- **[133]** (2026-08-02 17:23) review eval: fix consume-and-rebind false positive in move tracking

The post-lowering move pass (record_take_moves_in_stmt) re-marked a
consumed call argument as moved without observing that the enclosing
Bind/Assign re-initialized the target. `g is grow(g, x)` — the canonical
builder idiom — was therefore rejected as use-after-move, breaking 6 of
21 sample apps (blockchain_node, inventory_mgr, iot_telemetry, kv_store,
order_book, route_planner).

Clear the target's moved-state after recording the statement's moves, in
both Bind and Assign. Revival is precise: rebinding a different name
still leaves the source tombstoned.

apps 15/21 -> 21/21. Tests 2001 -> 2004 passing (3 new memory_model
regression tests pinning M2 revival: bind form, loop form, and the
negative case).
