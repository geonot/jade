# The Way of Jinn

*"Your Wish is My Command"*

---


Most languages give you only one of: fast, safe, or readable. Jinn promises to bring you all three. Without the cryptic syntax of Rust, the overhead of an interpreted language like Python or Ruby, or even the Virtual Machine and Garbage Collector of Java, and without the many classes of exploitable safety issues of C, Jinn shines from every facet for the modern developer.

```jinn
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)
```

Jinn compiles to machine code as fast as Clang would emit for the equivalent C.


```
ackermann(3,10)   jinn 186ms   clang -O3 202ms   0.92x
fibonacci(40)     jinn 339ms   clang -O3 336ms   1.01x
nbody             jinn 136ms   clang -O3 136ms   0.99x
total             jinn 1.21s   clang -O3 1.25s   0.97x
```

If a feature degrades performance, it doesn't ship. Period.

---

### Principles

1. **The compiler does the hard work.** 
2. **Syntax brings clarity, not confusion.** 
3. **Performance is non-negotiable.**
4. **Errors are values, not exceptions.** 
5. **One application, one binary.**
---

### The shape of the code

A function starts with `*`. Parens are optional. Bindings use `is`.
Indentation is structure.

```jinn
*greet name
    log 'hello {name}'

*square x
    aax * x

*max a, b
    a > b ? a ! b
```

Decisions in one breath:

```jinn
sign is x > 0 ? 1 ! -1

grade is score > 90 ? 'A'
       ! score > 80 ? 'B'
       ! score > 70 ? 'C'
       ! 'F'
```

Data flows left to right:

```jinn
result is foo ~ double ~ add_one ~ square
```

Pattern-directed definitions because that's how you'd write it on a
whiteboard:

```jinn
*gcd(a, 0) is a
*gcd a, b  is gcd b, a mod b
```

The compiler merges the cases into a single dispatch. No runtime cost.

---

### Memory

You don't manage it. The compiler does.

One owner per value. Reads borrow for free. When sharing is needed, references are counted; but analysis strips the ones that don't earn their keep. In-place reuse fires whenever the layout matches and the count is one. No `'a`. No `Rc<RefCell<…>>`. No `unsafe` escape hatch
to a system that became its own obstacle.

```jinn
a is 'hello'
b is a              // share, no copy
b is b + ' world'   // copy-on-write, a unchanged
```

Both freed at scope exit. Deterministic. No GC pauses. No surprise frees.

---

### Types

Inferred by default. Annotate when it helps the reader.

```jinn
*add a, b
    a + b

*connect(host as String, port as i64 is 8080)
    ...
```

Hindley-Milner with bidirectional flow, doing what type theory has been
ready to do for forty years. 

---

### Concurrency

Actors own state. Channels move values. `sim for` distributes work.

```jinn
actor Counter
    count is 0
    @increment amount
        count is count + amount

c is spawn Counter
c.increment(5)

ch is channel of i64(10)
send ch, 42
val is receive ch

sim for x in items
    process x
```

No thread pools to tune. No locks to forget. Dangerous primitives aren't exposed because with Jinn you don't need them.

---

### Errors


`! value` is the universal early return. `defer` runs on every exit path.
That's the whole error system.

```jinn
*open_and_use(path as string) returns Outcome
    f is open(path)
    defer
        close(f)
    if bad(f)
        ! Bad
    Ok(read_size(f))
```

No `try`. No `catch`. No stack unwinding. No `Result<T,E>` wrapper — your
err enum *is* the result.

---

## Down to the metal when you need it

Extern C, raw pointers, inline asm, packed and aligned types. They're
ordinary syntax, not a separate dialect.

```jinn
extern *printf(fmt as %i8, ...) returns i32

ptr is %value
val is @ptr

asm
    nop

type Pixel @packed
    r as u8
    g as u8
    b as u8

type Cache @align(64)
    data as [u8; 64]
```

Object files, DWARF, separate compilation. Whatever a C programmer
reaches for is here.

---

## The standard library

Plain `.jn` files. No special compiler hooks. The same language you write
your programs in.

```jinn
use fs
use json
use crypto
use regex
use net
```

Read the source. Patch it. Vendor it. It's just code.

---

## The toolchain

```bash
jinn compile hello.jn
./hello
```

Native binary. No GC. No JIT. No VM.

---
