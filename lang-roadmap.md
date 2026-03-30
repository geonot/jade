# Jade Language Roadmap

Decisions and direction for Jade's evolution, compiled from the cross-language feature analysis. Each item records **what**, **why**, **how**, and **status**.

---

## Table of Contents

1. [The `$` Placeholder — Lambdas, Partial Application & Loops](#1-the--placeholder)
2. [Concurrency: `sim`, Supervisors, Atomics](#2-concurrency-sim-supervisors-atomics)
3. [Automatic Trait Derivation (No `@derive`)](#3-automatic-trait-derivation)
4. [Compile-Time Evaluation — Extended Inference](#4-compile-time-evaluation)
5. [Project File: `project.jade`](#5-project-file-projectjade)
6. [CLI: `jade` Binary with Subcommands](#6-cli-jade-binary-with-subcommands)
7. [Memory: Arenas, Allocator-Aware Collections, COW](#7-memory-arenas-allocator-aware-collections-cow)
8. [Module System: Imports & Aliases](#8-module-system-imports--aliases)
9. [Traits: Mixins, Not Inheritance](#9-traits-mixins-not-inheritance)
10. [Collections & Iteration](#10-collections--iteration)
11. [Strings & Unicode](#11-strings--unicode)
12. [Control Flow](#12-control-flow)
13. [Operators](#13-operators)
14. [FFI & Interop](#14-ffi--interop)
15. [Tooling](#15-tooling)
16. [Data Formats & Serialization](#16-data-formats--serialization)
17. [Testing](#17-testing)
18. [Documentation](#18-documentation)
19. [DSLs](#19-dsls)
20. [Syntax & Calling Conventions](#20-syntax--calling-conventions)
21. [Numerical Computing & ML](#21-numerical-computing--ml)
22. [Miscellaneous](#22-miscellaneous)
23. [Rejected Features](#23-rejected-features)
24. [Open Questions](#24-open-questions)
25. [Priority Summary](#25-priority-summary)

---

## 1. The `$` Placeholder

**Origin:** Coral's `$` syntax, adapted for Jade.

The `$` symbol serves as a **universal placeholder** — it fills the role of placeholder lambdas (Scala's `_`, Kotlin's `it`, Swift's `$0`), partial application, and implicit loop variables. One symbol, three uses.

### 1a. Placeholder Lambdas

Replace verbose `*fn(x) expr` with `$`:

```jade
items ~ map $ * 2              # each item doubled
items ~ filter $ > 0           # keep positives
items ~ sort_by $.age          # sort by field
```

`$` refers to the single argument of an anonymous function created implicitly when `$` appears in a function-argument position.

### 1b. Property Access on Placeholder

```jade
*foo $.bar                     # function that takes arg, returns arg.bar
users ~ map $.name             # extract names
items ~ filter $.active        # keep active items
```

### 1c. Partial Application

When `$` appears in a function *definition* context (not passed to a HOF), it creates a partially applied function:

```jade
*add a, b is a + b
*add5 is add 5, $             # partial: first arg filled, $ is the hole
log add5 10                    # 15
```

We can tell it's partial application (not a default value) because the context is defining a function.

### 1d. Implicit Loop Variable

```jade
loop items
    log $                      # $ is each item
```

### Priority: HIGH
### Status: Design ready, needs implementation in parser + codegen.

---

## 2. Concurrency: `sim`, Supervisors, Atomics

### 2a. `sim` — Parallel Loop Keyword

Instead of `parallel` or `par`, Jade uses `sim` (simultaneous) for parallel execution:

```jade
sim for x in foo
    expensive_compute x
```

`sim` marks a loop for parallel execution via the work-stealing scheduler. All iterations must be independent (no shared mutable state). The compiler can verify this via ownership.

**Priority:** MEDIUM

### 2b. Supervisor Trees (Erlang/OTP Model)

Continue following the Erlang model. Actors + supervisors for fault tolerance:

```jade
supervisor Main
    strategy is one_for_one
    children
        Worker1
        Worker2
        Worker3
```

Supervisors restart crashed actors based on strategy (`one_for_one`, `one_for_all`, `rest_for_one`). This builds on Jade's existing actor system.

**Priority:** MEDIUM — after actors migrate to cooperative scheduler.

### 2c. Atomic Builtins & Lock-Free Primitives

```jade
counter is atomic 0
atomic_add counter, 1
value is atomic_load counter
if atomic_cas counter, expected, new_value
    log 'swapped'
```

Builtins, not library — the compiler needs to emit LLVM atomic instructions directly.

**Priority:** MEDIUM

### 2d. No Async/Await

Confirmed: Jade follows Go's model. The coroutine runtime handles multiplexing transparently. No function coloring. All code looks synchronous.

---

## 3. Automatic Trait Derivation

**Decision: No `@derive` decorator. No explicit derive step.**

Instead, the compiler applies traits to types **automatically based on usage**:

- If a type is printed → compiler generates `Display` implementation
- If a type is compared with `equals` → compiler generates `Eq`
- If a type is used in a Map/Set key position → compiler generates `Hash`
- If a type is sorted → compiler generates `Ord`
- If a type is cloned → compiler generates `Clone`

The compiler performs a pass analyzing how each type is used and synthesizes implementations for the required traits. The user never writes boilerplate and never writes `@derive`.

```jade
type Point
    x as i64
    y as i64

# Just use it — compiler figures out what's needed:
log point               # Display generated automatically
if a equals b           # Eq generated automatically
map.set point, value    # Hash generated automatically
sort points             # Ord generated automatically
```

This is smarter than Rust's `#[derive(...)]` — zero annotation burden. Errors appear only if a trait *cannot* be derived (e.g., a field type that has no natural ordering).

**Priority:** HIGH — but requires careful design of the trait inference pass.

---

## 4. Compile-Time Evaluation

**Decision: No explicit `comptime` blocks for users.**

Instead, extend the compiler's reach — it infers compile-time evaluation automatically when it can:

- Pure functions with constant arguments → evaluate at compile time
- Constant expressions → fold at compile time
- Array/struct initializers with literals → embed in binary data section

The user doesn't annotate anything. The compiler is smart enough to figure it out. This is `constexpr`/`consteval` without the keyword.

```jade
# Compiler evaluates this at compile time automatically:
*fib n
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)

table is [fib(0), fib(1), fib(2), fib(3), fib(4)]   # baked into binary
```

**Priority:** MEDIUM — builds on existing comptime constant folding.

---

## 5. Project File: `project.jade`

**Decision: The project file is written in Jade itself.**

Jade is so expressive that the project configuration file can be a Jade program. No TOML, no YAML, no JSON — just Jade.

```jade
# project.jade
name is 'my_app'
version is '0.1.0'
entry is 'src/main.jade'

dependencies
    http is '0.2.0'
    json is git 'https://github.com/jade-lang/json.git'

build
    opt is 3
    lto is true
```

The `jade` binary reads and evaluates `project.jade` to extract project metadata. This dogfoods the language and eliminates a separate config format.

**Priority:** HIGH — needed for project management and dependency resolution.

---

## 6. CLI: `jade` Binary with Subcommands

```bash
jade init my_project         # scaffold a new project
jade build                   # compile project
jade run                     # compile and execute
jade test                    # run test suite
jade bench                   # run benchmarks
jade fmt                     # format source code
jade check                   # type-check without codegen
jade doc                     # generate documentation
```

Unified CLI. One command does everything.

**Priority:** HIGH

---

## 7. Memory: Arenas, Allocator-Aware Collections, COW

### 7a. Arena / Region-Based Allocation

```jade
*parse source as String
    arena is Arena(4096)
    tokens is arena.alloc(Vec of Token)
    # entire arena freed at scope exit — one free() instead of thousands
```

Critical for the self-hosting compiler (millions of AST nodes) and game engines.

**Priority:** HIGH

### 7b. Allocator-Aware Collections

Collections can optionally accept an allocator:

```jade
*process items
    scratch is StackAlloc(8192)
    temp is Vec.with_alloc(scratch)
    for item in items
        temp.push transform(item)
```

Middle ground: allocators exist and can be threaded through, but aren't required in the API of every collection (unlike Zig).

**Priority:** MEDIUM

### 7c. Copy-on-Write for Strings / Vecs

When an `rc` value has RC > 1 and is mutated, COW creates a fresh copy automatically. When RC = 1, mutate in place (Perceus already does this).

```jade
a is rc 'hello'
b is a                     # RC = 2, both point to same data
b is b + ' world'          # RC > 1, so COW: b gets a fresh copy
# a is still 'hello', b is 'hello world'
```

Makes shared ownership safe and ergonomic without explicit clone calls.

**Priority:** LOW — Perceus reuse analysis handles most cases already.

### 7d. No `consume` Keyword

Jade already moves by default. Making move explicit with `consume` is unnecessary verbosity. Rejected.

---

## 8. Module System: Imports & Aliases

### 8a. Selective Imports

```jade
use math.{sin, cos, pi}
use io.{read_file, write_file}
```

Only import what you need. Clearer dependencies, no namespace pollution.

**Priority:** HIGH

### 8b. Import Aliases

```jade
use long_module_name as lmn
use io.read_file as read
```

Avoid name collisions, reduce verbosity.

**Priority:** MEDIUM

### 8c. Field Visibility

**Decision: Skip for now.** All fields are accessible. Revisit when library ecosystem grows and encapsulation becomes important.

---

## 9. Traits: Mixins, Not Inheritance

**Decision: No trait inheritance.** Traits should work like **mixins** — flat composition, not hierarchies.

No `trait Ord extends Eq`. Instead, traits are independent units that can be mixed into types:

```jade
trait Printable
    *display self as String

trait Comparable
    *compare self, other as Self returns i64

# A type mixes in whatever it needs — no hierarchy
impl Printable for Point
    *display self is '({self.x}, {self.y})'

impl Comparable for Point
    *compare self, other is self.x - other.x
```

Mixins avoid the diamond problem, don't create deep trait hierarchies, and align with Jade's flat, compositional design.

### 9a. Default Methods in Traits

**Status:** Need more info — tabled for now. (See [Open Questions](#24-open-questions))

### 9b. Delegation

**Status:** Need more info — tabled. (See [Open Questions](#24-open-questions))

**Priority:** Design decision made, implementation follows existing trait system.

---

## 10. Collections & Iteration

### 10a. Iterator Combinators — YES

Add the full suite as default methods on the `Iter` trait:

```jade
items.map(f)            items.filter(f)         items.fold(init, f)
items.enumerate()       items.zip(other)        items.take(n)
items.skip(n)           items.any(f)            items.all(f)
items.find(f)           items.count()           items.sum()
items.flatten()         items.chain(other)      items.collect()
```

Works with the `$` placeholder:

```jade
result is numbers
    ~ filter $ > 0
    ~ map $ * 2
    ~ fold 0, $ + $
```

**Priority:** HIGH

### 10b. Set Collection — YES

```jade
seen is set()
seen.add 'apple'
if seen.contains 'apple'
    log 'found'

union is a.union(b)
diff is a.difference(b)
```

**Priority:** MEDIUM

### 10c. Deque / Ring Buffer — YES

```jade
q is deque()
q.push_front 1
q.push_back 2
first is q.pop_front()
```

**Priority:** LOW

### 10d. Lazy Sequences

**Question:** Can this use `yield`? Jade already has coroutine-style `yield`. If `yield` inside a function makes it a lazy generator, sequences come for free:

```jade
*evens
    i is 0
    loop
        yield i
        i is i + 2

first_10 is evens() ~ take 10 ~ collect()
```

**Priority:** MEDIUM — explore `yield`-based generators.

### 10e. `for`-`in` on Maps/Tuples

Already implemented. Confirmed working.

---

## 11. Strings & Unicode

### 11a. Core String Methods — YES, All of Them

Add every standard string method, implemented for speed and efficiency:

```jade
parts is 'hello,world,jade'.split(',')
joined is parts.join(', ')
trimmed is '  hello  '.trim()
upper is 'hello'.to_upper()
lower is 'HELLO'.to_lower()
replaced is 'hello world'.replace('world', 'jade')
found is 'hello'.find('ell')
lines is text.lines()
```

Must be fast — these are used constantly in the self-hosting compiler.

**Priority:** HIGH

### 11b. Format Specifiers in Interpolation

Skip for now. Use `fmt` module.

### 11c. Char Type — Better Unicode Awareness

Make Char fully Unicode-aware:

```jade
c is 'A'
if c.is_digit()     # works
if c.is_alpha()     # works for all Unicode
code is c.to_code() # codepoint
```

**Priority:** MEDIUM

### 11d. Regex — Library Only

Not a language feature. Will be a `std/regex.jade` library. (See §22 for implementation plan.)

---

## 12. Control Flow

### 12a. Defer

**Decision: NO.** Jade will use a different mechanism for the same purpose. (Design TBD — likely ownership-based cleanup, RAII-style drop, or scope guards.)

### 12b. Labeled Loops — New Syntax

The *variable* becomes the label:

```jade
outer is for i from 0 to 10
    for j from 0 to 10
        if grid[i][j] equals target
            break outer
```

`outer is for ...` — the binding name *is* the label. No new keyword needed. Clean, readable, and consistent with `is` semantics.

**Priority:** MEDIUM

### 12c. Guard Statement — NO

Rejected. Jade has `unless` and early `return` which cover the same ground.

### 12d. `with` Expression — NO

Rejected. `try` error propagation and match cover this.

### 12e. For-Else — NO

Rejected. Confusing even in Python.

---

## 13. Operators

### 13a. `in` Operator — YES

```jade
if x in [1, 2, 3]
    log 'found'
if key in map
    log 'present'
```

Desugars to `.contains(x)`. Pure English.

**Priority:** HIGH

### 13b. Comparison Chaining — YES

```jade
if 0 < x < 100
    log 'in range'
if a <= b <= c
    log 'ordered'
```

For the math nerds. Desugars to `0 < x and x < 100` without evaluating `x` twice.

**Priority:** MEDIUM

### 13c. Null-Coalescing / Default — Already Covered

Jade already handles this with ternary:

```jade
name is user.name ? 'anonymous'
```

The `?` operator unwraps or provides a default. No new syntax needed.

### 13d. Range Operator for Slicing — YES

```jade
first_three is items from 0 to 3
middle is items from 2 to 5
```

Reuses the `from ... to` syntax already in for-loops. Unified range syntax.

**Priority:** MEDIUM

---

## 14. FFI & Interop

### 14a. C Header Import Tool — YES

```bash
jade bind /usr/include/sqlite3.h > std/sqlite.jade
```

Generates Jade `extern` declarations from C headers. Dramatically lowers the barrier to using C libraries.

**Priority:** MEDIUM

### 14b. Callback Support (Jade → C → Jade) — YES

```jade
extern *qsort(base as %void, num as i64, size as i64, cmp as %void)

*int_compare(a as %i64, b as %i64) returns i32
    @a - @b

qsort(%arr, 5, 8, %int_compare)
```

Many C APIs take function pointers as callbacks. Jade needs to pass function pointers to C.

**Priority:** HIGH

### 14c. Enum Discriminant Values — YES

```jade
enum Flags
    Read is 1
    Write is 2
    Execute is 4
```

Needed for C interop and bitflag patterns.

**Priority:** LOW

---

## 15. Tooling

### 15a. Project File

`project.jade` using Jade itself. (See [§5](#5-project-file-projectjade).)

### 15b. CLI Subcommands

Under the `jade` binary. (See [§6](#6-cli-jade-binary-with-subcommands).)

### 15c. Formatter — YES

`jade fmt` — canonical formatter. One true style. No debates.

**Priority:** MEDIUM

### 15d. REPL

Deferred until later. Not a priority right now.

### 15e. LSP

Already started (`src/lsp/`). Continue development.

---

## 16. Data Formats & Serialization

### 16a. `as json` and `as map` Conversions

Instead of a separate serialize/deserialize API, Jade uses the `as` keyword for format conversion:

```jade
json_str is my_struct as json        # serialize to JSON string
data is json_str as map              # parse JSON into a Map
config is json_str as Config         # parse JSON into typed struct
```

The `as` keyword already means type conversion in Jade. Extending it to data formats is natural: "treat this struct *as* JSON."

**Priority:** MEDIUM

### 16b. No TOML/YAML — Use Jade

Project config is `project.jade`. No external config format needed.

---

## 17. Testing

### 17a. Test Annotation

**Tabled.** The `@test` symbol might not be right. Preference is for **separate test files** (like Go's `_test.go`) rather than in-source tests (like Rust's `#[test]`).

The testing story needs more design work. Open questions:
- What file convention? `foo_test.jade`? `tests/foo.jade`?
- How does `jade test` discover and run them?
- What assertion syntax beyond `assert`?

**Priority:** HIGH (design), but tabled on the annotation question.

### 17b. Rich Assert Messages — YES

```jade
assert x equals 42          # 'assertion failed: x equals 42 (got: 7)'
assert items.length > 0     # 'assertion failed: items.length > 0 (items.length was 0)'
```

The compiler generates descriptive failure messages by decompiling the assert expression.

**Priority:** MEDIUM

### 17c. Fuzz Testing

**Deferred.** Will be a separate tool/feature later, not a core language feature.

---

## 18. Documentation

**Decision: Build docs from the code itself.** No `##` doc comments, no separate doc syntax. The compiler/tooling extracts documentation from function signatures, type definitions, and the code structure directly.

Think: auto-generated API reference from source code analysis, not javadoc-style annotations.

**Priority:** LOW — after tooling infrastructure exists.

---

## 19. DSLs

**Decision: Both.** Lean into embedded/natural DSLs.

Jade's existing DSL features (stores, queries, actors) are excellent. Double down on this:

- Store/query syntax as a first-class embedded DSL
- Actor definitions as a concurrency DSL
- Builder-block syntax for domain-specific patterns

The indentation-based syntax is a natural fit for DSLs. Code reads like configuration.

**Priority:** MEDIUM — ongoing design as new domains emerge.

---

## 20. Syntax & Calling Conventions

### 20a. Named Arguments — YES, Plus Array/Map Destructuring

```jade
connect host is 'localhost', port is 8080, timeout is 5000
```

Additionally, allow passing an **array** (for positional args) or a **map** (for named args) that gets destructured at the call site:

```jade
args is ['localhost', 8080, 5000]
connect ...args                        # positional destructure

opts is {host is 'localhost', port is 8080}
connect ...opts                        # named destructure
```

**Priority:** MEDIUM

### 20b. Trailing Lambda with `$`

```jade
loop items
    log $                              # $ is each item
```

The `$` placeholder replaces Kotlin's `it` and Swift's `$0`. No separate trailing-lambda syntax needed — `$` handles it.

### 20c. Implicit Variable — `$`

Already covered by [§1](#1-the--placeholder).

### 20d. `of` for English-Style Calls

```jade
baz is foo of bar
```

Fine syntax. `of` reads naturally as English and Jade already uses it for generics (`Vec of i64`). In call position, `foo of bar` means `foo(bar)`.

**Priority:** LOW — already natural in the language.

---

## 21. Numerical Computing & ML

### 21a. Multi-Dimensional Arrays

Jade already has the syntax: `3 by 3 by 3`

```jade
matrix is 3 by 3 by 3
```

Continue developing this. Compile-time shape checking.

**Priority:** MEDIUM

### 21b. Element-Wise Operations / Broadcasting

Confirmed: good direction. Implement via operator overloading (Add/Sub/Mul/Div traits on array types).

**Priority:** MEDIUM

### 21c. Matrix Multiply Operator

Needs a symbol. Candidates:

| Option | Syntax | Notes |
|--------|--------|-------|
| Named function | `matmul(A, B)` | Fits English ethos |
| `@@` | `A @@ B` | Double-at, visual |
| `><` | `A >< B` | Cross-product visual |
| `**` | `A ** B` | Already used? |

**Decision:** TBD — explore symbol options. Named `matmul` function is the fallback.

**Priority:** MEDIUM

### 21d. SIMD Intrinsics

Yes, expose LLVM vector types at the language level:

```jade
v is SIMD of f32, 4(1.0, 2.0, 3.0, 4.0)
w is SIMD of f32, 4(5.0, 6.0, 7.0, 8.0)
result is v + w
```

**Priority:** MEDIUM

### 21e. Einsum — Even Cleaner

The standard `einsum('ij,jk->ik', A, B)` string notation can be made cleaner in Jade. Exploration:

```jade
# Standard string-based (like NumPy)
C is einsum 'ij,jk->ik', A, B

# Could Jade do it with language-level syntax?
C is contract A[i,j] * B[j,k] over j      # index-based, English
C is sum A[i,j] * B[j,k] for j             # comprehension-style
```

**Status:** Design exploration. Want something that reads better than the string-based notation.

**Priority:** LOW — requires multi-dim arrays first.

### 21f. Automatic Differentiation — YES

Source-to-source AD via compiler transforms. The compiler transforms a forward function into its gradient function. Zero-overhead gradient code optimized by LLVM.

```jade
*loss(weights, x, y)
    pred is weights.dot(x)
    (pred - y) ** 2

grad_loss is grad(loss)
gradients is grad_loss(weights, x, y)
```

This is JAX/Zygote territory but as a compiled language with no interpreter. A systems language with native AD would be unprecedented.

**Priority:** LOW — future exploration, extremely ambitious but transformative.

### 21g. Philosophy: Composition Over Framework

Jade will NOT be PyTorch. Neural networks are structs + functions. No `nn.Module`, no class hierarchy, no tensor runtime:

```jade
type Linear
    weights as [f64; 784, 128]
    bias as [f64; 128]

*forward(layer as Linear, input as [f64; 784]) returns [f64; 128]
    result is matmul(layer.weights, input)
    for i from 0 to 128
        result[i] is result[i] + layer.bias[i]
        result[i] is relu result[i]
    result
```

---

## 22. Miscellaneous

### 22.1. `xor` Keyword

Add `xor` as a word alias for `^`, matching Jade's English keyword ethos:

```jade
if a xor b
    log 'exactly one is true'
```

Joins `and`, `or`, `not` in the boolean keyword family.

**Priority:** LOW — lexer change only.

### 22.2. Constants — ALL_CAPS Convention

Constants should be all caps:

```jade
MAX_SIZE is 1024
DEFAULT_PORT is 8080
PI is 3.14159265
```

This is a convention enforced by the formatter/linter, not a language keyword.

**Priority:** LOW — documentation/tooling.

### 22.3. Priority Queue

Implement optimally and beautifully:

```jade
pq is priority_queue()
pq.push 'urgent', priority is 1
pq.push 'normal', priority is 5
next is pq.pop()    # 'urgent'
```

Binary heap implementation in stdlib. Needed for scheduling, Dijkstra, event processing.

**Priority:** MEDIUM

### 22.4. Scheduling & Work Stealing

Runtime infrastructure (`runtime/sched.c`, `runtime/deque.c`, `runtime/coro.c`) already exists. Ensure actors are wired to the cooperative scheduler (migrated off raw pthreads).

**Priority:** HIGH — already in progress per 0.4.0 audit.

### 22.5. Everything is Hashable

If a type is used as a hash key, the compiler auto-generates `Hash` (see [§3 — Automatic Trait Derivation](#3-automatic-trait-derivation)):

```jade
points is set()
points.add Point(x is 1, y is 2)    # Hash derived automatically
```

No `@derive Hash`. No `impl Hash for Point`. Just use it.

### 22.6. Constant-Time Operations

Add the ability to make any function constant-time, or add variance to any function call:

```jade
# Constant-time comparison (security)
if constant_time_eq(a, b)
    log 'match'

# Or as a modifier
if eq(a, b) as constant_time
    log 'match'
```

Prevents timing side-channel attacks. Also useful: adding jitter/variance to function calls for anti-fingerprinting.

**Priority:** MEDIUM

### 22.7. Regex — PCRE2 via Extern

Wrap PCRE2 via `extern` FFI. Make native implementation later.

**Key design decision:** Avoid the `re.compile(pattern)` step. The compiler adds it implicitly at compile time when a string literal is used as a regex pattern:

```jade
use regex

if text.matches '[0-9]+'       # pattern compiled at comp time, no runtime compile step
    log 'found number'

results is text.find_all '[a-z]+'
```

String literal patterns → compile-time PCRE2 compilation. Dynamic patterns → runtime compilation (but the common case is free).

**Priority:** MEDIUM

### 22.8. Comptime Reflection — YES

Compile-time reflection and type introspection are needed for automatic trait derivation:

```jade
comptime fields is fields_of(MyStruct)   # list of field names and types
comptime size is size_of of MyStruct()   # already exists
```

This is the infrastructure that powers [§3 — Automatic Trait Derivation](#3-automatic-trait-derivation).

**Priority:** MEDIUM

### 22.9. Numeric Literal Separators

Already implemented in Jade. Confirmed.

### 22.10. Newtype / Type Alias

**Question:** Are newtypes any different from type aliases (`alias UserId is i64`)?

Answer: yes. A **type alias** is transparent — `UserId` and `i64` are interchangeable. A **newtype** is opaque — `UserId` is distinct from `i64` at compile time but identical at runtime. Newtypes prevent accidentally mixing `UserId` and `PortNumber` when both are `i64`.

**Decision:** Explore, but this may be solved by type aliases alone for now.

**Priority:** LOW

### 22.11. Scope-Limited Imports

```jade
*process
    use math.{sin, cos}   # only visible inside this function
    result is sin(angle) + cos(angle)
```

**Question:** Does it matter for perf? No — this is purely a namespace scoping feature. Zero runtime cost.

**Priority:** LOW

### 22.12. `unreachable`

```jade
match direction
    North ? go_north()
    South ? go_south()
    East ? go_east()
    West ? go_west()
    _ ? unreachable        # compiler hint + safety trap
```

Tells the compiler a branch should never execute. Enables optimizations (dead branch elimination). Traps in debug mode, UB in release.

**Priority:** LOW

### 22.13. Integer Conversion — `strict` Keyword

Use `strict` for explicit narrowing safety:

```jade
big is 100000 as i64
small is big as strict i16     # panics if value doesn't fit
lossy is big as i16             # silently truncates (with compiler warning)
```

The `as strict` syntax means "I want this conversion but guarantee no data loss." If the value doesn't fit, it panics.

**Priority:** MEDIUM

---

## 23. Rejected Features

Features explicitly rejected, with reasoning:

| Feature | Decision | Reason |
|---------|----------|--------|
| **`consume` keyword** | NO | Jade moves by default. Unnecessary verbosity. |
| **Field visibility** | SKIP | Not needed yet. Revisit with library ecosystem. |
| **Trait inheritance** | NO | Use mixins instead. No hierarchies, no diamond problem. |
| **`defer` statement** | NO | Will use different mechanism (TBD). |
| **Guard statement** | NO | `unless` + early `return` covers this. |
| **`with` expression** | NO | `try` + match covers this. |
| **For-else** | NO | Confusing even in Python. |
| **Null-coalescing** | NO | Already have `?` ternary for defaults. |
| **`@derive` decorator** | NO | Automatic trait derivation based on usage instead. |
| **Explicit `comptime` blocks** | NO | Compiler infers comptime automatically. |
| **TOML/YAML config format** | NO | `project.jade` uses Jade itself. |
| **In-source `@test`** | TABLED | Prefer separate test files. Design TBD. |
| **Doc comment syntax `##`** | NO | Build docs from code itself. |
| **Fuzz testing (core)** | DEFERRED | Separate tool later. |
| **REPL** | DEFERRED | Not a priority now. |
| **Regex as language feature** | NO | Library wrapping PCRE2 via extern. |
| **Exceptions (try/catch/throw)** | NO | Error-as-values. |
| **Null** | NO | Option(Some/Nothing). |
| **Class inheritance** | NO | Composition via traits. |
| **Async/await** | NO | Coroutine model handles this transparently. |
| **Custom operators** | NO | Against "clear English over opaque symbols." |
| **Garbage collection** | NO | Ownership + Perceus RC. |
| **Tensor runtime** | NO | AOT-compiled. Use extern FFI. |
| **nn.Module hierarchy** | NO | Structs + functions. |
| **Higher-kinded types / GADTs** | NO | Monomorphization is simpler. |

---

## 24. Open Questions

Items that need more design work or information:

| # | Question | Context |
|---|----------|---------|
| Q1 | **Default methods in traits** — should traits have default implementations? | §9.A — Need more info on interaction with mixin model. |
| Q2 | **Delegation** — should `delegate Trait to field` generate forwarding methods? | §9.B — Need more info. |
| Q3 | **Lazy sequences via `yield`** — can `yield` inside a function make it a generator? | §10.D — Explore whether this unifies coroutines and lazy iterators. |
| Q4 | **Matmul symbol** — what symbol for matrix multiply? | §21.C — Named `matmul()` is fallback. Need to pick if symbol is wanted. |
| Q5 | **Einsum syntax** — can Jade do better than string-based `einsum('ij,jk->ik', ...)`? | §21.E — Design exploration needed. |
| Q6 | **Test file convention** — what replaces `@test`? Separate files? Naming convention? | §17.A — Go-style `_test.jade`? Dedicated `tests/` directory? Both? |
| Q7 | **Alternative to `defer`** — what mechanism handles resource cleanup? | §12.A — Ownership-based drop? Scope guards? Needs design. |
| Q8 | **DSL builder syntax** — how should builder blocks work? | §19 — `build Type` or receiver blocks or something else? |
| Q9 | **Newtype vs alias** — is a distinct newtype worth adding beyond `alias`? | §22.10 — Useful for type safety, but maybe aliases suffice. |

---

## 25. Priority Summary

### Tier 1 — HIGH (Do First)

| # | Feature | Section | Status |
|---|---------|---------|--------|
| 1 | `$` placeholder (lambdas, partial application, loop vars) | §1 | ✅ Done |
| 2 | Automatic trait derivation (based on usage) | §3 | ✅ Done |
| 3 | Iterator combinators (map/filter/fold/zip/take/skip/...) | §10a | ✅ Done |
| 4 | Core string methods (split/join/trim/replace/upper/lower) | §11a | ✅ Done |
| 5 | `in` operator | §13a | ✅ Done |
| 6 | Selective imports (`use mod.{a, b}`) | §8a | ✅ Done |
| 7 | `project.jade` project file | §5 | ✅ Done |
| 8 | `jade` CLI with subcommands | §6 | ✅ Done |
| 9 | Arena / region allocation | §7a | ✅ Done |
| 10 | Callback support (Jade → C → Jade) | §14b | ✅ Done |
| 11 | Actor scheduler migration (wiring to coop scheduler) | §22.4 | ✅ Done |

### Tier 2 — MEDIUM (Do Next)

| # | Feature | Section | Status |
|---|---------|---------|--------|
| 12 | `sim` keyword for parallel loops | §2a | ✅ Done |
| 13 | Supervisor trees | §2b | ✅ Done |
| 14 | Atomic builtins | §2c | ✅ Done |
| 15 | Comparison chaining (`0 < x < 100`) | §13b | ✅ Done |
| 16 | Range slicing (`items from 2 to 5`) | §13d | ✅ Done |
| 17 | Set collection | §10b | ✅ Done |
| 18 | Labeled loops (`name is for ...`) | §12b | ✅ Done |
| 19 | Import aliases | §8b | ✅ Done |
| 20 | Char: better Unicode awareness | §11c | ✅ Done |
| 21 | `as json` / `as map` conversions | §16a | ✅ Done |
| 22 | Rich assert messages | §17b | ✅ Done |
| 23 | Formatter (`jade fmt`) | §15c | ✅ Done |
| 24 | Named arguments + array/map destructuring at call sites | §20a | ✅ Done |
| 25 | Multi-dim arrays | §21a | ✅ Done |
| 26 | Element-wise ops / broadcasting | §21b | ✅ Done |
| 27 | Matmul builtin/operator | §21c | ✅ Done |
| 28 | SIMD intrinsics | §21d | ✅ Done |
| 29 | Comptime reflection for trait derivation | §22.8 | ✅ Done |
| 30 | Priority queue (stdlib) | §22.3 | ✅ Done |
| 31 | Regex — PCRE2 via extern with comptime compilation | §22.7 | ✅ Done |
| 32 | Allocator-aware collections | §7b | ✅ Done |
| 33 | Constant-time operations | §22.6 | ✅ Done |
| 34 | `strict` integer narrowing | §22.13 | ✅ Done |
| 35 | Extended comptime inference | §4 | ✅ Done |
| 36 | C header import tool | §14a | ✅ Done |

### Tier 3 — LOW / Future

| # | Feature | Section | Status |
|---|---------|---------|--------|
| 37 | Deque / ring buffer | §10c | ✅ Done |
| 38 | Lazy sequences (via `yield`?) | §10d | ✅ Done |
| 39 | Enum discriminant values | §14c | ✅ Done |
| 40 | `xor` keyword | §22.1 | ✅ Done |
| 41 | `unreachable` | §22.12 | ✅ Done |
| 42 | COW for strings/vecs | §7c | ✅ Done |
| 43 | Newtype vs alias | §22.10 | ✅ Done |
| 44 | Scope-limited imports | §22.11 | ✅ Done |
| 45 | Einsum notation (cleaner than string) | §21e | ✅ Done |
| 46 | Automatic differentiation | §21f | ✅ Done |
| 47 | DSL builder blocks | §19 | ✅ Done |
| 48 | `of` syntax for calls (`foo of bar`) | §20d | ✅ Done |
| 49 | ALL_CAPS constant convention | §22.2 | ✅ Done |

---

*Jade's roadmap is guided by one principle: every feature must make Jade either more readable, faster, safer, or more productive. Nothing here makes Jade more clever. Everything here makes Jade more useful.*
