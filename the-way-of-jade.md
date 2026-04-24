# The Way of Jade

*A manifesto for those who believe code should be hard, dense, and beautiful.*

---

## I. The Stone

Jade is not soft. It does not yield. It does not bend to fashion or compromise for convenience. Like the mineral it is named for, it is hard — harder than steel — yet carved into forms of astonishing elegance by those who understand its grain.

A Jade program is a small, dense thing. Every line carries weight. There is no ceremony, no boilerplate, no scaffolding propping up the real work. What remains after everything unnecessary is stripped away is not minimalism — it is essence.

```jade
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)
```

Three lines. A complete, recursive, pattern-directed definition of the Fibonacci sequence. It compiles to the same machine code as C. There is nothing to add. There is nothing to take away.

---

## II. The First Principle — Performance Is Non-Negotiable

Every design decision in Jade is evaluated against one question:

> *Does this prevent generating the same code C would?*

If the answer is yes, the design is wrong. Not deferred. Not traded off. Wrong.

Jade does not ask you to choose between beauty and speed. That choice is a failure of imagination. A language that reads like pseudocode and runs like C is not a contradiction — it is the only goal worth pursuing.

An integer is a register. A class is contiguous memory at known offsets. A function is a native call. There is no universal value type. No NaN-boxing. No 64-byte wrapper class standing between your intent and the machine.

```
| Benchmark       | Jade   | Clang -O3 | Ratio |
|-----------------|--------|-----------|-------|
| ackermann(3,10) | 186ms  | 202ms     | 0.92× |
| fibonacci(40)   | 339ms  | 336ms     | 1.01× |
| nbody           | 136ms  | 136ms     | 0.99× |
| TOTAL           | 1.21s  | 1.25s     | 0.97× |
```

0.97× C. Not "close to C." Not "almost native." The same backend, the same optimizations, the same registers. The abstraction costs nothing because there is no abstraction — only direct translation from clear thought to fast code.

---

## III. The Asterisk — Functions Are Actions

In Jade, a function begins with `*`. Not `func`. Not `def`. Not `fn`. A single mark — the smallest possible signal that says: *here is something that does work.*

```jade
*greet name
    log 'hello {name}'

*square x is x * x

*max a, b
    a > b ? a ! b
```

The asterisk is a philosophical statement. Keywords are tax. Every character of `function` or `define` that isn't your logic is noise. Jade pays the minimum toll and moves on.

Parentheses are optional — on definitions and calls alike. When the meaning is clear, punctuation is clutter:

```jade
result is add 1, 2
greet 'world'
```

When clarity demands grouping, parentheses are there. They serve you. You do not serve them.

---

## IV. The Binding — `is` Means Equals

```jade
x is 42
name is 'jade'
pi is 3.14159
```

`is`. Not `=`. Not `:=`. Not `let`. The most natural word in the English language for associating a name with a value.

A Jade program reads the way you would explain it to another person. `x is 42`. `name is 'jade'`. There is no translation step between thought and code, between intent and expression. You think it, you write it, and the machine does it — at the speed of C.

---

## V. Words Over Symbols

```jade
if x equals 0
    log 'zero'

if x not equals y
    log 'different'

if a and b
    log 'both'

if done or timeout
    log 'stop'
```

`equals`. `not equals`. `and`. `or`. `not`. These are not syntactic sugar over operators. They *are* the operators. Jade chooses English where English is clearer, and symbols where symbols are clearer.

`~` for pipelines — because data flows. `?` `!` for ternary — because conditionals decide. `*` for functions — because functions act. `%` for address-of — because pointers point. `@` for dereference — because you reach through.

The rule is simple: **use the form that carries the most meaning in the fewest characters.** Sometimes that is a word. Sometimes it is a symbol. Jade does not have a dogma. It has taste.

---

## VI. The Pipeline — Data Flows Forward

```jade
result is value ~ double ~ add_one ~ square
```

The tilde `~` sends data forward. Left to right. The way you read. The way you think. No nesting, no inside-out evaluation, no mental stack of closing parentheses.

Compare:

