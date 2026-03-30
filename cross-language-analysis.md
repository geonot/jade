# Cross-Language Feature Analysis for Jade

**Scope:** Comprehensive examination of syntax, semantics, paradigms, and design patterns across 80+ languages, distilled into concrete recommendations for Jade.

**Method:** Each section identifies a concept, surveys how languages handle it, and evaluates fit with Jade's ethos: *clear English over opaque symbols, systems performance, scripting readability, zero-cost abstractions, ownership without GC.*

---

## Table of Contents

1. [Pattern Matching & Destructuring](#1-pattern-matching--destructuring)
2. [Type System Enhancements](#2-type-system-enhancements)
3. [Error Handling](#3-error-handling)
4. [Functional Programming Primitives](#4-functional-programming-primitives)
5. [Concurrency & Parallelism](#5-concurrency--parallelism)
6. [Metaprogramming & Compile-Time Computation](#6-metaprogramming--compile-time-computation)
7. [Memory & Ownership](#7-memory--ownership)
8. [Module System & Visibility](#8-module-system--visibility)
9. [Traits, Interfaces & Polymorphism](#9-traits-interfaces--polymorphism)
10. [Collections & Iteration](#10-collections--iteration)
11. [String & Text Processing](#11-string--text-processing)
12. [Control Flow Innovations](#12-control-flow-innovations)
13. [Operator Design & Extensibility](#13-operator-design--extensibility)
14. [FFI & Interop](#14-ffi--interop)
15. [Build, Package & Tooling](#15-build-package--tooling)
16. [Configuration & Data Formats](#16-configuration--data-formats)
17. [Testing & Verification](#17-testing--verification)
18. [Documentation as Code](#18-documentation-as-code)
19. [DSL & Embedded Language Support](#19-dsl--embedded-language-support)
20. [Syntax & Readability Philosophy](#20-syntax--readability-philosophy)
21. [AI, ML & Numerical Computing](#21-ai-ml--numerical-computing)
22. [Odds & Ends — Algebra, Encodings, Grammars & Everything Else](#22-odds--ends--algebra-encodings-grammars--everything-else)
23. [Summary: Priority-Ranked Recommendations](#23-summary-priority-ranked-recommendations)

---

## 1. Pattern Matching & Destructuring

### Survey

| Language | Feature | Syntax |
|----------|---------|--------|
| **Haskell** | Guards in patterns, where-clauses, as-patterns | `f xs@(x:_) \| x > 0 = ...` |
| **OCaml/F#** | Or-patterns, when-guards, nested match | `match x with A \| B -> ...` |
| **Rust** | Exhaustive match, if-let, while-let, `@` bindings, ref patterns | `if let Some(x) = opt { ... }` |
| **Scala** | Extractor objects, sealed traits, case classes | `case Person(name, _) => ...` |
| **Elixir** | Pin operator in patterns, binary patterns, map patterns | `^x = value` |
| **Swift** | Value binding, tuple patterns, optional patterns | `case let .some(x) where x > 0:` |
| **Erlang** | Bit-syntax patterns, map patterns, binary matching | `<<A:8, Rest/binary>>` |
| **Prolog** | Unification-based pattern matching | `append([H\|T], L, [H\|R]) :- append(T, L, R).` |
| **Raku** | Multi-dispatch, where-clauses, smart matching | `multi sub f(Int $x where * > 0) { }` |

### Jade Status

Jade has: enum destructuring, literal patterns, wildcards, or-patterns, nested match, exhaustiveness checking. Missing: `if let` / `while let`, as-patterns (`@`), guard clauses in match arms, map/struct destructuring in patterns, range patterns in match, named field patterns.

### Recommendations for Jade

**A. `if let` / `while let` (from Rust/Swift) — HIGH PRIORITY**

Already identified as gap G4 in the bootstrap roadmap. This is the single most impactful missing pattern feature. Every optional unwrap currently requires a full match block.

```jade
# Current (verbose)
match map.get('key')
    Some(v) ? log v
    Nothing ? log 'missing'

# Proposed
if let Some(v) is map.get('key')
    log v
else
    log 'missing'

# While-let for iterators/streams
while let Some(item) is stream.next()
    process item
```

The `let ... is` phrasing fits Jade's English ethos perfectly — it literally reads as a question: "if let some v is the result...".

**B. Guard Clauses on Match Arms (from Haskell/OCaml/Rust)**

```jade
match value
    x when x > 0 ? log 'positive'
    x when x < 0 ? log 'negative'
    _ ? log 'zero'
```

The `when` keyword already exists in the lexer. This adds expressive power without new syntax weight.

**C. As-Patterns / Named Bindings (from Haskell's `@`)**

```jade
match list
    whole as Cons(head, tail) ? log '{whole} starts with {head}'
```

Uses the `as` keyword Jade already has, giving it a third context (type annotation, cast, pattern binding) which is a natural English reading: "the whole thing _as_ Cons(head, tail)".

**D. Struct/Map Destructuring in Patterns (from Elixir/Scala)**

```jade
match config
    {host, port is 443} ? use_tls host
    {host, port} ? connect host, port
```

This would let Jade match on partial struct shapes, analogous to Elixir's map pattern matching.

**E. Range Patterns (from Rust)**

```jade
match char_code
    48 to 57 ? log 'digit'
    65 to 90 ? log 'upper'
    97 to 122 ? log 'lower'
    _ ? log 'other'
```

Jade already uses `to` in for-loops. Reusing it in patterns is natural.

---

## 2. Type System Enhancements

### Survey

| Language | Feature | Design |
|----------|---------|--------|
| **Haskell** | Typeclasses, higher-kinded types, GADTs, type families | `class Functor f where fmap :: (a -> b) -> f a -> f b` |
| **OCaml** | Polymorphic variants, structural typing, modules/functors | `` `Variant of int `` |
| **Rust** | Associated types, where-clauses, trait bounds, const generics | `impl<T: Display + Clone>` |
| **TypeScript** | Union types, intersection types, mapped types, conditional types, literal types | `type Result = Success \| Error` |
| **Kotlin** | Sealed classes, inline classes, reified generics, nullable types | `sealed class Result` |
| **Swift** | Protocol-oriented, associated types, opaque types (`some`) | `some View` |
| **Scala** | Path-dependent types, higher-kinded types, implicits, union/intersection | `type T = A & B` |
| **Julia** | Multiple dispatch on all arg types, abstract types, parametric types | `f(x::Int, y::Float64) = ...` |
| **Ada** | Subtypes, constrained types, discriminated records | `subtype Positive is Integer range 1..Integer'Last` |
| **Zig** | Comptime types, optional types, error unions | `?T`, `T!E` |

### Jade Status

Jade has: monomorphized generics via `of`, structs, enums, traits with associated types, dynamic dispatch via fat pointers, integer width coercion. Missing: union types, intersection types, type aliases, constrained types/subtypes, sealed enums (all are sealed by default—good), const generics, where-clauses on generics.

### Recommendations for Jade

**A. Type Aliases (from Haskell/Rust/TypeScript) — MEDIUM PRIORITY**

```jade
alias Callback is (i64) returns i64
alias Matrix is Vec of Vec of f64
alias Result of T is Option of T    # generic alias
```

Makes complex type signatures readable. The `alias` keyword reads as English. Zero runtime cost—pure compiler convenience.

**B. Where-Clauses on Generics (from Rust/Haskell) — MEDIUM PRIORITY**

```jade
*sort of T(items as Vec of T) where T has Ord
    ...

*serialize of T(value as T) where T has Display
    to_string value
```

The `where T has Trait` phrasing is pure English. Currently Jade monomorphizes everything, which means type errors only appear at instantiation sites. Where-clauses catch them at definition sites.

**C. Union Types (from TypeScript/Scala 3) — LOW PRIORITY (future)**

```jade
type StringOrInt is String or i64

*format(value as String or i64)
    match value
        s as String ? s
        n as i64 ? to_string n
```

This is a lightweight alternative to enums for ad-hoc polymorphism. TypeScript showed this is enormously useful. But it requires significant type system work.

**D. Constrained Numeric Types (from Ada/Pascal) — LOW PRIORITY (future)**

```jade
type Port is i64 where 1 to 65535
type Percentage is f64 where 0.0 to 100.0
```

Ada's subtypes catch entire classes of bugs at compile time. The `where ... to ...` syntax reuses keywords Jade already has.

**E. Literal Types (from TypeScript) — LOW PRIORITY**

```jade
type Direction is 'north' or 'south' or 'east' or 'west'
```

Powerful for DSLs and configuration types. Low priority because Jade's enums already serve this role.

---

## 3. Error Handling

### Survey

| Language | Approach | Mechanism |
|----------|----------|-----------|
| **Rust** | Result/Option types + `?` propagation | `let v = f()?;` |
| **Go** | Multiple return values, explicit checks | `val, err := f(); if err != nil { }` |
| **Zig** | Error unions, `try`, `catch`, `errdefer` | `const val = try f();` |
| **Swift** | throws/try/catch + Result type | `do { try f() } catch { }` |
| **Haskell** | Either/Maybe monads + do-notation | `do { x <- f; g x }` |
| **OCaml** | Result type + exceptions (dual system) | `match f () with Ok v -> ... \| Error e -> ...` |
| **Elixir** | Tagged tuples `{:ok, val}` / `{:error, reason}` | `with {:ok, x} <- f(), do: ...` |
| **Java/C#** | Checked/unchecked exceptions | `try { } catch (Exception e) { }` |
| **Erlang** | "Let it crash" + supervisors | `try Expr catch _:_ -> handle end` |
| **Kotlin** | Unchecked exceptions + Result type | `runCatching { f() }.getOrElse { default }` |

### Jade Status

Jade has: `err` definitions (like Rust enums for errors), `!` operator for error return (early return on error). Missing: `try`/`catch` syntax sugar, `?`-like propagation chaining, `errdefer` (cleanup on error path), error context/wrapping, error chaining.

### Recommendations for Jade

**A. Error Propagation Shorthand (Rust's `?` analogue) — HIGH PRIORITY**

Jade's `!` already returns errors. But there's no concise way to *unwrap-or-propagate*:

```jade
# Current pattern
result is read_file path
match result
    NotFound ? ! NotFound
    PermissionDenied(m) ? ! PermissionDenied(m)
    data ? process data

# Proposed: `try` keyword
data is try read_file path        # unwraps Ok, propagates error automatically
processed is try process data
```

The `try` keyword reads naturally: "try to read the file." If it fails, the error automatically returns from the current function. This is the single highest-value ergonomic improvement for error-heavy code.

**B. Error Context / Wrapping (from Rust's `anyhow`, Go's `fmt.Errorf`) — MEDIUM PRIORITY**

```jade
data is try read_file(path) with 'failed to read {path}'
```

The `with` clause adds context to propagated errors. This is invaluable for debugging — you get a chain of *why* rather than just the leaf error.

**C. `errdefer` / Cleanup on Error Path (from Zig) — LOW PRIORITY**

```jade
*open_connection host
    conn is connect host
    errdefer close conn        # only runs if this function returns an error
    try authenticate conn
    try handshake conn
    conn
```

This is a composable resource cleanup mechanism. It only fires on the error path, meaning the happy path has zero cost.

**D. `else` on Error Match (from Elixir's `with`) — MEDIUM PRIORITY**

```jade
result is read_file path else NotFound
    log 'file not found, using default'
    default_config
```

This combines error checking with handling in a single expression.

---

## 4. Functional Programming Primitives

### Survey

| Language | Feature | Mechanism |
|----------|---------|-----------|
| **Haskell** | map/filter/fold/zip/scan, lazy lists, function composition, partial application | `map f . filter g $ xs` |
| **OCaml/F#** | Pipeline operator, List module, partial application | `xs \|> List.filter f \|> List.map g` |
| **Scala** | Collection hierarchy, for-comprehensions, implicits | `xs.filter(_ > 0).map(_ * 2)` |
| **Elixir** | Enum/Stream modules, pipe operator, comprehensions | `xs \|> Enum.filter(&(&1 > 0))` |
| **Kotlin** | Sequences (lazy), scope functions, extension functions | `xs.filter { it > 0 }.map { it * 2 }` |
| **Rust** | Iterator trait, combinators, lazy evaluation, `collect()` | `xs.iter().filter(\|x\| **x > 0).map(\|x\| x * 2).collect()` |
| **Swift** | Higher-order methods on collections | `xs.filter { $0 > 0 }.map { $0 * 2 }` |
| **Clojure** | Transducers, threading macros | `(->> xs (filter pos?) (map #(* % 2)))` |
| **APL/J** | Tacit programming, trains, rank operator | `+/⍳10` (sum of 1 to 10) |
| **Julia** | Broadcasting, do-block syntax for closures | `map(x -> x^2, xs)` |
| **Raku** | Hyper operators, junctions, feed operator | `@xs».method` |

### Jade Status

Jade has: `~` pipeline operator, first-class functions, closures (capture by value), lambda syntax `*fn(x) expr`, higher-order functions. Missing: standard map/filter/reduce operations on collections, partial application, function composition operator, lazy iterators/sequences, comprehension syntax for arbitrary iterables.

### Recommendations for Jade

**A. Built-in Collection Methods (from Kotlin/Rust/Swift) — HIGH PRIORITY**

The iterator protocol exists (`impl Iter of T for Type`). Surface it with methods:

```jade
result is items
    ~ filter *fn(x) x > 0
    ~ map *fn(x) x * 2
    ~ fold 0, *fn(acc, x) acc + x

# Or as methods on Vec
evens is numbers.filter *fn(x) x mod 2 equals 0
total is numbers.fold 0, *fn(acc, x) acc + x
doubled is numbers.map *fn(x) x * 2
```

These are the bread and butter of modern programming. Every language from Haskell to JavaScript to Rust has them. They pair perfectly with Jade's pipe operator.

**B. Placeholder Lambda Shorthand (from Scala/Kotlin/Elixir) — MEDIUM PRIORITY**

```jade
# Current
items ~ filter *fn(x) x > 0

# Proposed: _ placeholder
items ~ filter(_ > 0)
items ~ map(_ * 2)
items ~ fold(0, _ + _)
```

Scala uses `_`, Kotlin uses `it`, Elixir uses `&(&1)`. The underscore is cleanest and already has meaning in Jade patterns (wildcard). Inside a function-argument position, `_` could create an anonymous lambda.

**C. Partial Application (from Haskell/OCaml/F#) — LOW PRIORITY**

```jade
*add a, b is a + b
add5 is add(5, _)       # partial: fills first arg
result is add5 10        # 15
```

Haskell auto-curries everything. That's too implicit for Jade. But explicit partial application with `_` gives the power without the magic.

**D. Spread / Splat Operator (from Ruby/JavaScript/Python) — MEDIUM PRIORITY**

```jade
*sum ...args
    total is 0
    for x in args
        total is total + x
    total

result is sum 1, 2, 3, 4, 5

# Spread into call
nums is [1, 2, 3]
result is sum ...nums
```

Variadic functions readable as English: "sum of these args." The `...` prefix is widely understood.

---

## 5. Concurrency & Parallelism

### Survey

| Language | Model | Mechanism |
|----------|-------|-----------|
| **Erlang/Elixir** | Actor model, message passing, supervisors | `spawn(fun() -> ... end)`, `receive ... end` |
| **Go** | CSP: goroutines + channels + select | `go func()`, `ch <- v`, `select { case ... }` |
| **Rust** | Ownership-based thread safety, async/await, channels | `tokio::spawn(async { })` |
| **Kotlin** | Coroutines, structured concurrency, channels, flows | `launch { }`, `async { }.await` |
| **Swift** | Structured concurrency, async/await, actors, task groups | `async let`, `await`, `actor` |
| **Julia** | Tasks, channels, `@threads`, `@distributed` | `@spawn expr` |
| **Haskell** | STM, MVars, green threads, async | `atomically $ do { ... }` |
| **Clojure** | STM, atoms, agents, core.async channels | `(go (>! ch val))` |
| **Java** | Virtual threads (Loom), CompletableFuture, structured concurrency | `Thread.startVirtualThread(() -> ...)` |
| **Zig** | Async frames, suspend/resume, no runtime | `async f()`, `await handle` |

### Jade Status

Jade has: actors (Erlang-style with pthread per actor), channels (bounded MPSC), coroutines (dispatch/yield), select blocks. But actors are still on raw pthreads (not migrated to the cooperative scheduler), channels/select/coroutines have zero tests.

### Recommendations for Jade

**A. Structured Concurrency (from Kotlin/Swift/Java Loom) — HIGH PRIORITY**

```jade
*fetch_all urls
    parallel
        for url in urls
            fetch url
    # all tasks complete before this line — no leaked goroutines
```

Or task groups:

```jade
*process_batch items
    results is tasks
        for item in items
            yield process item
    # results is Vec of all return values
```

Structured concurrency guarantees: no task outlives its parent scope. This eliminates the most common class of concurrent bugs (leaked goroutines, orphaned threads, dangling futures). Swift and Kotlin proved this model works at scale.

**B. Async/Await — Consider NOT Adding (Deliberate Omission)**

Go deliberately omits async/await. Everything is synchronous from the programmer's perspective; the runtime handles multiplexing. Jade's coroutine model could follow this path:

```jade
# Instead of async/await coloring the whole call stack:
*fetch url
    data is http_get url    # blocks this coroutine, not the OS thread
    parse data
```

The "colored function" problem (async vs sync) is one of the worst ergonomic failures in modern language design. If Jade's scheduler can multiplex coroutines onto OS threads transparently, async/await becomes unnecessary. This is a *feature*, not a gap.

**C. Supervisor Trees (from Erlang/OTP) — MEDIUM PRIORITY (future)**

```jade
supervisor Main
    strategy is one_for_one
    children
        Worker1
        Worker2
        Worker3
```

Erlang's "let it crash" philosophy with supervisor-based recovery is the most battle-tested approach to fault tolerance in concurrent systems. Jade's actor model is already Erlang-inspired; supervisors are the natural next step.

**D. Parallel Iterators (from Rust's Rayon, Java's parallelStream) — MEDIUM PRIORITY**

```jade
result is items
    ~ parallel_map *fn(x) expensive_compute x
    ~ fold 0, *fn(acc, x) acc + x
```

Or a simpler keyword form:

```jade
for x in items parallel
    process x
```

Data parallelism is the easiest form of parallelism to reason about. A parallel-for or parallel-map with automatic work stealing would make Jade competitive for data processing.

**E. Atomic Operations & Lock-Free Primitives — MEDIUM PRIORITY**

```jade
counter is atomic 0
atomic_add counter, 1
value is atomic_load counter
if atomic_cas counter, expected, new_value
    log 'swapped'
```

Jade already has `volatile_load`/`volatile_store`. Atomic operations are the next step for lock-free algorithms. These should be builtins, not library functions, because the compiler needs to emit the right LLVM atomic instructions.

---

## 6. Metaprogramming & Compile-Time Computation

### Survey

| Language | Feature | Mechanism |
|----------|---------|-----------|
| **Rust** | Procedural macros, `macro_rules!`, const fn, const generics | `#[derive(Debug)]` |
| **Zig** | Comptime execution, comptime types, `@typeInfo` | `comptime { }` |
| **Lisp/Scheme/Racket** | Homoiconic macros, syntax transformers, reader macros | `(defmacro when (test &body body) ...)` |
| **Julia** | Expression macros, generated functions | `@macro expr` |
| **Nim** | Templates, macros, compile-time execution | `macro name(args): untyped = ...` |
| **C/C++** | Preprocessor macros, templates, constexpr, concepts | `template<typename T>`, `consteval` |
| **Haskell** | Template Haskell, deriving, type-level computation | `$(deriveJSON defaultOptions ''MyType)` |
| **Elixir** | Hygienic macros, compile-time code generation | `defmacro unless(clause, do: expression) do ... end` |
| **D** | CTFE, mixins, string imports | `mixin("int x = 3;");` |

### Jade Status

Jade has: comptime constant folding only. No macros, no compile-time code generation, no derive/annotation system, no conditional compilation.

### Recommendations for Jade

**A. Decorators / Annotations (from Python/Java/Rust/TypeScript) — HIGH PRIORITY**

```jade
@derive Display, Eq
type Point
    x as i64
    y as i64

@inline
*hot_path x
    x * 2

@deprecated 'use new_api instead'
*old_api x
    new_api x

@test
*test_addition
    assert add(1, 2) equals 3
```

Decorators are the lightest-weight metaprogramming. They don't require a macro system — the compiler just recognizes a fixed set of annotations and acts on them. Jade already has `@packed`, `@strict`, `@align` for types; extending this to functions and arbitrary items is natural.

Key annotations for Jade:
- `@derive` — auto-generate trait implementations (Display, Eq, Ord, Hash, Clone)
- `@inline` / `@noinline` — optimization hints
- `@test` — mark test functions
- `@deprecated` — compiler warnings
- `@cold` / `@hot` — branch prediction hints
- `@extern "C"` — calling convention

**B. Compile-Time Function Execution (from Zig/D/C++ consteval) — MEDIUM PRIORITY**

```jade
comptime *fib n
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)

# This is computed at compile time, result baked into binary
table is comptime [fib(i) for i from 0 to 20]
```

Jade already has a `comptime` module that does constant folding. Extending it to evaluate arbitrary pure functions at compile time is the natural evolution. Zig proved this model is both powerful and understandable.

**C. Conditional Compilation (from Rust/C/Zig) — MEDIUM PRIORITY**

```jade
when platform equals 'linux'
    *open_file path
        # linux implementation

when platform equals 'macos'
    *open_file path
        # macos implementation
```

The `when` keyword exists in Jade's lexer. Using it for conditional compilation reads perfectly: "when the platform is linux..."

**D. Derive System for Traits (from Rust/Haskell/Kotlin) — HIGH PRIORITY**

This is the killer feature of annotation-based metaprogramming. Writing `Display`, `Eq`, `Ord`, `Hash` implementations by hand is pure boilerplate:

```jade
@derive Display, Eq, Ord, Hash
type User
    name as String
    age as i64

# Compiler generates:
# *display self is 'User(name is {self.name}, age is {self.age})'
# *eq self, other is self.name equals other.name and self.age equals other.age
# etc.
```

Haskell and Rust both proved that derive is one of the most-used language features. It eliminates the most common boilerplate.

---

## 7. Memory & Ownership

### Survey

| Language | Model | Details |
|----------|-------|---------|
| **Rust** | Affine types, borrow checker, lifetimes | `&'a T`, `&mut T`, `Box<T>`, `Arc<T>` |
| **Swift** | ARC (automatic reference counting), copy-on-write | Always ref-counted, compiler optimizes |
| **C++** | RAII, unique_ptr/shared_ptr, move semantics | `std::move()`, destructors |
| **Zig** | Manual, allocator-aware, no hidden allocations | `allocator.alloc(T, n)` |
| **Lobster** | Compile-time RC with elision (Perceus ancestor) | Automatic, zero-overhead for most code |
| **Koka** | Perceus reference counting, reuse analysis | Functional + owned values |
| **Vale** | Generational references, region-based | `&'r T` |
| **Austral** | Linear types (must use exactly once) | Enforced at type level |

### Jade Status

Jade has: ownership by default, borrowing (free), Perceus RC with 9-pass optimization (borrow elision, reuse analysis, speculative reuse, drop fusion), `rc`/`weak` for shared ownership, `@packed`/`@strict`/`@align(N)` layout control, SSO for strings. Performance: 0.72× Clang (28% *faster*).

### Recommendations for Jade

**A. Arena / Region-Based Allocation (from Zig/C++/Vale) — MEDIUM PRIORITY**

```jade
*parse source as String
    arena is Arena(4096)
    tokens is arena.alloc(Vec of Token)
    # ... build AST in arena ...
    # entire arena freed at scope exit — one free() call instead of thousands
```

Compilers, parsers, and game engines all benefit enormously from arena allocation. A single deallocation at scope exit instead of per-object drops. This is especially important for Jade's self-hosting goal — the compiler will allocate millions of AST nodes.

**B. Allocator-Aware Collections (from Zig/C++) — LOW PRIORITY**

```jade
*process items
    scratch is StackAlloc(8192)   # fixed buffer on stack
    temp is Vec.with_alloc(scratch)
    for item in items
        temp.push transform(item)
    # temp freed with scratch at scope exit
```

Zig's approach of threading allocators through all collection operations gives maximum control. But Jade's ethos leans toward simplicity. A middle ground: let arena/pool allocators exist, but don't require API-level allocator parameters on every collection.

**C. Move Semantics with `consume` Keyword (Explicit) — LOW PRIORITY**

Jade already moves by default. But for clarity at API boundaries:

```jade
*take_ownership(consume data as Vec of i64)
    # caller can no longer use `data`
    process data
```

The `consume` keyword makes the transfer of ownership visible at the call site. Rust has this implicitly; making it explicit fits Jade's "reads like what it means" philosophy.

**D. Copy-on-Write for Strings/Vecs (from Swift) — LOW PRIORITY**

When a shared `rc` value is mutated and has RC=1, Jade's Perceus reuse analysis already does in-place update. For RC>1, copy-on-write would create a fresh copy automatically. This is what Swift does for its collections. Low priority because Jade's existing RC model handles this adequately.

---

## 8. Module System & Visibility

### Survey

| Language | Model | Features |
|----------|-------|----------|
| **Rust** | mod/use, pub/pub(crate)/pub(super), workspace/crate | Fine-grained visibility |
| **Go** | Package = directory, capitalization = exported | Simplest model |
| **OCaml** | ML module system, signatures, functors | `module M : SIG = struct ... end` |
| **Python** | File = module, `__init__.py` for packages, `__all__` | `from x import y` |
| **Kotlin** | Packages + internal visibility | `internal`, `private` |
| **Zig** | File = struct, `pub` visibility, `@import` | `const std = @import("std")` |
| **Elixir** | Modules, behaviours, protocols | `defmodule M do ... end` |
| **Haskell** | Module with export list | `module M (f, g) where` |

### Jade Status

Jade has: file = module, `use` for imports, `pub` for public types. Missing: selective imports (`use math.{sin, cos}`), re-exports, nested modules, visibility for functions/fields, module-level documentation.

### Recommendations for Jade

**A. Selective Imports (from Python/Rust/Haskell) — HIGH PRIORITY**

```jade
use math.{sin, cos, pi}
use io.{read_file, write_file}
```

Without selective imports, every use statement imports the entire module namespace. This creates name collisions and makes it unclear which module provides which function.

**B. Import Aliases (from Python/Rust/Kotlin) — MEDIUM PRIORITY**

```jade
use long_module_name as lmn
use io.read_file as read
```

Essential for avoiding name collisions and reducing verbosity with deeply nested module paths.

**C. Field Visibility (from Rust/Kotlin) — MEDIUM PRIORITY**

```jade
type Connection
    pub host as String
    pub port as i64
    secret as String       # private by default (or explicitly `priv`)
```

Currently Jade fields are all accessible. For encapsulation — especially in library code — private fields are essential. The Go convention (capitalization) is too implicit. An explicit `pub` on fields, with private-by-default, fits Jade's explicit ethos.

**D. Re-exports (from Rust/Haskell) — LOW PRIORITY**

```jade
# std/prelude.jade
pub use math.{sin, cos, pi}
pub use io.{read_file, write_file}
pub use fmt.{format, pad_left, pad_right}
```

This lets library authors curate a clean public API surface without exposing internal module structure.

---

## 9. Traits, Interfaces & Polymorphism

### Survey

| Language | Mechanism | Key Feature |
|----------|-----------|-------------|
| **Rust** | Traits + associated types + default impls + coherence | `impl Display for MyType` |
| **Haskell** | Typeclasses + superclasses + default methods | `class Eq a => Ord a` |
| **Go** | Structural interfaces (implicit satisfaction) | `type Writer interface { Write([]byte) (int, error) }` |
| **Swift** | Protocols + extensions + conditional conformance | `extension Array: Equatable where Element: Equatable` |
| **Scala** | Traits with state + self-types + linearization | `trait Ordered[T] extends Comparable[T]` |
| **Kotlin** | Interfaces + delegation + extension functions | `class C(b: Base) : Base by b` |
| **Java** | Interfaces + default methods + sealed interfaces (Java 17+) | `interface Comparable<T>` |
| **C++** | Concepts (C++20), virtual dispatch, CRTP | `template<typename T> requires Printable<T>` |
| **Clojure** | Protocols + multimethods | `(defprotocol IFn (invoke [this]))` |
| **Julia** | Multiple dispatch, abstract types | `f(x::Animal, y::Food) = ...` |

### Jade Status

Jade has: trait definitions, impl blocks, associated types, dynamic dispatch via fat pointers (vtable), operator overloading (Add/Sub/Mul/Div/Lt/Gt/Le/Ge/Display). Missing: default methods in traits, trait inheritance/supertraits, conditional impls, extension methods, delegation.

### Recommendations for Jade

**A. Default Methods in Traits (from Rust/Java/Kotlin) — HIGH PRIORITY**

```jade
trait Printable
    *display self as String     # required

    *println self               # default implementation
        log self.display()
```

This dramatically reduces boilerplate when implementing traits. You define a minimal set of required methods and provide defaults for the rest.

**B. Trait Inheritance / Supertraits (from Rust/Haskell) — MEDIUM PRIORITY**

```jade
trait Ord extends Eq
    *compare self, other as Self returns i64

# Implementing Ord requires also implementing Eq
impl Ord for Point
    *compare self, other
        self.x - other.x
```

The `extends` keyword is clear English. This creates a hierarchy: Ord implies Eq, Display + Eq might imply Hashable, etc.

**C. Extension Methods (from Kotlin/Swift/C#) — MEDIUM PRIORITY**

```jade
extend i64
    *is_even self is self mod 2 equals 0
    *is_odd self is not self.is_even()
    *times self, f
        for i from 0 to self
            f i

# Usage
if n.is_even()
    log 'even'
5.times *fn(i) log i
```

Kotlin's extension functions are one of its most beloved features. They let users add methods to existing types without modifying them. The `extend` keyword reads naturally.

**D. Delegation (from Kotlin) — LOW PRIORITY**

```jade
type LoggingWriter
    inner as Writer

    delegate Writer to inner
```

The compiler generates forwarding methods for all trait methods. Eliminates the boilerplate of wrapper types.

---

## 10. Collections & Iteration

### Survey

| Language | Collections | Iteration Model |
|----------|-------------|-----------------|
| **Rust** | Vec, HashMap, BTreeMap, HashSet, BTreeSet, VecDeque, LinkedList | Iterator trait + combinators |
| **Haskell** | List, Map, Set, Seq, Vector, Array | Foldable/Traversable typeclasses |
| **Python** | list, dict, set, tuple, deque, Counter, defaultdict | `__iter__`/`__next__` protocol |
| **Kotlin** | List, MutableList, Map, Set, Sequence (lazy) | Iterable/Sequence + extension fns |
| **Scala** | Immutable + mutable variants, views (lazy), parallel | Collection hierarchy + views |
| **Swift** | Array, Dictionary, Set + Sequence/Collection protocols | for-in protocol |
| **Go** | Slice, map, channel (no generics until 1.18) | `range` keyword |
| **Java** | List, Map, Set, Queue, Deque + Stream API | Iterator + Stream |
| **C++** | vector, map, set, unordered_map, deque, list, array | begin/end iterators |
| **Julia** | Array, Dict, Set, Tuple + comprehensions + broadcasting | for-loop + iterator protocol |
| **APL/J** | Multi-dimensional arrays as fundamental type | Rank/shape operators |

### Jade Status

Jade has: Vec, Map (HashMap), Array (fixed), Tuple, Iterator protocol (`impl Iter of T`). Missing: Set, Deque, sorted map/set, lazy iterators/sequences, iterator combinators (map/filter/fold/zip/take/skip/enumerate).

### Recommendations for Jade

**A. Iterator Combinators (from Rust/Kotlin/Scala) — HIGH PRIORITY**

This is the most impactful collection feature. Every modern language has these:

```jade
# Core combinators
items.map(f)            # transform each element
items.filter(f)         # keep elements where f returns true
items.fold(init, f)     # reduce to single value
items.enumerate()       # yields (index, value) pairs
items.zip(other)        # pairs elements from two iterables
items.take(n)           # first N elements
items.skip(n)           # skip first N elements
items.any(f)            # true if any element satisfies f
items.all(f)            # true if all elements satisfy f
items.find(f)           # first element satisfying f (returns Option)
items.count()           # number of elements
items.sum()             # sum of numeric elements
items.flatten()         # flatten nested iterables
items.chain(other)      # concatenate two iterables
items.collect()         # materialize into a Vec

# All work with pipes
result is numbers
    ~ filter *fn(x) x > 0
    ~ map *fn(x) x * 2
    ~ fold 0, *fn(a, x) a + x
```

These should be implemented as default methods on the `Iter` trait, so any type implementing `next` automatically gets the full combinator suite.

**B. Set Collection (from most languages) — MEDIUM PRIORITY**

```jade
seen is set()
seen.add 'apple'
seen.add 'banana'
if seen.contains 'apple'
    log 'found'

# Set operations
union is a.union(b)
diff is a.difference(b)
inter is a.intersection(b)
```

Already identified as gap G7. Currently worked around with `Map of String, bool`.

**C. Deque / Ring Buffer (from Python/Rust/C++) — LOW PRIORITY**

```jade
q is deque()
q.push_front 1
q.push_back 2
first is q.pop_front()
last is q.pop_back()
```

Useful for BFS, sliding window algorithms, and work-stealing schedulers.

**D. Lazy Sequences (from Haskell/Kotlin/Clojure) — MEDIUM PRIORITY**

```jade
# Generates values on demand — no allocation for intermediate results
naturals is sequence from 1
first_100_evens is naturals
    ~ filter *fn(x) x mod 2 equals 0
    ~ take 100
    ~ collect()
```

Without lazy evaluation, chaining `filter ~ map ~ take` creates intermediate arrays. With lazy sequences, each combinator pulls from the previous one on demand. Zero intermediate allocation.

**E. `for`-`in` on Tuples and Maps (from Python/Kotlin/Go) — HIGH PRIORITY**

```jade
# Iterate map with key-value pairs
for key, value in map
    log '{key}: {value}'

# Iterate with index (enumerate)
for i, item in items.enumerate()
    log '{i}: {item}'
```

Map iteration was identified as gap G2 (now marked ✅). But tuple destructuring in for-in loops is essential for ergonomic map traversal.

---

## 11. String & Text Processing

### Survey

| Language | Features | Notable |
|----------|----------|---------|
| **Rust** | &str/String, UTF-8, char, formatting macros | `format!("{}", x)` |
| **Python** | f-strings, triple-quotes, raw strings, bytes | `f"{x:.2f}"` |
| **Kotlin** | String templates, raw strings, multiline | `"""..."""` |
| **Swift** | String interpolation, Unicode-correct, StringProtocol | `"\(x)"` |
| **Raku** | Grammars, regex as first-class, interpolation levels | `grammar JSON { ... }` |
| **Ruby** | Heredocs, regex literals, string methods | `<<~HEREDOC` |
| **Lua** | Long strings, pattern matching (not regex) | `string.match(s, "%d+")` |
| **Perl** | Regex built-in, s///, heredocs, quotation operators | `$s =~ s/foo/bar/g` |
| **Awk** | Field splitting, regex matching, printf | `split($0, a, FS)` |
| **Go** | Raw strings, `fmt.Sprintf`, bytes/runes | `` `raw string` `` |

### Jade Status

Jade has: String (heap-allocated, SSO for ≤23 bytes), single-quote strings with `{interpolation}`, double-quote raw strings, `contains`/`starts_with`/`ends_with`/`char_at`/`slice`/`length`, StringBuilder. Missing: regex, format specifiers in interpolation, split/join/trim/replace, char type, Unicode awareness.

### Recommendations for Jade

**A. Core String Methods (from Python/Rust/Kotlin) — HIGH PRIORITY**

```jade
parts is 'hello,world,jade'.split(',')         # ['hello', 'world', 'jade']
joined is parts.join(', ')                       # 'hello, world, jade'
trimmed is '  hello  '.trim()                    # 'hello'
upper is 'hello'.to_upper()                      # 'HELLO'
lower is 'HELLO'.to_lower()                      # 'hello'
replaced is 'hello world'.replace('world', 'jade')  # 'hello jade'
found is 'hello'.find('ell')                     # Some(1)
lines is text.lines()                            # split by newline
```

These are table-stakes string operations. Every language has them. They're needed for the self-hosting compiler (lexer needs `split`, `trim`, etc.).

**B. Format Specifiers in String Interpolation (from Python/Rust) — LOW PRIORITY**

```jade
pi is 3.14159
log '{pi:.2f}'          # '3.14'
log '{count:06d}'       # '000042'
log '{value:#x}'        # '0x2a'
```

Python's f-string format specifiers are the gold standard. Low priority because Jade can use the `fmt` module's explicit functions for now.

**C. Char Type (from Rust/Haskell/Swift) — MEDIUM PRIORITY**

Already identified as gap G1 (marked ✅ — char literals exist). But a full `Char` type that's Unicode-aware:

```jade
c is 'A'
if c.is_digit()
    log 'digit'
if c.is_alpha()
    log 'letter'
code is c.to_code()     # 65
```

**D. Regular Expressions (from Perl/Python/Ruby/Raku) — LOW PRIORITY (future)**

```jade
if text.matches('[0-9]+')
    log 'is numeric'

parts is text.match('(\w+)@(\w+\.com)')
log parts.group(1)
```

Regex is powerful but complex. For a systems language, it's better as a library than a language primitive. But basic pattern matching (like Lua's patterns) could be a lightweight alternative.

---

## 12. Control Flow Innovations

### Survey

| Language | Feature | Mechanism |
|----------|---------|-----------|
| **Kotlin** | Labels on loops + break/continue with labels, when-expression, scope functions (let/apply/run/also/with) | `break@outer` |
| **Rust** | Labeled loops + break-with-value, `loop` as expression | `'outer: loop { break 'outer val; }` |
| **Ruby** | Blocks, procs, lambdas, method_missing, open classes | `items.each { \|x\| puts x }` |
| **Swift** | Guard (early return), defer, labeled statements | `guard let x = opt else { return }` |
| **Go** | Defer (LIFO), multiple return values, naked return | `defer f.Close()` |
| **Zig** | Comptime branching, `errdefer`, optional chaining | `errdefer resource.deinit()` |
| **Elixir** | With-expression (chained pattern matching) | `with {:ok, x} <- f(), {:ok, y} <- g(x), do: ...` |
| **Python** | For-else, while-else, context managers (with) | `for x in xs: ... else: ...` |
| **Lua** | Coroutines as first-class | `coroutine.yield()` |

### Jade Status

Jade has: if/elif/else, while, for (range + in), loop, match, break (with value via `yield`), continue, `unless`/`until` (inverted conditionals). Missing: labeled loops, defer, guard/early-return syntax, for-else, scope functions, with-expression.

### Recommendations for Jade

**A. `defer` Statement (from Go/Swift/Zig) — HIGH PRIORITY**

```jade
*process_file path
    handle is open path
    defer close handle        # runs when function exits, regardless of how
    
    data is read handle
    # even if this fails/returns early, close(handle) still runs
    process data
```

`defer` is one of Go's best ideas. It replaces try/finally, RAII destructors, and scope guards in one clean concept. It reads perfectly: "defer the close of handle." This is especially important for FFI code where Jade interacts with C resources.

**B. Labeled Loops + Break/Continue to Label (from Rust/Kotlin/Java) — MEDIUM PRIORITY**

```jade
outer: for i from 0 to 10
    for j from 0 to 10
        if grid[i][j] equals target
            break outer
```

Without labels, breaking out of nested loops requires flag variables or return-from-function. Labels solve this cleanly.

**C. Guard Statement (from Swift) — MEDIUM PRIORITY**

```jade
*process data
    guard data.length > 0 else
        return Nothing
    guard data[0] equals MAGIC else
        ! InvalidHeader
    # happy path continues un-nested
    ...
```

Guard is inverted `if` — it checks a condition and exits early if it *fails*. The benefit: the happy path isn't nested inside conditionals. It reads as: "guard that data length is greater than 0, else return Nothing."

This is similar to `unless ... return` but more intentional and creates a binding scope for `if let`:

```jade
guard let Some(user) is find_user id else
    ! NotFound
# user is now bound in the rest of the function
```

**D. `with` Expression (from Elixir/Python) — LOW PRIORITY**

```jade
result is with
    file is try open path
    data is try read file
    parsed is try parse data
    parsed
```

A chained binding expression where each step can short-circuit. This is similar to do-notation in Haskell or Elixir's `with`. Lower priority because `try` (from §3) handles most of these cases.

**E. For-Else (from Python) — LOW PRIORITY**

```jade
for user in users
    if user.name equals target
        log 'found'
        break
else
    log 'not found'    # runs only if loop completed without break
```

Clever, but historically confusing even to Python programmers. Jade might skip this.

---

## 13. Operator Design & Extensibility

### Survey

| Language | Approach | Key Feature |
|----------|----------|-------------|
| **Haskell** | Custom operators, fixity declarations | `(+++) :: a -> a -> a; infixl 6 +++` |
| **Scala** | Methods as operators, any method can be infix | `a add b` same as `a.add(b)` |
| **Kotlin** | Operator overloading via named conventions | `operator fun plus(other: T): T` |
| **Swift** | Custom operators with precedence groups | `prefix operator √` |
| **Rust** | Trait-based overloading, no custom operators | `impl Add for MyType` |
| **Ruby** | Operator methods on objects | `def +(other)` |
| **Python** | Dunder methods | `def __add__(self, other)` |
| **C++** | Free-form operator overloading | `T operator+(const T& a, const T& b)` |
| **Raku** | Custom operators, Unicode operators, hyperoperators | `sub infix:<⊕>($a, $b) { }` |
| **APL** | Operators transform functions (higher-order) | `+/` (reduce with plus), `∘.×` (outer product) |

### Jade Status

Jade has: operator overloading via traits (Add/Sub/Mul/Div/Lt/Gt/Le/Ge/Display). Missing: custom operators, Eq/Ord/Hash derivation, comparison chain (`1 < x < 10`), in-operator, operator methods as named callable.

### Recommendations for Jade

**A. `in` Operator (from Python/Kotlin/SQL) — HIGH PRIORITY**

```jade
if x in [1, 2, 3]
    log 'found'

if name in users
    log 'exists'

if key in map
    log 'present'
```

The `in` keyword is one of the most readable membership tests in any language. It desugars to `.contains(x)`. Universal across collections.

**B. Comparison Chaining (from Python) — MEDIUM PRIORITY**

```jade
if 0 < x < 100
    log 'in range'

if a <= b <= c
    log 'ordered'
```

Python is the only major language that does this, and it's one of Python's best features. It desugars to `0 < x and x < 100` but avoids evaluating `x` twice. Perfect for range checks.

**C. Null-Coalescing / Default Operator (from C#/Kotlin/Swift/JavaScript) — MEDIUM PRIORITY**

```jade
name is user.name or else 'anonymous'
port is config.get('port') or else 8080
```

The `or else` phrasing is pure English and avoids the `??` symbol. It unwraps an Option, providing a default if None/Nothing.

**D. Range Operator for Collection Slicing (from Python/Ruby/Kotlin) — MEDIUM PRIORITY**

```jade
first_three is items from 0 to 3
last_two is items from items.length - 2 to items.length
middle is items from 2 to 5
```

Jade already uses `from ... to` in for-loops. Extending it to slicing creates a unified syntax.

---

## 14. FFI & Interop

### Survey

| Language | FFI Model | Quality |
|----------|-----------|---------|
| **Rust** | `extern "C"`, `#[repr(C)]`, `bindgen` for auto‑generation | Excellent—zero overhead |
| **Zig** | `@cImport`, seamless C header inclusion | Best-in-class—translates C headers |
| **Go** | `cgo` (slow due to goroutine stack switch) | Functional but slow |
| **Swift** | C/Obj-C bridging header, automatic | Good for Apple platform |
| **Kotlin** | JNI (Java), K/N cinterop (native) | Mixed quality |
| **Nim** | `{.importc.}` pragma, header wrapping | Good |
| **Julia** | `ccall` with type signatures | Simple and effective |
| **Lua** | LuaJIT FFI, dynamic loading | Excellent at runtime |
| **Python** | ctypes, cffi, Cython | Multiple approaches |
| **D** | `extern(C)`, can `import` C headers | Very good |

### Jade Status

Jade has: `extern` declarations for C functions, `@strict` for C-compatible struct layout, `%i8` for pointer types. Separate compilation via `--emit-obj` and `--link`. Missing: auto-generation of bindings from C headers, callback support (Jade → C → Jade), struct alignment matching, enum value specification for C compatibility.

### Recommendations for Jade

**A. C Header Import Tool (from Zig/Rust bindgen) — MEDIUM PRIORITY**

```bash
jadec --bind-c /usr/include/sqlite3.h > std/sqlite.jade
```

A tool (not necessarily in-language) that reads C headers and generates Jade `extern` declarations. This dramatically lowers the barrier to using C libraries.

**B. Callback Support — Jade Functions as C Callbacks — HIGH PRIORITY**

```jade
extern *qsort(base as %void, num as i64, size as i64, cmp as %void)

*int_compare(a as %i64, b as %i64) returns i32
    @a - @b

*main
    arr is [5, 3, 1, 4, 2]
    qsort(%arr, 5, 8, %int_compare)
```

Many C APIs take function pointers as callbacks (qsort, signal handlers, thread functions). Jade needs to be able to pass function pointers to C.

**C. Enum Discriminant Values (from Rust/C) — LOW PRIORITY**

```jade
enum Flags
    Read is 1
    Write is 2
    Execute is 4
```

For C interop, enum variants need to map to specific integer values. This also enables bitflag patterns.

---

## 15. Build, Package & Tooling

### Survey

| Language | Build System | Package Manager | Key Feature |
|----------|-------------|-----------------|-------------|
| **Rust** | Cargo | crates.io | Unified build/test/bench/doc |
| **Go** | go build/test/mod | proxy.golang.org | Minimal config, go.mod |
| **Zig** | build.zig (Zig itself) | Package manager built-in | Build system in the language |
| **Python** | setuptools/pip/poetry/pdm | PyPI | Too many options (fragmented) |
| **Node** | npm/yarn/pnpm | npmjs.com | package.json |
| **Deno** | Built-in, URL imports | deno.land/x | Zero config |
| **Swift** | Swift Package Manager | — | Package.swift |
| **Julia** | Pkg | General registry | Project.toml |
| **Elixir** | Mix | Hex.pm | mix.exs |
| **Haskell** | Cabal/Stack | Hackage | .cabal file |

### Jade Status

Jade has: `jadec` single-file compiler, `--emit-ir`, `--emit-obj`, `--link`, `--lib`, `--opt`, `--lto`, `--debug`. Missing: project config file, dependency management, `jade init`/`jade build`/`jade test`/`jade run`, workspace/multi-target support.

### Recommendations for Jade

**A. Project File (jade.toml or jade.project) — HIGH PRIORITY**

```toml
[project]
name = "my_app"
version = "0.1.0"
entry = "src/main.jade"

[dependencies]
http = "0.2.0"
json = { git = "https://github.com/jade-lang/json.git" }

[build]
opt = 3
lto = true
```

Every successful language has a project file. Cargo.toml (Rust), go.mod (Go), Package.swift, mix.exs (Elixir). This is table stakes for any language that expects more than single-file programs.

**B. CLI Subcommands — HIGH PRIORITY**

```bash
jade init my_project         # scaffold a new project
jade build                    # compile project
jade run                      # compile and execute
jade test                     # run test suite
jade bench                    # run benchmarks
jade fmt                      # format source code
jade check                    # type-check without codegen
jade doc                      # generate documentation
jade repl                     # interactive REPL
```

Go, Rust, Zig, Elixir — all have unified CLIs. A single `jade` command that does everything is essential for developer experience.

**C. Formatter (from Go's gofmt, Rust's rustfmt) — MEDIUM PRIORITY**

A canonical formatter eliminates style debates. Go proved that `gofmt` adoption approaches 100% because there's no decision to make. For indent-based Jade, the formatter would normalize indentation depth, spacing around operators, and newline conventions.

**D. REPL (from Python/Julia/Elixir) — MEDIUM PRIORITY**

```
$ jade repl
jade> fib 10
55
jade> [x * 2 for x from 1 to 5]
[2, 4, 6, 8, 10]
```

REPLs are essential for learning a language and exploratory programming. Julia's REPL is particularly good because it compiles each expression to native code before executing. Jade could do the same via LLVM JIT (ORC JIT).

**E. Language Server Protocol — MEDIUM PRIORITY (already identified)**

Jade has a `src/lsp/` directory but the audit notes "No LSP server at all." VS Code integration exists (syntax highlighting) but without go-to-definition, completions, or hover types. LSP is the difference between a hobby language and one people can actually use productively.

---

## 16. Configuration & Data Formats

### Survey

| Format/Language | Use | Strength |
|-----------------|-----|----------|
| **TOML** | Rust/Python config | Human-readable, typed values |
| **YAML** | K8s, CI/CD, Ansible | Readable but whitespace-sensitive pitfalls |
| **JSON** | Universal interchange | Ubiquitous but no comments |
| **INI** | Legacy config | Simplest possible |
| **HOCON** | JVM ecosystem | JSON superset with includes, substitutions |
| **Dhall** | Typed config | Programmable configuration |
| **Nix** | NixOS | Functional config language |
| **HCL** | Terraform | Block-structured config |
| **KDL** | Emerging | Document-oriented, clean syntax |
| **Pkl** | Apple (new) | Programmable, typed, validated |

### Recommendations for Jade

**A. Built-in JSON Support (from JavaScript/Python/Go) — MEDIUM PRIORITY**

```jade
use std.json

data is json.parse('{"name": "jade", "version": 1}')
name is data.get('name')
output is json.stringify(my_struct)
```

Given Jade's store system, JSON serialization is a natural extension. The bootstrap roadmap already mentions a working JSON parser in a test program.

**B. TOML for Project Config (see §15) — Implied by project file choice**

TOML is the best format for configuration files. It's readable, typed, and doesn't have YAML's whitespace pitfalls. Jade's project file should be TOML.

---

## 17. Testing & Verification

### Survey

| Language | Testing Built-in | Notable Feature |
|----------|-----------------|-----------------|
| **Rust** | `#[test]`, `#[cfg(test)]`, `assert_eq!` | Tests live alongside code |
| **Go** | `_test.go` files, `testing.T` | Benchmark support built-in |
| **Zig** | `test "name" { }` blocks | Comptime test evaluation |
| **Python** | `unittest`, `pytest` (third-party) | Doctest |
| **Elixir** | ExUnit, doctests | Tests in doc comments |
| **Julia** | `@test`, `@testset` | Test expressions |
| **D** | `unittest` blocks in source | Contracts (in/out conditions) |
| **Kotlin** | JUnit integration | Property-based testing libs |

### Recommendations for Jade

**A. `@test` Annotation for In-Source Tests (from Rust/Zig) — HIGH PRIORITY**

```jade
*add a, b is a + b

@test
*test_add
    assert add(1, 2) equals 3
    assert add(0, 0) equals 0
    assert add(-1, 1) equals 0
```

Tests live next to the code they test. Compiled only when running `jade test`, stripped from production builds. Rust proved this model works — close proximity between code and tests leads to better test coverage.

**B. Assert Expressions with Rich Failure Messages — MEDIUM PRIORITY**

```jade
assert x equals 42          # 'assertion failed: x equals 42 (got: 7)'
assert items.length > 0     # 'assertion failed: items.length > 0 (items.length was 0)'
assert result is_ok          # 'assertion failed: result is_ok (got: Err(NotFound))'
```

The compiler can generate descriptive failure messages by decompiling the assert expression. Zig does this; it's one of its best features.

**C. Property-Based Testing (from Haskell's QuickCheck) — LOW PRIORITY (future)**

```jade
@property
*test_sort_idempotent(items as Vec of i64)
    sorted is sort items
    assert sort(sorted) equals sorted
```

The test framework generates random inputs and checks that properties hold. This finds edge cases that unit tests miss.

---

## 18. Documentation as Code

### Survey

| Language | Doc System | Key Feature |
|----------|-----------|-------------|
| **Rust** | `///` doc comments, rustdoc | Compiles doc examples as tests |
| **Go** | Godoc, package comment conventions | Example functions run as tests |
| **Python** | Docstrings, Sphinx | Doctests (executable examples) |
| **Elixir** | `@doc`, `@moduledoc`, ExDoc | Doctests |
| **Julia** | Docstrings in Markdown | Integrated with help system |
| **Haskell** | Haddock | Type signatures as documentation |
| **Java** | Javadoc | Source-embedded HTML |
| **Swift** | Structured doc comments | Playground integration |

### Recommendations for Jade

**A. Doc Comments with `##` (from Rust/Python/Elixir) — MEDIUM PRIORITY**

```jade
## Computes the nth Fibonacci number.
## 
## Examples:
##     fib 0  # 0
##     fib 10 # 55
*fib n
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)
```

`##` for doc comments is distinct from `#` for regular comments. They're extractable by documentation tools and could become doctests.

---

## 19. DSL & Embedded Language Support

### Survey

| Language | DSL Feature | Mechanism |
|----------|-------------|-----------|
| **Kotlin** | Type-safe builders, receivers, DSL markers | `html { body { p("text") } }` |
| **Scala** | Implicits, apply/unapply, for-comprehensions | `for { x <- xs; y <- ys } yield (x, y)` |
| **Ruby** | Blocks, method_missing, open classes | `describe "foo" do it "bars" do ... end end` |
| **Groovy** | Closures, builder pattern, MOP | `json { name "jade" }` |
| **Raku** | Grammars, slangs, user-defined syntax | `grammar JSON { rule TOP { ... } }` |
| **Elixir** | Macros + keyword lists for DSL syntax | `schema "users" do field :name, :string end` |
| **Lisp** | Homoiconicity, macros — language IS the DSL | `(defroute "/" [] "hello")` |
| **Haskell** | Do-notation, monadic DSLs, phantom types | `do { x <- action; pure x }` |

### Jade Status

Jade already has DSL-like features: store definitions with query syntax, actor definitions with message handlers, `query` blocks. The question is whether to generalize this.

### Recommendations for Jade

**A. Builder Blocks (from Kotlin/Groovy) — LOW PRIORITY**

```jade
html is build HtmlDoc
    head
        title 'My Page'
    body
        p 'Hello {name}'
        ul
            for item in items
                li item
```

Jade's indentation-based syntax is natural for builders. A `build Type` expression could establish a receiver context where bare identifiers resolve to methods on the receiver.

**B. Jade's Store/Query System IS a DSL — Already Shipped**

Jade's most unique feature — persistent stores with compiled query syntax — is a better embedded DSL than most languages achieve. This should be leaned into harder:

```jade
store users
    name as String
    age as i64
    email as String

# This IS SQL-like DSL, compiled to native code
active is users where age > 18 and age < 65
sorted is query users
    where age > 21
    sort name
    limit 10
```

Rather than adding generic DSL mechanisms, Jade should invest in making its *existing* DSLs (stores, actors, queries) more powerful and more deeply integrated.

---

## 20. Syntax & Readability Philosophy

### Survey of Readability-First Languages

| Language | Readability Approach | Key Insight |
|----------|---------------------|-------------|
| **Python** | Significant whitespace, English keywords, PEP 8 | "Readability counts" |
| **Ruby** | Every method returns, blocks, English flow | "Optimized for programmer happiness" |
| **Lua** | Minimal syntax, `then`/`do`/`end` keywords | "Embedding-friendly simplicity" |
| **Swift** | Argument labels, Objective-C descendant | `move(to: point, speed: fast)` |
| **Kotlin** | Named arguments, data classes, when-expression | Concise but unambiguous |
| **Elm** | No runtime exceptions, enforced architecture | "Compiler as assistant" |
| **Go** | One way to do it, gofmt, no generics (initially) | "Less is exponentially more" |
| **COBOL** | Business English syntax | `ADD A TO B GIVING C` |
| **AppleScript** | Natural language programming | `tell application "Finder" to ...` |
| **Inform 7** | Programming in prose | `The lamp is in the living room.` |
| **SQL** | Declarative, English clauses | `SELECT name FROM users WHERE age > 21` |
| **Prolog** | Declarative logic | `parent(tom, bob).` |

### Jade's Position

Jade occupies a unique niche: **systems performance with scripting readability**. It reads like Python but compiles like C. The `is`-binding, `equals`/`and`/`or`/`not` keywords, optional parens, and indentation structure give it a "pseudocode that compiles" quality.

### Recommendations: Syntax Refinements

**A. Named Arguments at Call Sites (from Kotlin/Swift/Python) — MEDIUM PRIORITY**

```jade
*connect(host as String, port as i64, timeout as i64)
    ...

# Current: positional only
connect 'localhost', 8080, 5000

# Proposed: named args
connect host is 'localhost', port is 8080, timeout is 5000
```

Named arguments use the existing `is` keyword — it reads as an assignment at the call site. This is invaluable when a function takes multiple arguments of the same type (three `i64` parameters are indistinguishable positionally).

**B. Trailing Lambda (from Kotlin/Swift/Ruby) — MEDIUM PRIORITY**

```jade
# Current
items.each *fn(x) log x

# Proposed: trailing block
items.each
    log each
# OR
items.each x
    log x
```

Kotlin's trailing lambda syntax is one of its most ergonomic features. It enables DSL-like patterns where the last argument to a function is a block.

**C. Implicit `it` or `self` in Short Lambdas (from Kotlin) — LOW PRIORITY**

```jade
# Instead of *fn(x) x * 2
items ~ map(it * 2)
items ~ filter(it > 0)
```

Kotlin's `it` refers to the single parameter of a lambda. This is more readable than `_` (partial application) for simple transforms. But it adds implicit magic, which conflicts with Jade's explicitness.

**D. English Aliases for Common Operations — Already Part of Jade's DNA**

Jade already does this well: `equals` not `==`, `and`/`or`/`not` not `&&`/`||`/`!`, `is` not `=`, `from ... to` not `range()`. Additional candidates:

| Current | More English Alternative |
|---------|--------------------------|
| `arr[i]` | `arr at i` (already planned) |
| `x mod 2` | already done |
| `arr.length` | could also be `length of arr` |
| `true ? a ! b` | `if true then a else b` (already works as expression) |

---

## 21. AI, ML & Numerical Computing

### Survey: How Languages Handle Numerical / ML Workloads

| Language / Framework | Model | Key Features |
|---------------------|-------|--------------|
| **Python + NumPy** | Array-oriented, broadcasting, slice notation | `a @ b`, `a[0:3, :]`, shape inference, ufuncs |
| **PyTorch** | Eager-mode tensor graphs, autograd | `torch.tensor`, `.backward()`, `nn.Module`, CUDA dispatch |
| **TensorFlow** | Graph-mode, XLA compilation, tf.function | `tf.GradientTape()`, `@tf.function`, SavedModel |
| **JAX** | Functional transforms: `grad`, `jit`, `vmap`, `pmap` | `jax.grad(f)(x)`, `jax.vmap(f)(batch)`, XLA backend |
| **tinygrad** | Minimal tensor library, lazy evaluation, ~5K LOC | Shape tracker, lazy buffers, kernel fusion |
| **Mojo** | Python syntax + systems performance, MLIR backend | `fn` vs `def`, `var`/`let`, `SIMD[DType.float32, 4]`, `@parameter` |
| **Julia** | Multiple dispatch, broadcasting, differential programming | `Flux.jl`, `Zygote.jl` (source-to-source AD), `@.` broadcast |
| **MATLAB/Octave** | Matrix-first syntax, everything is a matrix | `A \ b` (linear solve), `A'` (transpose), `A .* B` (elementwise) |
| **Mathematica/Wolfram** | Symbolic computation, pattern-based rewriting | `D[f[x], x]` (symbolic derivative), `Solve`, `Integrate` |
| **R** | Statistical computing, vectorized operations, data frames | `lm()`, `glm()`, formula syntax `y ~ x1 + x2` |
| **Fortran** | Array intrinsics, `MATMUL`, `DOT_PRODUCT`, coarrays | `A = MATMUL(B, C)`, `FORALL`, array sections `A(1:N:2)` |
| **APL/J/BQN** | Array-oriented, rank polymorphism, tacit programming | `+/⍳10` (sum 1–10), `∘.×` (outer product), `⌹` (matrix inverse) |
| **Halide** | Separate algorithm from schedule for image processing | `f(x,y) = ...; f.vectorize(x,8).parallel(y)` |
| **Futhark** | Purely functional GPU language, `map`, `reduce`, `scan` | Compiles to CUDA/OpenCL, guaranteed parallel |
| **Dex** | Typed indices, effect-based AD, array-oriented functional | `for i. xs.i * xs.i` |
| **Einstein notation** | Index-based tensor contraction (NumPy, PyTorch, TF) | `np.einsum('ij,jk->ik', A, B)` = matmul |
| **ONNX** | Portable neural network interchange format | Graph-based IR, hardware-agnostic |
| **MLIR** | Multi-level IR (LLVM project), dialect-based | Linalg dialect, tensor dialect, affine loops |

### Core Concepts in ML/Numerical Computing

**Tensors and Shapes:** The fundamental abstraction. A tensor is an N-dimensional array with a shape (e.g., `[batch, channels, height, width]`). Shape checking — ensuring operand shapes are compatible — is the type system of numerical computing. PyTorch and TensorFlow do this at runtime; Julia and Futhark catch some at compile time.

**Automatic Differentiation (AD):** Not finite differences. Not symbolic derivatives. AD is a compiler technique that transforms a function into its derivative by applying the chain rule through the computation graph. Two modes:
- **Forward mode** (dual numbers): efficient when outputs >> inputs. Julia's `ForwardDiff.jl`.
- **Reverse mode** (backpropagation): efficient when inputs >> outputs. PyTorch autograd, JAX `grad`, TensorFlow `GradientTape`.

**Broadcasting:** Element-wise operations between arrays of different shapes by implicitly expanding dimensions. NumPy invented the modern rules; Julia, PyTorch, and JAX all follow them. `[1,2,3] + [[10],[20],[30]]` → 3×3 matrix.

**Vectorization / SIMD:** Operating on multiple data elements in a single instruction. Mojo's `SIMD[DType.float32, 8]` makes this explicit. LLVM auto-vectorizes, but explicit SIMD gives control.

**Kernel Fusion:** Combining multiple element-wise operations into a single pass over memory, avoiding intermediate allocations. tinygrad, XLA, and Halide all do this. Critical for GPU performance.

**Einstein Summation (einsum):** A notation for expressing tensor contractions. `einsum('ij,jk->ik', A, B)` is matmul. `einsum('ii->', A)` is trace. `einsum('ijk,ikl->ijl', A, B)` is batched matmul. Compact, general, and eliminates entire classes of loop-writing bugs.

**Computational Graphs:** The DAG of operations in a neural network. Eager mode (PyTorch) executes immediately. Graph mode (TensorFlow, XLA) builds a graph first, then optimizes and compiles it. Graph mode enables whole-program optimization but is harder to debug.

### Jade's Position

Jade is a systems language, not a numerical computing framework. But three things matter:

1. **Jade already has matrix multiplication in benchmarks** — `benchmarks/matrix_mul.jade` does dense matmul in nested loops, matching C performance.
2. **Jade compiles through LLVM** — the same backend that powers MLIR, XLA, and Mojo. LLVM's auto-vectorization and loop optimization already apply.
3. **Jade's `extern` FFI** can call BLAS, LAPACK, cuBLAS, and any C-callable numerical library.

The question isn't "should Jade be PyTorch?" — it shouldn't. The question is: what minimal language-level features would make numerical code *natural* in Jade, without bloating the core language?

### Recommendations for Jade

**A. Multi-Dimensional Array Type with Shape (from NumPy/Fortran/Julia) — MEDIUM PRIORITY**

```jade
# Fixed-shape arrays (compile-time known)
matrix as [f64; 3, 3] is [
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0]
]

# Access
val is matrix[1, 2]        # row 1, col 2
row is matrix[0, ..]       # first row (slice)
col is matrix[.., 0]       # first column (slice)
```

Jade already has fixed arrays (`[1, 2, 3]`). Extending to N-dimensional with compile-time shape checking is a natural evolution. Fortran proved 60 years ago that multi-dim arrays are essential for numerical code. The `[T; dims]` syntax mirrors Jade's existing array type but adds shape dimensions.

**B. Element-wise Operations via Broadcasting (from NumPy/Julia) — MEDIUM PRIORITY**

```jade
a is [1.0, 2.0, 3.0]
b is [4.0, 5.0, 6.0]

c is a + b              # [5.0, 7.0, 9.0] — element-wise
d is a * 2.0             # [2.0, 4.0, 6.0] — scalar broadcast
dot is a .dot b          # 32.0 — dot product
```

With operator overloading already in Jade (Add/Sub/Mul/Div traits exist), extending these to arrays is straightforward. The key insight from NumPy/Julia: when shapes are compatible, binary operators should work element-wise by default. This is implementable with the existing trait system:

```jade
impl Add for Vec of f64
    *add self, other
        # element-wise addition
```

**C. Matrix Multiply Operator or Builtin (from Python's `@`, MATLAB, Fortran) — MEDIUM PRIORITY**

```jade
# Option 1: named function
result is matmul(A, B)

# Option 2: method
result is A.matmul(B)

# Option 3: dedicated operator (like Python's @)
result is A @@ B
```

Python added `@` as a dedicated matrix multiply operator in PEP 465 because `*` was already taken (element-wise). Jade could use a named function (`matmul`) which fits the English-first ethos better than a symbol. Implemented as a builtin that calls LLVM's optimized loop or dispatches to BLAS when available.

**D. SIMD Intrinsics as Builtins (from Mojo/Zig/Rust) — MEDIUM PRIORITY**

```jade
# Explicit vector operations
a is simd_load(ptr, 4)          # load 4 floats from memory
b is simd_load(ptr2, 4)
c is simd_add(a, b)              # add 4 floats in one instruction
simd_store(out_ptr, c, 4)

# Or as a type
v is SIMD of f32, 4(1.0, 2.0, 3.0, 4.0)
w is SIMD of f32, 4(5.0, 6.0, 7.0, 8.0)
result is v + w                  # [6.0, 8.0, 10.0, 12.0]
```

Mojo makes SIMD a first-class type. Zig and Rust expose SIMD through intrinsics. Jade's LLVM backend already supports vector types — exposing them at the language level gives users explicit control over vectorization without dropping to inline assembly.

**E. Einsum-Style Contraction (from NumPy/PyTorch/TensorFlow) — LOW PRIORITY (future)**

```jade
# Matmul via einsum
C is einsum 'ij,jk->ik', A, B

# Trace
trace is einsum 'ii->', M

# Batched matmul
result is einsum 'bij,bjk->bik', batch_A, batch_B

# Outer product
outer is einsum 'i,j->ij', u, v
```

Einsum is the most powerful tensor operation notation ever invented. A single function replaces matmul, outer product, trace, transpose, contraction, and batched variants. The string-based notation is actually a tiny DSL that the compiler can parse and optimize. Low priority because it requires multi-dim arrays first, but the payoff is enormous for anyone doing linear algebra.

**F. Automatic Differentiation via Comptime Transform (from JAX/Zygote) — LOW PRIORITY (future exploration)**

```jade
*loss(weights, x, y)
    pred is weights.dot(x)
    (pred - y) ** 2

# grad transforms loss into its gradient function
grad_loss is grad(loss)
gradients is grad_loss(weights, x, y)

# Or as a comptime transform
comptime grad_fn is differentiate(loss, with_respect_to is 0)
```

This is the holy grail of ML language support. JAX's `grad` and Julia's `Zygote.jl` implement source-to-source automatic differentiation — the compiler transforms the forward function into its backward (gradient) function. This is fundamentally different from PyTorch's runtime tape-based AD: it produces zero-overhead gradient code that can be further optimized by LLVM.

A systems language with native AD would be unprecedented. Mojo is pursuing this via MLIR. Jade could pursue it via LLVM intrinsics and comptime function transformation. Extremely ambitious but transformative.

**G. Neural Network Patterns — Composition over Framework (from tinygrad's Minimalism) — PHILOSOPHY**

Jade should NOT try to be PyTorch. Instead, the philosophy should be:

1. **Primitives:** Multi-dim arrays, matmul, element-wise ops, broadcasting, SIMD.
2. **Composition:** Neural network layers are just functions that take weights and inputs. No class hierarchy, no Module base class. Just functions and structs.
3. **Extern for acceleration:** `extern` FFI to cuBLAS, MKL, Accelerate, or custom CUDA kernels.

```jade
# A neural network layer is just a struct + function
type Linear
    weights as [f64; 784, 128]
    bias as [f64; 128]

*forward(layer as Linear, input as [f64; 784]) returns [f64; 128]
    result is matmul(layer.weights, input)
    for i from 0 to 128
        result[i] is result[i] + layer.bias[i]
        result[i] is result[i] > 0.0 ? result[i] ! 0.0   # ReLU
    result

# A network is just composition
type MLP
    layer1 as Linear
    layer2 as Linear

*predict(net as MLP, input as [f64; 784]) returns [f64; 10]
    hidden is forward(net.layer1, input)
    forward(net.layer2, hidden)
```

tinygrad proved that a tensor library can be ~5K LOC. The insight: you don't need a framework. You need good primitives and the ability to compose them. This aligns perfectly with Jade's minimalist ethos.

### What Jade Should NOT Do for ML

| Anti-Pattern | Why Not |
|-------------|--------|
| **Build a tensor runtime** | Jade is a compiled language. No interpreter, no eager mode graph. |
| **Add Python-style dynamic dispatch for operators** | Breaks zero-cost abstraction principle. |
| **Create an `nn.Module` class hierarchy** | No class inheritance. Structs + functions. |
| **Implement a GPU runtime** | Use CUDA/Vulkan/Metal via extern FFI. |
| **Add a JIT** | Jade is AOT-compiled. Comptime handles what JIT would. |

---

## 22. Odds & Ends — Algebra, Encodings, Grammars & Everything Else

Concepts that don't fit neatly into one section but represent important ideas from across computing and formal systems.

### 22.1 Boolean & Abstract Algebra

**Boolean algebra** (AND, OR, NOT, XOR, NAND, NOR, implication) underpins hardware design, SAT solvers, and predicate logic. Jade already has `and`, `or`, `not` as keywords and `&`, `|`, `^` as bitwise operators. The gap:

| Concept | Status in Jade | Recommendation |
|---------|---------------|----------------|
| Boolean operators | ✅ `and`, `or`, `not` | — |
| Bitwise operators | ✅ `&`, `\|`, `^`, `<<`, `>>`, `~` | — |
| **Implication** (`→`, `implies`) | ❌ Missing | Add `implies` keyword: `a implies b` ≡ `not a or b`. Useful for assertions, contracts, and formal specifications. LOW PRIORITY. |
| **XOR as word** | ❌ Only `^` | Add `xor` as alias for `^`. Fits English-keyword ethos. LOW PRIORITY. |
| **Bit fields / flags** | ❌ No bitflag enum support | See §14.C (enum discriminant values). MEDIUM PRIORITY. |

**Abstract algebra concepts** — monoids, semigroups, groups, rings, fields, lattices — appear in Haskell's typeclass hierarchy, Scala's Cats/Scalaz, and Rust's `num` crate. Jade shouldn't formalize algebraic structures as typeclasses (too mathematical for the ethos), but recognizing operator laws enables optimizations:

- **Associativity** of `+` and `*` → loop reordering
- **Commutativity** of `+` and `*` → operand reordering for constant folding
- **Identity elements** (0 for `+`, 1 for `*`) → dead code elimination
- **Distributivity** → strength reduction

These are LLVM-level optimizations Jade already benefits from. No language changes needed.

### 22.2 Mathematical Notation & Numeric Computing

| Concept | Language Implementations | Jade Fit |
|---------|------------------------|---------|
| **Complex numbers** | Fortran (`COMPLEX`), Python (`complex`), Julia (`im`), C99 (`_Complex`) | LOW — add as a library type: `type Complex { re as f64, im as f64 }` with operator overloading |
| **Rational numbers** | Haskell (`Rational`), Python (`fractions`), Scheme | LOW — library type |
| **Arbitrary precision** | Python (`int`), Haskell (`Integer`), GMP | LOW — wrap GMP via extern |
| **Interval arithmetic** | Julia (`IntervalArithmetic.jl`), Ada (fixed-point) | LOW — library type |
| **Quaternions** | Mathematica, Julia, C++ (`boost::math`) | LOW — library type, useful for 3D/game dev |
| **Units of measure** | F# (`[<Measure>]`), Ada (dimension types), Fortress | MEDIUM — compile-time dimensional analysis eliminates Mars Climate Orbiter bugs. Could use phantom types or generic wrapper: `type Meters is f64` |
| **Fixed-point arithmetic** | Ada, Solidity, embedded C | MEDIUM — useful for financial/embedded: `type Fixed { value as i64, scale as i64 }` |
| **Numeric literals with underscores** | Rust (`1_000_000`), Python, Java, Kotlin, Swift | HIGH ✓ — readability for large numbers. `count is 1_000_000`. Lexer change only. |
| **Hex/octal/binary literals** | Every systems language | Should already exist; verify in lexer. `0xFF`, `0o77`, `0b1010` |
| **Scientific notation** | All — `1.5e10`, `3.14e-2` | Should exist; verify. |
| **Infinity / NaN literals** | Rust (`f64::INFINITY`), Python (`float('inf')`) | Add as builtins: `infinity`, `nan`. |

### 22.3 Human Language, Grammar & Linguistics

Jade's syntax already draws from natural language: `is` for binding, `equals` for comparison, `from ... to` for ranges, `and`/`or`/`not` for logic. This is not accidental — it's the language's core identity.

Here are linguistic concepts that could further inform Jade's design:

**A. Subject-Verb-Object (SVO) Order**

English is SVO: "The cat (S) eats (V) the fish (O)." Jade's syntax mostly follows SVO:
- `x (S) is (V) 42 (O)` ✓
- `log (V) x (O)` — VO (imperative mood, subject dropped) ✓
- `send (V) actor (O₁), message (O₂)` — ditransitive verb ✓
- `for (prep) x (S) in (prep) items (O)` — prepositional phrase ✓

But some constructs break SVO:
- `insert users 'Alice', 30` — VOS order (verb first, then store, then values)
- `delete users where age > 28` — VOS with relative clause

This is fine. Imperative/command sentences in English naturally start with the verb: "Open the door." "Delete the user." Store operations feel like commands, so VSO is natural.

**B. Noun Phrases as Types, Verb Phrases as Functions**

The naming convention of `type Point` (noun for data) and `*calculate_distance` (verb for action) mirrors natural language's noun/verb distinction. This could be encouraged by convention documentation:
- Types → nouns: `Point`, `Connection`, `User`, `Token`
- Functions → verbs: `connect`, `parse`, `sort`, `validate`
- Traits → adjectives/capabilities: `Printable`, `Sortable`, `Iterable`, `Hashable`
- Constants → adjectives/descriptors: `max_size`, `default_port`

**C. Articles & Determiners (Inspired by Inform 7)**

Inform 7 goes furthest: "The player is in the kitchen." Jade shouldn't go that far, but the principle — code should read like prose — is Jade's north star.

**D. Singular/Plural Convention**

```jade
# Singular for one, plural for many — natural English
user is find_user(id)
users is all users
for user in users
    log user.name
```

This is a convention, not a language feature, but it contributes to readability. Documentation should encourage it.

### 22.4 Character Encodings & Binary Data

| Topic | Current Status | Recommendation |
|-------|---------------|----------------|
| **UTF-8 strings** | ✅ Jade strings are UTF-8 | — |
| **Byte literals** | ❌ No `b'...'` syntax | Add `b'hello'` for `[u8]` byte arrays. Useful for network protocols, file formats. MEDIUM PRIORITY. |
| **Hex string literals** | ❌ | Add `x'DEADBEEF'` for hex-encoded byte arrays. Useful for crypto, hashing. LOW PRIORITY. |
| **Character escapes** | Partial — `\n`, `\t` exist | Verify `\0`, `\r`, `\\`, `\'`, `\x41`, `\u{1F600}` all work. |
| **Endianness** | `bswap` builtin exists | ✅ Good. Consider `to_be`/`to_le` method aliases. |
| **Base64 encoding** | ❌ | Library function: `encode_base64`, `decode_base64`. LOW PRIORITY. |
| **Bitwise struct packing** | `@packed` exists | ✅ Good. Consider bit-field support for protocol headers. |

### 22.5 Serialization & Marshalling

| Approach | Languages | Jade Fit |
|----------|----------|----------|
| **JSON** | JavaScript (native), Python (`json`), Go (`encoding/json`), Rust (`serde_json`) | HIGH — essential for web/API interop. `@derive Serialize` generates to/from JSON. |
| **Binary serialization** | Protocol Buffers (Google), FlatBuffers, MessagePack, CBOR | MEDIUM — Jade's `@packed @strict` structs are already binary-serializable. Add `to_bytes`/`from_bytes` methods. |
| **CSV/TSV** | Every data language | LOW — library |
| **TOML/YAML** | Config languages | MEDIUM — needed for project files (§15) |
| **Custom (de)serialization via traits** | Rust's `serde` (Serialize/Deserialize), Go's Marshal/Unmarshal | HIGH — a `Serialize`/`Deserialize` trait with `@derive` is the composable approach |

```jade
@derive Serialize, Deserialize
type Config
    host as String
    port as i64
    debug as bool

# Auto-generated by @derive
config is Config(host is 'localhost', port is 8080, debug is false)
json_str is serialize config           # '{"host":"localhost","port":8080,"debug":false}'
parsed is deserialize of Config(json_str)  # Config struct
```

### 22.6 Formal Grammars & Parsing

Jade's compiler IS a parser — recursive descent with indentation-based lexing. Relevant concepts:

| Concept | Relevance to Jade |
|---------|-------------------|
| **PEG (Parsing Expression Grammars)** | Raku's grammars, Rust's `pest`. Jade could expose a grammar DSL for user-defined parsers. LOW PRIORITY. |
| **LL(k) / LR(k)** | Jade's parser is LL(1) recursive descent with occasional LL(2) lookahead. Well-suited to the language. |
| **EBNF** | Jade already has [syntax-ebnf.md](syntax-ebnf.md) defining its grammar in EBNF. |
| **Tree-sitter** | Jade already has `tree-sitter-jade/` for editor integration. ✅ |
| **Bison/Yacc** | Table-driven parsers. Not applicable — Jade's hand-written parser is faster and generates better errors. |
| **Pratt parsing** | Operator-precedence parsing for expressions. Jade already uses this (precedence climbing in `parse_expr`). |

**Potential feature: Grammar DSL (from Raku) — VERY LOW PRIORITY**

```jade
grammar JSON
    rule value is string or number or object or array or 'true' or 'false' or 'null'
    rule string is '"' chars '"'
    rule number is digits ['.' digits]
    rule object is '{' [pair (',' pair)*] '}'
    rule pair is string ':' value
    rule array is '[' [value (',' value)*] ']'

*main
    result is JSON.parse(input)
```

Raku's grammars are the best declarative parsing system in any general-purpose language. This is aspirational for Jade — it would make Jade exceptional for building parsers, config readers, and protocol decoders. But it's a massive feature.

### 22.7 Graph Theory & Data Structures

Concepts not in §10 (Collections) that matter for systems programming:

| Structure | Use Case | Jade Status |
|-----------|----------|------------|
| **Graph (adjacency list/matrix)** | Dependency resolution, networking, pathfinding | Build from Vec + Map |
| **Priority Queue / Heap** | Scheduling, Dijkstra, event processing | Missing — should be in stdlib. MEDIUM PRIORITY. |
| **Trie** | String prefix lookup, autocomplete, routing | Library |
| **B-Tree** | Databases, file systems | Library (Jade's stores use flat files) |
| **Bloom filter** | Probabilistic membership, caching | Library |
| **Union-Find / Disjoint Set** | Jade's own type inference uses this internally (`InferCtx`) | Library |
| **Persistent / immutable data structures** | Functional programming, undo/redo | Not needed (Jade is imperative-first) |

A **priority queue** is the most impactful missing data structure for systems programming:

```jade
pq is priority_queue()
pq.push 'urgent', priority is 1
pq.push 'normal', priority is 5
next is pq.pop()    # 'urgent' (lowest number = highest priority)
```

### 22.8 Scheduling, Job Systems & Work Stealing

| Concept | Implementations | Jade Fit |
|---------|----------------|----------|
| **Work-stealing deques** | Go's scheduler, Tokio, Rayon, Intel TBB, Jade's `runtime/deque.c` | Already implemented in runtime ✅ |
| **Green threads / M:N scheduling** | Go (goroutines), Erlang (processes), Jade (coroutines) | In progress (actors still on pthreads) |
| **Event loops** | Node.js (libuv), Python (asyncio), Rust (tokio) | Not targeted — Jade uses coroutines instead |
| **Fiber / continuation-based** | Ruby (Fiber), Lua (coroutines), Zig (async frames) | Jade's `dispatch`/`yield` is this model |
| **CSP** | Go (channels), Clojure (core.async) | ✅ Jade has channels + select |
| **Actor mailboxes** | Erlang, Akka, Jade | ✅ Jade has actors with ring-buffer mailboxes |

Jade's runtime already has the infrastructure (`runtime/sched.c`, `runtime/deque.c`, `runtime/coro.c`). The gap is wiring actors to the cooperative scheduler (identified in the 0.4.0 audit).

### 22.9 Type Theory & Category Theory (What's Useful, What's Not)

Haskell, Scala, and PureScript draw heavily from category theory. What's actually useful vs. what's academic?

| Concept | Academic | Practical in Jade? |
|---------|----------|--------------------|
| **Functors** (things you can `map` over) | `fmap :: (a -> b) -> f a -> f b` | YES — this is just `.map()`. Jade's iterator combinators (§10.A) are functors in disguise. |
| **Monads** (things you can `flatMap` / chain) | `>>= :: m a -> (a -> m b) -> m b` | YES — this is `.flat_map()` and error propagation (`try`). Don't call them monads. |
| **Applicatives** | `<*> :: f (a -> b) -> f a -> f b` | MARGINAL — useful for parallel validation. Not worth language support. |
| **Monoids** | `mempty`, `mappend` | YES — `fold` with an initial value is a monoid. Already in §4. |
| **Higher-kinded types** | `f :: * -> *` | NO — requires complex type system. Monomorphization doesn't support this. |
| **GADTs** | `data Expr a where ...` | NO — massive complexity. Not needed for systems programming. |
| **Phantom types** | `newtype Tagged tag a = Tagged a` | MAYBE — useful for type-safe IDs. Low effort if alias types exist. |
| **Dependent types** | `Vec (n : Nat) a` | NO — research language territory. |

Principle: **Use the ideas, not the vocabulary.** Jade's users should write `.map()`, `.filter()`, `.fold()` and `try` — they shouldn't need to know what a functor or monad is. The abstractions serve them silently.

### 22.10 Cryptography & Hashing

```jade
# Hash builtins or stdlib
hash is hash_bytes(data)              # general-purpose hash
sha is sha256(message)                # SHA-256
hmac is hmac_sha256(key, message)     # HMAC

# Constant-time comparison (prevents timing attacks)
if constant_time_eq(a, b)
    log 'match'
```

Jade needs: a `hash()` builtin for HashMap keys (already needed for Set support), and eventually a crypto stdlib wrapping OpenSSL or libsodium via extern. The `constant_time_eq` function is a security-critical primitive.

**Hash trait for user types:**

```jade
@derive Hash
type Point
    x as i64
    y as i64

# Can now be used as Map key or Set element
points is set()
points.add Point(x is 1, y is 2)
```

### 22.11 Date, Time & Duration

| Approach | Language | Notes |
|----------|----------|-------|
| **Unix timestamp** | C (`time_t`), Go | Simple but error-prone |
| **Structured datetime** | Python (`datetime`), Java (`java.time`), Rust (`chrono`) | Correct but heavyweight |
| **Duration types** | Rust (`Duration`), Go (`time.Duration`), Kotlin | `5.seconds`, `100.milliseconds` |
| **Monotonic clock** | Rust (`Instant`), Go (`time.Now()`) | For measuring elapsed time |

Jade has `time_now()` returning nanosecond timestamp. For a systems language, this is sufficient as a primitive. A `std/time.jade` module with duration helpers and formatting would round this out:

```jade
use time

start is time.now()
# ... work ...
elapsed is time.since(start)
log 'took {time.format_ms elapsed}ms'
```

### 22.12 State Machines & Protocols

| Language | Feature | Mechanism |
|---------|---------|----------|
| **Rust** | Typestate pattern | Different types for different states: `TcpStream<Connecting>`, `TcpStream<Connected>` |
| **TLA+** | Formal specification | Model checking state transitions |
| **Erlang** | `gen_statem` | OTP state machine behavior |
| **UML** | State diagrams | Visual specification |
| **XState** (JavaScript) | Statecharts | Declarative state machines |

Jade's enum + match pattern naturally encodes state machines:

```jade
enum ConnState
    Disconnected
    Connecting(String)
    Connected(Socket)
    Error(String)

*handle_event(state as ConnState, event as Event) returns ConnState
    match state
        Disconnected ? match event
            Connect(host) ? Connecting(host)
            _ ? state
        Connecting(host) ? match event
            Success(sock) ? Connected(sock)
            Failure(msg) ? Error(msg)
            _ ? state
        Connected(sock) ? match event
            Disconnect ? Disconnected
            Data(bytes) ? process(sock, bytes); state
            _ ? state
        Error(msg) ? match event
            Retry ? Disconnected
            _ ? state
```

This is already idiomatic. No language changes needed — but `@derive Display` (§6.D) would make debugging state machines much easier.

### 22.13 Cellular Automata, Simulation & Game-of-Life Patterns

Jade's test suite includes `game_of_life.jade`. CA and simulation patterns care about:

- **2D grid access with wrapping** — modular arithmetic on indices: `grid[(x + dx) mod width]`
- **Neighbor access patterns** — stencil operations over grids
- **Double-buffering** — read from one grid, write to another, swap
- **Step functions** — pure `state → state` transforms

Jade handles all of these with existing features. Multi-dimensional arrays (§21.A) would make grid access cleaner. Parallel iterators (§5.D) would make step functions fast on multi-core.

### 22.14 Regular Expressions & Pattern Languages

| Language | Regex Integration | Level |
|---------|-------------------|-------|
| **Perl/Raku** | First-class, built into syntax | Deep |
| **Ruby** | Literals (`/pattern/`), `=~` operator | Deep |
| **Python** | `re` module, compiled patterns | Library |
| **Rust** | `regex` crate, no built-in | Library |
| **Go** | `regexp` package | Library |
| **Lua** | Pattern matching (subset of regex) | Lightweight |
| **AWK** | Built-in, `~` operator | Deep |

For Jade, regex should be a **library**, not a language feature. Reasons:
- Regex is a mini-language (its own parser, compiler, VM). Building it into the lexer adds complexity.
- Perl and Ruby show that deep regex integration leads to write-only code — against Jade's readability ethos.
- A `std/regex.jade` wrapping PCRE2 via extern gives full power without language bloat.

```jade
use regex

pattern is regex.compile('[0-9]+')
if regex.matches(text, pattern)
    log 'found number'

results is regex.find_all(text, pattern)
```

### 22.15 Miscellaneous Concepts & Language Features

**A. Assertions & Contracts (from Eiffel/D/Ada)**

```jade
*binary_search(arr, target)
    assert arr.length > 0    # precondition
    # ...
```

Jade has `assert`. Full design-by-contract (preconditions, postconditions, invariants) is overkill, but assert is sufficient.

**B. Compile-Time Reflection / Type Introspection (from Zig/D)**

```jade
comptime fields is fields_of(MyStruct)   # list of field names and types
comptime size is size_of of MyStruct()   # already exists
```

Limited comptime reflection (field names, field count, field types) enables `@derive` implementations. MEDIUM PRIORITY — needed if `@derive` is built.

**C. Numeric Literal Separators (from Rust/Python/Java/Kotlin/Swift)**

```jade
population is 7_900_000_000
hex_mask is 0xFF_00_FF_00
binary is 0b1010_0101_1100_0011
```

Pure readability improvement. Single lexer change — skip `_` inside numeric literals. HIGH PRIORITY.

**D. Tuple Structs / Newtype Pattern (from Rust/Haskell)**

```jade
type UserId is i64       # newtype: distinct from raw i64
type Meters is f64
type Seconds is f64

*velocity(distance as Meters, time as Seconds) returns f64
    distance.value / time.value
```

Newtypes prevent mixing up semantically different values of the same underlying type. `UserId` and `i64` are the same at runtime but different at compile time. This is the simplest form of Ada's strong typing and F#'s units of measure. MEDIUM PRIORITY — pairs with type aliases.

**E. Scope-Limited Imports (from Rust/Go)**

```jade
*process
    use math.{sin, cos}   # only visible inside this function
    result is sin(angle) + cos(angle)
```

Imports scoped to a block rather than the whole file. Reduces namespace pollution. LOW PRIORITY.

**F. Unreachable / Absurd (from Rust/Zig/Haskell)**

```jade
match direction
    North ? go_north()
    South ? go_south()
    East ? go_east()
    West ? go_west()
    _ ? unreachable        # compiler optimization hint + safety check
```

Tells the compiler (and reader) that a branch should never execute. If it does, it's a bug — trap or UB depending on build mode. Useful after exhaustive matching where the wildcard is a safety net. LOW PRIORITY.

**G. Compile-Time Assertions / Static Assert (from C++/Rust/Zig)**

```jade
comptime assert size_of of Header() equals 16
comptime assert align_of of CacheAligned() equals 64
```

Verify invariants at compile time. Prevents ABI mismatches in FFI code. MEDIUM PRIORITY.

**H. Integer Conversion Safety**

Jade already coerces integer widths. But explicit narrowing should be visible:

```jade
big is 100000 as i64
small is big as i16          # compile warning: possible truncation
small is narrow(big, i16)    # explicit: panics if value doesn't fit
small is truncate(big, i16)  # explicit: silently truncates
```

Rust requires explicit `as` for narrowing. Jade should at minimum warn. MEDIUM PRIORITY.

---

## 23. Summary: Priority-Ranked Recommendations

### Tier 1 — High Priority (Foundational, High Impact)

These are features that nearly every modern language has, that Jade's users will expect, and that align perfectly with Jade's ethos:

| # | Feature | Source Inspiration | Impact | Effort |
|---|---------|-------------------|--------|--------|
| 1 | **`if let` / `while let`** | Rust, Swift | Eliminates verbose match blocks for optional unwrap | S |
| 2 | **`try` error propagation** | Rust (?), Zig (try) | Concise error chaining; essential for real-world code | M |
| 3 | **Iterator combinators** (map/filter/fold/zip/take/skip/enumerate) | Rust, Kotlin, Scala | Foundation of modern collection processing | M |
| 4 | **`defer` statement** | Go, Swift, Zig | Resource cleanup without RAII complexity | S |
| 5 | **`@derive` for traits** | Rust, Haskell | Eliminates most trait implementation boilerplate | M |
| 6 | **`@test` in-source testing** | Rust, Zig | Tests next to code, compiled conditionally | S |
| 7 | **Selective imports** (`use mod.{a, b}`) | Rust, Python | Namespace control, clearer dependencies | S |
| 8 | **`in` operator** | Python, Kotlin, SQL | Universal membership test, reads as English | S |
| 9 | **Default methods in traits** | Rust, Java, Kotlin | Reduces trait implementation burden | S |
| 10 | **Decorators / annotations** (`@inline`, `@deprecated`, etc.) | Python, Rust, Java | Lightweight metaprogramming, compiler hints | M |
| 11 | **Core string methods** (split/join/trim/replace/find) | Python, Rust | Table-stakes string operations | M |
| 12 | **Project file + CLI subcommands** | Cargo, Go, Mix | Professional project management | M–L |

### Tier 2 — Medium Priority (Valuable, Good Fit)

Features that would differentiate Jade or solve common pain points:

| # | Feature | Source | Impact |
|---|---------|--------|--------|
| 13 | **Guard clauses on match arms** (`when`) | Haskell, Rust | More expressive pattern matching |
| 14 | **Named arguments at call sites** | Kotlin, Swift, Python | Clarity for multi-param functions |
| 15 | **Where-clauses on generics** | Rust, Haskell | Catch type errors at definition site |
| 16 | **Type aliases** | Rust, Haskell | Readable complex types |
| 17 | **Extension methods** (`extend Type`) | Kotlin, Swift, C# | Add methods to existing types |
| 18 | **Set collection** | Every language | Missing fundamental data structure |
| 19 | **Labeled loops** | Rust, Kotlin, Java | Clean nested loop breaking |
| 20 | **Comparison chaining** (`0 < x < 100`) | Python | Readable range checks |
| 21 | **Null-coalescing** (`or else`) | C#, Kotlin, Swift | Concise optional defaulting |
| 22 | **Lazy sequences** | Haskell, Kotlin, Clojure | Zero-allocation iterator chains |
| 23 | **Guard statement** | Swift | Flat happy-path code |
| 24 | **Trait inheritance** (`extends`) | Rust, Haskell | Trait hierarchies |
| 25 | **Error context** (`try ... with 'context'`) | Rust anyhow, Go | Debuggable error chains |
| 26 | **Structured concurrency** | Kotlin, Swift, Java | No leaked tasks/threads |
| 27 | **Formatter** (`jade fmt`) | Go, Rust | Eliminates style debates |
| 28 | **Doc comments** (`##`) | Rust, Elixir | Extractable documentation |
| 29 | **Spread/splat** (`...args`) | JS, Python, Ruby | Variadic functions |
| 30 | **Arena allocator** | Zig, C++ | Bulk deallocation for compilers/parsers |
| 31 | **Parallel iterators** | Rust Rayon, Java | Data parallelism |
| 32 | **Atomic operations** | Every systems language | Lock-free concurrency |
| 33 | **REPL** | Python, Julia, Elixir | Interactive exploration |
| 34 | **LSP** | All modern IDEs | IDE integration |

### Tier 3 — Low Priority / Future (Aspirational, High Effort)

Features worth exploring but not urgent:

| # | Feature | Source | Notes |
|---|---------|--------|-------|
| 35 | Union types | TypeScript, Scala 3 | Lightweight ad-hoc polymorphism |
| 36 | Constrained numeric types | Ada | Compile-time range checking |
| 37 | Regex support | Perl, Python, Ruby | Better as library |
| 38 | Partial application | Haskell, F# | Explicit with `_` placeholder |
| 39 | Property-based testing | Haskell QuickCheck | Advanced testing |
| 40 | `errdefer` | Zig | Error-path cleanup |
| 41 | Builder blocks | Kotlin, Groovy | DSL authoring |
| 42 | Delegation | Kotlin | Wrapper type boilerplate |
| 43 | Placeholder lambdas (`_`) | Scala, Kotlin | Terser anonymous functions |
| 44 | C header auto-binding | Zig, Rust bindgen | Tooling, not language feature |
| 45 | Trailing lambda syntax | Kotlin, Swift | DSL-friendly blocks |
| 46 | Copy-on-write | Swift | Optimization for shared collections |
| 47 | For-else | Python | Confusing even in Python |
| 48 | Custom operators | Haskell, Scala | Against Jade's readable ethos |
| 49 | Literal types | TypeScript | Enums already serve this role |
| 50 | Format specifiers in interpolation | Python f-strings | `fmt` module suffices |
| 51 | Numeric literal separators (`1_000_000`) | Rust, Python, Kotlin | Lexer-only change, pure readability | S |
| 52 | Multi-dim arrays with shape | NumPy, Fortran, Julia | Foundation for numerical code | M |
| 53 | Matmul builtin/method | MATLAB, NumPy, Fortran | Essential linear algebra primitive | M |
| 54 | SIMD intrinsics | Mojo, Zig, Rust | Explicit vectorization control | M |
| 55 | `@derive Serialize` / JSON | Rust serde, Go encoding/json | Interop with web/APIs | M |
| 56 | Newtype / tuple structs | Rust, Haskell, F# | Type-safe wrappers, units of measure | M |
| 57 | Byte literals (`b'...'`) | Rust, Python | Network/binary protocol support | M |
| 58 | Priority queue | Every stdlib | Scheduling, graph algorithms | M |
| 59 | Comptime struct reflection | Zig, D | Enables @derive implementations | M |
| 60 | Einsum notation | NumPy, PyTorch, JAX | General tensor contraction (future) | L |
| 61 | Automatic differentiation | JAX, Julia Zygote | Source-to-source AD (future exploration) | L |
| 62 | Grammar DSL | Raku | Declarative parsing (aspirational) | L |

### Anti-Recommendations: What Jade Should NOT Add

Features that would harm Jade's identity:

| Feature | Why Not |
|---------|---------|
| **Exceptions (try/catch/throw)** | Jade uses error-as-values. Exceptions break local reasoning, infect call stacks, and require runtime support. |
| **Implicit conversions** | Type coercion surprises. Jade's explicit `as` casting is better. |
| **Null** | Jade uses Option(Some/Nothing). Null is the billion-dollar mistake. |
| **Inheritance (class-based OOP)** | Composition via traits is strictly superior. No vtable hierarchies, no diamond problem, no fragile base class. |
| **Async/await coloring** | Go proved you don't need it. Jade's coroutine model should make all code "synchronous" to the programmer. |
| **Custom operators** (arbitrary symbols) | Directly contradicts "clear English over opaque symbols." |
| **Garbage collection** | Jade's ownership + Perceus RC achieves memory safety without GC overhead. |
| **Global mutable state** | Against ownership principles. Actors/channels for shared state. |
| **Preprocessor macros** | C's preprocessor is a source of bugs. Jade should use comptime and decorators instead. |
| **Method overloading by arity** | Ambiguous dispatch. Use named arguments or different function names. |
| **Multiple inheritance** | Diamond problem, complexity explosion. Traits + composition. |
| **Reflection / runtime type queries** | Zero-cost abstractions means no runtime type metadata. Use generics/traits. |
| **Tensor runtime / eager graph execution** | Jade is AOT-compiled. No interpreter loop for tensor ops. Use extern FFI to BLAS/cuBLAS. |
| **nn.Module class hierarchy** | No class inheritance. Neural networks are structs + functions. |
| **Built-in regex syntax** | Write-only code. Use `std/regex` library wrapping PCRE2 via extern. |
| **Higher-kinded types / GADTs / dependent types** | Research-language territory. Monomorphization is simpler and faster. |

---

## Appendix A: Paradigm Alignment Matrix

How each major paradigm maps to Jade:

| Paradigm | Jade's Position | Implementation |
|----------|----------------|----------------|
| **Imperative** | Core — statements, mutation, loops | `is` binding, `while`/`for`/`loop` |
| **Functional** | Strong — first-class fns, pipes, closures | `~` pipeline, `*fn` lambdas, HOF |
| **Object-Oriented** | Selective — structs + methods, no classes | `type` with `*method self`, traits |
| **Actor-Based** | Native — Erlang-style actors | `actor`, `spawn`, `send`, `receive` |
| **CSP (Channels)** | Native — Go-style channels | `channel of T`, `send`, `receive`, `select` |
| **Declarative** | Emerging — store queries, pattern matching | `store`, `query`, `match`, `where` |
| **Systems** | Core — raw pointers, FFI, inline asm | `%ptr`, `extern`, `asm`, `volatile_*` |
| **Generic** | Core — monomorphized generics | `of T`, `impl Trait of T for Type` |
| **Aspect-Oriented** | Via decorators (proposed) | `@inline`, `@test`, `@derive` |
| **Data-Oriented** | Via stores and struct layout control | `@packed`, `@strict`, `@align(N)` |
| **Logic** | Not targeted | — |
| **Array-Oriented (APL)** | Emerging — via multi-dim arrays, broadcasting | `[f64; 3, 3]`, element-wise ops |
| **Numerical** | Via primitives + extern FFI | matmul, SIMD, extern BLAS/LAPACK |
| **Concatenative (Forth)** | Not targeted | — |

## Appendix B: Keyword Budget

Jade's current keyword count: ~42. Proposed additions from this analysis:

| Keyword | Purpose | Precedent |
|---------|---------|-----------|
| `try` | Error propagation | Zig, Swift |
| `defer` | Cleanup on scope exit | Go, Swift, Zig |
| `in` | Membership test | Python, Kotlin, SQL |
| `guard` | Early return on failure | Swift |
| `extend` | Extension methods | Kotlin, Swift |
| `alias` | Type alias | Rust (type), Haskell |
| `parallel` | Parallel execution | Julia |
| `atomic` | Atomic operations | — |

That's +8 keywords, bringing the total to ~50. Additional candidates from §21–22:

| Keyword | Purpose | Precedent |
|---------|---------|-----------||
| `implies` | Boolean implication | Logic languages |
| `xor` | Bitwise XOR alias | English keyword ethos |
| `unreachable` | Dead-branch marker | Rust, Zig |

That would be ~53 total. Go has 25, Rust has 51, Python has 35, Kotlin has 79, Swift has 89. Jade at ~53 is in the moderate range — enough power without keyword bloat.

## Appendix C: Cross-Reference to Existing Roadmap

| Roadmap Item | This Analysis Reference |
|-------------|------------------------|
| G4: if let / while let | §1.A — HIGH PRIORITY ✓ |
| G7: Set collection | §10.B — MEDIUM PRIORITY ✓ |
| G8: Enum tag access | §14.C — LOW PRIORITY ✓ |
| G9: Closure by reference | §4 — Not recommended (by-value is safer) |
| Bootstrap: self-hosting | §6 (comptime), §8 (modules), §11 (strings), §15 (tooling) |
| 0.5.0: LSP | §15.E — MEDIUM PRIORITY ✓ |
| 0.5.0: CLI subcommands | §15.B — HIGH PRIORITY ✓ |
| 0.4.0: Actor scheduler migration | §5 — Structured concurrency ✓ |
| DDR: demand-driven resolution | §2.B — Where-clauses strengthen this ✓ |
| Numerical computing / matmul | §21.A–C — Multi-dim arrays, broadcasting, matmul |
| Self-hosting compiler | §22.6 — Grammar/parsing concepts; §22.5 — serialization for `.jadei` files |
| Benchmarks vs C | §21.D — SIMD intrinsics for tighter parity |

---

*Jade doesn't need to be every language. It needs to be the language that makes systems programming readable, fast, and safe — and to be deliberate about every feature it adds. The recommendations above are filtered through that lens: each one either makes Jade more readable (English keywords, named args), faster (arenas, parallel iterators, atomics), safer (try, defer, structured concurrency), or more productive (derive, iterators, testing). Nothing here makes Jade more clever. Everything here makes Jade more useful.*
