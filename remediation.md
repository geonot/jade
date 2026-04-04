# Jade — Feature Remediation Plan

Issues discovered during the jade.md audit. Each item describes the current state, root cause, and what needs to happen to make the feature fully functional.

---

## P0 — Wiring Gaps (feature exists in HIR/codegen, just not connected)

### 1. Atomic operations not registered as builtins
- **Current:** `atomic_add`, `atomic_load`, `atomic_store`, `atomic_cas` have full HIR enum variants and LLVM codegen, but are not in the typer's builtin dispatch table.
- **Root cause:** Missing entries in `src/typer/builtins.rs`.
- **Fix:** Add `"atomic_add" | "atomic_load" | "atomic_store" | "atomic_cas"` to the builtin match in `src/typer/builtins.rs`, mapping to the existing `hir::BuiltinFn::Atomic*` variants with correct arg counts and types.

### 2. Syscall not reachable from source
- **Current:** `Expr::Syscall` exists in the AST and codegen handles it, but the parser never produces it — `syscall` is not recognized as a keyword or builtin.
- **Root cause:** No `Token::Syscall` in the lexer keyword table and no parse rule in `src/parser/expr.rs`.
- **Fix:** Add `("syscall", Token::Syscall)` to the lexer keyword list. Add a parse branch in `parse_primary` or `parse_call` that recognizes `syscall` followed by comma-separated args and produces `Expr::Syscall`.

### 3. Default parameters rejected by HIR validator
- **Current:** `*fn(host as String, port as i64 is 8080)` parses correctly and the default is stored in the AST, but calling with fewer args than declared produces a validation error.
- **Root cause:** `src/hir_validate.rs` checks arg count against param count without accounting for params that have defaults.
- **Fix:** In the arg-count validation logic, compute `min_args = params.count(|p| p.default.is_none())`. Accept calls where `min_args <= call_args.len() <= params.len()`. In the typer/codegen, fill missing args with their default expressions.

### 4. `$` placeholder codegen failure
- **Current:** `nums ~ $ * 2` parses into an implicit lambda, but codegen fails with "cannot call non-function type."
- **Root cause:** The desugared lambda's type isn't propagated correctly through the pipeline — the `~` codegen doesn't recognize the generated closure as callable.
- **Fix:** Trace the desugar output in `src/parser/expr.rs` (placeholder expansion) and ensure the resulting `Expr::Lambda` carries a proper `FnType`. Verify the pipeline codegen in `src/codegen/expr.rs` handles anonymous lambdas the same way it handles named function references.

### 5. Vec slicing runtime not linked
- **Current:** `v from 1 to 3` parses and codegen emits a call to `__jade_vec_slice`, but linking fails — the symbol is undefined.
- **Root cause:** The runtime function `__jade_vec_slice` is not implemented in any `.c` file under `runtime/`, or not linked.
- **Fix:** Implement `__jade_vec_slice(vec_ptr, start, end) -> vec_ptr` in a runtime C file (e.g., `runtime/vec.c`). Ensure `build.rs` compiles and links it.

### 6. `in` on array literals
- **Current:** `x in [1, 2, 3]` may fail at codegen when the RHS is a fixed-size array literal.
- **Root cause:** The `in` desugaring to `.contains()` assumes a Vec receiver; fixed arrays lack the contains method dispatch.
- **Fix:** In the `in` codegen path (`src/codegen/expr.rs`), detect fixed-array RHS and emit an inline linear scan loop instead of dispatching to a method.

---

## P1 — Codegen Missing (parsed & typed, needs LLVM IR generation)

### 7. Signal handling i64/i32 type mismatch
- **Current:** `signal_handle`, `signal_ignore`, `signal_raise` compile but the signal number is passed as i64 where libc's `signal()` expects i32.
- **Root cause:** Codegen in `src/codegen/builtins.rs` doesn't truncate the i64 arg to i32 before calling the libc function.
- **Fix:** Add `build_int_truncate(sig_val, i32_type, "sig.trunc")` before the libc call in each signal builtin's codegen.

### 8. Comptime reflection returns wrong values
- **Current:** `size_of('Point')` returns 0 for primitives. `type_of(42)` returns garbled output. `fields_of('Point')` returns a raw pointer.
- **Root cause:** The codegen for `CompTimeSizeOf`, `CompTimeTypeOf`, `CompTimeFieldsOf` in `src/codegen/builtins.rs` uses placeholder implementations.
- **Fix:**
  - `size_of`: Use `self.target_data.get_store_size(llvm_type)` to get actual byte size.
  - `type_of`: Return a global constant string with the type's display name.
  - `fields_of`: Build a Vec of Strings from the struct's field names at compile time and return it as a proper Jade Vec.

