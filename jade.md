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
    x as i64
    y as i64
    z as i64

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
*max of T(a as T, b as T)
    a > b ? a ! b

type Pair of A, B
    first as A
    second as B

enum Option of T
    Some(T)
    None
```

Single uppercase letters by convention. Monomorphized at compile time — zero runtime cost.

### Type Aliases & Newtypes

```jade
# Alias — transparent, interchangeable with the underlying type
alias Seconds is f64
alias UserId is i64

# Newtype — opaque, distinct type at compile time
type Celsius
    value as f64

type Fahrenheit
    value as f64
# Celsius and Fahrenheit are NOT interchangeable even though both wrap f64
```

---

## Bindings

```jade
x is 42                    # inferred i64
name is 'jade'             # String
pi is 3.14159              # f64
done is true               # bool

# Typed binding
count as i32 is 0

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

`is` is binding, not comparison. Comparison uses `equals` and `neq`. `eq` is shorthand for `equals`.

### ALL_CAPS Constants

Top-level constants use `ALL_CAPS` by convention. The formatter enforces this.

```jade
MAX_SIZE is 1024
PI is 3.14159265
DEFAULT_PORT is 8080
```
The same convention applies inside types, where `FIELD is value` provides a default:
```jade
type Foo
    BAR is 10
    BAZ as i64

x is Foo(BAZ is 5)
log x.BAR    # 10 (default)
log x.BAZ    # 5
```
**Note:** Default-valued fields are not yet enforced as immutable. `x.BAR is 20` will compile and overwrite the default. Compile-time immutability enforcement for ALL_CAPS fields is planned.
---

## Functions

```jade
# Parentheses are optional on definitions and calls
*add a, b
    a + b

*greet name as String
    'hello {name}'

# With defaults
*connect(host as String, port as i64 is 8080)
    ...
```

> **Note:** Default parameter values are parsed but the HIR validator does not yet account for them — calling a function with fewer arguments than declared (relying on defaults) produces a validation error. Provide all arguments explicitly for now.

```jade
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

### The `of` Call Syntax

`of` can be used as an alternative to parentheses for single-argument calls:

```jade
*double x is x * 2

result is double of 5      # same as double(5)
```

`of` after a user-defined function name treats the next expression as its argument. Does not work with builtins like `log`.

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
*square(x as i64) is x * x
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
*apply(f as (i64) returns i64, x as i64)
    f(x)

*main
    double is *fn(x as i64) x * 2
    log apply(double, 21)    # 42
```

Function-typed parameters use the form `f as (ParamTypes) returns RetType`. Parentheses on the function definition are required when using `as` type annotations.

### Lambdas

```jade
# Inline
square is *fn(x as i64) x * x

# Placeholder shorthand
doubled is items ~ *fn(x) x * 2

# Multi-line — just indent the body
result is items ~ *fn(x)
    y is x * 2
    y + 1
```

The `*fn(params) body` form is required — `fn` is not optional.

### Pipelines

```jade
result is value ~ double ~ add_one ~ square
```

`~` pipes the left value as the first argument to the right function.

### Named Arguments

```jade
*connect(host as String, port as i64 is 8080)
    log 'connecting to {host}:{port}'

connect(host is 'localhost', port is 3000)
```

Parentheses are required when using `as` type annotations on parameters.

### `$` Placeholder

Placeholder for partial application in pipelines. In pipeline context, `$ expr` desugars to an implicit lambda at parse time:

```jade
# In named calls (pipeline + call with $ in args)
result is value ~ add(5, $)       # → add(5, value)

# Numbered: $0, $1, $2 for multi-arg
pairs ~ combine($0, $1)
```

> **Status**: The `$` placeholder in bare expressions (`nums ~ $ * 2`) parses and creates an implicit lambda, but does not compile — codegen fails with "cannot call non-function type". Use explicit lambdas instead: `nums.map(*fn(x) x * 2)` or define a named function and use method syntax.

---

## Control Flow

### Ternary — `? !`

The preferred conditional expression. `condition ? then ! else`.

```jade
# Basic
sign is x > 0 ? 1 ! -1

# Absolute value
abs_x is x >= 0 ? x ! 0 - x

# Nested ternary (right-associative)
grade is score > 90 ? 'A' ! score > 80 ? 'B' ! score > 70 ? 'C' ! 'F'

# In function calls
log x > 0 ? 'positive' ! 'non-positive'

# Assigning different types (branches must unify)
result is ready ? compute() ! fallback()
```

