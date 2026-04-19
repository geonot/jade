# Jade Error Handling — Analysis & Design Document

> Comprehensive review of Jade's current error model, comparison with 20+ languages,
> and concrete proposals for error propagation syntax.

---

## Part 1 — Current State (Verified from Code)

### 1.1 Error Type Declaration (`err`)

Jade has a dedicated `err` keyword for declaring error types. These compile identically
to `enum` (tagged unions) but carry semantic meaning:

```jade
err MathError
    DivisionByZero
    Overflow
    NegativeRoot(String)

err IoError
    FileNotFound(String)
    PermissionDenied(String)
    Timeout(i64)
```

**Implementation**: `src/parser/decl.rs:536` → `ast::Decl::ErrDef` → `hir::ErrDef` with
`ErrVariant` (name, fields, tag). Codegen treats them identically to enums — same tagged
union representation, same match dispatch. The `err` keyword exists purely for readability
and intent signaling.

**Verified**: `tests/integration.rs:1038` (`err_def_parse`), `tests/programs/error_handling.jade`.

### 1.2 Early Error Return (`!` / `!!`)

The `!` operator performs an early return of an error value:

```jade
*safe_divide(a, b)
    if b equals 0
        ! DivisionByZero
    a / b
```

**Implementation**: `src/parser/stmt.rs:259` parses `!` and `!!` as `Stmt::ErrReturn`.
The HIR lowers this to `hir::Stmt::ErrReturn(expr, ret_ty, span)`. MIR compiles it to
a plain `Terminator::Return(Some(v))` — it is syntactic sugar for `return ErrorVariant`.

There is no stack unwinding. There are no exceptions. The `!` is a local goto to
the function exit with a specific value.

**Verified**: `tests/integration.rs:1050` (`bang_return_basic`):
```jade
*check(x as i64) returns i64
    if x < 0
        ! -1
    x * 2
```
Output: `10\n-1`.

### 1.3 Error Propagation (`try` — prefix)

Jade has a **prefix** `try` keyword that desugars to a match + early return:

```jade
*process()
    val is try get_option()    # unwrap or propagate Nothing
    result is try compute(val) # unwrap or propagate Err
    result + 1
```

**Implementation**: `src/parser/expr.rs:307` parses `try expr` as `ast::Expr::Try`.
The typer (`src/typer/expr.rs:1582`, `lower_try()`) desugars it to:

- **For Option**: `match expr { Some(v) => v, Nothing => return Nothing }`
- **For Result**: `match expr { Ok(v) => v, Err(e) => return Err(e) }`

The enclosing function must return a compatible Option/Result type.

**Test coverage**: **No integration tests exist for `try`.** The typer code is implemented
but untested. The desugaring was verified by reading `src/typer/expr.rs:1580-1780`.

### 1.4 Option and Result Types (`std/result.jade`)

```jade
enum Option
    Some(i64)
    None

enum Result
    Ok(i64)
    Err(String)
```

Helpers: `is_some`, `is_none`, `unwrap`, `unwrap_or`, `map_opt`, `is_ok`, `is_err`,
`unwrap_ok`, `unwrap_err`, `ok_or`, `map_res`.

**Critical limitation**: These are **monomorphic**. `Option` wraps `i64` only. `Result`
is `Ok(i64) / Err(String)` only. User programs must define their own enums for other types:

```jade
enum MaybeString
    Some(String)
    Nothing
```

### 1.5 Standard Library Error Patterns

The std library does **not** use `Result`/`Option` from `std/result.jade`:

| Module | Error Pattern | Example |
|--------|--------------|---------|
| `http.jade` | Sentinel struct | `HttpResponse(status is 0, body is "failed")` |
| `fs.jade` | Empty string | `cwd()` returns `""` on failure |
| `io.jade` | Empty string | `read_line()` returns `""` on EOF/error |
| `net.jade` | Negative int | `result < 0` |
| `crypto.jade` | Boolean field | `CipherResult(ok is false)` |
| `sqlite.jade` | Method query | `db.error()` returns last error string |

This inconsistency exists because: (a) generic enums don't monomorphize well yet,
(b) the `Result` and `Option` in `std/result.jade` are i64-only, and (c) returning
structs with ok/error fields is simpler when you can't parameterize the type.

### 1.6 What `?` Does Today

The `?` token is used for two unrelated purposes:

1. **Ternary operator**: `cond ? true_val ! false_val`
2. **Match arm separator**: `Some(v) ? v`

