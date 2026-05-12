#!/usr/bin/env bash
# Generate 100 Jinn snippets at /tmp/jinn_snippets/s001..s100.jn
cd /tmp/jinn_snippets
rm -f s*.jn

write() { local n=$1; shift; cat > "$(printf 's%03d.jn' "$n")"; }

# ─── 1-10: bindings, literals, arithmetic ─────────────────
write 1 <<'EOF'
*main
    x is 42
    log x
EOF

write 2 <<'EOF'
*main
    pi is 3.14159
    log pi
EOF

write 3 <<'EOF'
*main
    a is 10
    b is 3
    log a + b
    log a - b
    log a * b
    log a / b
    log a % b
EOF

write 4 <<'EOF'
*main
    x is 5
    x += 3
    x -= 1
    x *= 2
    log x
EOF

write 5 <<'EOF'
*main
    x is 2 pow 10
    log x
EOF

write 6 <<'EOF'
*main
    a is 0xFF
    b is 0b1010
    log a
    log b
EOF

write 7 <<'EOF'
*main
    x is 0xF0
    log x & 0x0F
    log x | 0x0F
    log x ^ 0xFF
    log x << 2
    log x >> 4
EOF

write 8 <<'EOF'
*main
    a as i32 is 100
    b as f64 is 1.5
    log a
    log b
EOF

write 9 <<'EOF'
*main
    big is 1000000
    n is big as f64
    log n
EOF

write 10 <<'EOF'
*main
    MAX_SIZE is 1024
    log MAX_SIZE
EOF

# ─── 11-20: strings ──────────────────────────────────────
write 11 <<'EOF'
*main
    s is 'hello'
    log s
EOF

write 12 <<'EOF'
*main
    name is 'world'
    log 'hello {name}'
EOF

write 13 <<'EOF'
*main
    x is 7
    log 'x={x} squared={x * x}'
EOF

write 14 <<'EOF'
*main
    s is 'hello world'
    log s.length
    log s.to_upper()
    log s.to_lower()
EOF

write 15 <<'EOF'
*main
    s is '  spaced  '
    log s.trim()
EOF

write 16 <<'EOF'
*main
    s is 'hello'
    log s.contains('ell')
    log s.starts_with('he')
    log s.ends_with('lo')
EOF

write 17 <<'EOF'
*main
    s is 'a,b,c,d'
    parts is s.split(',')
    log parts.length
EOF

write 18 <<'EOF'
*main
    s is 'foo bar baz'
    log s.replace('bar', 'XXX')
EOF

write 19 <<'EOF'
*main
    a is 'hello'
    b is 'world'
    log a + ' ' + b
EOF

write 20 <<'EOF'
*main
    s is 'abc'
    log s.repeat(3)
EOF

# ─── 21-30: conditionals & comparison ───────────────────
write 21 <<'EOF'
*main
    x is 10
    if x > 5
        log 'big'
    else
        log 'small'
EOF

write 22 <<'EOF'
*main
    x is 0
    if x > 0
        log 'pos'
    elif x equals 0
        log 'zero'
    else
        log 'neg'
EOF

write 23 <<'EOF'
*main
    x is 5
    sign is x > 0 ? 'pos' ! 'neg'
    log sign
EOF

write 24 <<'EOF'
*main
    score is 85
    grade is score > 90 ? 'A' ! score > 80 ? 'B' ! 'C'
    log grade
EOF

write 25 <<'EOF'
*main
    x is 50
    if 0 < x < 100
        log 'in range'
EOF

write 26 <<'EOF'
*main
    a is 5
    b is 5
    if a equals b
        log 'eq'
    if a neq 6
        log 'neq6'
EOF

write 27 <<'EOF'
*main
    a is true
    b is false
    if a and not b
        log 'yes'
    if a or b
        log 'or'
    if a xor b
        log 'xor'
EOF

write 28 <<'EOF'
*main
    x is 7
    if x in [1, 3, 5, 7]
        log 'in list'
EOF

write 29 <<'EOF'
*main
    if 'world' in 'hello world'
        log 'substr'
EOF

write 30 <<'EOF'
*main
    x is -5
    abs_x is x >= 0 ? x ! 0 - x
    log abs_x
EOF

# ─── 31-40: loops ────────────────────────────────────────
write 31 <<'EOF'
*main
    n is 5
    while n > 0
        log n
        n is n - 1
EOF

write 32 <<'EOF'
*main
    for i from 0 to 5
        log i
EOF

write 33 <<'EOF'
*main
    for i in 1 to 4
        log i
EOF

write 34 <<'EOF'
*main
    for i from 0 to 10 by 2
        log i
EOF

write 35 <<'EOF'
*main
    n is 0
    loop
        n is n + 1
        if n > 3
            break
    log n
EOF

write 36 <<'EOF'
*main
    sum is 0
    for i from 1 to 11
        sum is sum + i
    log sum
EOF

write 37 <<'EOF'
*main
    for i from 0 to 5
        if i equals 2
            continue
        log i
EOF

write 38 <<'EOF'
*main
    outer is for i from 0 to 5
        for j from 0 to 5
            if i * j > 6
                break outer
            log i * 10 + j