Ternary binds looser than pipelines — `value ~ transform ? check ! default` works as expected.

### Conditionals

```jade
if x > 0
    log 'positive'
elif x equals 0
    log 'zero'
else
    log 'negative'

# If as expression — use ternary
sign is x > 0 ? 1 ! -1
```

### Loops

```jade
# While
while n > 0
    n is n - 1

# For range (with 'from')
for i from 0 to 100
    log i

# For range (with 'in')
for i in 1 to 100
    log i

# For with step
for i from 0 to 100 by 2
    log i

# Infinite loop
loop
    if done
        break

# Labeled loops — binding name IS the label
outer is for i from 0 to 10
    for j from 0 to 10
        if i * j > 50
            break outer

# Parallel loop (work-stealing, all iterations must be independent)
sim for x in items
    process(x)

# Range slicing
sub is items from 2 to 5    # elements at indices 2, 3, 4
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
| 4 | `xor` | Logical XOR |
| 5 | `and` | Logical AND |
| 6 | `equals` `eq` `neq` | Equality |
| 7 | `< > <= >=` `in` | Comparison / membership |
| 8 | `\|` | Bitwise OR |
| 9 | `^` | Bitwise XOR |
| 10 | `&` | Bitwise AND |
| 11 | `<< >>` | Shift |
| 12 | `+ -` | Additive |
| 13 | `* / % mod` | Multiplicative |
| 14 | `**` | Exponent |
| 15 | `- not` | Unary |
| 16 | `() [] . as` | Postfix |

### Comparison

`equals` (shorthand `eq`) and `neq` — not `==` or `!=`. Reads like language.

```jade
if x equals 0
    log 'zero'
if x eq 0
    log 'also zero'
if x neq y
    log 'different'
```

### Comparison Chaining

Math-style chained comparisons without double-evaluating the middle operand:

```jade
if 0 < x < 100
    log 'in range'
if a <= b <= c
    log 'sorted'
```

### Membership — `in`

```jade
if x in [1, 2, 3]
    log 'found'
if key in my_map
    log 'exists'
if 'world' in greeting
    log 'found substring'
```

Desugars to `.contains()` on the collection. Works with:
- **Vec/Array:** linear scan for element equality
- **String:** substring search
- **Map:** key lookup (equivalent to `.has()`)
- **Set:** membership check

**Note:** `in` on array literals (e.g., `x in [1, 2, 3]`) may fail at codegen — use a `vec()` instead.

### Logical

`and`, `or`, `not`, `xor` — not `&&`, `||`, `!`.

```jade
if a xor b
    log 'exactly one is true'
```

### Type Casting

```jade
x is 42
y is x as f64           # widening — always safe
z is big as strict i16   # strict narrowing — panics if value doesn't fit
w is big as i16          # truncating — silently truncates (compiler warning)
```

### Serialization Casts

```jade
data is my_struct as json    # serialize struct to JSON string
config is json_str as Config # deserialize JSON string to struct
m is my_struct as map        # convert struct to Map
```

**Status:** Serialization casts parse but are not yet implemented — they currently return an empty string. Full serialization requires runtime support.

---

## Structs

```jade
type Point
    x as i64
    y as i64

# Constructor
p is Point(x is 10, y is 20)

# Field access
log p.x

# Methods
type Vec3
    x as i64
    y as i64
    z as i64

    *length(self)
        ((self.x * self.x + self.y * self.y + self.z * self.z) as f64) ** 0.5

    *dot(self, other as Vec3)
        self.x * other.x + self.y * other.y + self.z * other.z
```

Structs are value types. Passed by value (move), stack allocated. Methods take `self` explicitly, or omit it and access fields by name directly — `self` is injected by the compiler.

```jade
type Vec3
    x as i64
    y as i64
    z as i64

    # Explicit self
    *dot(self, other as Vec3)
        self.x * other.x + self.y * other.y + self.z * other.z

    # Implicit self — fields resolve to self.field automatically
    *sum()
        x + y + z
```

---

## Enums

```jade
enum Color
    Red
    Green
    Blue
    Custom(u8, u8, u8)

*describe c as Color
    match c
        Red ? 1
        Green ? 2
        Blue ? 3
        Custom(r, g, b) ? r + g + b