### 9. Deque codegen
- **Current:** `deque()` is parsed and typed, but codegen does not implement it. Runtime support exists in `runtime/deque.c`.
- **Root cause:** No codegen path for deque construction or method calls in `src/codegen/`.
- **Fix:** Add deque builtin codegen similar to Vec — `deque()` calls a runtime constructor, methods (`push_back`, `push_front`, `pop_front`, `pop_back`, `len`) dispatch to corresponding `runtime/deque.c` functions. Wire up in `src/codegen/builtins.rs` and `src/codegen/expr.rs` (method dispatch).

### 10. Serialization casts (`as json`, `as map`)
- **Current:** `my_struct as json` parses but returns an empty string at runtime.
- **Root cause:** Codegen for serialization casts in `src/codegen/expr.rs` returns a placeholder empty string.
- **Fix:** For `as json`: walk the struct's fields at compile time, generate code that builds a JSON string by formatting each field. For `as map`: generate code that creates a Map and inserts each field. For `json_str as Config`: parse JSON at runtime using a simple runtime JSON parser, populate struct fields.

### 11. Builder blocks (HIR path)
- **Current:** `build TypeName` blocks are parsed. MIR desugars them to struct construction. HIR codegen does not handle them.
- **Root cause:** No `build` handling in `src/codegen/stmt.rs` or `src/codegen/expr.rs` for the HIR path.
- **Fix:** In HIR codegen, treat `build TypeName` the same way MIR does — desugar to a struct constructor call with the builder body's bindings as field initializers. Alternatively, desugar at the HIR level in the typer.

### 12. Supervisor trees
- **Current:** `supervisor` blocks are parsed but skipped during type checking and codegen.
- **Root cause:** No type-checking or codegen implementation.
- **Fix:** In the typer, validate supervisor blocks — check that `strategy` is one of `one_for_one|one_for_all|rest_for_one`, validate `children` are `spawn` expressions. In codegen, generate a supervisor actor that monitors child actors and implements the restart strategy. Requires actor failure detection (trap exits) in the runtime (`runtime/actor.c`).

### 13. SIMD — no vector IR
- **Current:** `SIMD of f32, 4(...)` is parsed and typed. The type system has SIMD types. But codegen does not emit LLVM vector types or SIMD instructions.
- **Root cause:** No SIMD codegen path in `src/codegen/`.
- **Fix:** Map `Type::SIMD(elem, lanes)` to LLVM's `VectorType::get(elem_type, lanes)`. Implement arithmetic ops (`+`, `-`, `*`, `/`) on SIMD types using LLVM vector instructions. Support lane extraction/insertion via indexing.

### 14. Einsum notation
- **Current:** `einsum 'ij,jk->ik', A, B` is parsed and typed but has no codegen.
- **Root cause:** No implementation in codegen.
- **Fix:** Parse the Einstein index string at compile time. For known patterns (`ij,jk->ik` = matmul, `ii->` = trace, `i,i->` = dot), emit calls to optimized runtime functions or inline loop nests. For general contractions, emit nested loops with the appropriate index structure.

### 15. Automatic differentiation (`grad`)
- **Current:** `grad(loss)` is parsed and typed but has no codegen.
- **Root cause:** No AD transform implemented.
- **Fix:** Implement reverse-mode AD as a source-to-source transform. For each supported function, generate the adjoint. Run the AD transform before codegen — produce a new function that computes the gradient. Restrict to `f64` arithmetic initially. Error on unsupported ops (side effects, control flow beyond `if`).

### 16. Regex string methods type mismatch
- **Current:** `.matches()`, `.find_all()`, `.replace_re()` dispatch in codegen but fail with a type mismatch — String value passed where ptr expected.
- **Root cause:** The regex runtime functions expect raw `char*` pointers, but codegen passes Jade's String fat pointer (ptr+len+cap) without extracting the data pointer.
- **Fix:** In the regex method codegen path in `src/codegen/strings.rs`, extract the string's data pointer via SSO-aware logic (same as other string methods) before passing to the PCRE2 runtime functions. Ensure null-termination or pass length explicitly.