EOF

write 39 <<'EOF'
*main
    items is [10, 20, 30]
    loop items
        log $
EOF

write 40 <<'EOF'
*main
    items is [10, 20, 30]
    loop items
        log '{$$}: {$}'
EOF

# ─── 41-50: functions ───────────────────────────────────
write 41 <<'EOF'
*add a, b
    a + b

*main
    log add(2, 3)
EOF

write 42 <<'EOF'
*square x is x * x

*main
    log square(7)
EOF

write 43 <<'EOF'
*greet name as String
    log 'hi {name}'

*main
    greet 'jinn'
EOF

write 44 <<'EOF'
*connect(host as String, port as i64 is 8080)
    log '{host}:{port}'

*main
    connect('localhost')
    connect('example.com', 443)
EOF

write 45 <<'EOF'
*double x is x * 2

*main
    log double of 21
EOF

write 46 <<'EOF'
*fib(0) is 0
*fib(1) is 1
*fib n is fib(n - 1) + fib(n - 2)

*main
    log fib(10)
EOF

write 47 <<'EOF'
*fact(0) is 1
*fact n
    n * fact(n - 1)

*main
    log fact(6)
EOF

write 48 <<'EOF'
*gcd(a, 0) is a
*gcd a, b
    gcd(b, a % b)

*main
    log gcd(48, 18)
EOF

write 49 <<'EOF'
*max of T(a as T, b as T)
    a > b ? a ! b

*main
    log max(3, 7)
    log max(1.5, 2.5)
EOF

write 50 <<'EOF'
*apply(f as (i64) returns i64, x as i64)
    f(x)

*main
    double is |x as i64| x * 2
    log apply(double, 21)
EOF

# ─── 51-60: vectors / collections ───────────────────────
write 51 <<'EOF'
*main
    v is vector()
    v.push(1)
    v.push(2)
    v.push(3)
    log v.length
EOF

write 52 <<'EOF'
*main
    v is vector[10, 20, 30]
    print(v)
EOF

write 53 <<'EOF'
*main
    v is [1, 2, 3, 4, 5]
    log v[2]
    log v.length
EOF

write 54 <<'EOF'
*main
    v is vector[1, 2, 3]
    v.push(4)
    log v.length
    log v.pop()
    log v.length
EOF

write 55 <<'EOF'
*main
    v is vector[5, 2, 8, 1, 9, 3]
    sorted is v.sort()
    print(sorted)
EOF

write 56 <<'EOF'
*main
    v is vector[1, 2, 3, 4, 5]
    total is v.sum()
    log total
EOF

write 57 <<'EOF'
*main
    v is vector[1, 2, 3, 4, 5]
    doubled is v.map($ * 2)
    print(doubled)
EOF

write 58 <<'EOF'
*main
    v is vector[1, 2, 3, 4, 5, 6]
    evens is v.filter($ % 2 equals 0)
    print(evens)
EOF

write 59 <<'EOF'
*main
    v is vector[1, 2, 3, 4]
    total is v.fold(0, |acc, x| acc + x)
    log total
EOF

write 60 <<'EOF'
*main
    v is vector[1, 2, 3]
    log v.contains(2)
    log v.contains(99)
EOF

# ─── 61-70: maps & comprehensions ───────────────────────
write 61 <<'EOF'
*main
    m is map()
    m.set('a', 1)
    m.set('b', 2)
    log m.get('a')
    log m.has('b')
EOF

write 62 <<'EOF'
*main
    sq is [x * x for x in 0 to 6]
    print(sq)
EOF

write 63 <<'EOF'
*main
    evens is [x for x in 0 to 20 if x % 2 equals 0]
    print(evens)
EOF

write 64 <<'EOF'
*main
    a is [1, 2, 3]
    b is [4, 5, 6]
    c is a.zip(b)
    log c.length
EOF

write 65 <<'EOF'
*main
    v is vector[1, 2, 3, 4, 5]
    has_neg is v.any($ < 0)
    has_all_pos is v.all($ > 0)
    log has_neg
    log has_all_pos
EOF

write 66 <<'EOF'
*main
    v is vector[1, 2, 3, 4]
    found is v.find($ equals 3)
    log found
EOF

write 67 <<'EOF'
*main
    v is vector[1, 2, 3]
    rev is v.reverse()
    print(rev)
EOF

write 68 <<'EOF'
*main
    v is vector['a', 'b', 'c']
    log v.join(',')
EOF

write 69 <<'EOF'
*main
    v is vector[10, 20, 30, 40, 50]
    s is v from 1 to 4
    print(s)
EOF

write 70 <<'EOF'
*main
    v is vector[1, 2, 3, 4]
    t is v.take(2)
    sk is v.skip(2)
    print(t)
    print(sk)
EOF

# ─── 71-80: classes / enums / methods ───────────────────
write 71 <<'EOF'
type Point
    x as i64
    y as i64

*main
    p is Point(x is 3, y is 4)
    log p.x
    log p.y
EOF

write 72 <<'EOF'
type Vec3
    x as i64
    y as i64
    z as i64

    *sum()
        x + y + z