```

Enums compile to tagged unions. Pattern matching is the primary dispatch mechanism.

### Enum Discriminant Values

Explicit discriminant values for C interop and bitflags:

```jade
enum Permission
    Read is 1
    Write is 2
    Execute is 4

enum HttpStatus
    Ok is 200
    NotFound is 404
    ServerError is 500
```

---

## Error Handling

Errors are values, not exceptions.

```jade
err FileError
    NotFound
    PermissionDenied(String)

*read_file(path as String)
    if path equals ''
        ! NotFound
    'file contents here'

*main
    match read_file('test.txt')
        NotFound ? log 'not found'
        PermissionDenied(msg) ? log msg
        _ ? log 'ok'
```

`!` is the error return operator — returns the error value from the current function. The non-error path returns normally (here, the string on the last line).

---

## List Comprehensions

```jade
squares is [x ** 2 for x in 0 to 10]
evens is [x for x in 0 to 100 if x % 2 eq 0]
```

Syntax: `[expr for bind in start to end]` or `[expr for bind in start to end if cond]`. Produces a `Vec`.

---

## Iterator Combinators

Vec methods for functional data transformation. Chain with `.method()` syntax or `~` pipelines with named functions.

```jade
*double(x as i64) returns i64 is x * 2
*big(x as i64) returns bool is x > 10

# map, filter as method chains
doubled is nums.map(double)
result is nums.map(double).filter(big)

# fold
total is nums.fold(0, *fn(acc, x) acc + x)

# zip, take, skip
pairs is a.zip(b).take(5)

# any, all, find
has_neg is nums.any(*fn(x) x < 0)
found is items.find(*fn(x) x eq target)

# chain, flatten
combined is a.chain(b)
flat is nested.flatten()
```

Available methods: `map`, `filter`, `fold`, `any`, `all`, `find`, `zip`, `take`, `skip`, `chain`, `flatten`, `enumerate`, `reverse`, `sort`, `sum`, `count`, `contains`, `join`, `collect`.

---

## Generators (Lazy Sequences)

A function containing `yield` is automatically a generator. Calling it returns a lazy sequence.

```jade
*fibonacci()
    a is 0
    b is 1
    loop
        yield a
        temp is a
        a is b
        b is temp + b

*main()
    gen is fibonacci()
    log gen.next()     # 0
    log gen.next()     # 1
    log gen.next()     # 1
    log gen.next()     # 2
```

**Status:** Generators are parsed and typed. The MIR codegen path has coroutine infrastructure (context switching, suspend/resume) but end-to-end `.next()` dispatch on generator return values is not yet working. For-in iteration over generators is supported in the MIR path.

---

## Collections

### Vec (dynamic array)

```jade
v is vec()
v.push(1)
v.push(2)
log v.length      # 2
log v.pop()       # 2
```

### Map (hash map)

```jade
m is map()
m.set('key', 42)
log m.get('key')   # 42
log m.has('key')   # true
```

### Set

```jade
s is set(1, 2, 3)
s.add(4)
s.add(1)           # no-op, already present
log s.contains(1)  # true
log s.len()        # 4
s.remove(1)

# Set operations
a_set is set(1, 2, 3)
b_set is set(2, 3, 4)
u is a_set.union(b_set)
d is a_set.difference(b_set)
i is a_set.intersection(b_set)
```

**Note:** `set` is a parser keyword (used for store updates). Creating sets requires the call form `set(...)` — bare `set()` without arguments may conflict with the store `set` statement. Initialize with values: `set(1, 2, 3)`.

### Priority Queue

```jade
pq is priority_queue()
pq.push('urgent', 10)     # value, priority (higher = first out)
pq.push('normal', 1)
pq.push('critical', 100)