### 17. Weak references
- **Current:** `weak` is a reserved keyword and `Type::Weak` exists, but `weak expr` is not parseable as an expression and `weak_upgrade` is not a builtin.
- **Root cause:** No parse rule for `weak` as a prefix expression. No builtin registration.
- **Fix:**
  - Parser: Add `Token::Weak` as a prefix in `parse_primary` — `weak expr` produces `Expr::Weak(inner)`.
  - Typer: Type `Expr::Weak` — inner must be `Type::Rc(T)`, result is `Type::Weak(T)`.
  - Builtins: Register `weak_upgrade` — takes `Type::Weak(T)`, returns `Type::Option(Type::Rc(T))`.
  - Codegen: `weak` = rc_retain a weak pointer (separate weak count in RC header). `weak_upgrade` = check strong count > 0, return Some(rc) or None.
  - Runtime: Extend the RC header in `runtime/` to include a weak count field.

### 18. COW (Copy-on-Write)
- **Current:** `Type::Cow` exists in the type system but codegen is not implemented.
- **Root cause:** No COW detection or clone-on-mutate insertion.
- **Fix:** Implement COW for Strings and Vecs:
  - When assigning `b is a` for a String/Vec, share the backing buffer and increment the RC.
  - On mutation of `b`, check RC > 1: if so, clone the buffer before writing (COW trigger).
  - Requires RC-aware string/vec mutation paths in codegen.

---

## P2 — MIR Backend Parity

### 19. Integer overflow builtins (MIR)
- **Current:** `wrapping_add`, `saturating_add`, `checked_add` (and sub/mul variants) work with HIR codegen but not MIR.
- **Root cause:** MIR codegen in `src/codegen/mir_codegen.rs` doesn't handle these builtin variants.
- **Fix:** Add match arms for all overflow builtin enum variants in the MIR codegen's builtin dispatch. Emit the same LLVM intrinsics (`llvm.sadd.with.overflow.*`, `llvm.sadd.sat.*`, etc.) as the HIR path.

### 20. Generators end-to-end
- **Current:** Generators are parsed and typed. MIR has coroutine infrastructure (context switching, suspend/resume). But `.next()` dispatch on generator return values is not working.
- **Root cause:** The generator's return type isn't properly wrapped as an iterator-like object that supports `.next()`. The method dispatch doesn't recognize generator objects.
- **Fix:** When a function contains `yield`, the typer should wrap its return type as `Type::Generator(T)`. Method dispatch for `.next()` on `Type::Generator(T)` should emit a coroutine resume call. Ensure the MIR coroutine infrastructure (context save/restore in `runtime/coro.c`) is reachable from the `.next()` codegen path.

---

## P3 — Enforcement & Polish

### 21. ALL_CAPS immutability enforcement
- **Current:** `BAR is 10` inside a struct provides a default but `x.BAR is 20` compiles and overwrites it.
- **Root cause:** No immutability check for ALL_CAPS fields.
- **Fix:** In `src/hir_validate.rs` or the typer, detect assignments to ALL_CAPS fields and emit an error: "cannot reassign constant field `BAR`."

### 22. Integer methods codegen (`to_float`, `clamp`)
- **Current:** `to_float`, `clamp` are listed in the typer's integer method table (`src/typer/lower.rs:856`) but codegen emits "unknown char method."
- **Root cause:** The codegen's method dispatch for integer types doesn't handle these methods.
- **Fix:** In the integer method codegen path (likely `src/codegen/expr.rs` or `src/codegen/builtins.rs`):
  - `to_float`: emit `sitofp` (signed int to f64) or `uitofp` (unsigned).
  - `clamp(lo, hi)`: emit `max(lo, min(x, hi))` using integer comparisons and selects.
  - `abs`: emit `x < 0 ? -x : x`.
  - `min`/`max`: emit integer compare + select.

### 23. Float `clamp` method
- **Current:** `clamp` is in the integer methods list but NOT in the float methods list. Neither works at codegen.
- **Root cause:** Missing from the float methods array in `src/typer/lower.rs:796`.
- **Fix:** Add `"clamp"` to the `float_methods` array. In `src/codegen/builtins.rs`, the `FloatMethod("clamp")` case already has a codegen implementation using `build_select` — verify it's reachable.

---

## Summary

| Priority | Count | Theme |
|----------|-------|-------|
| **P0** | 6 | Wiring — feature exists, needs connection |
| **P1** | 12 | Codegen — parsed/typed, needs IR generation |
| **P2** | 2 | MIR parity — works in HIR, missing in MIR |
| **P3** | 3 | Polish — enforcement and method completeness |
| **Total** | **23** | |

Lowest-effort / highest-impact items: **#1** (atomics — add 4 lines to builtins.rs), **#3** (defaults — adjust one arg-count check), **#7** (signals — add one truncate), **#23** (float clamp — add one string to an array).
