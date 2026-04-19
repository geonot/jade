# Jade Language — Review: Remaining Action Items

**Date:** 2026-04-14 (revised)  
**Scope:** Unactioned items from comprehensive technical review, post-remediation  
**Prior status:** 73 of 95 original items resolved (fixed, confirmed non-issues, or obsolete). 22 remain.

---

## Table of Contents

1. [Summary](#1-summary)
2. [Critical — Must Fix](#2-critical--must-fix)
3. [High — Correctness / Compliance](#3-high--correctness--compliance)
4. [Medium — Design & Robustness](#4-medium--design--robustness)
5. [Low — Quality of Life](#5-low--quality-of-life)
6. [Consolidated Tracker](#6-consolidated-tracker)

---

## 1. Summary

After remediation, 22 items remain across the compiler, runtime, standard library, and language design. Two are critical soundness/integrity issues that were previously marked complete but verified as **still open** in the codebase.

| Severity | Remaining |
|----------|-----------|
| Critical | 2 |
| High | 2 |
| Medium | 9 |
| Low | 9 |
| **Total** | **22** |

### What was fixed (not repeated here)

All original critical items C2–C7, C9–C10 are confirmed fixed. All original high items H1–H19 are confirmed fixed (H15 partially — tracked below). All original medium items M1–M3, M5–M8, M10, M12, M14–M18, M20–M24, M26 are confirmed fixed or non-issues. All original low items L2–L7, L9–L12, L20–L21 are confirmed fixed or non-issues.

---

## 2. Critical — Must Fix

### 2.1 Non-Deterministic Type Resolution via HashMap (was C1)

**Status: STILL OPEN — not remediated despite prior marking**

`src/typer/mod.rs` uses `HashMap` for `structs`, `methods`, `enums`, `fns`, `actors`, `store_schemas`, and `traits`. When a method or field access occurs on an unresolved TypeVar and multiple candidate structs match, the first HashMap iteration hit wins. Since HashMap iteration order depends on the hasher seed, the **same program can type-check differently across compilations**.

This is a Heisenbug class — type errors are non-reproducible and method dispatch can silently select the wrong implementation.

**Impact:** Soundness. Silent wrong-code generation.  
**Fix:** Replace `HashMap` with `BTreeMap` or `IndexMap` for all type resolution maps in the typer, or sort candidates deterministically before selection.  
**Files:** `src/typer/mod.rs` (lines 41–79), `src/typer/call.rs`, `src/typer/expr.rs`

### 2.2 Transaction Keyword is Cosmetic (was C8/C12)

**Status: STILL OPEN — not remediated despite prior marking**

The `transaction` keyword in the language generates a plain sequential block — no WAL integration, no rollback on error, no isolation from concurrent readers. A crash mid-transaction leaves the store in a partial state with no recovery path.

`runtime/wal.c` has `fflush()` calls but no transaction begin/commit/rollback protocol. `src/codegen/stmt.rs` line 281-284 simply calls `self.compile_block(body)`.

**Impact:** Data integrity. Users who rely on the `transaction` keyword for atomicity will experience silent data corruption on failure.  
**Fix:** Either implement WAL-based transaction semantics (begin/commit/rollback) or remove/deprecate the `transaction` keyword to avoid false safety guarantees.  
**Files:** `src/codegen/stmt.rs`, `runtime/wal.c`

---

## 3. High — Correctness / Compliance

### 3.1 JSON Parser: Incomplete Unicode Escape Handling (was H15)

**Status: PARTIALLY FIXED**

Basic `\uXXXX` parsing was added — reads 4 hex digits and converts to a code point. However:
- **Surrogate pairs** (`\uD800`–`\uDFFF` for characters > U+FFFF) are not handled
- Non-ASCII printable Unicode code points are replaced with `"?"` instead of being emitted as UTF-8

This makes the parser non-compliant with RFC 8259 for any JSON containing characters outside ASCII or above the Basic Multilingual Plane.

**Impact:** Correctness. Data loss for non-ASCII JSON content.  
**Fix:** Implement surrogate pair decoding and UTF-8 emission for all code points.  
**File:** `std/json.jade` (lines 113–127)

### 3.2 SemVer Ordering Not Implemented (was M27)

**Status: PARTIALLY FIXED**

`SemVer` struct in `src/pkg.rs` has `major`, `minor`, `patch` as `u32` with correct parsing. Derives `Eq`/`PartialEq` but does **not** implement `PartialOrd`/`Ord`. Version equality works; version comparison (e.g., "is 1.2.3 compatible with >=1.2.0?") does not.

**Impact:** Package resolution cannot express version ranges or minimum-version constraints.  
**Fix:** Derive or implement `Ord`/`PartialOrd` on `SemVer`.  
**File:** `src/pkg.rs` (lines 5–35)

---

## 4. Medium — Design & Robustness

### 4.1 Formatter Idempotency Untested (was C11)

**Status: PARTIALLY FIXED**

The formatter was rewritten to be AST-based (parse → pretty-print), which is structurally idempotent. However, there are no round-trip tests asserting `format(format(src)) == format(src)`. Comment preservation is impossible since the lexer discards comments. Edge cases with complex expressions may still exist.

**Impact:** Risk of code destruction if edge cases remain.  
**Fix:** Add idempotency property tests; consider comment-preserving formatter design.  
**File:** `src/fmt.rs`

### 4.2 No Reference Cycle Detection in Perceus (was M9)

Perceus uses reference counting with no cycle collector. Circular `Rc` references leak memory permanently. The `Weak` type exists but nothing enforces its use to break cycles.

**Impact:** Memory leaks in programs with cyclic data structures.  
**Fix:** Add compile-time cycle detection heuristics, or a runtime cycle collector, or document the limitation prominently.  
**Files:** `src/perceus/`

### 4.3 No Preemption in Scheduler (was M19)

The M:N scheduler is fully cooperative. A compute-bound coroutine that never yields starves all other coroutines on the same worker thread.

**Impact:** Starvation under CPU-bound workloads.  
**Fix:** Timer-based preemption or automatic yield-point injection at loop back-edges.  
**Files:** `runtime/sched.c`

### 4.4 Grammar Specification Drift (was M4)

`jade.ebnf` is out of sync with the parser: `select`, store query clauses (`limit`, `sort`, `take`, `skip`, `set`, `delete`), `extern.method()`, and match guard keyword (`when` vs `if`) are undocumented or incorrect.

**Impact:** Developer confusion; grammar cannot be used for tooling or documentation.  
**Fix:** Regenerate `jade.ebnf` from the parser, or add CI checks for grammar/parser sync.  
**File:** `jade.ebnf`

### 4.5 Store Field Names as Code Identifiers (was M13)

Store queries encode field names directly into generated LLVM function names (`__store_query_{name}__{field}__{op}`) without sanitization. Safe in practice because names pass the parser first, but creates implicit coupling.

**Impact:** Low risk; maintenance concern.  
**Fix:** Sanitize or hash field names in generated identifiers.  
**File:** `src/codegen/mir_codegen.rs`

### 4.6 `send` Keyword Ambiguity (was M28)

`send` is used for both actor messaging (`send target, @handler(args)`) and channel sends (`send ch, value`) with different semantics. Distinction is based on the `@` prefix — fragile and prevents polymorphic abstractions.

**Impact:** Language design friction; potential for user confusion.  
**Fix:** Consider separate keywords (e.g., `tell` for actors) or unify under a trait/interface.

### 4.7 No Visibility Modifiers (was M29)

All top-level declarations are implicitly public. No `pub`/`private`/`export` mechanism exists. Internal implementation details leak through module boundaries.

**Impact:** No encapsulation; all modules expose everything.  
**Fix:** Add visibility modifiers to the declaration AST and enforce at import resolution.  
**File:** `src/ast.rs`, `src/parser/decl.rs`

### 4.8 Generator Semantics Underspecified (was M30)

No formal specification for: bidirectional yield, cleanup on abandonment, cross-channel transfer of generators, or generator panic behavior. The codegen control block is documented in code comments but not at the language level.

**Impact:** User confusion; undefined corner-case behavior.  
**Fix:** Write a generator semantics specification document.

### 4.9 Error Handling Model Unclear (was M31)

The language has `result` types, `??` for error returns, and pattern matching on results — but no `try`/`catch`, no `defer`/`finally`, and the relationship between panics, errors, and normal control flow is undefined.

**Impact:** Users cannot reason about error propagation guarantees.  
**Fix:** Define and document the error model (Rust-style Result vs exceptions vs hybrid).

---

## 5. Low — Quality of Life

### 5.1 Raw Strings Cannot Contain Quotes (was L1)

No escape mechanism for `"` inside raw strings. Languages like Rust solve this with `r#"..."#`.

**File:** `src/lexer.rs` (`lex_raw_string()`)

### 5.2 Select Single Wait Node Inefficiency (was L8)

Coroutines use a single `next` pointer for wait queues, requiring poll-retry loops (up to 256 retries) instead of multi-wait. Correct but less efficient.

**File:** `runtime/select.c`

### 5.3 `?` Operator in Three Contexts (was L13)

`?` serves as ternary operator, match arm separator, and interacts with `??` (error return). Cognitive load for users.

### 5.4 No Re-export Mechanism (was L14)

Modules cannot re-export imported symbols. Wrapper libraries must force users to import inner dependencies directly.

### 5.5 Incomplete Pattern Matching in for/let (was L15)

Pattern matching works in `match` and pattern-directed functions but not in `for` loop destructuring or lambda parameters.

### 5.6 Implicit Main Wrapping Fragility (was L16)

Top-level statements auto-wrap in `*main()`. Scoping rules for how `use`, type declarations, and store declarations interact with the implicit main are underspecified.

### 5.7 Persistence Leaks Implementation (was L17)

The `store` abstraction exposes: field order dependence, 256-byte string maximum, full-table-scan semantics, and no schema evolution story.

### 5.8 Matrix Operations Always Allocate (was L18)

`3 by 3` matrix syntax allocates on every operation. No in-place operations, stride specification, or allocation control.

### 5.9 Test Coverage Gaps (was L22)

Not tested at all: formatter idempotency, LSP protocol compliance, lock file round-tripping, package resolution, OOM behavior, concurrent channel contention, store crash recovery.

Poorly tested (<20%): store migrations, generic bounds, trait dispatch, advanced patterns, Unicode handling, generator lifecycle, Perceus correctness, incremental cache correctness.

---

## 6. Consolidated Tracker

### Critical

| # | Area | Issue | Status |
|---|------|-------|--------|
| 1 | Type System | Non-deterministic method/field resolution via HashMap | **Open** — verified in `src/typer/` |
| 2 | Store/Codegen | Transaction keyword is cosmetic (no atomicity) | **Open** — verified in `src/codegen/stmt.rs` |

### High

| # | Area | Issue | Status |
|---|------|-------|--------|
| 3 | Stdlib | JSON `\uXXXX` missing surrogate pairs + non-ASCII | **Partial** — basic support added |
| 4 | Pkg | SemVer has no `Ord` impl (equality only) | **Partial** — struct exists, ordering missing |

### Medium

| # | Area | Issue | Status |
|---|------|-------|--------|
| 5 | Tooling | Formatter idempotency untested | **Partial** — structural fix, no tests |
| 6 | Ownership | No reference cycle detection in Perceus | **Deferred** — design needed |
| 7 | Runtime | No preemption in scheduler | **Deferred** — design needed |
| 8 | Parser | Grammar spec drift from implementation | **Deferred** |
| 9 | Codegen | Store field names as code identifiers | **Deferred** — low risk |
| 10 | Design | `send` keyword ambiguity | **Deferred** — design needed |
| 11 | Design | No visibility modifiers | **Deferred** — design needed |
| 12 | Design | Generator semantics underspecified | **Deferred** — spec needed |
| 13 | Design | Error handling model unclear | **Deferred** — spec needed |

### Low

| # | Area | Issue | Status |
|---|------|-------|--------|
| 14 | Lexer | Raw strings can't contain quotes | **Deferred** |
| 15 | Runtime | Select single wait node inefficiency | **Deferred** |
| 16 | Design | `?` operator in three contexts | **Deferred** |
| 17 | Design | No re-export mechanism | **Deferred** |
| 18 | Design | Incomplete pattern matching in for/let | **Deferred** |
| 19 | Design | Implicit main wrapping fragility | **Deferred** |
| 20 | Design | Persistence leaks implementation | **Deferred** |
| 21 | Design | Matrix operations always allocate | **Deferred** |
| 22 | Testing | Coverage gaps across subsystems | **Deferred** |

---

*Revised from 95-item review. 73 items confirmed resolved. This document tracks the 22 remaining items, reprioritized by verified impact.*