log pq.peek()              # 'critical'
log pq.pop()               # 'critical'
log pq.len()               # 2
log pq.is_empty()          # false
pq.clear()
```

### Deque (Double-Ended Queue)

```jade
d is deque()
d.push_back(1)
d.push_back(2)
d.push_front(0)
log d.pop_front()          # 0
log d.pop_back()           # 2
log d.len()                # 1
```

**Status:** Deque is parsed and typed but codegen is not yet implemented. Runtime support exists in `runtime/deque.c`.

### Allocator-Aware Collections

Collections can optionally use a custom allocator:

```jade
scratch is Arena(4096)
v is vec_with_alloc(scratch)
m is map_with_alloc(scratch)
# All allocations go to the arena; freed in one shot at scope exit
```

---

## Regex

String methods for pattern matching via PCRE2:

```jade
# String methods
log 'hello123'.matches('[0-9]+')         # true
results is 'a1b2c3'.find_all('[0-9]+')   # ['1', '2', '3']
cleaned is 'foo  bar'.replace_re('\\s+', ' ')  # 'foo bar'
```

> **Status**: String regex methods (`matches`, `find_all`, `replace_re`) are parsed and dispatched in codegen, but currently fail with a type mismatch (String value vs ptr parameter). The `regex.compile` / `regex.is_match` stdlib API shown in earlier docs does not exist — regex is purely string-method-based. Requires runtime PCRE2 linkage.

---

## Query Blocks

Native query syntax for structured data operations. Store queries are operational; general query blocks are parsed but execution is deferred.

```jade
# Query with clauses
query users
    where age > 21
    sort name
    limit 10

# Available clauses: where, sort, limit, take, skip, set, delete
```

Query blocks produce a `query` expression over a source with typed clauses. The compiler validates clause structure at parse time. Store-specific queries (using persistent stores) are fully implemented.

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

### Selective Imports

```jade
use math [sin, cos, pi]      # import only specific symbols
log sin(pi)
```

### Import Aliases

```jade
use long_module_name as lmn
lmn.do_thing()
```

### Scope-Limited Imports

`use` inside a function body limits visibility to that scope:

```jade
*compute()
    use math [sin, cos]
    log sin(3.14)          # sin/cos available here

*main()
    # sin and cos are NOT visible here
    log 'done'
```

---

## Persistent Stores

Stores are typed, persistent data collections that survive across program runs. They compile to flat binary files with compile-time query validation.

```jade
# Define a store with typed fields
store users
    name as String
    age as i64

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
extern *printf(fmt as %i8, ...) returns i32

*main
    printf 'hello from jade\n'
```

### System Calls

```jade
*main
    syscall 1, 1, 'hello\n', 6   # write(stdout, msg, len)
```

**Status:** The `syscall` AST node exists but is not reachable from source code — the parser does not recognize `syscall` as a keyword or builtin function. Use `extern` declarations for system-level calls instead.

### Inline Assembly

```jade
asm
    nop
```

Assembly lines are bare instructions (no quotes). The parser collects indented lines as raw assembly text and emits them via LLVM inline asm.

### Raw Pointers

```jade
ptr is %value
val is @ptr        # dereference
```

### Volatile Memory Operations

Hardware-observable reads and writes. No compiler reordering, no elision.

```jade
*poll_device
    x is 0
    ptr is %x
    volatile_store(ptr, 99)
    v is volatile_load(ptr)      # Always reads from memory
    log v                        # 99
```

### Weak References

Explicit cycle-breaking for reference-counted values. The compiler warns when weak refs are used without upgrading.

```jade
type Node
    value as i64
    parent as weak rc Node     # weak reference breaks the cycle

*main
    root is rc(Node(value is 1, parent is none))
    child_parent is weak root            # downgrade to weak
    strong is weak_upgrade child_parent  # upgrade: returns rc or none
```

**Status:** `weak` is a reserved keyword and `Type::Weak` exists in the type system, but `weak` is not yet parsed as an expression or type modifier. The `weak()` and `weak_upgrade()` function calls are not registered as builtins. Use `rc()` for now; weak references are planned.

### Copy-on-Write (COW)

Strings and vectors use copy-on-write when reference count > 1. Shared reads are zero-copy; mutation transparently clones on first write.

```jade
a is 'hello'
b is a              # shared — no copy
b is b + ' world'   # COW triggers: b gets its own copy
```

**Status:** COW types exist in the type system but codegen is not yet implemented.

### Signal Handling

POSIX signal infrastructure.

```jade
*handler(sig as i32)
    log sig

*main
    signal_handle 2, handler     # SIGINT → handler
    signal_ignore 13             # SIGPIPE → ignore
    signal_raise 2               # raise SIGINT