*main
    v is Vec3(x is 1, y is 2, z is 3)
    log v.sum()
EOF

write 73 <<'EOF'
type Counter
    count as i64

    *bump(self)
        self.count + 1

*main
    c is Counter(count is 5)
    log c.bump()
EOF

write 74 <<'EOF'
enum Shape
    Circle(f64)
    Rect(f64, f64)

*area s as Shape
    match s
        Circle(r) ? 3.14 * r * r
        Rect(w, h) ? w * h

*main
    log area(Circle(2.0))
    log area(Rect(3.0, 4.0))
EOF

write 75 <<'EOF'
enum Color
    Red
    Green
    Blue

*name c as Color
    match c
        Red ? 'red'
        Green ? 'green'
        Blue ? 'blue'

*main
    log name(Green)
EOF

write 76 <<'EOF'
type Pair of A, B
    first as A
    second as B

*main
    p is Pair(first is 1, second is 'hi')
    log p.first
    log p.second
EOF

write 77 <<'EOF'
enum Option of T
    Some(T)
    None

*unwrap_or(o as Option of i64, d as i64)
    match o
        Some(v) ? v
        None ? d

*main
    log unwrap_or(Some(42), 0)
    log unwrap_or(None, -1)
EOF

write 78 <<'EOF'
type Celsius
    value as f64

type Fahrenheit
    value as f64

*c_to_f(c as Celsius) returns Fahrenheit
    Fahrenheit(value is c.value * 1.8 + 32.0)

*main
    c is Celsius(value is 100.0)
    f is c_to_f(c)
    log f.value
EOF

write 79 <<'EOF'
enum Permission
    Read is 1
    Write is 2
    Execute is 4

*main
    log Read as i64
    log Write as i64
EOF

write 80 <<'EOF'
type Box
    val as i64
    LABEL is 99

*main
    b is Box(val is 5)
    log b.val
    log b.LABEL
EOF

# ─── 81-90: pipelines, lambdas, errors, defer ───────────
write 81 <<'EOF'
*double x is x * 2
*inc x is x + 1

*main
    r is 10 ~ double ~ inc
    log r
EOF

write 82 <<'EOF'
*main
    f is |x| x + 100
    log f(5)
EOF

write 83 <<'EOF'
*main
    items is [1, 2, 3, 4]
    doubled is items ~ |x| x * 2
    print(doubled)
EOF

write 84 <<'EOF'
err Outcome
    Ok(i64)
    Bad

*compute(x as i64) returns Outcome
    if x equals 0
        ! Bad
    Ok(x + 1)

*main
    r is compute(5)
    match r
        Ok(v) ? log v
        Bad ? log -1
EOF

write 85 <<'EOF'
*main
    defer
        log 'cleanup'
    log 'work'
EOF

write 86 <<'EOF'
*lookup k as String
    if k equals 'missing'
        ! -1
    42

*main
    log lookup('missing')
    log lookup('found')
EOF

write 87 <<'EOF'
*main
    assert 1 + 1 equals 2
    log 'ok'
EOF

write 88 <<'EOF'
*main
    n is 42
    s is to_string(n)
    log s
    log s.length
EOF

write 89 <<'EOF'
*main
    t is time_now()
    log t > 0
EOF

write 90 <<'EOF'
*main
    x is -3.5
    log x.abs()
    log x.floor()
    log x.ceil()
EOF

# ─── 91-100: stdlib / advanced ──────────────────────────
write 91 <<'EOF'
use math

*main
    log math.sin(0.0)
    log math.sqrt(16.0)
EOF

write 92 <<'EOF'
use regex

*main
    log regex.is_match('abc123', '[0-9]+')
    log regex.find('hello42world', '[0-9]+')
EOF

write 93 <<'EOF'
use json

*main
    s is '{"x": 1, "y": 2}'
    j is json.parse(s)
    log j.get('x')
EOF

write 94 <<'EOF'
use random

*main
    r is random.int(0, 100)
    log r >= 0
EOF

write 95 <<'EOF'
*counter()
    n is 0
    loop
        yield n
        n is n + 1

*main
    g is counter()
    log g.next()
    log g.next()
    log g.next()
EOF

write 96 <<'EOF'
actor Tally
    count is 0

    @add n
        count is count + n

    @show
        log count

*main
    t is spawn Tally
    t.add(5)
    t.add(7)
    t.show
EOF

write 97 <<'EOF'
*main
    ch is channel of i64(4)
    send ch, 1
    send ch, 2
    a is receive ch
    b is receive ch
    log a
    log b
EOF

write 98 <<'EOF'
*main
    m is 3 by 3
    log m[1][1]
    n is m + 1.0
    log n[0][0]
EOF

write 99 <<'EOF'
*main
    popcount_x is popcount(0xFF)
    clz_x is clz(1)
    log popcount_x
    log clz_x
EOF

write 100 <<'EOF'
*main
    items is [3, 1, 4, 1, 5, 9, 2, 6]
    total is items.fold(0, |acc, x| acc + x)
    avg is total / items.length
    log 'sum={total} avg={avg}'
EOF

ls s*.jn | wc -l
