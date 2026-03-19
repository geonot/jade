# Jade Technical Discovery Review 2026

## Scope

This review evaluates Jade as it exists in the current repository, not just as described in its aspirational design documents. The perspective here is intentionally mixed: compiler engineering, systems implementation, and programming-languages research.

Primary evidence came from the implementation in [src](../src), the design docs in [ARCHITECTURE.md](../ARCHITECTURE.md), [SPECIFICATION.md](../SPECIFICATION.md), [LANGUAGE_REFERENCE.md](../LANGUAGE_REFERENCE.md), [MANIFESTO.md](../MANIFESTO.md), [ROADMAP.md](../ROADMAP.md), the overview in [jade.md](../jade.md), the test harnesses in [tests](../tests), and the benchmark tooling in [run_benchmarks.py](../run_benchmarks.py) plus [benchmarks](../benchmarks).

Current build health was revalidated locally with `LLVM_SYS_211_PREFIX=/usr/lib/llvm-21 cargo test`, which passes all 535 tests described in [ROADMAP.md](../ROADMAP.md).

## Executive Judgment

Jade is not vaporware. It is a real, capable compiler prototype with an unusually broad surface area for its size, strong test coverage, and a direct native-code story through LLVM. The frontend is materially better than a typical experiment, the language surface is coherent, and the current implementation already supports a meaningful systems-language subset.

At the same time, Jade is not yet the architecture described in its most ambitious design documents. The central technical gap is that semantics are still concentrated inside LLVM code generation rather than factored into distinct passes for lowering, resolution, type inference, ownership, and verification. That is acceptable for a fast-moving prototype. It is not acceptable for the long-term goals Jade claims for ownership inference, borrow checking, Perceus-style reuse, separate compilation, and industrial-quality diagnostics.

Short version:

- Jade today is a strong single-compiler-pass prototype.
- Jade is not yet a research-grade ownership language.
- Jade can plausibly become one, but only if it narrows and formalizes its semantic core before adding further surface features.

## What Jade Is Today

The most accurate high-level description of the current compiler is the one in [jade.md](../jade.md): source goes through lexer, parser, and direct LLVM IR generation. That matches the actual binary entrypoint in [src/main.rs](../src/main.rs), which performs lexing, parsing, recursive module loading, code generation, object emission, and final native linking in a tight pipeline.

The implementation boundary is small and clear:

- Module loading and whole-program assembly live in [src/main.rs](../src/main.rs).
- Syntax trees live in [src/ast.rs](../src/ast.rs).
- Lexing and layout-sensitive tokenization live in [src/lexer.rs](../src/lexer.rs).
- Parsing lives in [src/parser.rs](../src/parser.rs).
- Types are represented in [src/types.rs](../src/types.rs).
- Diagnostics are rendered in [src/diagnostic.rs](../src/diagnostic.rs).
- Most semantic behavior, generic instantiation, and runtime policy lives in [src/codegen.rs](../src/codegen.rs).

This matters because several other documents, especially [ARCHITECTURE.md](../ARCHITECTURE.md), describe a future multi-pass compiler with explicit lowering, name resolution, inference, ownership checking, and monomorphization stages. That future architecture is not implemented today.

## Round-Table Findings

### Compiler Engineering Panel

#### 1. The frontend is stronger than the backend architecture

The parser in [src/parser.rs](../src/parser.rs) is a real asset. It handles a large language surface cleanly: functions, structs, enums, matches, lambdas, interpolation, list comprehensions, pipes, placeholders, raw pointers, syscalls, inline assembly, `use`, and error definitions. The AST in [src/ast.rs](../src/ast.rs) is expressive enough to support this breadth without collapsing into ad hoc token-level hacks.

The lexer/parser combination therefore looks like a solid experimental frontend.

The backend architecture is less mature. The decisive signal is not just file size; it is semantic location. In [src/codegen.rs](../src/codegen.rs), code generation also performs or approximates:

- expression typing via `expr_ty`
- return inference via `infer_ret`
- field inference via `infer_field_ty`
- generic normalization and monomorphization
- RC allocation and lifetime operations
- parts of call coercion and ABI adaptation

That concentration accelerates prototyping, but it creates three scaling problems:

- error quality degrades because semantic intent is discovered too late
- correctness becomes backend-dependent
- every new feature raises the coupling cost superlinearly

#### 2. The real compiler boundary is AST to LLVM, with no semantic IR