```

**Status:** Signal builtins are parsed and recognized, but `signal_handle` has a codegen type mismatch (passes `i64` signal number where libc expects `i32`). `signal_ignore` compiles but has the same issue. Planned fix.

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

> **Note:** Integer overflow builtins work with the default HIR codegen backend. The MIR codegen backend does not yet support them.

### Atomic Operations

Lock-free atomic instructions for concurrent programming:

```jade
counter is 0
atomic_add(%counter, 1)           # atomic increment
val is atomic_load(%counter)      # atomic read
atomic_store(%counter, 0)         # atomic write
old, ok is atomic_cas(%counter, 0, 1)  # compare-and-swap
```

**Status:** Atomic operations exist in the HIR and codegen, but are not registered as builtins — calling them from source code produces "undefined function." Wiring them up in the typer's builtin table is required.

### Arena / Region Allocation

Scope-based bulk allocation — the entire arena is freed in one shot at scope exit:

```jade
scratch is Arena(4096)
# All allocations in this scope use the arena
v is vec_with_alloc(scratch)
m is map_with_alloc(scratch)
# Arena freed when scratch goes out of scope
```

### Constant-Time Operations

Prevent timing side-channel attacks:

```jade
if constant_time_eq(user_token, expected_token)
    log 'authorized'
```

For integer args, compiles to XOR + compare. For string args, calls a constant-time comparison runtime function.

### C Header Import

Generate Jade extern declarations from C headers automatically:

```bash
jade bind /usr/include/sqlite3.h > std/sqlite.jade
```

Parses function declarations, structs, typedefs and generates corresponding Jade `extern` declarations with correct type mappings.

---

## Concurrency

### Actors

```jade
actor Counter
    count is 0

    @increment amount
        count is count + amount

    @get_count
        count

*main
    c is spawn Counter
    send c, @increment(5)
    send c, @increment(3)
```

Actor handlers use `@name` syntax. Fields are defined in the actor body. Messages are sent with `send target, @handler(args)`. Actors run on a cooperative work-stealing scheduler. Message sends are non-blocking.

### Supervisor Trees

Erlang/OTP-style supervision for fault-tolerant actor hierarchies:

```jade
supervisor my_system
    strategy one_for_one    # restart only the failed child
    children
        spawn Worker('task-a')
        spawn Worker('task-b')
        spawn Logger
```

Strategies: `one_for_one`, `one_for_all`, `rest_for_one`.

**Status:** Parsed but not yet compiled. Supervisor definitions are accepted by the parser but skipped during type checking and codegen.

### Channels

```jade
ch is channel of i64(10)     # buffered channel, capacity 10
send ch, 42                  # send value
val is receive ch            # receive value
close ch                     # close channel
```

### Select

```jade
select
    receive ch1 as val
        log 'got {val} from ch1'
    receive ch2 as val
        log 'got {val} from ch2'
    default
        log 'no messages'
```

---

## Numeric Computing

### Multi-Dimensional Arrays

```jade
# 3×3 matrix (created with the `by` keyword)
m is 3 by 3

# Access
log m[1][2]

# Element-wise arithmetic (broadcasting)
a is 3 by 3
b is 3 by 3
c is a + b       # element-wise add
d is a * 2.0     # scalar broadcast
```

The `by` keyword creates an NDArray. `3 by 3` produces a 3×3 matrix of f64 zeros.

### Matrix Multiplication

```jade
result is matmul(A, B)    # A and B are 2D NDArrays
```

### SIMD Intrinsics

Expose LLVM vector types at the language level:

```jade
# 4-lane f32 SIMD vector
v is SIMD of f32, 4(1.0, 2.0, 3.0, 4.0)
w is SIMD of f32, 4(5.0, 6.0, 7.0, 8.0)

# Arithmetic operates on all lanes in parallel
sum is v + w     # (6.0, 8.0, 10.0, 12.0)
prod is v * w    # (5.0, 12.0, 21.0, 32.0)
```

**Status:** SIMD types exist in the type system but do not yet generate SIMD-specific LLVM IR.

### Einsum Notation

Einstein summation for tensor contractions:

```jade
# Matrix multiplication: C[i,k] = sum_j A[i,j] * B[j,k]
C is einsum 'ij,jk->ik', A, B

# Trace of a matrix: sum of diagonal
t is einsum 'ii->', M

# Dot product
d is einsum 'i,i->', u, v
```

**Status:** Parsed and typed but codegen is not yet implemented.

### Automatic Differentiation

Source-to-source AD via `grad`:

```jade
*loss(x as f64) returns f64
    x ** 2 + 3.0 * x + 1.0

