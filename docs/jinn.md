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
to; where a feature is incomplete, the gap is stated inline and carries an id
into [`roadmap.md`](roadmap.md). Every fenced `jinn` example is extracted and
compiled against the current compiler by `tests/doc_examples.rs`, so an example
that stops compiling fails CI.

The tour is the starting point. The normative specifications live alongside it:
[`memory-model.md`](memory-model.md) for ownership,
[`concurrency.md`](concurrency.md) for tasks and shutdown,
[`error-effects.md`](error-effects.md) for the error model,
[`strings.md`](strings.md) for text, and [`jinn.ebnf`](jinn.ebnf) for the
grammar. [`README.md`](README.md) maps the rest.

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
    - [Frozen values](#frozen-values)
    - [Views](#views)
  - [Idiomatic Jinn](#idiomatic-jinn)
  - [Reserved words](#reserved-words)

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
s.length              # number of Unicode scalars (use .byte_count for bytes)
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

A lambda may reference bindings from its enclosing scope; the result is a
closure that **owns its environment**. Captures follow the same category rules
as every other ownership boundary ([Memory and ownership](#memory-and-ownership)):
scalars and `String`s copy into the environment at creation, and aggregates
**move** into it — reading a captured `Vec` afterwards is the ordinary
use-after-move error, and binding `copy xs` to a fresh name first keeps the
original. Because the environment is owned, a closure may outlive the frame
that created it:

```jinn
*make_adder(base as i64) returns (i64) returns i64
    |x| x + base

*main
    add10 is make_adder(10)
    log(add10(5))              # 15
```

The closure value is itself an aggregate: assigning it moves it, it moves into
at most one task, and dropping it drops everything it captured. A function-typed
parameter (`f as (i64) returns i64`) borrows the closure for the call, so the
caller keeps it. There is no capture by reference: what looks like "mutating a
captured variable" in other languages is expressed with actors or by rebuilding
the value — the closure's copy and the original are independent. The full
specification is
[`design/closure-captures.md`](design/closure-captures.md).

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

A type's ownership category — whether assignment copies it or moves it — is
inferred from its fields (see [Memory and ownership](#memory-and-ownership)).
`@value` and `@aggregate` assert the intended category, so adding a `Vec`
field to a type that promises copy semantics is a compile error naming the
field instead of a silent behavior change several embeddings away:

```jinn
type Point @value
    x as i64
    y as i64

type Basket @aggregate
    items as Vec of i64

p is Point(x is 10, y is 20)
q is p
log(p.x + q.x)
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
    Nothing
```

Generic types appear in annotations with `of`, for example `Vec of Account`.
`of` takes a single type argument. For a type with several parameters, annotate
with angle brackets — `Pair<i64, String>` — which stay unambiguous where a
comma already separates parameters or fields. The two forms are
interchangeable for one argument (`Box of i64` and `Box<i64>` name the same
type), and annotations nest: `Pair<i64, Pair<i64, String>>`.

```jinn
type Pair of A, B
    first as A
    second as B

*flip(p as Pair<A, B>) returns Pair<B, A>
    Pair(first is p.second, second is p.first)

*main
    p is Pair(first is 5, second is 'hello')
    q is flip(p)
    log(q.first)
    log(q.second)
    0
```

When a type parameter appears only in return position, the call site cannot
infer it from the arguments. Either annotate the bind — the annotation flows
into the call — or supply the type argument explicitly with `of`:

```jinn
*empty of T() returns Vec of T
    v is vec()
    return v

*main
    xs as Vec of string is empty()
    ys is empty of i64()
    xs.push('one')
    ys.push(2)
    log(xs.get(0))
    log(ys.get(0))
    0
```

The same two spellings bind a constructor's type parameter when no field
mentions it: `Box of string(tag is 5)`, or `b as Box<string> is Box(tag is 5)`.
A parameter left unbound defaults to `i64`, and the compiler warns when that
happens.

---

## Aliases and newtypes

An `alias` is a second name for an existing type. It is a *distinct* type
to the checker: passing an `f64` where a `Seconds` is expected is a type
error, exactly as for a newtype below. Use one when you want the name to
carry meaning at call sites.

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

Map keys are strings: `Map of V` in an annotation is shorthand for
`Map<string, V>`, and a non-string key type — `Map<i64, string>` — is a
compile error naming this rule.

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
sequence; `next()` advances it. The frame outlives the call, so a generator
**consumes its aggregate arguments** — passing a `Vec` to a generator moves
it into the frame, and using the original afterwards is the ordinary
use-after-move error (scalars and `String`s copy, as everywhere).

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

> **Current limitation, stated so you can design around it:**
>
> - A store is durable for a **single writer process**. Two processes
>   writing the same store is unsupported and currently undetected
>   (file locking is planned).

```jinn
store users
    name as String
    age as i64

# Insert records in field order
insert users 'Alice', 30
insert users 'Bob', 25

# A query can miss, so it has type `Result of <row>, StoreError` — match it
match users where age < 30
    Ok(young) ? log(young.name)
    Err(e) ? log('nobody under 30')

# … or collapse it with the quaternary; compound filters compose with `and`/`or`
adult_age is users where age > 20 and name equals 'Alice' ? $.age ! 0 - 1

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

### Filter vocabulary

Statement filters accept, besides the comparisons composed with `and`/`or`:
`between lo and hi`, membership `in [v1, v2]` (an equality chain), the text
predicates `contains`, `starts_with`, `ends_with`, and their ASCII
case-insensitive forms `iequals`, `icontains`, `istarts_with`, `iends_with`.

<!-- doctest:prelude
store users
    name as String
    age as i64
-->
```jinn
teens is count users where age between 13 and 19
named is count users where name iequals 'ALICE'
polite is count users where name istarts_with 'al'
```

Query blocks take the same comparisons, plus `field in [..]` (combinable with
`and`, not `or`) and the text predicates in method form:

<!-- doctest:prelude
store users
    name as String
    age as i64
-->
```jinn
insert users 'Alice', 30
r is users query
    where name.icontains('ALI') and age in [30, 31]
log r.age
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

Queries are fallible the same way: `users where …` is a
`Result of <row>, StoreError` whose miss is `Err(Missing)`. In a
non-fallible function it must be handled where it appears — `match` it or
collapse it with the quaternary. In a function whose error union includes
`StoreError`, binding the query needs no ceremony: the row comes out
unwrapped and a miss propagates to the caller:

<!-- doctest:prelude
store users
    name as String
    age as i64
-->
```jinn
*age_of(who as String) returns i64 ! StoreError
    r is users where name equals who   # miss propagates as Err(Missing)
    r.age
```

A `transaction` block is atomic with respect to escaping errors: if an error
propagates out of the block (`err`, a failed `?` propagation), if `return`
leaves mid-block, the writes are handled as a unit. Normal completion and
`return` commit the batch durably (one group fsync instead of one per record);
an escaping error or a runtime trap rolls every store touched inside the block
back to its pre-transaction state — data files, WAL, and indexes alike.
Nested `transaction` blocks join the outermost one: only the outermost commit
makes the batch durable, and any rollback aborts the whole nest.

> **Current limitation:** rollback is ordered for safety (WAL truncated
> first, then the data file restored atomically), but a crash exactly
> between the two steps leaves the last transaction's writes in the data
> file — structurally intact, not torn. Transaction state is per-task:
> concurrent tasks each get their own, and one task's open transaction
> does not weaken the durability of others' writes.

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

Jinn manages memory for you, without a garbage collector. You never write
allocation or free calls. The rule you need is three words: **numbers and text
copy; containers move; reads borrow.**

- Scalars and `String` are values: assignment copies, and both names stay
  usable.
- Heap aggregates — `Vec`, `Map`, and any struct or enum containing one — have
  a single owner. `b is a` **moves** ownership, and `a` is unusable until it is
  reassigned. Reading a moved value is a compile error that names the move site
  and the fix.
- Reads borrow. Method calls, field reads, and argument passing take a borrow
  that ends with the statement, so there is nothing to annotate: no lifetimes,
  no `&`, no `clone()`, no `Box`/`Rc`/`Arc`.
- An unannotated parameter borrows, unless the callee's body consumes it — by
  returning it, storing it, or sending it — in which case the call moves the
  argument. That is inferred from the body, not declared.
- `copy a` clones explicitly; `take a` moves explicitly. Every ownership
  diagnostic names whichever one you wanted.

An aggregate moves into at most one task, so two `dispatch` blocks cannot share
one `Vec` — that is a compile error, not a data race. Channels and actors are
how data moves between tasks.

Two properties hold by construction: there are no shared reference counts, so
reference cycles cannot be constructed and cycle leaks are impossible; and every
owned value has exactly one drop site, so cleanup is deterministic.

Doubly-linked lists, parent pointers, and cyclic graphs are therefore
inexpressible with direct ownership — by design. The blessed replacement is
`std/arena`: an `Arena of T` owns the nodes, and `Handle` values (index +
generation) stand in for pointers. A stale handle is detected, not dangling —
`contains` returns false and `get` traps once the slot is removed or reused.

`@value` and `@aggregate` on a `type` assert its ownership category, turning
an accidental category flip (adding a `Vec` field to a value type) into a
compile error at the definition.

### Frozen values

`freeze x` is a one-way transition to deep immutability. It consumes an
aggregate operand (`x` is tombstoned, like any move) and produces a
`Frozen of T` — the same bits at runtime, but a type through which every write
is a compile error: mutating methods, field assignment, moving a part out, and
passing to a parameter the mutation inference marks mutating are all rejected,
each diagnostic naming the fix. Every read works through the same syntax —
fields, elements, iteration, read-only methods, and passing to read-only
parameters:

```jinn
type Config
    retries as i64
    hosts as Vec of String

*attempts(c as Config) returns i64
    c.retries + 1

*main
    cfg is Config(retries is 3, hosts is vector('a', 'b'))
    frozen is freeze cfg
    log(frozen.retries)        # reads pass through
    log(attempts(frozen))      # read-only parameters accept frozen values
```

`Frozen of T` is a real type: a parameter or field declared `Frozen of T`
*demands* immutability from its callers. Freezing requires the type to be
built of data — scalars, `String`, `Vec`, `Map`, and structs or enums of the
same, recursively; `@resource` types, channels, actors, coroutines, and
functions are live handles, not data, and are rejected with the offending
field named. There is no `thaw`: to get a mutable value back, `copy` a part
out and build anew.

Sharing is what freezing buys. Inside a `together`, every `dispatch` may
capture the *same* frozen value — the one exception to "an aggregate moves
into at most one task" — because the tasks are joined before the owner's
scope ends, no copy and no reference count is needed, and there is no state
left to race on:

```jinn
*worker(cfg as Vec of i64, id as i64) returns i64
    cfg.sum() + id

*main
    xs is vector(10, 20, 30)
    frozen is freeze xs
    together
        dispatch
            log(worker(frozen, 1))
        dispatch
            log(worker(frozen, 2))
    log(frozen.length)      # still owned and readable after the join
```

Moving the shared value away while the `together` runs is a compile error;
after it joins, the owner keeps reading it, and one drop at the owner's scope
end frees it. The standard library follows the pattern for configuration:
`toml.parse_frozen(text)` returns a `Frozen of TomlTable`, ready to be read
through `toml.get`/`get_int`/`get_bool` and shared across a `together`
without copies. The design is [`design/freeze.md`](design/freeze.md).

### Views

`xs.view(a, b)` produces a `View of T`: a zero-copy window (`pointer + length`)
into `xs`'s buffer. `s.view(a, b)` does the same over a string's bytes, and
`xs.at_view(i)` views one element. A view supports `.length`, `.get(i)`, and
iteration, and a `View of T` parameter accepts a whole `Vec`, a sub-slice
view, or an array — and a `View of u8` parameter additionally accepts a whole
`String` as its byte window. The callee cannot tell and cannot keep it:

```jinn
*total(v as View of i64) returns i64
    t is 0
    for x in v
        t is t + x
    t

*main
    xs is vector(1, 2, 3, 4)
    log(total(xs))               # the whole vector, no copy
    log(total(xs.view(1, 3)))    # elements 1 and 2, no copy
```

A view may also be **bound** — `v is xs.view(1, 3)` — and the bind locks
its root: mutating, moving, or reassigning `xs` while `v` is live is a
compile error naming the view, and the lock releases when `v`'s block ends.
A view of a temporary cannot be bound (the data would die with the
statement). For iteration without per-element copies, `for p in pts.views()`
binds each element as a view; reading a field through an element view
(`pts.at_view(i).x`, `p.x` in the loop) reads through the pointer, and a
*read-only* method call through one (`p.norm2()`) operates on the original
element — no copy either way, which is the honest fix for the old hidden
deep-copy on nested-container reads. A method that mutates or consumes its
receiver is a compile error through a view:

```jinn
type Point
    x as i64
    y as i64

*main
    pts is vector(Point(x is 1, y is 2), Point(x is 3, y is 4))
    total is 0
    for p in pts.views()
        total is total + p.x
    log(total)                 # 4, and no Point was copied
```

What makes views safe without lifetime annotations is *second-classness*: a
view flows **down** — into calls, expressions, loop bodies, and block-scoped
binds — but never **out**. Returning one, storing one in a struct field,
container, or store, sending one across a task boundary, capturing one in a
task or closure, and yielding one are all compile errors, so a view never
outlives its root's borrow. The copying `slice` stays available when you
need an owned sub-sequence. The full design is
[`design/second-class-refs.md`](design/second-class-refs.md).

The full contract — the rules, the exact diagnostics, the tiers, and where the
implementation does not yet meet the contract — is
[`memory-model.md`](memory-model.md).

---

## Idiomatic Jinn

Jinn's premise is that the programmer expresses intent and the compiler does the
mechanism. Code that spells out mechanism the compiler already handles reads as
a transliteration from another language. The idioms below are what the corpus
converges on; the formatter's design ([`design/fmt-and-lint.md`](design/fmt-and-lint.md))
is built to move code toward them mechanically.

**Iterate collections; do not drive an index.** `loop xs` binds `$` to each
element and `$$` to its index. A counted loop over a pure integer range is
`for i in 0 to n`.

```
# avoid                                  # prefer
loop(0, $ < xs.len(), $ + 1)             loop xs
    a is xs.get($)                           use($)
    use(a)
```

**Do not accumulate strings with `+` in a loop.** It is O(n²) and noisy. Use
`join` when the concatenation is uniform, or a builder when it is not.

```
# avoid                                  # prefer
out is ''                                items.map(stringify).join(',')
loop items
    out is out + stringify($)
```

**Use `bool`, not an integer flag.** A variable only ever set to `0` or `1` and
only ever compared against `0` or `1` is a `bool` wearing a costume. A function
whose body is `if cond` / `return 1` / `return 0` is just `cond`.

**Collapse `else` containing a lone `if` into `elif`,** and turn a chain that
tests one scrutinee for equality into a `match`.

**Drop `self.` inside methods** where the name unambiguously resolves to a field
of the receiver and nothing shadows it. Field access inside a method resolves to
the receiver automatically.

```
# avoid                                  # prefer
*write(s as String)                      *write(s as String)
    self.parts.push(s)                       parts.push(s)
    self.total is self.total + s.length      total is total + s.length
```

**Drop the tail `return`.** A function returns its last expression.

**Drop casts and annotations the inferencer supplies.** Write the type when it
documents intent or pins a choice, not to restate what inference already knows.

**Use the collection literal.** `vec()` followed by repeated `.push` of known
values is a list literal or a comprehension.

**Prefer the scalar string APIs over byte loops.** `char_at` with magic numeric
constants is fast and unreadable, and it ties behaviour to ASCII. Reach for it
only when you genuinely mean bytes — and when you do, read
[`strings.md`](strings.md) first, because the byte and scalar views are
deliberately distinct.

---

## Reserved words

**91 spellings, 86 distinct tokens** — the five extras are the comparison
aliases. The list is the `KEYWORDS` table in `src/lexer/mod.rs`; the grammar
itself is [`jinn.ebnf`](jinn.ebnf), which `tests/ebnf_roundtrip.rs` keeps honest.

| Group | Keywords |
| --- | --- |
| Comparison and boolean | `is`, `eq`/`equals`, `neq`, `lt`/`ngte`, `gt`/`nlte`, `lte`/`ngt`, `gte`/`nlt`, `and`, `or`, `not`, `xor`, `in`, `pow`, `mod` |
| Control flow | `if`, `elif`, `else`, `unless`, `until`, `while`, `for`, `loop`, `break`, `continue`, `return`, `match`, `when`, `do`, `end`, `defer`, `yield` |
| Declarations and types | `type`, `enum`, `trait`, `impl`, `dispatch`, `pub`, `use`, `as`, `from`, `to`, `by`, `of`, `extern`, `asm`, `embed`, `alias`, `global`, `atomic`, `strict`, `returns`, `at`, `nop`, `freeze` |
| Concurrency | `actor`, `spawn`, `send`, `receive`, `channel`, `close`, `select`, `sim`, `supervisor`, `stop`, `default`, `together` |
| Stores | `store`, `migration`, `insert`, `delete`, `set`, `transaction`, `view`, `query`, `err` |
| Diagnostics and meta | `test`, `assert`, `log`, `unreachable`, `build`, `syscall`, `grad`, `einsum` |
| Literals | `true`, `false`, `none` |

The five comparison aliases (`ngte`, `nlte`, `ngt`, `nlt`) are double negatives
— `ngte` is "not greater-than-or-equal", i.e. `lt`. They parse, and the
formatter rewrites them to the positive spelling.

The access modifiers `copy`, `take`, and `const` are **not** reserved words:
they are recognized in modifier position and are ordinary identifiers
everywhere else.