There is **no postfix `?`** for error propagation. Jade uses prefix `try` instead.

### 1.7 Summary of Strengths

- **No exceptions**: Errors are values. No hidden control flow. No try/catch/finally.
- **`err` keyword**: Semantic distinction between "this is an error type" and "this is a data enum".
- **`!` early return**: Clean, visible, minimal syntax for error exits.
- **`try` prefix propagation**: Handles the unwrap-or-propagate pattern.
- **Exhaustive match**: Compiler validates all error variants are handled.
- **Zero-cost**: Error types are tagged unions. No heap allocation, no vtable.

### 1.8 Summary of Weaknesses

1. **No generic Option/Result**: `Option` is `i64`-only. Every type needs its own enum.
2. **`try` is untested**: No integration test exercises the prefix `try` desugaring.
3. **Std library ignores its own types**: Every module uses ad-hoc sentinels.
4. **No error chaining**: Cannot wrap inner errors (no `From` trait, no `?` with conversion).
5. **No structured error context**: No way to attach source location, backtrace, or message to an error variant.
6. **No `try` block**: Cannot scope error propagation to a sub-expression.
7. **No `catch`-equivalent**: To handle propagated errors, you must match at the call site.

---

## Part 2 — Error Handling Across Languages

### 2.1 Return-Code Based (C, Go)

**C**: Functions return int error codes. Caller checks `if (ret < 0)`. No enforcement.
Out-of-band error info via `errno`. Errors are trivially ignored.

**Go**: Multiple return values `val, err := fn()`. Convention-enforced `if err != nil`.
No sum types — error is an interface. Verbose but explicit. Errors are values.

```go
val, err := strconv.Atoi(s)
if err != nil {
    return fmt.Errorf("parse failed: %w", err)  // error wrapping
}
```

**Relevance to Jade**: Jade's `err` types are strictly better than C's int codes and Go's
interface errors. The tagged union gives exhaustive checking that Go lacks. But Go's
`%w` wrapping and `errors.Is()` / `errors.As()` for error chains are missing from Jade.

### 2.2 Sum Type + Pattern Match (Rust, OCaml, Haskell)

**Rust**: `Result<T, E>` and `Option<T>` are generic enums. The `?` operator desugars to
early return with `From` trait conversion. `#[must_use]` warns on ignored Results.

```rust
fn parse(s: &str) -> Result<Config, ParseError> {
    let val = s.parse::<i64>()?;  // propagate with From conversion
    Ok(Config { val })
}
```

**OCaml**: `result` type since 4.08. `match` is the primary mechanism. No built-in
propagation operator — use `Result.bind` or `let*` ppx syntax extensions.

```ocaml
let (let*) = Result.bind
let* x = parse s in
let* y = validate x in
Ok (process y)
```

**Haskell**: `Either a b` with monadic do-notation. `ExceptT` monad transformer for
composable error handling. The `>>=` operator is the propagation mechanism.

```haskell
process :: String -> Either AppError Config
process s = do
    val <- parse s          -- propagate via >>=
    validated <- check val
    Right (Config validated)
```