*main()
    dloss is grad(loss)    # returns a function: f64 returns f64
    log dloss(2.0)         # 7.0  (derivative: 2x + 3)
```

**Status:** Parsed and typed but codegen is not yet implemented.

---

## DSL Builder Blocks

Builder-pattern blocks for domain-specific construction:

```jade
page is build HtmlElement
    tag is 'div'
    class is 'container'
    id is 'main'

config is build ServerConfig
    port is 8080
    host is 'localhost'
    workers is 4
```

**Status:** Parsed and typed. The MIR codegen path desugars builder blocks to struct construction. The HIR codegen path has not yet implemented builder blocks.

---

## Compile-Time Evaluation

### Comptime Reflection

Introspect types at compile time:

```jade
fields is fields_of('Point')    # field names (array)
t is type_of(42)                # type as string
s is size_of('Point')           # byte size
```

**Status:** Comptime reflection builtins compile and run but produce incorrect results — `size_of` returns 0 for primitives, `type_of` returns garbled output, `fields_of` returns a raw pointer. These are placeholders awaiting proper implementation.

### Extended Comptime Inference

Pure functions with constant arguments are evaluated at compile time automatically — no keyword needed:

```jade
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)

x is fib(10)    # computed at compile time → 55
```

The compiler detects pure functions (no side effects) and evaluates them when all arguments are constants. Recursion depth limited to 100.

### Rich Assert Messages

The compiler auto-generates descriptive failure messages:

```jade
assert x > 0 and x < 100
# On failure: "assertion failed: x > 0 and x < 100 where x = -5"
```

---

## Compiler

### Pipeline

```
Source → Lexer → Parser → AST → Typer → HIR → Perceus → Ownership → Codegen → LLVM IR → Native Binary
```

Implemented in Rust with inkwell (LLVM 21). Multi-pass compilation: parse to AST, type-check and lower to HIR, run Perceus optimization pass, verify ownership, then codegen to LLVM IR.

### CLI

```
jadec <INPUT> [-o OUTPUT] [--emit-ir] [--emit-llvm] [--emit-hir] [--emit-mir] [--emit-obj] [--opt 0-3] [--lto] [-g/--debug] [--mir-codegen] [--fast-math] [--deterministic-fp] [--incremental] [--threads N]
```

Subcommands:

```bash
jade init [name]           # create new project with project.jade
jade build [-o out] [--opt N] [--lto]  # compile the project
jade run [-- args]         # compile and run
jade test                  # run project tests
jade check                 # type-check without codegen
jade fmt [files]           # format Jade source files
jade fetch                 # download dependencies
jade update                # update dependency lock file
jade bind header.h         # generate extern declarations from C header
```

- `--emit-ir` — print LLVM IR instead of compiling
- `--emit-llvm` — print LLVM IR (alias)
- `--emit-hir` — print HIR (typed intermediate representation)
- `--emit-mir` — print MIR (mid-level IR)
- `--emit-obj` — emit object file only
- `--opt` — optimization level (default: 3)
- `--lto` — link-time optimization
- `-g` / `--debug` — emit DWARF debug info (for lldb/gdb)
- `--mir-codegen` — use MIR-based backend instead of HIR-based
- `--fast-math` — enable fast-math optimizations (nnan, ninf, nsz, arcp, contract, afn, reassoc)
- `--deterministic-fp` — guarantee deterministic floating-point results
- `--incremental` — cache unchanged function artifacts
- `--threads N` — parallel codegen threads (0 = auto-detect)

### Codegen Optimizations

- **Integer literal coercion:** literals match operand width automatically
- **Call/return coercion:** arguments and returns coerced to match declared types
- **Function attributes:** `nounwind`, `nosync`, `nofree`, `mustprogress`, `willreturn` (non-recursive only), `noundef` on params
- **Internal linkage:** non-main functions marked internal for cross-function optimization
- **Arithmetic flags:** `nsw`/`nuw` on integer operations where provable
- **Integer exponentiation:** square-and-multiply algorithm, no float roundtrip
- **Boolean results:** `zext i1` for correct 0/1 values
- **Printf format strings:** width-correct (`%d`/`%ld`/`%u`/`%lu`)

---

## Performance

Jade compiles to identical LLVM IR as equivalent C. Benchmark suite tested against C (Clang 21 -O3, same LLVM backend). Five runs, median reported.

| Benchmark | Jade | Clang | J/C |
|-----------|------|-------|-----|
| ackermann(3,10) | 186ms | 202ms | 0.92× |
| fibonacci(40) | 339ms | 336ms | 1.01× |
| collatz(1M) | 169ms | 172ms | 0.99× |
| sieve(1M) | 142ms | 142ms | 1.00× |
| gcd_intensive | 24ms | 24ms | 0.99× |
| spectral_norm | 209ms | 232ms | 0.90× |
| nbody | 136ms | 136ms | 0.99× |
| math_compute | 380μs | 580μs | 0.66× |
| matrix_mul | 370μs | 460μs | 0.80× |
| struct_ops | 430μs | 410μs | 1.05× |
| enum_dispatch | 380μs | 450μs | 0.84× |
| array_ops | 390μs | 470μs | 0.83× |
| closure_capture | 380μs | 450μs | 0.84× |
| tight_loop | 390μs | 450μs | 0.87× |
| **TOTAL** | **1.21s** | **1.25s** | **0.97×** |

Jade matches Clang across the full compute suite — **0.97× C performance**.

Run benchmarks:
```
python3 run_benchmarks.py --opt=3 --runs=5 --save=v0.5.0
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
    a as u8
    b as u64
    c as u8

