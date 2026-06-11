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
features. Every example is valid Jinn.
         
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
    - [System calls](#system-calls)
    - [Raw pointers](#raw-pointers)
    - [Volatile access](#volatile-access)
    - [Signals](#signals)
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

```jinn
x += 1
x -= 2
x *= 3
x /= 4
x %= 5
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

```jinn
if x equals 0
    log('zero')
if x neq y
    log('different')
```

Comparisons can be chained the way they read in mathematics:

```jinn
if 0 < x < 100
    log('in range')
if a <= b <= c
    log('sorted')
```

### Logical

`and`, `or`, `not`, and `xor`:

```jinn
if a and not b
    log('a only')
if a xor b
    log('exactly one')
```

### Membership

`in` tests membership in arrays, vectors, maps (keys), and strings
(substrings).

```jinn
if x in [1, 2, 3]
    log('found')
if 'lo' in 'hello'
    log('substring')
```

### Arithmetic and bitwise

```jinn
a + b    a - b    a * b    a / b    a % b    a mod b
a pow b                    # exponentiation
a & b    a | b    a ^ b    # bitwise and / or / xor
a << b   a >> b            # shifts
```

### Casting

```jinn
y is x as f64            # widen — always safe
z is big as strict i16   # narrow, panics if the value does not fit
w is big as i16          # narrow, truncates
```

---

## Control flow

### Conditionals

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

```jinn
sign is x > 0 ? 1 ! -1
label is ready ? 'go' ! 'wait'
```

Ternaries can nest (they associate to the right) and can span multiple lines
when the branches are large:

```jinn
grade is score > 90 ? 'A' ! score > 80 ? 'B' ! 'C'

result is condition
    ? do_something()
    ! do_something_else()
```

### Loops

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

`sim for` runs iterations in parallel on a work-stealing scheduler. Iterations
must be independent.

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

A lambda is written `|params| body`:

```jinn
square is |x| x * x
double is |x as i64| x * 2

# Multi-line: indent the body
transform is |x|
    y is x * 2
    y + 1
```

The pipeline operator `~` feeds the left value as the first argument of the
function on the right:

```jinn
result is value ~ double ~ add_one
```

Inside a pipeline, `$` marks where the piped value goes when you need it in a
different position:

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

`!` is shorthand for returning early — typically an error value:

```jinn
*open(path as String) returns FileError
    if path equals ''
        ! NotFound
    ...
```

A function may declare which error enums it returns with a trailing `! E`:

```jinn
*read(path as String) returns i64 ! FileError
    ...
```

`defer` registers cleanup that runs when the function exits, whichever way it
exits. Deferred blocks run in reverse order of registration.

```jinn
*process()
    defer
        log('cleanup')
    ...
```

---

## Modules

Each file is a module. A module's name is its file name, and its functions and
types are referred to through that name.

```jinn
# math.jn
*add a, b
    a + b
```

```jinn
# main.jn
use math

*main
    log(math.add(1, 2))
```

`use` accepts a path for files in subdirectories, and an alias:

```jinn
use models/account
use long_module_name as lmn
```

---

## Concurrency

### Actors

An `actor` has private fields and message handlers. Spawn one with `spawn`, send
it a message by calling a handler, and shut it down with `stop`.

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
```

Handlers introduced with `@` are asynchronous (fire-and-forget). A handler
introduced with `*` is synchronous and can return a value to the caller.

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

A `transaction` block is atomic with respect to escaping errors: if an error
propagates out of the block (`err`, a failed `?` propagation), if `return`
leaves mid-block, the writes are handled as a unit. Normal completion and
`return` commit the batch durably (one group fsync instead of one per record);
an escaping error or a runtime trap rolls every store touched inside the block
back to its pre-transaction state — data files, WAL, and indexes alike.
Nested `transaction` blocks join the outermost one: only the outermost commit
makes the batch durable, and any rollback aborts the whole nest.

Field types are `i64`, `f64`, `bool`, and `String`. Query operators are
`equals`, `neq`, `<`, `>`, `<=`, and `>=`, combined with `and` / `or`. Data is
stored in a `<name>.store` file in the working directory.

---

## Systems programming

### C interop

Declare an external C function with `extern *`:

```jinn
extern *printf(fmt as %i8, ...) returns i32

*main
    printf('hello from jinn\n')
```

### System calls

```jinn
syscall 1, 1, 'hello\n', 6    # write(stdout, msg, len)
```

### Raw pointers

`%` takes a pointer; `@` dereferences one.

```jinn
ptr is %value
val is @ptr
```

### Volatile access

The `volatile` module performs reads and writes that are never reordered or
elided — for memory-mapped I/O and hardware registers.

```jinn
use volatile

ptr is %reg
volatile.write(ptr, 1)
v is volatile.read(ptr)
```

### Signals

```jinn
use signal

*handler(sig as i32)
    log(sig)

*main
    signal.handle(2, handler)    # SIGINT
```

---

## Standard library

A few commonly used pieces.

### Numeric methods

```jinn
x.sqrt()    x.sin()    x.cos()    x.abs()
x.floor()   x.ceil()   x.round()
x.min(y)    x.max(y)
x.is_nan()  x.is_finite()
```

### Integer bit operations

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

```jinn
log(value)        # print a line to stdout
to_string(x)      # convert a value to a String
assert(cond)      # check a condition at runtime
```

---

## Memory and ownership

Jinn manages memory for you, without a garbage collector. Each value has a
single owner, and its memory is released automatically when the owner goes out
of scope. Reading a value borrows it without copying, so passing data around is
cheap.

You do not write allocation or free calls, and the language prevents
use-after-free, double-free, and data races. Most of the time memory management
is invisible — you write code in terms of values, and the compiler takes care of
the rest.