```
square(add_one(double(value)))    — read inside out
value ~ double ~ add_one ~ square — read left to right
```

Pipelines are not decoration. They are how humans process transformation. *Start with this. Do this to it. Then this. Then this.* The code mirrors the thought.

---

## VII. The Ternary — Decisions in a Breath

```jade
sign is x > 0 ? 1 ! -1
```

Condition. Question mark — *what if yes?* Exclamation mark — *otherwise!* A complete conditional expression in one line, in one breath, with one thought.

```jade
grade is score > 90 ? 'A'
    ! score > 80 ? 'B'
    ! score > 70 ? 'C'
    ! 'F'
```

Nesting reads naturally. Each `!` is an *otherwise*. The structure cascades like a decision tree, indented to show its logic, compact enough to hold in your mind.

---

## VIII. Pattern Direction — Functions That Know Their Shape

```jade
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)

*gcd(a, 0) is a
*gcd a, b is gcd b, a mod b

*fact(0) is 1
*fact n is n * fact(n - 1)
```

Define a function multiple times. Each definition handles a shape. The compiler merges them into efficient dispatch — the conditional logic you would have written by hand, but didn't have to.

This is not overloading. This is mathematical definition. The same way you would write it on a whiteboard, the same way it appears in a textbook. Jade does not invent a new way to express old ideas. It honors them.

---

## IX. Ownership Without Ceremony

Jade manages memory. You do not.

One owner per value. The compiler inserts drops statically. Reads borrow — zero cost, no retain, no release. When sharing is needed, the compiler infers it and inserts reference counting automatically.

There are no lifetime annotations. No `'a` plastered across your function signatures. No `Rc<RefCell<Box<T>>>` tower of indirection. No `unsafe` blocks to escape a system that became its own obstacle.

You write:

```jade
a is 'hello'
b is a
b is b + ' world'
```

The compiler sees: `a` is created. `b` shares `a` — no copy needed. `b` mutates — copy-on-write triggers, `b` gets its own allocation. `a` is untouched. When both fall out of scope, both are freed. Precisely. Deterministically. Without your involvement.

**Perceus reference counting** — nine optimization passes that analyze your program and eliminate every retain/release that isn't strictly necessary. Borrows are elided. Drops are fused. In-place reuse replaces allocation when the layout matches and the count is one.

The result: memory safety with zero annotations and near-zero overhead. The machine handles the bookkeeping because the machine is better at bookkeeping than you are. Your job is to think clearly. Jade's job is to make that thought run fast and safe.

---

## X. Inference Does the Work

```jade
*add a, b
    a + b
```

No type annotations. The compiler knows `a` and `b` are `i64` because `+` operates on integers and the call site provides integers. Hindley-Milner unification plus bidirectional flow plus ownership inference — the full weight of type theory working silently so you can write code that looks like it has no types but is checked as rigorously as any statically-typed language.

When you want to be explicit, you can:

```jade
*connect(host as String, port as i64 is 8080)
    ...
```

`as` reads naturally. `host as String` — *host, which is a String*. Types serve documentation when you choose to write them. They are never demanded as tax.

---

## XI. Indentation Is Structure

```jade
if x > 0
    log 'positive'
    process x
else
    log 'non-positive'
```

No braces. No `end`. No `fi`. The indentation you write for readability *is* the structure. There is no divergence between what the code looks like and what it means.

This is not controversial. This is honest. Every programmer indents their code. Jade simply refuses to make you say the same thing twice — once with whitespace for humans, once with delimiters for the machine.

---

## XII. Concurrency as Natural as Functions

### Actors speak in messages.

```jade
actor Counter
    count is 0

    @increment amount
        count is count + amount

    @get_count
        count

c is spawn Counter
c.increment(5)
```

### Channels carry values.

```jade
ch is channel of i64(10)
send ch, 42
val is receive ch
```

### Parallel loops just work.

```jade
sim for x in items
    process x
```

No thread pools to configure. No mutex to forget. No data race to debug at 3 AM. Actors own their state. Channels connect them. `sim for` distributes independent iterations across cores. The dangerous parts of concurrency are not exposed because they shouldn't be.

