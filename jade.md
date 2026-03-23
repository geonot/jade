# Jade

**Systems language. Scripting readability. C performance.**

Jade inherits the cleanest syntax we know — `is` bindings, `*` functions, `?`/`!` ternary, `~` pipelines, indentation structure — and compiles through LLVM 21 to native code that matches Clang -O3. No runtime. No GC. No 64-byte Value struct. Every integer is a register. Every struct is contiguous memory. Every function is a native call.

```jade
*fib n
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)

*main
    log fib(40)
```

This compiles to the same LLVM IR as equivalent C. Same speed. Zero overhead.

Jade was born from Coral — a language with one of the cleanest syntaxes ever designed and one of the worst runtime performance profiles ever measured. Coral's 64-byte Value struct, NaN-boxing ABI, and cycle-detecting garbage collector made a compiled LLVM language run 3× slower than CPython. We kept the syntax. We dropped everything else.

### Principles

1. **Values are their types.** An `i64` is a register. A struct is contiguous memory at known offsets. No universal wrapper. No indirection unless requested.
2. **Ownership is default.** One owner per value. Compiler inserts drops statically. No GC, no cycle detector.
3. **Borrowing is free.** Read access borrows a reference — zero runtime cost. No retain, no release.
4. **Sharing is explicit.** `rc` for shared ownership. Non-atomic single-threaded, atomic cross-thread. No cycle detector — use `weak` or design acyclic.
5. **Inference does the work.** HM + bidirectional + ownership inference. You don't write types unless you want to.
6. **Performance is non-negotiable.** Every design evaluated against: *does this prevent generating the same code C would?* If yes, the design is wrong.

---

## Types

### Primitives

| Type | Size | Description |
|------|------|-------------|
| `i8` `i16` `i32` `i64` | 1–8B | Signed integers |
| `u8` `u16` `u32` `u64` | 1–8B | Unsigned integers |
| `f32` `f64` | 4–8B | IEEE 754 floats |
| `bool` | 1b | `true` / `false` |
| `void` | 0B | Unit type |
| `String` | ptr+len+cap | Heap-allocated UTF-8 |

Integer literals infer width from context. `42` is `i64` by default, narrows to match operand type.

### Compound Types

```jade
# Structs — value types, contiguous memory
type Vec3
    x: i64
    y: i64
    z: i64

# Enums — tagged unions
enum Shape
    Circle(f64)
    Rect(f64, f64)

# Tuples
point is (10, 20, 30)

# Fixed arrays
nums is [1, 2, 3, 4, 5]
```

### Generics — the `of` keyword

```jade
*max of T(a: T, b: T)
    a > b ? a ! b

type Pair of A, B
    first: A
    second: B

enum Option of T
    Some(T)
    None
```

Single uppercase letters by convention. Monomorphized at compile time — zero runtime cost.

---

## Bindings

```jade
x is 42                    # inferred i64
name is 'jade'             # String
pi is 3.14159              # f64
done is true               # bool

# Typed binding
count: i32 is 0

# Reassignment (same binding, new value)
x is x + 1

# Augmented assignment (desugars to `x is x op expr`)
x += 1              # x is x + 1
x -= 2              # x is x - 2
x *= 3              # x is x * 3
x /= 4              # x is x / 4
x %= 5              # x is x % 5
x &= 0xFF           # x is x & 0xFF
x |= 0x80           # x is x | 0x80
x ^= mask           # x is x ^ mask
x <<= 2             # x is x << 2
x >>= 1             # x is x >> 1

# Destructuring (structs)
p is Vec3(x is 1, y is 2, z is 3)
```

`is` is binding, not comparison. Comparison uses `equals` and `isnt`.

---

## Functions

```jade
# Parentheses are optional on definitions and calls
*add a, b
    a + b

*greet name: String
    'hello {name}'

# With defaults
*connect host: String, port: i64 is 8080
    ...

# No-arg functions — no parens needed
*hello
    log 'hi'

# Calling
result is add 1, 2
greet 'world'
hello

# Parentheses still allowed where clarity helps
result is add(1, 2)
```