There is no implemented HIR, typed IR, or ownership IR between [src/parser.rs](../src/parser.rs) and [src/codegen.rs](../src/codegen.rs). The architecture diagram in [ARCHITECTURE.md](../ARCHITECTURE.md) shows multiple intermediate phases, but the code in [jade.md](../jade.md) and [src/main.rs](../src/main.rs) correctly reflects the current reality: direct AST-to-LLVM compilation.

That is the single most important architectural fact about Jade today.

For a prototype, this is a valid choice. For the roadmap in [ARCHITECTURE.md](../ARCHITECTURE.md) and [SPECIFICATION.md](../SPECIFICATION.md), it is the main blocker.

#### 3. Module handling is textual whole-program assembly, not separate compilation

`resolve_modules` in [src/main.rs](../src/main.rs) recursively loads `.jade` files, parses them, and appends declarations into one program. This is simple and works. It is not separate compilation, not independent module codegen, and not yet the architecture claimed in [ARCHITECTURE.md](../ARCHITECTURE.md).

This design has immediate consequences:

- no stable module interface representation
- no cross-module invalidation discipline
- no principled namespace or visibility model yet
- no true incremental compilation

If Jade wants serious tooling and scale, this area must be redesigned before the language surface expands much further.

#### 4. Diagnostics are readable but still minimal

The renderer in [src/diagnostic.rs](../src/diagnostic.rs) is good enough for a compact compiler. It emits spans and notes, and it integrates with codegen errors cleanly.

However, Jade does not yet have:

- structured diagnostic codes tied to semantic phases
- recovery-oriented parse diagnostics
- phase-specific fix-it suggestions
- a robust distinction between syntax, name, type, and ownership errors

That is not a cosmetic issue. Once ownership and generics become stricter, diagnostics quality will determine whether the language is usable.

### Systems Panel

#### 5. Jade already behaves like a serious low-level language experiment

On the systems side, Jade is more credible than most young languages. It already exposes:

- C FFI
- raw pointers
- inline assembly
- syscalls
- native LLVM calling conventions and object emission

The CLI path in [src/main.rs](../src/main.rs) emits an object file and links with `cc`, which is a straightforward and respectable native-toolchain story.

That said, the current implementation is still backend-centric rather than memory-model-centric.

#### 6. `Rc` exists, but Jade does not yet implement the ownership story it advertises

The docs repeatedly describe compiler-inferred ownership, borrowing, move semantics, and Perceus-style optimization in [MANIFESTO.md](../MANIFESTO.md), [LANGUAGE_REFERENCE.md](../LANGUAGE_REFERENCE.md), [SPECIFICATION.md](../SPECIFICATION.md), and [ARCHITECTURE.md](../ARCHITECTURE.md).

What is actually implemented today is more limited and more concrete:

- `Type::Rc` exists in [src/types.rs](../src/types.rs)
- RC allocation and lifetime operations exist in [src/codegen.rs](../src/codegen.rs)
- raw pointers exist in the AST and codegen path
- there is no separate ownership inference pass
- there is no separate borrow checker or ownership verifier module

This distinction matters. Ordinary retain/release machinery is not the same thing as an ownership discipline. And neither of those is the same thing as Perceus.

Jade currently has RC support. It does not yet have the ownership-and-borrow system described as the language default.

#### 7. “Perceus-style” should be treated as a future direction, not a present claim

Perceus is not just reference counting. Perceus, as developed in Koka, relies on precise retain/release insertion, garbage-freedom, and reuse analysis over an appropriate core language. The important point is not the presence of `retain` and `release`; it is the compiler analysis that proves when reuse and early deallocation are valid.

The RC helpers in [src/codegen.rs](../src/codegen.rs) are useful infrastructure, but they are not yet a Perceus implementation in the research sense. The review recommendation is simple: Jade should stop claiming Perceus as a present-tense implementation detail and instead describe it as the intended optimization family for a later ownership IR.

#### 8. The benchmark story is promising but not yet fully disciplined

The benchmark harness in [run_benchmarks.py](../run_benchmarks.py) is better than average for a prototype. It builds Jade in release mode, times medians across multiple runs, compares against C, Rust, and Python, and stores history with timestamps and platform metadata.

That is the good news.

The caution is that the checked-in artifacts are mixed:

- [benchmarks/results.json](../benchmarks/results.json) currently contains only Python timings.
- [benchmarks/history.json](../benchmarks/history.json) contains the fuller C and Rust comparisons.
- adjacent saved runs show materially different comparative outcomes on some benchmarks, which suggests toolchain sensitivity or benchmark instability that is not yet normalized in presentation.