**Relevance to Jade**: Jade already has the core — `err` types ARE sum types, `match`
works, `try` does prefix propagation. The gap is generics (Jade's Option is i64-only)
and conversion (no `From` trait for coercing error types during propagation).

### 2.3 Exception-Based (Java, Python, C#, C++)

**Java**: Checked exceptions force callers to either `catch` or declare `throws`.
Unchecked exceptions (RuntimeException) bypass this. `try/catch/finally` with
multi-catch (`catch (A | B e)`). Verbose, widely criticized but proves the need for
forced error awareness.

**Python**: Everything is an exception. `try/except/else/finally`. No enforcement.
Context managers (`with`) for cleanup. Exception chaining (`raise X from Y`).

**C#**: Unchecked exceptions only. `try/catch/finally/when`. Exception filters
(`catch (E e) when (e.Code == 42)`) are unique.

**Relevance to Jade**: Jade explicitly rejects exceptions — no hidden control flow,
no stack unwinding, no performance cliff. This is the right call. But exception
languages prove the value of: (a) forced error awareness (Java checked), (b) error
chaining (Python `from`), and (c) cleanup blocks (finally/defer).

### 2.4 Effect Systems (Koka, Eff, Unison)

**Koka**: Algebraic effects — errors are effects that can be handled at any scope.
The handler resumes, aborts, or transforms. Sound, composable, but complex.

```koka
fun safe-div(x: int, y: int): exn int
  if y == 0 then throw("division by zero")
  x / y

fun main(): console ()
  with handler
    ctl throw(msg) println("caught: " ++ msg)
  println(safe-div(10, 0))
```

**Unison**: Abilities (algebraic effects). Error handling is an ability you request.
Handler decides policy. Functions are pure descriptions; handlers interpret them.

**Relevance to Jade**: Effect systems are theoretically superior but practically
unfamiliar and complex. Jade's target audience (readability, English-like syntax)
would be poorly served by algebraic effects. However, the *idea* of handlers that
decide policy (retry, log, transform, propagate) is worth noting.

### 2.5 Contract/Assert Based (Eiffel, Ada/SPARK)

**Eiffel**: Design by Contract — preconditions (`require`), postconditions (`ensure`),
invariants. Violations are exceptions but the philosophy is "contracts prevent errors
rather than handling them after the fact."

**Ada/SPARK**: `raise` for exceptions, but SPARK subset uses contracts + formal
verification to prove exceptions cannot occur. Avionics standard.

**Relevance to Jade**: Jade's `assert` builtin is a minimal contract. Adding
`require`/`ensure` blocks on functions would complement error types — contracts
prevent errors at boundaries, error types handle recoverable failures.

### 2.6 Condition Systems (Common Lisp)

**Common Lisp**: The condition system allows signaling a condition without unwinding,
then deciding at a *higher* scope what to do (restart, retry, use-value, abort).
This is the most powerful error system ever built — the handler runs *without*
destroying the intermediate stack frames.

```lisp
(handler-bind
  ((division-by-zero
    (lambda (c) (invoke-restart 'use-value 0))))
  (/ x y))
```

**Relevance to Jade**: Too complex for Jade's philosophy. But the "separate policy
from mechanism" principle is sound — the function signals a problem, the caller
decides the recovery strategy.

### 2.7 Monadic / Railway (Elm, F#, Gleam)

**Elm**: No exceptions. `Result` and `Maybe` types with `andThen` (flatMap).
Pipeline-oriented: `"42" |> String.toInt |> Result.andThen validate`.

**F#**: `Result<'a, 'e>` with computation expressions (`result { ... }`).
Railway-oriented programming — happy path on one track, error path on another.

**Gleam**: `Result(value, error)` with `use` keyword for monadic unwrap:
```gleam
use config <- result.try(parse(input))
use validated <- result.try(validate(config))
Ok(process(validated))
```

**Relevance to Jade**: Jade's pipeline `~` operator could compose with Result types
naturally: `input ~ parse ~ validate ~ process` where each step can fail. This is
the "railway" pattern. Jade's `try` already does the unwrap-or-propagate, but there's
no pipeline-aware error composition.

### 2.8 Defer/Cleanup (Go, Zig, Swift)

**Go**: `defer fn()` runs at function exit regardless of return path.
**Zig**: `errdefer` runs only on error return path. `defer` runs always.
**Swift**: `defer` block runs at scope exit.

**Relevance to Jade**: Jade has no `defer`. Functions that open files, acquire locks,
or allocate resources have no guaranteed cleanup on error paths. Perceus RC handles
memory, but non-memory resources (file handles, network connections) need explicit
close calls that can be missed on `!` error returns.

---

## Part 3 — Design Options for Jade

### Constraints

Any error handling enhancement must:

1. **Not break existing code** — `err`, `!`, `try`, `match` on errors must work unchanged.
2. **Not add exceptions** — No stack unwinding. Errors remain values.
3. **Preserve readability** — Jade's English-like syntax is its identity.
4. **Zero-cost where unused** — No overhead for functions that don't fail.
5. **Work with current `?` usage** — `?` is used for ternary and match arms.

### Option A: Add Postfix `try` (Recommended)

Instead of Rust's `?`, use postfix `try` as a method-like call:

```jade
*load_config(path as String)
    text is read_file(path).try      # propagate error
    config is json.parse(text).try   # propagate error
    config
```

Or equivalently with pipelines:

```jade
*load_config(path as String)
    path ~ read_file ~ try ~ json.parse ~ try
```

**Implementation**: Parse `.try` as a postfix operator in `parse_method_call()`.
Desugar identically to the existing prefix `try` — same HIR `lower_try()` code.
This is purely a parser change; no new semantics.

**Pros**:
- Reads left-to-right (matches data flow)
- No new symbols — reuses existing keyword
- Pipeline-compatible
- Existing prefix `try` remains valid

**Cons**:
- `.try` looks like a method call but isn't
- Two ways to do the same thing (prefix and postfix)

### Option B: Keyword `or` Chains for Error Recovery

```jade
text is read_file(path) or ! IoError:ReadFailed(path)
text is read_file(path) or ''          # provide default
text is read_file(path) or log('warn') # side-effect and propagate
```

This overloads `or` (currently boolean-only) to mean "if error/nothing, then...".

**Implementation**: In the typer, when `or`'s left-hand side is Option/Result,
desugar to a match: `match lhs { Ok(v)/Some(v) => v, _ => rhs }`.

**Pros**:
- Extremely readable English
- Handles both propagation (`or !`) and recovery (`or default`)
- Natural for beginners

**Cons**:
- Overloads `or` which currently means boolean disjunction
- May confuse when LHS is a regular boolean

### Option C: Pipeline-Aware Error Propagation

```jade
*process(input)
    input ~ parse ~ validate ~ transform ~? finalize
```

The `~?` operator means "pipe, but propagate error if previous step failed":

```jade
# Desugars to:
_tmp1 is parse(input)
if is_err(_tmp1) then return _tmp1
_tmp2 is validate(unwrap(_tmp1))
if is_err(_tmp2) then return _tmp2
...
```

**Pros**:
- Integrates naturally with Jade's pipeline syntax
- Compact for chains of fallible operations
- Visually distinct from regular pipes

**Cons**:
- New operator `~?`
- Only works in pipeline context, not standalone expressions
- Complex desugaring

### Option D: `defer` for Resource Cleanup

Orthogonal to error propagation but critical for correct error handling:

```jade
*process_file(path)
    fd is open(path)
    defer close(fd)      # runs at function exit, even on !
    data is read(fd)
    transform(data)
```

**Implementation**: Collect `defer` statements during HIR lowering. At every return
point (normal exit and `!` exits), insert the deferred calls in reverse order.
Alternatively, insert cleanup in the MIR before every `Terminator::Return`.

**Pros**:
- Solves real resource leak problem
- Well-understood semantics (Go, Zig, Swift)
- Composable with `!` error returns

**Cons**:
- New keyword
- Interaction with Perceus RC drops needs specification
- Cannot capture error value (unlike Zig's `errdefer`)

### Option E: `try` Blocks (Scoped Error Handling)

```jade
result is try
    text is read_file(path).try
    config is json.parse(text).try
    config

match result
    Ok(cfg) ? use_config(cfg)
    Err(e) ? log('failed: {e}')
```

A `try` block wraps a scope where `.try` or prefix `try` propagations are caught
at the block boundary rather than the function boundary.

**Pros**:
- Scoped error handling without polluting function signature
- Can handle errors locally without match at every call

**Cons**:
- Complex desugaring
- Blurs the "errors are just values" philosophy

---

## Part 4 — Recommendation

### Immediate (Low-risk, high-value)

1. **Add postfix `.try`** — Parser-only change. Reuses all existing `lower_try()` machinery.
   Enables left-to-right error propagation chains.

2. **Add integration tests for `try`** — The prefix `try` desugaring is untested.
   Add tests for Option propagation, Result propagation, type mismatch errors.

3. **Add `defer`** — Critical for resource cleanup on error paths. Well-understood
   semantics. Moderate implementation effort (MIR insertion before returns).

### Medium-term

4. **Generic Option/Result** — Requires generic collections work (remediation 2.1).
   Once `of T` works for collections, `Option of T` and `Result of T, E` follow.

5. **Standardize std library error patterns** — Once generic Result exists, migrate
   std modules from sentinel values to `Result of T, String`.

### Long-term

6. **Error wrapping/chaining** — `From` trait equivalent for error type conversion
   during `.try` propagation. Enables composing errors from different modules.

7. **`or` for error recovery** — After `or` boolean semantics are well-established
   and the type system can disambiguate Option/Result `or` from boolean `or`.

---

## Part 5 — Script Mode Evaluation

### What `jadec run` Does Today

Implemented in `src/main.rs:2008`. Accepts optional file argument:

```bash
jadec run hello.jade           # compile, cache, execute
jadec run hello.jade -- arg1   # with program arguments
```

**Caching**: Hashes source content, stores compiled binary in `~/.cache/jade/`
(via `dirs_cache()`). Subsequent runs with unchanged source skip compilation.

**Shebang**: The lexer (`src/lexer.rs:418`) skips `#!` lines at position 0:

```jade
#!/usr/bin/env jadec run
*main()
    log('hello from script')
```

### How Other LLVM Languages Handle This

| Language | Command | Mechanism | Cache |
|----------|---------|-----------|-------|
| **Rust** | `cargo run` | Full compile → binary | Incremental via cargo |
| **Swift** | `swift file.swift` | JIT via LLVM MCJIT | None — recompiles each run |
| **Julia** | `julia file.jl` | JIT via LLVM ORC | Package precompilation |
| **Crystal** | `crystal run file.cr` | Full compile → temp binary | None |
| **Nim** | `nim r file.nim` | Full compile → nimcache | Incremental via nimcache |
| **Zig** | `zig run file.zig` | Full compile → temp, execute, delete | None |
| **D** | `rdmd file.d` | Compile → cache dir by hash | Content hash cache |

### Is Script Mode Necessary?

**Yes, but it's already implemented.** The combination of:

- `jadec run file.jade` (compile + cache + execute)
- Shebang support (`#!/usr/bin/env jadec run`)
- Content-hash caching (instant re-runs on unchanged source)

...is equivalent to the best approaches from other compiled languages. Jade's approach
is closest to D's `rdmd` (hash-based caching) and Nim's `nim r` (compile-and-run).

**What would NOT be worthwhile**:

- **Full REPL with JIT** (remediation 5.4): Massive implementation effort for OrcJIT
  integration. Julia spent years on this and it's their core differentiator. Not
  realistic for Jade's team size. The compile-run cycle with caching is sub-second
  for typical scripts.

- **Interpreter mode**: Would require a second complete execution backend. Every
  other compiled language that tried this (Go's `go run`, Rust's `cargo run`) just
  compiles and runs — no interpreter.

**What would be worthwhile** (and is already done):

- Content-hash caching (done)
- Shebang support (done)
- Optional file argument on `jadec run` (done)

### Benchmark Considerations

The key metric for script mode is **cold start time** (first compile) and **warm start
time** (cached binary). Relevant benchmarks:

- Compile time for a minimal program (`log('hello')`)
- Compile time for a program using 3-4 std modules
- Cached execution time (should be ~1ms overhead)
- Compare with `python3 -c`, `node -e`, `go run` for equivalent programs

These can be measured with the existing `run_benchmarks.py` infrastructure by timing
`jadec run` on small programs.

---

## Appendix: Test Coverage Gaps

The following error handling features have **no integration test coverage**:

1. `try` with `Option` — prefix `try` unwrapping Option type
2. `try` with `Result` — prefix `try` unwrapping Result type
3. `try` type mismatch — `try` on non-Option/Result should error
4. `try` return type validation — function must return compatible type
5. `!` with `err` payloads — `! FileNotFound(path)` preserving payload
6. Chained `!` across function calls — propagation through multiple layers
7. `err` variant exhaustiveness in `match` — missing variant should warn/error

## Current State (verified 2026-04-15)

Jade's error handling is **errors-as-values** — no exceptions, no stack unwinding, no hidden control flow. Errors are algebraic data types (tagged unions) with the same representation as enums.

### What Exists

#### 1. `err` Definitions — Custom Error Types
```jade
err MathError
    DivisionByZero
    Overflow
    NegativeRoot

err IoError
    FileNotFound(String)
    PermissionDenied(String)
    Timeout(i64)
```
Compiled as tagged unions identical to `enum`. Each variant gets a sequential tag. Payload fields are stored in a byte array alongside the tag.

#### 2. `!` / `!!` — Error Return Operator
```jade
*safe_divide(a, b)
    if b equals 0
        ! DivisionByZero    # early return with error value
    a / b
```
`!` compiles to a plain `return`. The `!!` variant exists for disambiguation from the ternary else operator (`cond ? then ! else`).

#### 3. Pattern Matching on Errors
```jade
match safe_divide(10, 0)
    DivisionByZero ? log('caught division by zero')
    _ ? log('ok')
```
Since errors are enums, the existing match infrastructure handles them — including variants with payloads:
```jade
match open_file('')
    FileNotFound(msg) ? log(msg)
    PermissionDenied(msg) ? log(msg)
    _ ? log('file opened')
```

#### 4. Qualified Variant Syntax
```jade
! FileError:NotFound           # explicit type qualification
! IoError:Timeout(5000)        # qualified with payload
```

#### 5. Prelude `Result of T, E` and `Option of T`
Generic types auto-registered in the prelude:
- `Result of T, E` with variants `Ok(T)` and `Err(E)`
- `Option of T` with variants `Some(T)` and `Nothing`

These get monomorphized: `Result_i64_String`, `Option_f64`, etc.

#### 6. `try` — Error Propagation Keyword
```jade
*do_thing() returns Option
    v is try get_val()     # unwraps Some or early-returns Nothing
    Some(v + 1)
```
Desugars in the typer to:
- **Option**: `match expr { Some(v) => v, Nothing => return Nothing }`
- **Result**: `match expr { Ok(v) => v, Err(e) => return Err(e) }`

Verified by 4 passing tests: `try_option_some`, `try_option_nothing`, `try_result_ok`, `try_result_err`.

#### 7. Runtime Traps
Compiler-inserted aborts for unrecoverable conditions:
- Division by zero
- Index out of bounds
- Strict cast overflow
- Float NaN cast

These call `__jade_trap(msg)` which prints to stderr and aborts. Not user-facing.

#### 8. `std/result.jade` — Helper Functions
Non-generic helpers: `is_some`, `is_none`, `unwrap`, `unwrap_or`, `map_opt`, `is_ok`, `is_err`, `unwrap_ok`, `unwrap_err`, `ok_or`, `map_res`. These are i64-specific and redundant with the prelude generics.

### Working Examples (all tested)

```jade
# Error definition + return
err IoError
    NotFound
    Permission

*check(x as i64) returns i64
    if x < 0
        ! -1
    x * 2

# Result with match
enum Result
    Ok(i64)
    Err(i64)

*checked_add(a, b) returns Result
    sum is a + b
    if sum > 100
        return Err(sum)
    Ok(sum)

# try propagation (Option)
*get_val() returns Option
    Some(42)

*do_thing() returns Option
    v is try get_val()
    Some(v + 1)
```

### Test Coverage
| Feature | Tests | Status |
|---------|-------|--------|
| `err` definitions | `err_def_basic` | Passing |
| `!` error return | `bang_return` | Passing |
| `match` on error variants | `error_handling.jade` | Passing |
| `try` with Option (Some) | `try_option_some` | Passing |
| `try` with Option (Nothing) | `try_option_nothing` | Passing |
| `try` with Result (Ok) | `try_result_ok` | Passing |
| `try` with Result (Err) | `try_result_err` | Passing |

---

## What's Missing or Lacking

### 1. No `try` with `err` Types
`try` only works with `Option` and `Result` — not with custom `err` definitions. You can define `err MathError` but cannot use `try do_math()` to auto-propagate it. Must manually match every error site.

### 2. No Error Context / Wrapping
No way to add context when propagating: "file not found *while parsing config*". Must manually construct new error values at each site.

### 3. No Stack Traces or Source Locations
When `! FileNotFound('x')` propagates up 5 levels, there's no way to know where the error originated. No `__FILE__`, `__LINE__` equivalent.

### 4. No `finally` / Cleanup Guarantees
No mechanism to guarantee cleanup runs (close file, release lock) when errors short-circuit a function via `!`. Perceus drops handle memory, but not logical resources.

### 5. No Typed Return Annotation for Errors
Functions don't declare what errors they can return:
```jade
*open_file(path) returns ???   # What errors can this produce?
```
The caller must read the source to know what to match against.

### 6. No Error Conversion / Coercion
No automatic conversion between error types. If `parse()` returns `ParseError` and `open()` returns `IoError`, a function calling both must manually map one to the other.

### 7. Collections Helpers for Result/Option Are i64-Only
`std/result.jade` defines non-generic `Option` and `Result` (both with `i64`). The prelude generics exist but lack helper methods — no `map`, `and_then`, `or_else`, `flatten` for generic variants.

---

## Survey of Error Handling Across Languages

### A. Exceptions (Java, Python, C#, Ruby)
```java
try {
    file = open(path);
} catch (FileNotFoundException e) {
    log(e.getMessage());
} finally {
    file.close();
}
```
**Pros**: Familiar; separates error handling from logic; `finally` guarantees cleanup; stack traces automatic.
**Cons**: Hidden control flow — any function can throw without declaring it; performance cost of stack unwinding; exception-as-flow-control abuse.

### B. Checked Exceptions (Java)
```java
void readFile(String path) throws IOException, ParseException { ... }
```
**Pros**: Caller knows exactly what can fail; compiler enforces handling.
**Cons**: Extremely verbose; leads to `throws Exception` everywhere; combinator composition is painful; widely considered a failed experiment — Kotlin, Scala, C# all abandoned it.

### C. Result + `?` Operator (Rust)
```rust
fn parse_config(path: &str) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path)?;   // propagates io::Error
    let config = toml::from_str(&text)?;     // propagates toml::Error
    Ok(config)
}
```
**Pros**: Zero-cost; explicit in type signature; `?` is concise; `From` trait enables automatic error conversion; no hidden control flow.
**Cons**: Viral — once you use `Result`, every caller must too; boilerplate for custom error types (`thiserror`, `anyhow`); `From` impls proliferate.

### D. Error Values (Go)
```go
file, err := os.Open(path)
if err != nil {
    return fmt.Errorf("opening config: %w", err)
}
```
**Pros**: Explicit; no hidden control flow; wrapping with `%w` adds context; `errors.Is()` / `errors.As()` for matching.
**Cons**: Extremely verbose — `if err != nil` on every other line; easy to forget checking; no exhaustiveness guarantee.

### E. Algebraic Effects (OCaml 5, Koka, Eff)
```
effect Fail { fun fail(msg: string): a }

fun safe_div(x, y)
  if y == 0 then fail("division by zero")
  x / y

with handler { fun fail(msg) -> 0 }
  safe_div(10, 0)
```
**Pros**: First-class; composable; handler can resume, abort, or transform; subsumes exceptions, async, generators.
**Cons**: Complex mental model; limited ecosystem support; implementation overhead.

### F. Monadic Error Handling (Haskell)
```haskell
parseConfig :: FilePath -> IO (Either ConfigError Config)
parseConfig path = runExceptT $ do
    text <- liftIO (readFile path) `catchE` (throwE . FileErr)
    config <- liftEither (parseToml text)
    pure config
```
**Pros**: Composable via `>>=` / do-notation; `ExceptT` transformer stacks; pure.
**Cons**: Monad transformer hell; difficult for newcomers; `IO` exceptions exist alongside `Either` — two error systems.

### G. Pattern Matching on Sum Types (ML family, Elixir, Erlang)
```elixir
case File.read(path) do
  {:ok, content} -> parse(content)
  {:error, :enoent} -> {:error, "file not found: #{path}"}
end
```
**Pros**: Explicit; exhaustive matching enforced; natural in algebraic type systems.
**Cons**: Verbose when chaining many fallible operations; no short-circuit propagation.

### H. Zig's Error Union + `try` + `catch`
```zig
fn readFile(path: []const u8) ![]u8 {
    const file = try std.fs.openFile(path, .{});
    defer file.close();
    return try file.readAll();
}
```
**Pros**: `!T` return type is concise; `try` propagates; `catch` converts; `defer` handles cleanup; error sets are compile-time known; no heap allocation.
**Cons**: Error sets can be opaque; no error payloads (only an enum tag); limited to one error per call site without wrapping.

### I. Swift's `throws` + `try` + `do/catch`
```swift
func readConfig(_ path: String) throws -> Config {
    let data = try Data(contentsOf: URL(fileURLWithPath: path))
    return try JSONDecoder().decode(Config.self, from: data)
}
do {
    let config = try readConfig("app.json")
} catch {
    print("Failed: \(error)")
}
```
**Pros**: Clean syntax; `try` marks each fallible call explicitly; typed throws in Swift 6; automatic stack traces.
**Cons**: Pre-Swift 6 throws are untyped (just `Error`); performance cost of existential boxing.

### J. Gleam's `use` + Result
```gleam
pub fn parse_config(path: String) -> Result(Config, ConfigError) {
  use text <- result.try(file.read(path))
  use config <- result.try(toml.parse(text))
  Ok(config)
}
```
**Pros**: `use` is extremely clean — avoids callback hell while maintaining explicitness; no special syntax needed; just monadic bind as syntactic sugar.
**Cons**: Every step must be a `use` line; deeper nesting still possible.

---

## Analysis: What Fits Jade?

### Jade's Existing Strengths
1. **`err` types** — first-class error definitions, clean syntax
2. **`!` return** — minimal ceremony for error paths
3. **Pattern matching** — exhaustive handling of error variants
4. **`try`** — already exists for Option/Result propagation
5. **No exceptions** — aligned with Jade's deterministic philosophy
6. **English-like syntax** — errors-as-values reads naturally

### Design Constraints
- Must work with existing `err` definitions (not just Result/Option)
- Must maintain zero-cost abstraction (no hidden allocation)
- Must not introduce stack unwinding or hidden control flow
- Must compose well with pattern-directed functions
- Should feel like natural English, consistent with Jade's syntax philosophy

---

## Proposed Approaches

### Approach 1: Extend `try` to Custom Error Types

The simplest extension — make `try` work with any `err`-defined type, not just Option/Result.

```jade
err ParseError
    BadSyntax(String)
    UnexpectedEof

*parse_value(text as String) returns ParseError | i64
    if text.length equals 0
        ! UnexpectedEof
    parse_int(text)

*parse_config(path as String) returns ParseError | Config
    text is try read_file(path)    # propagates ParseError automatically
    value is try parse_value(text)
    Config(value is value)
```

**How it works**: `try expr` checks if the return value's tag matches any error variant. If so, early-returns it. Otherwise, extracts the non-error value.

**Pros**: Minimal syntax addition; extends existing `try`; familiar to Rust/Zig users.
**Cons**: Requires a convention for which variant is the "success" case; doesn't handle mixed error types.

### Approach 2: `raises` Annotation + Automatic Propagation

Declare what errors a function can produce, and let `try` auto-propagate matching types.

```jade
*read_file(path as String) raises IoError
    if not exists(path)
        ! FileNotFound(path)
    ...contents...

*parse_config(path as String) raises IoError, ParseError
    text is try read_file(path)         # IoError propagates
    config is try parse_json(text)      # ParseError propagates
    config
```

**How it works**: `raises` becomes part of the function signature in the type system. `try` checks if the current function's `raises` set includes the callee's error type and auto-propagates.

**Pros**: Self-documenting; compiler can verify exhaustive error handling; clean.
**Cons**: Viral like Java checked exceptions; more complex type system changes; needs `raises` inference to avoid annotation burden.

### Approach 3: Error Context / Wrapping with `try ... or`

Allow `try` with an error transformation clause.

```jade
*load_config(path as String) raises ConfigError
    text is try read_file(path) or ConfigError:ReadFailed(path)
    data is try parse_toml(text) or ConfigError:ParseFailed
    data
```

**How it works**: `try expr or ErrorVariant` — if `expr` returns an error, wraps it in the specified variant instead of propagating the original.

**Pros**: Adds context naturally; works with existing `err` types; composable.
**Cons**: Only handles one-to-one mapping; nested errors need a design for unwrapping.

### Approach 4: `defer` for Cleanup (Orthogonal, Pairs with Any Error Approach)

Borrowed from Go/Zig. Runs a statement when the current scope exits, regardless of how.

```jade
*process_file(path as String)
    fd is open(path)
    defer close(fd)
    # ... fd is closed when function returns, whether via ! or normally
```

**How it works**: `defer stmt` registers cleanup; compiler inserts the deferred call before every return point (normal and `!`).

**Pros**: Solves resource cleanup without `finally` blocks; composable; zero-cost (static insertion).
**Cons**: Execution order (LIFO) can surprise; captured values may have changed.

### Approach 5: `catch` Blocks (Local Error Handling)

Allow catching errors without full match, for recovery.

```jade
val is try parse_int(text) catch 0              # default on any error
val is try parse_int(text) catch BadSyntax ? 0  # catch specific variant
```

**How it works**: `try expr catch [pattern ?] fallback` — like Zig's `catch`, provides a local recovery path.

**Pros**: Concise for default-value patterns; avoids full match when you just want a fallback.
**Cons**: Risk of swallowing errors silently; catch-all is the Go `if err != nil` antipattern.

---

## Recommendation

**Phase A** (immediate, small): Extend `try` to work with custom `err` types — not just Option/Result. This is a typer-only change. The existing `lower_try` function already handles Option and Result; adding a third path for `err`-defined types is straightforward.

**Phase B** (medium-term): Add `defer` for cleanup. This is orthogonal to error propagation and valuable regardless. Requires compiler insertion of deferred calls before all return/error-return points.

**Phase C** (longer-term): Add `try ... catch` for local recovery and `try ... or` for error wrapping. These compose with Phase A and provide the context-addition pattern that's currently missing.

**Defer**: `raises` annotations. They add complexity for modest benefit. The Jade community can develop conventions (like Go's `error` interface) before committing to compiler-enforced error sets. The risk of Java-style checked-exception fatigue is high.

The key insight: **Jade already has 80% of a great error system.** The `err` + `!` + `match` + `try` combination is clean and principled. What's missing is `try` generality (Phase A), cleanup guarantees (Phase B), and error recovery sugar (Phase C). None of these require fundamental changes to the language's philosophy.