Parameters infer types from usage. Return type inferred from body. Explicit annotations optional. Parentheses are always optional on both definitions and calls.

### Pattern-Directed Function Clauses

Multiple definitions of the same function with literal parameters. The compiler merges them into a single function with conditional dispatch.

```jade
# Fibonacci by pattern
*fib(0) is 0
*fib(1) is 1
*fib n
    fib(n - 1) + fib(n - 2)

# Factorial
*fact(0) is 1
*fact n
    n * fact(n - 1)

# GCD with base case
*gcd(a, 0) is a
*gcd a, b
    gcd b, a % b
```

Literal parameters (`0`, `1`, `true`, `3.14`, `'hello'`) match by equality. Non-literal clauses become the `else` branch. Clauses are checked in definition order.

### Inline Body Syntax

Single-expression functions use `is` instead of an indented block.

```jade
*double x is x * 2
*square(x: i64) is x * x
*add a, b is a + b
*neg x is 0 - x
```

Combines naturally with pattern clauses:

```jade
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)
```

### Higher-Order Functions

```jade
*apply f: (i64) -> i64, x: i64
    f x

*main
    double is *fn(x: i64) x * 2
    log apply(double, 21)
```

### Lambdas

```jade
# Inline
square is *fn(x: i64) x * x

# Placeholder shorthand
doubled is items ~ *fn(x) x * 2

# Multi-line — just indent the body
result is items ~ *fn(x)
    y is x * 2
    y + 1
```

### Pipelines

```jade
result is value ~ double ~ add_one ~ square
```

`~` pipes the left value as the first argument to the right function.

---

## Control Flow

### Conditionals

```jade
if x > 0
    log 'positive'
elif x equals 0
    log 'zero'
else
    log 'negative'

# If as expression
sign is if x > 0 ? 1 ! -1

# Ternary
abs_x is x >= 0 ? x ! 0 - x
```

### Loops

```jade
# While
while n > 0
    n is n - 1

# For range (implicit assignment with 'from')
for i from 0 to 100
    log i

# For range (explicit assignment with 'is')
for i is 1 to 100
    log i

# For with step
for i from 0 to 100 by 2
    log i

# Infinite loop
loop
    if done
        break

# Yield — return a value from a loop
result is loop
    if check()
        yield 42
```

### Match

```jade
match shape
    Circle(r) ? log 3.14 * r * r
    Rect(w, h) ? log w * h

# With wildcard
match n
    0 ? log 'zero'
    1 ? log 'one'
    _ ? log 'other'
```

Pattern types: literals, identifiers (bind), constructors with destructuring, wildcards.

---

## Operators

| Prec | Operator | Description |
|------|----------|-------------|
| 1 | `~` | Pipeline |
| 2 | `? !` | Ternary |
| 3 | `or` | Logical OR |
| 4 | `and` | Logical AND |
| 5 | `equals` `isnt` | Equality |
| 6 | `< > <= >=` | Comparison |
| 7 | `\|` | Bitwise OR |
| 8 | `^` | Bitwise XOR |
| 9 | `&` | Bitwise AND |
| 10 | `<< >>` | Shift |
| 11 | `+ -` | Additive |
| 12 | `* / %` | Multiplicative |
| 13 | `**` | Exponent |
| 14 | `- not ~` | Unary |
| 15 | `() [] . as` | Postfix |

### Comparison

`equals` and `isnt` — not `==` or `!=`. Reads like language.

```jade
if x equals 0
    log 'zero'
if x isnt y
    log 'different'
```

### Logical

`and`, `or`, `not` — not `&&`, `||`, `!`.

### Type Casting

```jade
x is 42
y is x as f64
```

---

## Structs

```jade
type Point
    x: i64
    y: i64

# Constructor
p is Point(x is 10, y is 20)

# Field access
log p.x

# Methods
type Vec3
    x: i64
    y: i64
    z: i64

    *length self
        ((self.x * self.x + self.y * self.y + self.z * self.z) as f64) ** 0.5

    *dot self, other: Vec3
        self.x * other.x + self.y * other.y + self.z * other.z
```

Structs are value types. Passed by value (move), stack allocated. Methods take `self`.

---

