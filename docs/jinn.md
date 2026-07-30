# Jinn

Jinn is a compiled, statically typed systems language with an indentation-based
syntax and word-based operators. It reads close to pseudocode, infers most
types, and has no garbage collector — memory is managed automatically through
ownership.

```jinn
*main
    log('hello, world')
```

This document is a tour of the language, from the basics to the more advanced
features. It describes the language **as implemented today**, not as aspired
to; where a feature is incomplete, the gap is stated inline. Every fenced
example is extracted and compiled against the current compiler by
`tests/doc_examples.rs`, so an example that stops compiling fails CI.

---

## Contents

- [Jinn](#jinn)
  - [Contents](#contents)
  - [Source files](#source-files)
  - [Bindings](#bindings)
    - [Constants](#constants)
  - [Primitive types](#primitive-types)
  - [Strings](#strings)
  - [Operators](#operators)
    - [Equality and comparison](#equality-and-comparison)
    - [Logical](#logical)
    - [Membership](#membership)
    - [Arithmetic and bitwise](#arithmetic-and-bitwise)
    - [Casting](#casting)
  - [Control flow](#control-flow)
    - [Conditionals](#conditionals)
    - [Ternary](#ternary)
    - [Loops](#loops)
    - [Parallel loops](#parallel-loops)
  - [Functions](#functions)
    - [Default and named arguments](#default-and-named-arguments)
    - [Inline bodies](#inline-bodies)
    - [Pattern clauses](#pattern-clauses)
    - [Higher-order functions](#higher-order-functions)
  - [Lambdas and pipelines](#lambdas-and-pipelines)
  - [Types](#types)
    - [Methods](#methods)
  - [Enums](#enums)
  - [Pattern matching](#pattern-matching)
  - [Generics](#generics)
  - [Aliases and newtypes](#aliases-and-newtypes)
  - [Collections](#collections)
    - [Vectors](#vectors)
    - [Maps](#maps)
    - [Array literals](#array-literals)
  - [Comprehensions and iterators](#comprehensions-and-iterators)
  - [Generators](#generators)
  - [Error handling](#error-handling)
  - [Modules](#modules)
  - [Concurrency](#concurrency)
    - [Actors](#actors)
    - [Channels](#channels)
    - [Select](#select)
  - [Persistent stores](#persistent-stores)
  - [Systems programming](#systems-programming)
    - [C interop](#c-interop)
    - [Raw pointers](#raw-pointers)
    - [Volatile access](#volatile-access)
  - [Standard library](#standard-library)
    - [Numeric methods](#numeric-methods)
    - [Integer bit operations](#integer-bit-operations)
    - [Regular expressions](#regular-expressions)
    - [Built-ins](#built-ins)
  - [Memory and ownership](#memory-and-ownership)

---

## Source files

Jinn uses indentation for structure — no braces, no semicolons. Indent with
spaces; tabs are rejected. Comments begin with `#` and run to end of line.

```jinn
# This is a comment.
*main
    x is 1      # trailing comment
    log(x)
```

A function named `main` is the program entry point.

---

## Bindings

`is` introduces a binding. The type is inferred from the value.

```jinn
x is 42                # i64
pi is 3.14159          # f64
name is 'jinn'         # String
ready is true          # bool
```

You can annotate a type explicitly with `as`:

```jinn
count as i32 is 0
```

Reassigning a variable uses the same `is`:

```jinn
x is 0
x is x + 1
```

Augmented assignments update a variable in place:

<!-- doctest:prelude
x is 0
mask is 1
-->
```jinn
x += 1
x -= 2
x *= 3
x /= 4
x &= 0xFF
x |= 0x80
x ^= mask
x <<= 2
x >>= 1
```

### Constants

Top-level names written in `ALL_CAPS` are the convention for constants and
cannot be reassigned.

```jinn
MAX_SIZE is 1024
DEFAULT_PORT is 8080
```

---

## Primitive types

| Type                     | Description              |
| ------------------------ | ------------------------ |
| `i8` `i16` `i32` `i64`   | Signed integers          |
| `u8` `u16` `u32` `u64`   | Unsigned integers        |
| `f32` `f64`              | Floating-point numbers   |
| `bool`                   | `true` or `false`        |
| `String`                 | UTF-8 text               |

Integer literals default to `i64` and adapt to the type they are used with.
Underscores may be used as digit separators: `1_000_000`.

---

## Strings

String literals use single or double quotes. Both support `{expr}`
interpolation.

```jinn
name is 'world'
log('hello {name}')           # hello world

x is 42
log('x={x}, doubled={x * 2}')  # x=42, doubled=84
```

Common string methods:

<!-- doctest:prelude
s is 'abc'
start is 0
end is 1
delim is ','
old is 'a'
new is 'b'
sub is 'b'
n is 2
-->
```jinn
s.length              # number of bytes
s.contains('sub')
s.starts_with('pre')
s.ends_with('suf')
s.slice(start, end)
s.split(delim)
s.trim()
s.to_upper()
s.to_lower()
s.replace(old, new)
s.find(sub)           # index, or -1 if absent
s.lines()
s.repeat(n)
s.is_empty()
```

---

## Operators

Jinn uses words for logical and equality operators, and symbols for arithmetic.

### Equality and comparison

`equals` (or `eq`) and `neq` test equality. Ordering uses `<`, `>`, `<=`, `>=`.

<!-- doctest:prelude
x is 1
y is 2
-->
```jinn
if x equals 0
    log('zero')
if x neq y
    log('different')
```

Comparisons can be chained the way they read in mathematics:

<!-- doctest:prelude
a is 1
b is 2
c is 3
x is 50
-->
```jinn
if 0 < x < 100
    log('in range')
if a <= b <= c
    log('sorted')
```

### Logical

`and`, `or`, `not`, and `xor`:

<!-- doctest:prelude
a is true
b is false
-->
```jinn
if a and not b
    log('a only')
if a xor b
    log('exactly one')
```

### Membership

`in` tests membership in arrays, vectors, maps (keys), and strings
(substrings).

<!-- doctest:prelude
x is 2
-->
```jinn
if x in [1, 2, 3]
    log('found')
if 'lo' in 'hello'
    log('substring')
```

### Arithmetic and bitwise

<!-- doctest:skip reference table, one operator per cell rather than a program -->
```jinn
a + b    a - b    a * b    a / b    a % b    a mod b
a pow b                    # exponentiation
a & b    a | b    a ^ b    # bitwise and / or / xor
a << b   a >> b            # shifts
```

### Casting

<!-- doctest:prelude
x is 1
big is 1000
-->
```jinn
y is x as f64            # widen — always safe
z is big as strict i16   # narrow, panics if the value does not fit
w is big as i16          # narrow, truncates
```

---

## Control flow

### Conditionals

<!-- doctest:prelude
x is 1
-->
```jinn
if x > 0
    log('positive')
elif x equals 0
    log('zero')
else
    log('negative')
```

### Ternary

`condition ? then ! else` is a conditional expression. It is the idiomatic way
to choose a value.

<!-- doctest:prelude
x is 1
ready is true
-->
```jinn
sign is x > 0 ? 1 ! -1
label is ready ? 'go' ! 'wait'
```

Ternaries chain in the **else** position (like an `elif` ladder). Nesting a
ternary in the *then* position is not supported — parenthesize or restructure
instead. A ternary bound with `is` stays on one line; the multi-line arm form
(leading `?` / `!` markers) is available in statement position, shown under
[Error handling](#error-handling).

<!-- doctest:prelude
score is 85
-->
```jinn
grade is score > 90 ? 'A' ! score > 80 ? 'B' ! 'C'
```

### Loops

<!-- doctest:prelude
n is 3
items is [1, 2, 3]
done is true
*process x is x
-->
```jinn
# While
while n > 0
    n is n - 1

# Count over a range
for i in 0 to 100
    log(i)

# Range with a step
for i in 0 to 100 by 2
    log(i)

# Iterate a collection
for item in items
    process(item)

# Infinite loop
loop
    if done
        break
```

A C-style counted loop is written `loop(init, cond, step)`, where `$` is the
current value. It is the common idiom for indexed iteration over a collection:

<!-- doctest:prelude
items is [1, 2, 3]
-->
```jinn
loop(0, $ < items.len(), $ + 1)
    log(items.get($))
```

`break` and `continue` work in any loop. A loop bound to a name can be used as
a label for breaking out of nested loops:

```jinn
outer is for i in 0 to 10
    for j in 0 to 10
        if i * j > 50
            break outer
```

### Parallel loops

`sim for` declares that iterations are independent and may run in parallel.

> **Current status:** `sim for` compiles and runs, but it currently lowers to
> a **sequential** counted loop — measured timings are identical to `for`
> (`src/mir/lower/loops.rs`). Write it only where iterations really are
> independent, so the code stays correct when parallel lowering lands.

<!-- doctest:prelude
items is [1, 2, 3]
*process x is x
-->
```jinn
sim for x in items
    process(x)
```

---

## Functions

Functions are declared with `*`. Parentheses are optional on both definitions
and calls. Parameter and return types are inferred when omitted.

```jinn
*add a, b
    a + b

*main
    log(add(1, 2))
```

The last expression in a body is its result; `return` is available for early
exit.

```jinn
*fib n
    if n < 2
        return n
    fib(n - 1) + fib(n - 2)
```

Annotate types with `as` and `returns` when you want them. Parentheses are
required around parameters once you annotate them.

```jinn
*greet(name as String) returns String
    'hello {name}'
```

### Default and named arguments

```jinn
*connect(host as String, port as i64 is 8080)
    log('connecting to {host}:{port}')

connect(host is 'localhost', port is 3000)
```

### Inline bodies

A single-expression function can use `is` instead of an indented block.

```jinn
*double x is x * 2
*square(x as i64) is x * x
```

### Pattern clauses

A function may be defined in several clauses with literal parameters. Clauses
are tried in order; the first matching one runs.

```jinn
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)

*gcd(a, 0) is a
*gcd a, b is gcd(b, a % b)
```

### Higher-order functions

Functions are values. A function parameter is typed `(ParamTypes) returns Ret`.

```jinn
*apply(f as (i64) returns i64, x as i64)
    f(x)
```

---

## Lambdas and pipelines

A lambda is written `|params| body`, where the body is a single expression.
Lambda parameters are always inferred — `as` annotations are not supported
inside `|…|` — and multi-line lambda bodies are not supported; use a named
function for anything larger.

```jinn
square is |x| x * x
```

The pipeline operator `~` feeds the left value as the first argument of the
function on the right:

<!-- doctest:prelude
value is 1
*double x is x * 2
*add_one x is x + 1
-->
```jinn
result is value ~ double ~ add_one
```

Inside a pipeline, `$` marks where the piped value goes when you need it in a
different position:

<!-- doctest:prelude
value is 1
*add a, b is a + b
-->
```jinn
result is value ~ add(5, $)     # add(5, value)
```
        
---

## Types

A `type` declares a record with named fields. Values are constructed with named
fields and accessed with `.`.

```jinn
type Point
    x as i64
    y as i64

p is Point(x is 10, y is 20)
log(p.x)
```

A field may have a default value, which can be supplied with `is`:

```jinn
type Config
    retries is 3
    host as String
```

### Methods

Methods are functions declared inside the type. They take `self` explicitly, or
omit it and refer to fields by name.

```jinn
type Vec3
    x as f64
    y as f64
    z as f64

    *dot(self, other as Vec3)
        self.x * other.x + self.y * other.y + self.z * other.z

    *length
        (x * x + y * y + z * z).sqrt()
```

Functions that operate on a type can also be written as free functions in a
module and called through the module name — both styles are common.

---

## Enums

An `enum` is a tagged union. Variants may carry data.

```jinn
enum Shape
    Circle(f64)
    Rect(f64, f64)
    Unit
```

You handle an enum by matching on it:

<!-- doctest:prelude
enum Shape
    Circle(f64)
    Rect(f64, f64)
    Unit
-->
```jinn
*area(s as Shape) returns f64
    match s
        Circle(r) ? 3.14159 * r * r
        Rect(w, h) ? w * h
        Unit ? 0.0
```

Variants can be given explicit integer values, which is useful for flags and C
interop:

```jinn
enum HttpStatus
    Ok is 200
    NotFound is 404
    ServerError is 500
```

---

## Pattern matching

`match` selects a branch by pattern. Patterns include literals, binding names,
enum constructors with destructuring, and the wildcard `_`.

<!-- doctest:prelude
enum Shape
    Circle(f64)
    Rect(f64, f64)
n is 1
shape is Circle(1.0)
-->
```jinn
match n
    0 ? log('zero')
    1 ? log('one')
    _ ? log('many')

match shape
    Circle(r) ? log(r)
    Rect(w, h) ? log(w * h)
```

A branch body may be a single expression after `?`, or an indented block:

<!-- doctest:prelude
enum Outcome
    Ok(i64)
    Err(String)
result is Ok(1)
-->
```jinn
match result
    Ok(v) ?
        log('ok')
        log(v)
    Err(e) ?
        log(e)
```

---

## Generics

`of` introduces type parameters. Generic code is specialized for each concrete
type at compile time.

```jinn
*max of T(a as T, b as T)
    a > b ? a ! b

type Pair of A, B
    first as A
    second as B

enum Option of T
    Some(T)
    None
```

Generic types appear in annotations with `of`, for example `Vec of Account`.

---

## Aliases and newtypes

An `alias` is a second name for an existing type — interchangeable with it.

```jinn
alias Seconds is f64
alias UserId is i64
```

A `type` that wraps a single field is a distinct type, even if two such types
wrap the same underlying type:

```jinn
type Celsius
    value as f64

type Fahrenheit
    value as f64
# Celsius and Fahrenheit cannot be used interchangeably.
```

---

## Collections

### Vectors

A vector is a growable array, created with `vec()`.

```jinn
v is vec()
v.push(1)
v.push(2)
log(v.len())     # 2
log(v.get(0))    # 1
log(v.pop())     # 2
```

### Maps

```jinn
m is map()
m.set('key', 42)
log(m.get('key'))    # 42
log(m.has('key'))    # true
```

### Array literals

```jinn
nums is [1, 2, 3, 4, 5]
```

---

## Comprehensions and iterators

A comprehension builds a vector from a range, with an optional filter:

```jinn
squares is [x pow 2 for x in 0 to 10]
evens is [x for x in 0 to 100 if x mod 2 equals 0]
```

Vectors also provide functional combinators, which chain with `.` or `~`:

<!-- doctest:prelude
nums is [1, 2, 3]
items is [1, 2, 3]
target is 2
-->
```jinn
doubled is nums.map(|x| x * 2)
big is nums.filter(|x| x > 10)
total is nums.fold(0, |acc, x| acc + x)
found is items.find(|x| x equals target)
```

Available combinators include `map`, `filter`, `fold`, `any`, `all`, `find`,
`zip`, `take`, `skip`, `chain`, `flatten`, `enumerate`, `reverse`, `sort`,
`sum`, `count`, and `contains`.

---

## Generators

A function that contains `yield` is a generator. Calling it produces a lazy
sequence; `next()` advances it.

```jinn
*counter()
    n is 0
    loop
        yield n
        n is n + 1

*main
    g is counter()
    log(g.next())    # 0
    log(g.next())    # 1
```

Generators can also be iterated with `for`.

---

## Error handling

Errors are ordinary values. There are no exceptions. You model an error with an
enum, return it, and handle it with `match` at the call site.

```jinn
enum Result
    Ok(i64)
    Err(i64)

*checked_add(a as i64, b as i64) returns Result
    sum is a + b
    if sum > 100
        return Err(sum)
    Ok(sum)

*main
    match checked_add(60, 50)
        Ok(v) ? log(v)
        Err(e) ? log(0 - e)
```

The `err` keyword marks an enum as an error type. It behaves like a regular
enum and documents intent:

```jinn
err FileError
    NotFound
    Denied
```

A function declares which error enums it can return with a trailing `! E`,
and raises one with `err <Variant>`, which returns early:

```jinn
err FileError
    NotFound
    Denied

*open(path as String) returns i64 ! FileError
    if path equals ''
        err NotFound
    42
```

At the call site the quaternary handles both outcomes: `?` binds the success
value as `$`, `!!` binds the error as `err`. Inside a fallible function, a
bare call propagates the error to the caller with no ceremony. In statement
position the arms may span multiple lines with leading markers.

<!-- doctest:prelude
err FileError
    NotFound
    Denied

*open(path as String) returns i64 ! FileError
    if path equals ''
        err NotFound
    42
-->
```jinn
open('config') ? log($) !! log('open failed')
fd is open('config') !! -1        # default on error

open('config')
    ? log($)
    !! log('open failed')
```

`defer` registers cleanup that runs when the function exits, whichever way it
exits. Deferred blocks run in reverse order of registration.

```jinn
*process()
    defer
        log('cleanup')
    log('work')          # prints work, then cleanup
```

---

## Modules

Each file is a module. A module's name is its file name, and its functions and
types are referred to through that name.

<!-- doctest:file mymath.jn -->
```jinn
# mymath.jn
*add a, b
    a + b
```

```jinn
# main.jn
use mymath

*main
    log(mymath.add(1, 2))
```

`use` accepts a path for files in subdirectories, and an alias:

<!-- doctest:file models/account.jn -->
```jinn
# models/account.jn
*balance
    0
```

<!-- doctest:file long_module_name.jn -->
```jinn
# long_module_name.jn
*version
    1
```

```jinn
use models/account
use long_module_name as lmn
```

---

## Concurrency

### Actors

An `actor` has private fields and message handlers. Spawn one with `spawn`,
send it a message by calling a handler, shut it down with `stop`, and wait for
it to finish with `join`.

```jinn
actor Counter
    count as i64

    @init start as i64
        count is start

    @increment amount as i64
        count is count + amount

    @report
        log(count)

*main
    c is spawn Counter
    c.init(0)
    c.increment(5)
    c.increment(3)
    c.report()
    stop c
    join c        # prints 8
```

Handlers introduced with `@` are asynchronous: the call enqueues a message on
the actor's mailbox and returns immediately. Messages are processed in order.
Actors run as **daemon** tasks — the program does not wait for them at exit —
so a graceful shutdown is `stop` (close the mailbox; every already-enqueued
message is still delivered) followed by `join` (wait for the actor to drain
and exit). Without the `join` above, `*main` can return before `@report`
runs and the program prints nothing.

A handler introduced with `*` is called synchronously and returns a value.
Two current limitations, both tracked for fixes: a `returns` annotation on a
`*` handler does not parse (write the handler unannotated), and a synchronous
call does **not** wait for earlier `@` messages to be processed — it reads
the actor's state as it is at the moment of the call. See
[`docs/concurrency.md`](concurrency.md) for the full shutdown contract.

### Channels

A channel carries values of one type between concurrent tasks.

```jinn
ch is channel of i64(16)    # buffered, capacity 16
send ch, 42
v is receive ch
close ch
```

### Select

`select` waits on several channel operations and runs the first one ready.

<!-- doctest:prelude
ch1 is channel of i64(4)
ch2 is channel of i64(4)
send ch1, 1
-->
```jinn
select
    receive ch1 as val
        log('from ch1: {val}')
    receive ch2 as val
        log('from ch2: {val}')
    default
        log('nothing ready')
```

---

## Persistent stores

A `store` is a typed collection that persists to disk between runs. Queries are
checked at compile time.

> **Current limitations, stated so you can design around them:**
>
> - A store is durable for a **single writer process**. Two processes
>   writing the same store is unsupported and currently undetected
>   (file locking is planned — task 8-21).
> - A query that matches **nothing currently yields a zero-initialized
>   record** (`name` empty, numbers `0`) that is indistinguishable from real
>   data. Guard with `count` until misses become `Option of Record`
>   (task 8-25, decision D3).
> - Iterating a whole store (`for u in all users`) currently crashes at
>   runtime (task 8-25). Iterate via queries you know match, or keep your
>   own vector.

```jinn
store users
    name as String
    age as i64

# Insert records in field order
insert users 'Alice', 30
insert users 'Bob', 25

# Query — returns the first matching record
young is users where age < 30
log(young.name)

# Compound filters
adult is users where age > 20 and name equals 'Alice'

# Update matching records
set users where name equals 'Alice' age 31

# Delete matching records
delete users where age > 28

# Count
total is count users

# Group operations atomically
transaction
    insert users 'Dave', 40
    delete users where age > 50
```

### Constraint failures are errors

`insert` and `set` are fallible: attach handler arms (`?` / `!!`) and they
become expressions of type `Result of i64, StoreError`. The Ok payload is the
new record's sid for `insert` and the number of updated rows for `set`.
`StoreError` is a built-in error enum with variants `Duplicate` (an `@unique`
violation), `Missing` (a `set` filter that matched no rows), `Constraint`
(an empty `@required` string), and `Io`:

<!-- doctest:prelude
store users
    name as String
    age as i64
-->
```jinn
insert users 'Alice', 30 ? log($) !! log('insert failed')

set users where name equals 'Alice' age 31
    ? log($)
    !! log('no such user')

*signup(name as String) returns Result of i64, StoreError
    sid is insert users name, 0 ? $ !! err
    Ok(sid)
```

A bare `insert` with no handler arms that violates a constraint traps with a
diagnostic — silent data loss is never an option.

A `transaction` block is atomic with respect to escaping errors: if an error
propagates out of the block (`err`, a failed `?` propagation), if `return`
leaves mid-block, the writes are handled as a unit. Normal completion and
`return` commit the batch durably (one group fsync instead of one per record);
an escaping error or a runtime trap rolls every store touched inside the block
back to its pre-transaction state — data files, WAL, and indexes alike.
Nested `transaction` blocks join the outermost one: only the outermost commit
makes the batch durable, and any rollback aborts the whole nest.

> **Current limitations:** transaction state is process-global, not
> per-task — two concurrent tasks must not run `transaction` blocks at the
> same time, and one task's open transaction weakens the durability of other
> tasks' writes; rollback is not crash-safe (a crash mid-rollback can leave
> the store corrupt). Both are being fixed (task 8-24).

Field types are `i64`, `f64`, `bool`, and `String`. Query operators are
`equals`, `neq`, `<`, `>`, `<=`, and `>=`, combined with `and` / `or`. Data is
stored in a `<name>.store` file in the working directory.

---

## Systems programming

### C interop

Declare an external C function with `extern *`. Variadic declarations
(`...`) are not supported yet — bind fixed-arity functions:

```jinn
extern *puts(s as %i8) returns i32

*main
    puts('hello from jinn')
```

### Raw pointers

`%` takes a pointer; `@` dereferences one.

```jinn
value is 42
ptr is %value
val is @ptr
```

### Volatile access

The `volatile` module performs reads and writes that are never reordered or
elided — for memory-mapped I/O and hardware registers.

```jinn
use volatile

*main
    reg is 0
    ptr is %reg
    volatile.write(ptr, 1)
    v is volatile.read(ptr)
```

---

## Standard library

A few commonly used pieces.

### Numeric methods

> **Known gap:** the return type of numeric method calls currently fails to
> infer in many positions (`r is x.sqrt()` mis-defaults to `i64`), which is
> the same inference gap as task 8-16. The block below is excluded from the
> doc-compile gate until that lands; re-enable it there.

<!-- doctest:skip blocked on 8-16: numeric method return-type inference -->
```jinn
x.sqrt()
x.sin()
x.cos()
x.abs()
x.floor()
x.ceil()
x.round()
x.min(y)
x.max(y)
x.is_nan()
x.is_finite()
```

### Integer bit operations

<!-- doctest:prelude
x is 5
n is 1
-->
```jinn
popcount(x)         # set bits
clz(x)              # leading zeros
ctz(x)              # trailing zeros
rotate_left(x, n)
rotate_right(x, n)
bswap(x)            # byte swap
```

### Regular expressions

```jinn
use regex

regex.is_match('abc123', '[0-9]+')      # true
regex.find('abc123', '[0-9]+')          # '123'
regex.find_all('a1b2c3', '[0-9]+')      # ['1', '2', '3']
```

### Built-ins

<!-- doctest:prelude
value is 1
x is 1
cond is true
-->
```jinn
log(value)        # print a line to stdout
to_string(x)      # convert a value to a String
assert(cond)      # check a condition at runtime
```

---

## Memory and ownership

Jinn manages memory for you, without a garbage collector. You do not write
allocation or free calls: scalars and strings are values (assignment copies),
and each heap aggregate (`Vec`, `Map`, aggregate-containing structs) has a
single owner whose scope exit releases it.

**The target model** (decision D1 in
[`remediation-2026-07.md`](remediation-2026-07.md), specified by task 8-5) is
move-on-assign with compiler-inferred borrows for heap aggregates: `b is a`
moves ownership — `a` is unusable until reassigned — reads borrow without
copying, and a program that would corrupt memory does not compile. No
lifetime annotations, no `&`, no explicit `clone()`.

> **Current status — not yet enforced.** Today the checker does **not** yet
> deliver that guarantee for heap aggregates, and these programs misbehave
> (each is pinned in `tests/review_2026_07.rs`, owned by tasks 8-6..8-8):
>
> - returning an aggregate parameter double-frees
>   (`*ident(v) returns Vec of i64` / `return v`);
> - two `dispatch` tasks mutating one `Vec` corrupt the allocator instead of
>   being rejected at compile time;
> - `b is a; b.push(4)` on a `Vec` is silent shared mutable aliasing —
>   the mutation is visible through `a`.
>
> Until tasks 8-5..8-8 land, treat aggregate assignment and
> aggregate-returning helpers with care, and share data across tasks only
> through channels or actors. Strings are unaffected — they already have
> value semantics.

One property that holds by construction: there are no shared reference
counts, so reference cycles cannot be constructed and cycle leaks are
impossible.