Therefore the performance claims in [ROADMAP.md](../ROADMAP.md) are plausible, but not yet publication-grade. Jade needs pinned toolchains, machine specs, input-size disclosure, and CI-based benchmark reproduction before the ratio claims should be treated as stable evidence.

### Programming Languages Theory Panel

#### 9. Jade has a surface language worthy of formalization, but not yet a semantic core worthy of ownership claims

This is the central theoretical judgment.

Jade already has enough language structure to deserve a formal core:

- algebraic data types
- generics
- closures
- pattern matching
- low-level escape hatches
- RC and pointer forms

But the ownership story in the docs is ahead of the formal and implementation story in the compiler.

If Jade wants to claim inferred ownership with no user-visible lifetime annotations, it needs a core calculus that makes provenance, aliasing, and resource transfer explicit somewhere, even if the source language stays lightweight.

The closest research lesson here comes from Oxide: Rust-like ownership and borrowing become tractable only when the core model of references and provenances is made explicit enough to reason about. Jade does not need to copy Rust. It does need an internal model at that level of clarity.

#### 10. Bidirectional typing is the right next move

Jade’s docs repeatedly mention HM plus bidirectional typing. That is directionally correct.

The reason is not fashion. Bidirectional typing remains the cleanest way to scale inference while keeping both predictability and good diagnostics, especially once generics, expected-type propagation, and richer low-level types interact. Dunfield and Krishnaswami remain the most practical foundation for that move.

Today, type behavior is still partly heuristic and backend-coupled in [src/codegen.rs](../src/codegen.rs). Jade should move to:

- a synthesis mode for expressions that naturally produce types
- a checking mode for expressions validated against an expected type
- a typed intermediate representation emitted before LLVM

That change would improve both correctness and errors immediately.

#### 11. Jade should resist the temptation to stack advanced type features too early

There is frontier work available on principal inference with levels, row/effect systems, algebraic subtyping, and richer overload-resolution theories. Those are all interesting.

They are not Jade’s current bottleneck.

The bottleneck is simpler:

- factor semantics out of codegen
- formalize ownership categories
- define coercions and ABI rules precisely
- introduce module interfaces and typed IR

Jade will get more value from a disciplined core than from adding another expressive surface feature.

## Research Alignment

### Perceus, Koka, and FBIP

Koka and Perceus are relevant to Jade for two reasons:

- they show that precise RC with reuse can be competitive
- they show that “functional but in-place” optimization is real when the compiler’s semantic core is designed for it

The lesson for Jade is not “add retain/release helpers.” The lesson is “build a core language and ownership IR where uniqueness, reuse opportunities, and lifetime endpoints are explicit enough to optimize mechanically.”

Jade’s current codebase is compatible with that destination, but it has not reached it.

### Bidirectional Typing

The right theoretical move for Jade is a bidirectional typing architecture beneath the current surface syntax. That directly supports:

- generic functions without backend guesswork
- better integer literal coercion rules
- predictable function and method checking
- cleaner error reporting for low-level operations

This is likely the highest-leverage theoretical upgrade available right now.

### Oxide and Borrow Semantics

If Jade wants real borrowing without Rust syntax, it still needs a borrow semantics somewhere. Oxide’s major lesson is that the hard part is not syntax; it is internal accounting of reference provenance and exclusivity.

Jade therefore needs to decide whether its future ownership model is closer to:

- Rust-like exclusive and shared borrows with explicit internal provenance tracking
- Koka-like uniqueness and reuse with RC fallback
- or a hybrid model with a deliberately smaller set of guarantees

All three are possible. What is not possible is to claim all of them at once without a sharply defined core.

### Verona and Future Concurrency

If Jade eventually wants actors, cloud isolation, or compartmentalized services, Project Verona is the more relevant frontier reference than Rust. Verona’s questions around regions, isolation, and eliminating concurrent mutation are highly relevant to Jade’s future systems roadmap.

This is not a suggestion to import Verona wholesale. It is a warning that concurrency-safe ownership models change the language core. Jade should delay concurrency design until its single-threaded ownership and aliasing story is formalized.

## Strengths

- The implemented compiler is real, coherent, and test-backed.
- The parser and AST already support a rich, nontrivial language.
- LLVM integration is direct and effective.
- The language is unusually ambitious while still readable.
- The tests in [tests](../tests) are strong evidence that current behavior is not accidental.
- The benchmark harness is a meaningful start rather than an afterthought.

## Risks