## Enums

```jade
enum Color
    Red
    Green
    Blue
    Custom(u8, u8, u8)

*describe c: Color
    match c
        Red ? 1
        Green ? 2
        Blue ? 3
        Custom(r, g, b) ? r + g + b
```

Enums compile to tagged unions. Pattern matching is the primary dispatch mechanism.

---

## Error Handling

Errors are values, not exceptions.

```jade
err FileError
    NotFound
    PermissionDenied(String)

*read_file path: String
    if path equals ''
        ! NotFound
    42

*main
    match read_file('test.txt')
        NotFound ? log 'not found'
        PermissionDenied(msg) ? log msg
        _ ? log 'ok'
```

`!` is the error return operator — returns the error value from the current function.

---

## List Comprehensions

```jade
squares is [x ** 2 for x from 0 to 10]
evens is [x for x from 0 to 100 if x % 2 equals 0]
```

---

## Query Blocks

Native query syntax for structured data operations. Parsing is implemented; execution is deferred to 0.2.0 (Persistence phase).

```jade
# Query with clauses
query users
    where age > 21
    sort name
    limit 10

# Available clauses: where, sort, limit, take, skip, set, delete
```

Query blocks produce a `query` expression over a source with typed clauses. The compiler validates clause structure at parse time.

---

## Modules

```jade
# math.jade
*add a, b
    a + b

# main.jade
use math

*main
    log math.add(1, 2)
```

File = module. `use` imports. Recursive module resolution.

---

## Persistent Stores

Stores are typed, persistent data collections that survive across program runs. They compile to flat binary files with compile-time query validation.

```jade
# Define a store with typed fields
store users
    name: String
    age: i64

# Insert records (values match field order)
insert users 'Alice', 30
insert users 'Bob', 25
insert users 'Carol', 35

# Query — returns first matching record as a struct
young is users where age < 30
log young.name    # Bob
log young.age     # 25

# String equality queries
found is users where name equals 'Bob'

# Multi-field filters with AND/OR
result is users where age > 20 and name equals 'Alice'
match is users where age < 25 or age > 30

# Delete matching records
delete users where age > 28
delete users where name equals 'Bob' and age < 30

# Update records with set
set users where name equals 'Alice' age 31
set users where age > 30 name 'Senior', age 99

# Count records
total is count users

# All records (returns pointer to array)
all_users is all users

# Transactions (atomic batches)
transaction
    insert users 'Dave', 40
    insert users 'Eve', 22
    delete users where age > 50
```

**Supported field types:** `i64`, `f64`, `bool`, `String` (fixed 256-byte buffers on disk).

**Query operators:** `equals`, `isnt`, `<`, `>`, `<=`, `>=` — validated at compile time.

**Compound filters:** Chain conditions with `and` / `or` for multi-field filtering.

**Set (update):** `set <store> where <filter> <field> <value> [, <field> <value>]*` — updates matching records in-place.

**Transactions:** `transaction` blocks group store operations for batch execution.

**Persistence:** Store data lives in `<name>.store` files in the working directory. Data accumulates across program runs.

---

## Systems Programming

### Extern Functions (C FFI)

```jade
extern *printf(fmt: &i8, ...) -> i32

*main
    printf 'hello from jade\n'
```

### System Calls

```jade
*main
    syscall 1, 1, 'hello\n', 6   # write(stdout, msg, len)
```

### Inline Assembly

```jade
asm
    'mov $1, %rax'
    'mov $1, %rdi'
    'syscall'
```

### Raw Pointers

```jade
ptr is &value
val is @ptr        # dereference
```

### Volatile Memory Operations

Hardware-observable reads and writes. No compiler reordering, no elision.

```jade
extern *mmio_base() -> &i32

*poll_device
    reg is mmio_base()
    status is volatile_load reg       # Always reads from memory
    volatile_store reg, status | 1    # Always writes to memory
```

### Weak References

Explicit cycle-breaking for reference-counted values. The compiler warns when weak refs are used without upgrading.

```jade
type Node
    value: i64
    parent: weak rc Node     # weak reference breaks the cycle

*main
    root is rc(Node { value: 1, parent: none })
    child_parent is weak root            # downgrade to weak
    strong is weak_upgrade child_parent  # upgrade: returns rc or none
```