# C-compatible — declaration order preserved
type CStruct @strict
    magic as u32
    version as u16
    flags as u16
    data as u64

# Packed — no padding
type Pixel @packed
    r as u8
    g as u8
    b as u8

# Cache-aligned
type CacheAligned @align(64)
    data as [u8; 64]

# Combinable
type NetPacket @packed @strict @align(4)
    header as u32
    payload as [u8; 1024]
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
```

`weak()` and `weak_upgrade()` are planned but not yet callable (see Weak References section).

### Float

```jade
x.sqrt()    x.sin()     x.cos()     x.tan()
x.abs()     x.floor()   x.ceil()    x.round()
x.is_nan()  x.is_infinite()  x.is_finite()
x.min(y)    x.max(y)
```

### Array/Slice

```jade
arr.length              # Vec length (property access)
arr.len()               # Vec length (method call)
arr[i]                  # bounds-checked
arr from i to j         # slice (Vec only — links but runtime not yet wired)
arr.contains(x)
arr.join(sep)           # join elements with separator string
```

### String

```jade
s.contains('sub')       # true if s contains substring
s.starts_with('pre')    # true if s starts with prefix
s.ends_with('suf')      # true if s ends with suffix
s.char_at(i)            # byte at index i (as i64)
s.slice(start, end)     # substring [start, end)
s.split(delim)          # split into array of strings
s.trim()                # strip leading/trailing whitespace
s.to_upper()            # uppercase copy
s.to_lower()            # lowercase copy
s.replace(old, new)     # replace all occurrences
s.find(sub)             # index of first occurrence (-1 if not found)
s.lines()               # split by newlines
s.repeat(n)             # repeat string n times
s.is_empty()            # true if length is 0
s.trim_left()           # strip leading whitespace
s.trim_right()          # strip trailing whitespace

# Regex (requires PCRE2)
s.matches(pattern)      # true if pattern matches anywhere in s
s.find_all(pattern)     # all matches as array of strings
s.replace_re(pat, rep)  # regex replace
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
to_string(x)            # convert to string
time_now()              # nanosecond timestamp
assert(cond)            # rich assert with auto-generated messages
constant_time_eq(a, b)  # constant-time equality comparison

# Collections
set()                   # new empty set
priority_queue()        # new empty priority queue
vec_with_alloc(alloc)   # vec using custom allocator
map_with_alloc(alloc)   # map using custom allocator

# Comptime reflection
fields_of('TypeName')   # field names as array of strings
type_of(expr)           # type as string
size_of('TypeName')     # byte size as i64

# Matrix / SIMD
matmul(A, B)            # matrix multiplication
# ndarray: use `3 by 3` syntax (the `by` keyword)
```

### Debug

Compile with `-g` / `--debug` to emit DWARF debug info. Use with lldb or gdb:

```bash
jadec main.jade -o main -g
lldb ./main
```

---

*Jade: Hard. Dense. Beautiful.*