- Semantic logic is still too concentrated in [src/codegen.rs](../src/codegen.rs).
- Ownership claims outrun ownership implementation.
- Module handling will not scale to a real package ecosystem.
- Diagnostics will become a major usability bottleneck once ownership rules harden.
- Benchmark claims are ahead of reproducibility discipline.
- The design docs mix current state and future state, which makes technical status easy to misread.

## Recommended Technical Program

### Stage 1: Stabilize the truth

First, align documentation around two categories:

- implemented today
- planned architecture

In particular, [ARCHITECTURE.md](../ARCHITECTURE.md) should be marked as target architecture, while [jade.md](../jade.md) and [ROADMAP.md](../ROADMAP.md) should continue to describe present reality.

### Stage 2: Introduce one semantic IR

Add exactly one intermediate representation before LLVM. Not three. One.

Recommended shape:

- parsed AST
- resolved and typed HIR
- LLVM lowering

This HIR should carry:

- resolved names
- explicit generic instantiations
- explicit ownership category per binding and parameter
- explicit coercions
- explicit drops or ownership events

Without this layer, Jade will keep rediscovering semantics inside codegen.

### Stage 3: Make ownership explicit internally

Do not try to infer ownership directly into LLVM. Infer it into the HIR.

At minimum, Jade needs internal categories for:

- owned values
- immutable borrows
- mutable borrows
- RC values
- raw pointer escapes

Then add a verifier pass that checks exclusivity, move-after-use, and drop placement. Only after that should reuse optimization or Perceus-style transformations be attempted.

### Stage 4: Keep `rc` explicit and conservative

The cleanest near-term strategy is:

- keep `rc` explicit at the source level for now
- make non-`rc` values follow a simpler ownership discipline
- infer borrowing conservatively
- delay aggressive auto-promotion to shared ownership

This avoids overpromising on invisible ownership decisions that may become unpredictable for users.

### Stage 5: Upgrade diagnostics before expanding surface area

Before adding traits, effects, or more advanced storage abstractions, Jade should add:

- phase-specific errors
- stable diagnostic codes
- notes that explain inferred ownership decisions
- ownership-focused help messages

This will matter more to real users than another syntax feature.

### Stage 6: Harden benchmark methodology

The benchmark suite should be pinned and reproducible:

- lock compiler versions and flags
- record CPU model and governor
- publish median and variance
- distinguish historical best runs from current checked-in results
- validate generated outputs across languages automatically

This is the difference between a compelling engineering signal and a marketing number.

## Final Assessment

Jade is in the narrow band of language projects that are already technically interesting before 1.0. The frontend quality, passing tests, and direct native code path make it much more serious than a manifesto project.

Its main challenge is not ambition. Its main challenge is sequencing.

If Jade now freezes the semantic core, introduces one typed IR, and formalizes ownership internally before continuing to widen the language surface, it has a credible path toward becoming a distinctive systems language.

If instead it keeps adding features while leaving types, ownership, reuse, and modules embedded inside backend logic, the project will hit the classic wall: growing syntax, shrinking semantic clarity, and rapidly worsening compiler maintainability.

The recommendation from all three panels is therefore the same: narrow, formalize, then grow.

## Evidence Appendix

Implementation anchors reviewed:

- [src/main.rs](../src/main.rs)
- [src/ast.rs](../src/ast.rs)
- [src/types.rs](../src/types.rs)
- [src/diagnostic.rs](../src/diagnostic.rs)
- [src/lexer.rs](../src/lexer.rs)
- [src/parser.rs](../src/parser.rs)
- [src/codegen.rs](../src/codegen.rs)
- [tests/integration.rs](../tests/integration.rs)
- [tests/bulk_tests.rs](../tests/bulk_tests.rs)

Design and status anchors reviewed:

- [jade.md](../jade.md)
- [ARCHITECTURE.md](../ARCHITECTURE.md)
- [SPECIFICATION.md](../SPECIFICATION.md)
- [LANGUAGE_REFERENCE.md](../LANGUAGE_REFERENCE.md)
- [MANIFESTO.md](../MANIFESTO.md)
- [ROADMAP.md](../ROADMAP.md)
- [run_benchmarks.py](../run_benchmarks.py)
- [benchmarks/history.json](../benchmarks/history.json)
- [benchmarks/results.json](../benchmarks/results.json)

External research touchstones used to frame recommendations:

- Perceus and FP2 / FBIP from the Koka line of work
- Dunfield and Krishnaswami on bidirectional typing
- Oxide as a core formal account of Rust-style ownership and borrowing
- Verona as a guide for later isolation and region-based concurrency design