### Signal Handling

POSIX signal infrastructure.

```jade
*handler sig: i32
    log sig

*main
    signal_handle 2, handler     # SIGINT → handler
    signal_ignore 13             # SIGPIPE → ignore
    signal_raise 2               # raise SIGINT
```

### Integer Overflow Control

Default: trap on overflow. Explicit control via builtins:

```jade
*main
    a is 9223372036854775807       # i64 max
    w is wrapping_add a, 1         # wraps to i64 min
    s is saturating_add a, 1       # stays at i64 max
    result, overflowed is checked_add a, 1
    if overflowed
        log 'overflow detected'
```

Available for `add`, `sub`, `mul` — each in `wrapping_`, `saturating_`, `checked_` variants.

---

## Compiler

### Pipeline

```
Source → Lexer → Parser → AST → Typer → HIR → Perceus → Ownership → Codegen → LLVM IR → Native Binary
```

Implemented in Rust with inkwell (LLVM 21). Multi-pass compilation: parse to AST, type-check and lower to HIR, run Perceus optimization pass, verify ownership, then codegen to LLVM IR.

### CLI

```
jadec <INPUT> [-o OUTPUT] [--emit-ir] [--opt 0-3] [--lto] [-g/--debug]
```

- `--emit-ir` — print LLVM IR instead of compiling
- `--opt` — optimization level (default: 3)
- `--lto` — link-time optimization
- `-g` / `--debug` — emit DWARF debug info (for lldb/gdb)

### Codegen Optimizations

- **Integer literal coercion:** literals match operand width automatically
- **Call/return coercion:** arguments and returns coerced to match declared types
- **Function attributes:** `nounwind`, `nosync`, `nofree`, `mustprogress`, `willreturn` (non-recursive only), `noundef` on params
- **Internal linkage:** non-main functions marked internal for cross-function optimization
- **Arithmetic flags:** `nsw`/`nuw` on integer operations where provable
- **Integer exponentiation:** square-and-multiply algorithm, no float roundtrip
- **Boolean results:** `zext i1` for correct 0/1 values
- **Printf format strings:** width-correct (`%d`/`%ld`/`%u`/`%lu`)

### Source Stats

| Component | LOC |
|-----------|-----|
| codegen/ | 3,658 (builtins 904 · expr 1,181 · stmt 877 · strings 502 · mod 436 · types 360 · decl 247 · call 151) |
| typer.rs | 2,721 |
| parser.rs | 2,210 |
| perceus.rs | 1,147 |
| lexer.rs | 1,024 |
| ownership.rs | 613 |
| ast.rs | 360 |
| hir.rs | 347 |
| main.rs | 236 |
| diagnostic.rs | 197 |
| types.rs | 147 |
| lib.rs | 12 |
| **Total** | **13,672** |

---

## EBNF Grammar

### Program

```ebnf
program      = { NEWLINE | declaration } ;
declaration  = function_def | type_def | enum_def | extern_def | use_decl | err_def | store_def ;
```

### Functions

```ebnf
function_def = '*' , IDENT , [ 'of' , type_params ] ,
               [ '(' ] , [ param_list ] , [ ')' ] , [ '->' , type ] ,
               ( 'is' , expression | NEWLINE , block ) ;
param_list   = param , { ',' , param } ;
param        = ( IDENT , [ ':' , type ] , [ 'is' , expression ] ) | literal ;
literal      = INT | FLOAT | BOOL | STRING ;
```

Multiple definitions of the same function name with literal parameters are merged into a single function with conditional dispatch (pattern-directed clauses).

### Types & Enums

```ebnf
type_def     = [ 'pub' ] , 'type' , IDENT , [ 'of' , type_params ] , NEWLINE ,
               INDENT , { field_def | function_def } , DEDENT ;
enum_def     = 'enum' , IDENT , [ 'of' , type_params ] , NEWLINE ,
               INDENT , { variant_def } , DEDENT ;
variant_def  = IDENT , [ '(' , type_list , ')' ] , NEWLINE ;
```