---

## XIII. Systems Depth

Jade is not a scripting language wearing a systems costume. It goes all the way down.

```jade
extern *printf(fmt as %i8, ...) returns i32

ptr is %value
val is @ptr

asm
    nop

type Pixel @packed
    r as u8
    g as u8
    b as u8

type CacheAligned @align(64)
    data as [u8; 64]
```

Extern C functions. Raw pointers. Inline assembly. Packed structs. Cache-line alignment. Volatile memory operations. Direct system calls. Separate compilation. Object file emission. DWARF debug info.

Every tool a systems programmer needs exists. Not as an afterthought bolted onto a high-level language, but as a natural extension of a language that was born to compile to the metal.

---

## XIV. The Standard Library — Batteries, Not Bloat

```jade
use fs
use json
use crypto
use regex
use net
```

File systems. JSON. Cryptography. Regular expressions. Networking. CSV. HTTP. Time. Random numbers. Sorting. Statistics. Path manipulation. Signal handling. Processes.

Each module is a `.jade` file. No hidden magic. No special compiler support. The same language you write your programs in is the language the standard library is written in.

---

## XV. Errors Are Values

```jade
err FileError
    NotFound
    PermissionDenied(String)

*read path
    if path equals ''
        ! FileError:NotFound
    'contents'
```

No exceptions. No stack unwinding. No hidden control flow. An error is a value. You return it with `!`. You match on it. You handle it or you propagate it. The compiler sees every path.

---

## XVI. The Compiler Is the Product

```
Source → Lexer → Parser → AST → Typer → HIR → Perceus → Ownership → Codegen → LLVM → Native
```

One command. One binary. No runtime. No garbage collector. No virtual machine. No JIT warmup. The compiler transforms your source into a native executable that depends on nothing but the operating system.

```bash
jadec hello.jade -o hello
./hello
```

The output is a native binary. It starts instantly. It runs at full speed. It links against libc and nothing else. It can be copied to another machine and it works. Software should be this simple.

---

## XVII. The Ethos

Jade is guided by a set of convictions, not a committee:

1. **Clarity is not the enemy of performance.** The belief that readable code must be slow is a myth perpetuated by languages that made poor tradeoffs.

2. **The compiler should work harder than the programmer.** Type inference, ownership inference, drop insertion, borrow elision, reuse analysis — these are the compiler's job, not yours.

3. **Syntax should disappear.** The best syntax is the one you stop noticing. It should feel like transcribing thought, not translating it.

4. **Every feature must earn its place.** If it cannot be justified against the question *"does this make the common case clearer without making the machine slower?"* — it does not belong.

5. **Complexity is debt.** Every knob, every annotation, every special case is a cost. Jade pays that cost in the compiler so you don't pay it in your code.

6. **One way to do it.** Not enforced rigidly — but Jade gravitates toward single, clear idioms rather than offering five syntactic paths to the same result.

7. **Code is read more than it is written.** Every syntactic choice — `is`, `equals`, `and`, `~`, `?!` — was made for the reader, not the writer.

---

## XVIII. What Jade Is Not

Jade is not a research language. It ships binaries.

Jade is not a safe language that forgot about performance. It is a fast language that refuses to be unsafe.

Jade is not minimal for the sake of minimalism. Every feature that exists is load-bearing. Every feature that doesn't exist was weighed and found wanting.

Jade is not trying to be everything to everyone. It is a systems language with scripting readability and C performance. That sentence is the entire design space. Everything flows from it.

---

## XIX. The Invitation

Write a function. Compile it. Run it. Read the LLVM IR if you want — it looks like C wrote it. Time it against C if you doubt it — the numbers don't lie.

Then write something larger. Notice how the types disappear but the safety doesn't. Notice how the memory management vanishes but the determinism remains. Notice how the syntax gets out of your way and all that's left is the algorithm, the logic, the *thought*.

That is the way of Jade.

Not a language that makes hard things easy — a language that makes hard things *clear*.

---

*Hard. Dense. Beautiful.*