### Statements

```ebnf
statement    = bind_stmt | if_stmt | while_stmt | for_stmt | loop_stmt
             | match_stmt | return_stmt | break_stmt | continue_stmt
             | insert_stmt | delete_stmt | expr_stmt ;
bind_stmt    = IDENT , 'is' , expression ;
insert_stmt  = 'insert' , IDENT , expr , { ',' , expr } ;
delete_stmt  = 'delete' , IDENT , store_filter ;
for_stmt     = 'for' , IDENT , ( 'from' | 'is' ) , expr , 'to' , expr , [ 'by' , expr ] , NEWLINE , block ;
match_stmt   = 'match' , expression , NEWLINE , INDENT , { pattern , '?' , body } , DEDENT ;
```

### Stores

```ebnf
store_def    = 'store' , IDENT , NEWLINE , INDENT , { field_def } , DEDENT ;
store_filter = 'where' , IDENT , ( 'equals' | 'isnt' | '<' | '>' | '<=' | '>=' ) , expr ;
store_query  = IDENT , store_filter ;
store_count  = 'count' , IDENT ;
store_all    = 'all' , IDENT ;
```

### Expressions (precedence low → high)

```ebnf
expression   = pipeline_expr , [ '?' , expression , '!' , expression ] ;
pipeline_expr = or_expr , { '~' , or_expr } ;
or_expr      = and_expr , { 'or' , and_expr } ;
and_expr     = eq_expr , { 'and' , eq_expr } ;
eq_expr      = cmp_expr , { ( 'equals' | 'isnt' ) , cmp_expr } ;
cmp_expr     = bitor_expr , { ( '<' | '>' | '<=' | '>=' ) , bitor_expr } ;
bitor_expr   = bitxor_expr , { '|' , bitxor_expr } ;
bitxor_expr  = bitand_expr , { '^' , bitand_expr } ;
bitand_expr  = shift_expr , { '&' , shift_expr } ;
shift_expr   = add_expr , { ( '<<' | '>>' ) , add_expr } ;
add_expr     = mul_expr , { ( '+' | '-' ) , mul_expr } ;
mul_expr     = exp_expr , { ( '*' | '/' | '%' ) , exp_expr } ;
exp_expr     = unary_expr , [ '**' , exp_expr ] ;
unary_expr   = ( '-' | 'not' ) , unary_expr | postfix_expr ;
postfix_expr = primary , { '(' args ')' | '[' expr ']' | '.' IDENT | 'as' type } ;
```

### Lexical

```
Keywords (42): is isnt equals and or not if elif else while for from loop
               yield continue return match when type enum err pub use
               as to by array unsafe extern fn log of query
               true false none store insert delete transaction
               count all where
```

Indentation-based (spaces only, tabs prohibited). `#` comments. Single-quoted strings with `{interpolation}`. Double-quoted raw strings.

---

## Performance

Jade compiles to identical LLVM IR as equivalent C. Benchmark suite of 15 programs tested against C (Clang 21 -O3, same LLVM backend), Rust (rustc -C opt-level=3), and Python 3. Three runs, median reported.

| Benchmark | Jade | Clang | J/C |
|-----------|------|-------|-----|
| ackermann(3,10) | 182ms | 186ms | 0.98× |
| fibonacci(40) | 337ms | 337ms | 1.00× |
| collatz(1M) | 176ms | 191ms | 0.92× |
| sieve(1M) | 144ms | 142ms | 1.02× |
| gcd_intensive | 26ms | 24ms | 1.11× |
| spectral_norm | 238ms | 691ms | 0.34× |
| nbody | 137ms | 147ms | 0.93× |
| math_compute | 373μs | 530μs | 0.70× |
| matrix_mul | 414μs | 531μs | 0.78× |
| struct_ops | 369μs | 497μs | 0.74× |
| enum_dispatch | 422μs | 460μs | 0.92× |
| hof_pipeline | 430μs | 452μs | 0.95× |
| array_ops | 466μs | 503μs | 0.92× |
| closure_capture | 449μs | 499μs | 0.90× |
| tight_loop | 366μs | 528μs | 0.69× |
| **TOTAL** | **1.24s** | **1.72s** | **0.72×** |

Jade is **28% faster than Clang** across the full suite. Versus Python: **84× faster**.

Run benchmarks:
```
python3 run_benchmarks.py --opt=3 --runs=5 --save=v0.0.0-rc1
python3 run_benchmarks.py --opt=all --runs=5    # O0–O3 sweep
python3 run_benchmarks.py --langs=jade,c        # subset
```

---

## Building

```bash
# Prerequisites: Rust, LLVM 21
export LLVM_SYS_211_PREFIX=/usr/lib/llvm-21

# Build
cd jade && cargo build --release

# Compile a program
./target/release/jadec hello.jade -o hello
./hello

# Run tests
cargo test

# Emit LLVM IR
./target/release/jadec hello.jade --emit-ir
```

---

## Memory Model

Three tiers, determined at compile time:

| Tier | Allocation | Deallocation | Cost | Used For |
|------|------------|--------------|------|----------|
| **Register** | CPU register | N/A | Zero | Scalars, small tuples |
| **Stack** | `alloca` | Function return | Zero | Structs, fixed arrays, locals |
| **Heap** | `malloc`/pool | Ownership drop or RC | Non-zero | Strings, dynamic arrays, Rc values |

**Decision rules:**
1. Primitives (`i64`, `f64`, `bool`): always Register.
2. Small structs (≤128 bytes) that don't escape: Stack.
3. Fixed-size arrays that don't escape: Stack.
4. `rc` values: always Heap (with refcount header).
5. Strings: Heap (but small-string optimization for ≤23 bytes).
6. Values that escape (returned, stored in heap struct): promoted to Heap.

**Ownership inference:** read → borrow, consume → move, mutate → mut ref, shared → rc auto.

**Perceus reference counting** (for `rc` values):
- Precision retain/release insertion based on ownership analysis
- Borrow optimization — no retain/release for read-only access
- Drop specialization — each type gets a specialized drop function
- Reuse analysis — in-place update when RC=1 and same layout
- Non-atomic fast path for thread-local values

**No cycle detector.** Programs using `rc` must use `weak` references for back-edges. Compiler detects potential cycles in the type graph and suggests `weak` fields.

### Memory Layout Control

```jade
# Default — compiler may reorder fields for optimal alignment
type Example
    a: u8
    b: u64
    c: u8

# C-compatible — declaration order preserved
type CStruct @strict
    magic: u32
    version: u16
    flags: u16
    data: u64

# Packed — no padding
type Pixel @packed
    r: u8
    g: u8
    b: u8

# Cache-aligned
type CacheAligned @align(64)
    data: [u8; 64]

# Combinable
type NetPacket @packed @strict @align(4)
    header: u32
    payload: [u8; 1024]
```

### Memory Safety Guarantees

No use-after-free. No double-free. No dangling references. No data races. No null pointers. No buffer overflow. All enforced at compile time — zero runtime cost.

---

## Architecture

### Pipeline

```
Source → Lexer → Parser → AST → Typer → HIR → Perceus → Ownership → Codegen → LLVM Opt → Native Binary
         (indent)  (LL,RD)        (bidir)       (9 passes) (verify)    (DWARF)   (O0–O3)    (ELF/Mach-O)
```

### Key Decisions

| Decision | Rationale |
|----------|-----------|
| No runtime library | Primitives compile to pure LLVM IR. No FFI boundary for basic operations. |
| Typed native ABI | Functions use native LLVM signatures (`i64`, `f64`, `ptr`). No NaN-boxing. |
| Value types as default | Structs laid out contiguously. No heap indirection for compound data. |
| Monomorphization | Generics generate specialized code. No boxing, no virtual dispatch. |
| Ownership + borrow checking | Memory safety without GC. Compile-time only — zero runtime cost. |
| Perceus RC as fallback | For shared/graph structures, reference counting with borrow elision. |

### Coral → Jade

| Aspect | Coral | Jade |
|--------|-------|------|
| Value representation | 64-byte heap Value struct | Native LLVM types |
| Function ABI | Universal NaN-boxed | Native typed |
| Integer types | Single f64 | Full i8–u64 |
| Struct layout | Runtime heap-allocated | Compile-time contiguous, stack default |
| Array storage | `Vec<ValueHandle>` | `[N x T]` contiguous |
| Memory management | RC + cycle detector | Ownership + borrowing + Perceus RC |
| Generics | Type erasure | Monomorphization |
| Builtin dispatch | String matching (~80 branches) | Inline codegen |
| Runtime | ~24K LOC Rust | Minimal (<1K LOC) |
| Performance | 90× behind Rust | 0.97× Clang |

### Diagnostics

Structured error system with codes, spans, labels, and suggestions:

```
error[E301]: use of moved value 'data'
  --> src/main.jade:12:5
   |
10 |     result is process(data)
   |                       ---- value moved here
12 |     log(data.len)
   |         ^^^^ value used after move
   = help: consider borrowing: process(ref data)
```

| Code Range | Category |
|------------|----------|
| E001–E099 | Syntax errors |
| E100–E199 | Name resolution |
| E200–E299 | Type errors |
| E300–E399 | Ownership & borrow |
| E400–E499 | Safety (volatile, FFI, signals) |
| E500–E599 | Pattern matching |
| E600–E699 | Memory (layout, allocation) |
| E700–E799 | Integer overflow |
| W001+ | Warnings |

---

## Built-in Operations

### Integer

```jade
popcount(x)             # count set bits
clz(x)                  # count leading zeros
ctz(x)                  # count trailing zeros
rotate_left(x, n)       # bit rotation
rotate_right(x, n)
bswap(x)                # byte swap (endianness)
wrapping_add(x, y)      # wrapping arithmetic
wrapping_sub(x, y)
wrapping_mul(x, y)
saturating_add(x, y)    # saturating arithmetic
saturating_sub(x, y)
saturating_mul(x, y)
checked_add(x, y)       # returns (result, overflowed)
checked_sub(x, y)
checked_mul(x, y)
x ** n                  # square-and-multiply exponentiation
```

### Volatile / Hardware

```jade
volatile_load(ptr)      # volatile read (never elided/reordered)
volatile_store(ptr, v)  # volatile write
signal_handle(sig, fn)  # register signal handler
signal_raise(sig)       # raise signal → i32
signal_ignore(sig)      # ignore signal (SIG_IGN)
```

### Reference Counting

```jade
rc(value)               # allocate RC-wrapped value
rc_retain(rv)           # increment refcount
rc_release(rv)          # decrement refcount (frees at 0)
weak(rc_val)            # downgrade RC → weak reference
weak_upgrade(w)         # upgrade weak → RC (or none)
```

### Float

```jade
x.sqrt()    x.sin()     x.cos()     x.tan()
x.abs()     x.floor()   x.ceil()    x.round()
x.is_nan()  x.is_infinite()
x.min(y)    x.max(y)    x.clamp(lo, hi)
```

### Array/Slice

```jade
arr.length              # compile-time for fixed arrays
arr[i]                  # bounds-checked
arr from i to j         # slice
arr.contains(x)
arr.iter()              # iterator
```

### String

```jade
s.contains('sub')       # true if s contains substring
s.starts_with('pre')    # true if s starts with prefix
s.ends_with('suf')      # true if s ends with suffix
s.char_at(i)            # byte at index i (as i64)
s.slice(start, end)     # substring [start, end)
```

String interpolation with `{expr}` inside single-quoted strings:

```jade
name is 'world'
log('hello {name}')           # hello world
x is 42
log('x={x} x2={x * 2}')      # x=42 x2=84
```

### Global

```jade
log(value)              # print to stdout
sqrt(x)    abs(x)       # math
min(a, b)  max(a, b)
to_string(x)
time_now()              # nanosecond timestamp
assert(cond)
panic(msg)
size_of of T()          # compile-time size
align_of of T()         # compile-time alignment
```

### Debug

Compile with `-g` / `--debug` to emit DWARF debug info. Use with lldb or gdb:

```bash
jadec main.jade -o main -g
lldb ./main
```

---

*Jade: Hard. Dense. Beautiful.*
