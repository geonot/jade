#!/usr/bin/env python3
"""Generate 100 more-complex Jinn snippets s101..s200."""
import os
out = "/tmp/jinn_snippets2"
os.makedirs(out, exist_ok=True)

snippets = {}

snippets[106] = '''# Quicksort (Lomuto partition).
*partition(v, lo as i64, hi as i64) returns i64
    pivot is v.get(hi)
    i is lo - 1
    j is lo
    while j < hi
        if v.get(j) <= pivot
            i is i + 1
            t is v.get(i)
            v.set(i, v.get(j))
            v.set(j, t)
        j is j + 1
    t is v.get(i + 1)
    v.set(i + 1, v.get(hi))
    v.set(hi, t)
    i + 1

*qsort(v, lo as i64, hi as i64)
    if lo < hi
        p is partition(v, lo, hi)
        qsort(v, lo, p - 1)
        qsort(v, p + 1, hi)

*main
    a is vec(8, 3, 1, 7, 0, 10, 2, 5, 4, 9, 6)
    qsort(a, 0, a.length - 1)
    for x in a
        log x
'''

snippets[107] = '''# Merge sort on Vec of i64.
*merge(left, right) returns Vec of i64
    out is vec()
    i is 0
    j is 0
    while i < left.length and j < right.length
        a is left.get(i)
        b is right.get(j)
        if a <= b
            out.push(a)
            i is i + 1
        else
            out.push(b)
            j is j + 1
    while i < left.length
        out.push(left.get(i))
        i is i + 1
    while j < right.length
        out.push(right.get(j))
        j is j + 1
    out

*msort(v) returns Vec of i64
    if v.length <= 1
        return v
    mid is v.length / 2
    left is vec()
    right is vec()
    i is 0
    while i < v.length
        if i < mid
            left.push(v.get(i))
        else
            right.push(v.get(i))
        i is i + 1
    merge(msort(left), msort(right))

*main
    a is vec(5, 2, 8, 1, 9, 3, 7, 4, 6)
    s is msort(a)
    for x in s
        log x
'''

snippets[108] = '''# GCD via Euclid.
*gcd(a as i64, b as i64) returns i64
    while b != 0
        t is b
        b is a mod b
        a is t
    a

*main
    log gcd(48, 18)
    log gcd(1071, 462)
    log gcd(1000000, 999983)
'''

snippets[109] = '''# LCM via gcd.
*gcd(a as i64, b as i64) returns i64
    while b != 0
        t is b
        b is a mod b
        a is t
    a

*lcm(a as i64, b as i64) returns i64
    (a / gcd(a, b)) * b

*main
    log lcm(12, 18)
    log lcm(7, 13)
'''

snippets[110] = '''# Power of two via bit ops.
*is_pow2(n as i64) returns bool
    n > 0 and (n & (n - 1)) equals 0

*main
    log is_pow2(1)
    log is_pow2(2)
    log is_pow2(3)
    log is_pow2(1024)
'''

snippets[111] = '''# Population count (Brian Kernighan).
*popcount(n as i64) returns i64
    c is 0
    while n != 0
        n is n & (n - 1)
        c is c + 1
    c

*main
    log popcount(0)
    log popcount(7)
    log popcount(255)
    log popcount(1023)
'''

snippets[112] = '''# Reverse bits in 64-bit int.
*revbits(n as i64) returns i64
    r is 0
    i is 0
    while i < 64
        r is (r << 1) | (n & 1)
        n is n >> 1
        i is i + 1
    r

*main
    log revbits(1)
'''

snippets[113] = '''# Sum digits.
*digit_sum(n as i64) returns i64
    if n < 0
        n is 0 - n
    s is 0
    while n > 0
        s is s + (n mod 10)
        n is n / 10
    s

*main
    log digit_sum(12345)
    log digit_sum(99999)
'''

snippets[114] = '''# Reverse digits of an i64.
*rev(n as i64) returns i64
    r is 0
    while n > 0
        r is r * 10 + n mod 10
        n is n / 10
    r

*main
    log rev(12345)
'''

snippets[115] = '''# Palindrome int check.
*is_palindrome(n as i64) returns bool
    if n < 0
        return false
    orig is n
    r is 0
    while n > 0
        r is r * 10 + n mod 10
        n is n / 10
    r equals orig

*main
    log is_palindrome(12321)
    log is_palindrome(12345)
'''

snippets[116] = '''# Collatz step count.
*collatz(n as i64) returns i64
    c is 0
    while n != 1
        if n mod 2 equals 0
            n is n / 2
        else
            n is 3 * n + 1
        c is c + 1
    c

*main
    log collatz(27)
'''

snippets[117] = '''# Newton-Raphson square root for f64.
*sqrt(x as f64) returns f64
    if x <= 0.0
        return 0.0
    g is x
    i is 0
    while i < 30
        g is (g + x / g) / 2.0
        i is i + 1
    g

*main
    log sqrt(2.0)
    log sqrt(16.0)
'''

snippets[118] = '''# Power via repeated squaring.
*ipow(base as i64, exp as i64) returns i64
    r is 1
    while exp > 0
        if exp & 1 equals 1
            r is r * base
        base is base * base
        exp is exp >> 1
    r

*main
    log ipow(2, 10)
    log ipow(3, 7)
'''

snippets[119] = '''# Modular exponentiation.
*modpow(base as i64, exp as i64, m as i64) returns i64
    r is 1
    base is base mod m
    while exp > 0
        if exp & 1 equals 1
            r is (r * base) mod m
        base is (base * base) mod m
        exp is exp >> 1
    r

*main
    log modpow(7, 100, 13)
    log modpow(2, 1000, 1000000007)
'''

snippets[120] = '''# Two-sum: return indices of two numbers that sum to target. -1,-1 if none.
*two_sum(nums, target as i64)
    i is 0
    while i < nums.length
        j is i + 1
        while j < nums.length
            if nums.get(i) + nums.get(j) equals target
                log i
                log j
                return
            j is j + 1
        i is i + 1
    log 0 - 1

*main
    a is vec(2, 7, 11, 15)
    two_sum(a, 9)
'''

# Continue with more snippets...
snippets[121] = '''# Maximum subarray (Kadane).
*max_subarray(v) returns i64
    best is v.get(0)
    cur is v.get(0)
    i is 1
    while i < v.length
        x is v.get(i)
        if cur + x > x
            cur is cur + x
        else
            cur is x
        if cur > best
            best is cur
        i is i + 1
    best

*main
    a is vec(0 - 2, 1, 0 - 3, 4, 0 - 1, 2, 1, 0 - 5, 4)
    log max_subarray(a)
'''

snippets[122] = '''# Counting sort for bytes.
*csort(v) returns Vec of i64
    cnt is vec()
    i is 0
    while i < 256
        cnt.push(0)
        i is i + 1
    j is 0
    while j < v.length
        x is v.get(j)
        cnt.set(x, cnt.get(x) + 1)
        j is j + 1
    out is vec()
    k is 0
    while k < 256
        c is cnt.get(k)
        while c > 0
            out.push(k)
            c is c - 1
        k is k + 1
    out

*main
    a is vec(5, 200, 17, 5, 200, 99, 17)
    s is csort(a)
    for x in s
        log x
'''

snippets[123] = '''# Reverse a vec in place.
*rev(v)
    i is 0
    j is v.length - 1
    while i < j
        t is v.get(i)
        v.set(i, v.get(j))
        v.set(j, t)
        i is i + 1
        j is j - 1

*main
    a is vec(1, 2, 3, 4, 5)
    rev(a)
    for x in a
        log x
'''

snippets[124] = '''# Rotate vec by k positions.
*rotate(v, k as i64) returns Vec of i64
    n is v.length
    if n equals 0
        return v
    k is k mod n
    out is vec()
    i is 0
    while i < n
        out.push(v.get((i + n - k) mod n))
        i is i + 1
    out

*main
    a is vec(1, 2, 3, 4, 5)
    r is rotate(a, 2)
    for x in r
        log x
'''

snippets[125] = '''# Linked list as struct of vec slots.
struct Node
    val as i64
    next as i64

*main
    nodes is vec()
    nodes.push(Node(val is 1, next is 1))
    nodes.push(Node(val is 2, next is 2))
    nodes.push(Node(val is 3, next is 0 - 1))
    i is 0
    while i >= 0
        n is nodes.get(i)
        log n.val
        i is n.next
'''

snippets[126] = '''# Stack via vec.
*main
    s is vec()
    s.push(1)
    s.push(2)
    s.push(3)
    while s.length > 0
        log s.pop()
'''

snippets[127] = '''# Queue via two stacks.
*main
    inq is vec()
    outq is vec()
    inq.push(1)
    inq.push(2)
    inq.push(3)
    while inq.length > 0
        outq.push(inq.pop())
    while outq.length > 0
        log outq.pop()
'''

snippets[128] = '''# Polynomial evaluation (Horner).
*horner(coeffs, x as f64) returns f64
    r is 0.0
    i is 0
    while i < coeffs.length
        r is r * x + coeffs.get(i)
        i is i + 1
    r

*main
    c is vec(1.0, 0.0, 0 - 2.0, 1.0)
    log horner(c, 2.0)
'''

snippets[129] = '''# Matrix multiplication 2x2 via Vec of Vec.
*main
    a is vec(vec(1, 2), vec(3, 4))
    b is vec(vec(5, 6), vec(7, 8))
    c is vec(vec(0, 0), vec(0, 0))
    i is 0
    while i < 2
        j is 0
        while j < 2
            s is 0
            k is 0
            while k < 2
                s is s + a.get(i).get(k) * b.get(k).get(j)
                k is k + 1
            c.get(i).set(j, s)
            j is j + 1
        i is i + 1
    for row in c
        for x in row
            log x
'''

snippets[130] = '''# Closure capturing a counter.
*make_counter()
    c is 0
    $($
        c is c + 1
        c
    )

*main
    f is make_counter()
    log f()
    log f()
    log f()
'''

snippets[131] = '''# Higher-order map via closures over Vec.
*main
    a is vec(1, 2, 3, 4, 5)
    b is a.map($($ * $))
    for x in b
        log x
'''

snippets[132] = '''# Filter even numbers.
*main
    a is vec(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    b is a.filter($($ mod 2 equals 0))
    for x in b
        log x
'''

snippets[133] = '''# Reduce to product.
*main
    a is vec(1, 2, 3, 4, 5)
    p is a.reduce(1, $$($$ * $))
    log p
'''

snippets[134] = '''# Sum & average via fold.
*main
    a is vec(10.0, 20.0, 30.0, 40.0, 50.0)
    s is 0.0
    for x in a
        s is s + x
    log s
    log s / 5.0
'''

snippets[135] = '''# Word frequency via map.
use map_mod

*main
    words is vec("a", "b", "a", "c", "a", "b")
    counts is map()
    for w in words
        counts.set(w, 1)
    log counts.has("a")
    log counts.has("z")
'''

snippets[136] = '''# Multiple return via tuple-like struct.
struct MinMax
    lo as i64
    hi as i64

*find_min_max(v) returns MinMax
    lo is v.get(0)
    hi is v.get(0)
    i is 1
    while i < v.length
        x is v.get(i)
        if x < lo
            lo is x
        if x > hi
            hi is x
        i is i + 1
    MinMax(lo is lo, hi is hi)

*main
    a is vec(3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5)
    r is find_min_max(a)
    log r.lo
    log r.hi
'''

snippets[137] = '''# Concrete Result enum.
enum Res
    Ok(i64)
    Err

*divide(a as i64, b as i64) returns Res
    if b equals 0
        return Err
    Ok(a / b)

*main
    match divide(10, 2)
        Ok(v) ? log v
        Err   ? log 0 - 1
    match divide(10, 0)
        Ok(v) ? log v
        Err   ? log 0 - 1
'''

snippets[138] = '''# String split on space, count tokens.
*main
    s is "the quick brown fox jumps over the lazy dog"
    parts is s.split(" ")
    log parts.length
'''

snippets[139] = '''# Reverse a string by iterating bytes.
*reverse_str(s as String) returns String
    out is ""
    i is s.length - 1
    while i >= 0
        out is out + s.slice(i, i + 1)
        i is i - 1
    out

*main
    log reverse_str("jinn")
    log reverse_str("hello world")
'''

snippets[140] = '''# Caesar cipher (ASCII only).
*shift(s as String, k as i64) returns String
    out is ""
    i is 0
    while i < s.length
        c is s.char_at(i)
        if c >= 97 and c <= 122
            c is ((c - 97 + k) mod 26) + 97
        out is out + s.slice(i, i + 1)
        i is i + 1
    out

*main
    log shift("hello", 3)
'''

snippets[141] = '''# Trim leading spaces.
*ltrim(s as String) returns String
    i is 0
    while i < s.length and s.char_at(i) equals 32
        i is i + 1
    s.slice(i, s.length)

*main
    log ltrim("    jinn")
    log ltrim("nope")
'''

snippets[142] = '''# Count vowels in a string.
*is_vowel(c as i64) returns bool
    if c equals 97
        return true
    if c equals 101
        return true
    if c equals 105
        return true
    if c equals 111
        return true
    if c equals 117
        return true
    false

*count_vowels(s as String) returns i64
    n is 0
    i is 0
    while i < s.length
        if is_vowel(s.char_at(i))
            n is n + 1
        i is i + 1
    n

*main
    log count_vowels("the quick brown fox")
'''

snippets[143] = '''# Run-length encoding via count of repeats.
*main
    s is "aaabbcdddde"
    i is 0
    while i < s.length
        j is i + 1
        c is s.char_at(i)
        while j < s.length and s.char_at(j) equals c
            j is j + 1
        log s.slice(i, i + 1)
        log j - i
        i is j
'''

snippets[144] = '''# Longest common prefix of two strings.
*lcp(a as String, b as String) returns String
    n is a.length
    if b.length < n
        n is b.length
    i is 0
    while i < n and a.char_at(i) equals b.char_at(i)
        i is i + 1
    a.slice(0, i)

*main
    log lcp("introduction", "internet")
'''

snippets[145] = '''# Prime test (trial division).
*is_prime(n as i64) returns bool
    if n < 2
        return false
    if n < 4
        return true
    if n mod 2 equals 0
        return false
    i is 3
    while i * i <= n
        if n mod i equals 0
            return false
        i is i + 2
    true

*main
    log is_prime(2)
    log is_prime(15)
    log is_prime(97)
'''

snippets[146] = '''# Tower of Hanoi step count.
*hanoi(n as i64) returns i64
    if n <= 0
        return 0
    2 * hanoi(n - 1) + 1

*main
    log hanoi(10)
'''

snippets[147] = '''# Pascal's triangle row.
*row(n as i64) returns Vec of i64
    r is vec()
    r.push(1)
    i is 1
    while i <= n
        r.push(r.get(i - 1) * (n - i + 1) / i)
        i is i + 1
    r

*main
    r is row(8)
    for x in r
        log x
'''

snippets[148] = '''# Catalan numbers via formula.
*cat(n as i64) returns i64
    if n equals 0
        return 1
    r is 1
    i is 0
    while i < n
        r is r * 2 * (2 * i + 1) / (i + 2)
        i is i + 1
    r

*main
    log cat(5)
    log cat(10)
'''

snippets[149] = '''# Triangle area via Heron's formula.
*sqrt(x as f64) returns f64
    if x <= 0.0
        return 0.0
    g is x
    i is 0
    while i < 30
        g is (g + x / g) / 2.0
        i is i + 1
    g

*area(a as f64, b as f64, c as f64) returns f64
    s is (a + b + c) / 2.0
    sqrt(s * (s - a) * (s - b) * (s - c))

*main
    log area(3.0, 4.0, 5.0)
'''

snippets[150] = '''# Compound interest after n years.
*compound(p as f64, r as f64, n as i64) returns f64
    out is p
    i is 0
    while i < n
        out is out * (1.0 + r)
        i is i + 1
    out

*main
    log compound(1000.0, 0.05, 20)
'''

snippets[151] = '''# Greatest digit in n.
*greatest_digit(n as i64) returns i64
    if n < 0
        n is 0 - n
    g is 0
    while n > 0
        d is n mod 10
        if d > g
            g is d
        n is n / 10
    g

*main
    log greatest_digit(31729)
'''

snippets[152] = '''# Spiral matrix traversal (4x4).
*main
    grid is vec()
    n is 4
    i is 0
    while i < n
        row is vec()
        j is 0
        while j < n
            row.push(i * n + j + 1)
            j is j + 1
        grid.push(row)
        i is i + 1
    top is 0
    bot is n - 1
    lft is 0
    rgt is n - 1
    while top <= bot and lft <= rgt
        c is lft
        while c <= rgt
            log grid.get(top).get(c)
            c is c + 1
        top is top + 1
        r is top
        while r <= bot
            log grid.get(r).get(rgt)
            r is r + 1
        rgt is rgt - 1
        if top <= bot
            c is rgt
            while c >= lft
                log grid.get(bot).get(c)
                c is c - 1
            bot is bot - 1
        if lft <= rgt
            r is bot
            while r >= top
                log grid.get(r).get(lft)
                r is r - 1
            lft is lft + 1
'''

snippets[153] = '''# Knapsack 0/1 (DP).
*knap(weights, values, cap as i64) returns i64
    n is weights.length
    dp is vec()
    i is 0
    while i <= n
        row is vec()
        j is 0
        while j <= cap
            row.push(0)
            j is j + 1
        dp.push(row)
        i is i + 1
    i is 1
    while i <= n
        w is weights.get(i - 1)
        v is values.get(i - 1)
        c is 0
        while c <= cap
            best is dp.get(i - 1).get(c)
            if w <= c
                cand is dp.get(i - 1).get(c - w) + v
                if cand > best
                    best is cand
            dp.get(i).set(c, best)
            c is c + 1
        i is i + 1
    dp.get(n).get(cap)

*main
    w is vec(2, 3, 4, 5)
    v is vec(3, 4, 5, 6)
    log knap(w, v, 5)
'''

snippets[154] = '''# Levenshtein edit distance.
*lev(a as String, b as String) returns i64
    m is a.length
    n is b.length
    dp is vec()
    i is 0
    while i <= m
        row is vec()
        j is 0
        while j <= n
            row.push(0)
            j is j + 1
        dp.push(row)
        i is i + 1
    i is 0
    while i <= m
        dp.get(i).set(0, i)
        i is i + 1
    j is 0
    while j <= n
        dp.get(0).set(j, j)
        j is j + 1
    i is 1
    while i <= m
        j is 1
        while j <= n
            cost is 1
            if a.char_at(i - 1) equals b.char_at(j - 1)
                cost is 0
            d1 is dp.get(i - 1).get(j) + 1
            d2 is dp.get(i).get(j - 1) + 1
            d3 is dp.get(i - 1).get(j - 1) + cost
            best is d1
            if d2 < best
                best is d2
            if d3 < best
                best is d3
            dp.get(i).set(j, best)
            j is j + 1
        i is i + 1
    dp.get(m).get(n)

*main
    log lev("kitten", "sitting")
'''

snippets[155] = '''# Compute GCD of a vec via reduce-style fold.
*gcd(a as i64, b as i64) returns i64
    while b != 0
        t is b
        b is a mod b
        a is t
    a

*main
    a is vec(48, 36, 60, 24)
    g is a.get(0)
    i is 1
    while i < a.length
        g is gcd(g, a.get(i))
        i is i + 1
    log g
'''

snippets[156] = '''# Fibonacci modulo m using fast doubling-ish iterative.
*fib_mod(n as i64, m as i64) returns i64
    a is 0
    b is 1
    i is 0
    while i < n
        t is (a + b) mod m
        a is b
        b is t
        i is i + 1
    a

*main
    log fib_mod(100, 1000000007)
'''

snippets[157] = '''# Prefix sums.
*main
    a is vec(1, 2, 3, 4, 5, 6, 7, 8)
    pre is vec()
    pre.push(0)
    i is 0
    while i < a.length
        pre.push(pre.get(i) + a.get(i))
        i is i + 1
    log pre.get(8) - pre.get(2)
    log pre.get(5) - pre.get(0)
'''

snippets[158] = '''# Sliding-window maximum sum of size k.
*max_window(a, k as i64) returns i64
    n is a.length
    if n < k
        return 0
    s is 0
    i is 0
    while i < k
        s is s + a.get(i)
        i is i + 1
    best is s
    while i < n
        s is s + a.get(i) - a.get(i - k)
        if s > best
            best is s
        i is i + 1
    best

*main
    a is vec(2, 1, 5, 1, 3, 2)
    log max_window(a, 3)
'''

snippets[159] = '''# Two-pointer pair sum on sorted vec.
*main
    a is vec(1, 2, 4, 7, 11, 15)
    target is 13
    i is 0
    j is a.length - 1
    while i < j
        s is a.get(i) + a.get(j)
        if s equals target
            log a.get(i)
            log a.get(j)
            return
        if s < target
            i is i + 1
        else
            j is j - 1
    log 0 - 1
'''

snippets[160] = '''# Closure as predicate.
*main
    a is vec(3, 1, 4, 1, 5, 9, 2, 6)
    threshold is 4
    big is a.filter($($ > threshold))
    for x in big
        log x
'''

snippets[161] = '''# String concatenation via fold.
*main
    parts is vec("hello", " ", "from", " ", "jinn")
    out is ""
    for p in parts
        out is out + p
    log out
'''

snippets[162] = '''# Sum of squares 1..n.
*main
    n is 100
    s is 0
    i is 1
    while i <= n
        s is s + i * i
        i is i + 1
    log s
'''

snippets[163] = '''# Project Euler 1 (multiples of 3 or 5 below 1000).
*main
    s is 0
    i is 1
    while i < 1000
        ok is false
        if i mod 3 equals 0
            ok is true
        if i mod 5 equals 0
            ok is true
        if ok
            s is s + i
        i is i + 1
    log s
'''

snippets[164] = '''# Pythagorean triple < 1000.
*main
    found is false
    a is 1
    while a < 500 and not found
        b is a + 1
        while b < 500 and not found
            c is 1000 - a - b
            if c > b and a * a + b * b equals c * c
                log a
                log b
                log c
                log a * b * c
                found is true
            b is b + 1
        a is a + 1
'''

snippets[165] = '''# Number of trailing zeros in n!.
*trailing_zeros(n as i64) returns i64
    z is 0
    while n > 0
        n is n / 5
        z is z + n
    z

*main
    log trailing_zeros(100)
'''

snippets[166] = '''# Hamming distance between two i64 (popcount of XOR).
*popcount(n as i64) returns i64
    c is 0
    while n != 0
        n is n & (n - 1)
        c is c + 1
    c

*main
    log popcount(1 ^ 4)
    log popcount(0xff ^ 0x0f)
'''

snippets[167] = '''# Compute mean & std-dev of a vec.
*sqrt(x as f64) returns f64
    if x <= 0.0
        return 0.0
    g is x
    i is 0
    while i < 30
        g is (g + x / g) / 2.0
        i is i + 1
    g

*main
    a is vec(2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0)
    s is 0.0
    for x in a
        s is s + x
    mean is s / 8.0
    log mean
    v is 0.0
    for x in a
        d is x - mean
        v is v + d * d
    log sqrt(v / 8.0)
'''

snippets[168] = '''# Bubble sort.
*bsort(v)
    n is v.length
    i is 0
    while i < n
        j is 0
        while j < n - i - 1
            if v.get(j) > v.get(j + 1)
                t is v.get(j)
                v.set(j, v.get(j + 1))
                v.set(j + 1, t)
            j is j + 1
        i is i + 1

*main
    a is vec(64, 25, 12, 22, 11)
    bsort(a)
    for x in a
        log x
'''

snippets[169] = '''# Selection sort.
*ssort(v)
    n is v.length
    i is 0
    while i < n
        m is i
        j is i + 1
        while j < n
            if v.get(j) < v.get(m)
                m is j
            j is j + 1
        if m != i
            t is v.get(i)
            v.set(i, v.get(m))
            v.set(m, t)
        i is i + 1

*main
    a is vec(5, 1, 4, 2, 8, 3)
    ssort(a)
    for x in a
        log x
'''

snippets[170] = '''# Sum of first n odd numbers (should be n^2).
*main
    n is 50
    s is 0
    i is 0
    while i < n
        s is s + (2 * i + 1)
        i is i + 1
    log s
'''

snippets[171] = '''# Manhattan distance.
*main
    points is vec(vec(0, 0), vec(3, 4), vec(7, 1))
    base is points.get(0)
    i is 1
    while i < points.length
        p is points.get(i)
        d is (p.get(0) - base.get(0)) + (p.get(1) - base.get(1))
        if d < 0
            d is 0 - d
        log d
        i is i + 1
'''

snippets[172] = '''# Matrix transpose 3x3.
*main
    m is vec(vec(1, 2, 3), vec(4, 5, 6), vec(7, 8, 9))
    t is vec(vec(0, 0, 0), vec(0, 0, 0), vec(0, 0, 0))
    i is 0
    while i < 3
        j is 0
        while j < 3
            t.get(i).set(j, m.get(j).get(i))
            j is j + 1
        i is i + 1
    for row in t
        for x in row
            log x
'''

snippets[173] = '''# DFS on adjacency-list graph (no cycles).
*main
    adj is vec(vec(1, 2), vec(3), vec(3), vec())
    stack is vec()
    stack.push(0)
    visited is vec(false, false, false, false)
    while stack.length > 0
        n is stack.pop()
        if not visited.get(n)
            visited.set(n, true)
            log n
            nb is adj.get(n)
            i is nb.length - 1
            while i >= 0
                stack.push(nb.get(i))
                i is i - 1
'''

snippets[174] = '''# BFS shortest path lengths from node 0.
*main
    adj is vec(vec(1, 2), vec(3), vec(3, 4), vec(5), vec(5), vec())
    dist is vec(0 - 1, 0 - 1, 0 - 1, 0 - 1, 0 - 1, 0 - 1)
    dist.set(0, 0)
    queue is vec()
    queue.push(0)
    head is 0
    while head < queue.length
        n is queue.get(head)
        head is head + 1
        nb is adj.get(n)
        for v in nb
            if dist.get(v) equals 0 - 1
                dist.set(v, dist.get(n) + 1)
                queue.push(v)
    for d in dist
        log d
'''

snippets[175] = '''# Topological sort via Kahn's algorithm.
*main
    n is 6
    adj is vec(vec(1, 2), vec(3), vec(3), vec(4, 5), vec(), vec())
    indeg is vec(0, 0, 0, 0, 0, 0)
    i is 0
    while i < n
        for v in adj.get(i)
            indeg.set(v, indeg.get(v) + 1)
        i is i + 1
    queue is vec()
    i is 0
    while i < n
        if indeg.get(i) equals 0
            queue.push(i)
        i is i + 1
    head is 0
    while head < queue.length
        u is queue.get(head)
        head is head + 1
        log u
        for v in adj.get(u)
            indeg.set(v, indeg.get(v) - 1)
            if indeg.get(v) equals 0
                queue.push(v)
'''

snippets[176] = '''# Dijkstra on dense graph (4 nodes).
*main
    INF is 1000000000
    g is vec(vec(0, 4, 1, INF), vec(4, 0, 2, 5), vec(1, 2, 0, 8), vec(INF, 5, 8, 0))
    n is 4
    dist is vec(INF, INF, INF, INF)
    visited is vec(false, false, false, false)
    dist.set(0, 0)
    i is 0
    while i < n
        u is 0 - 1
        best is INF
        j is 0
        while j < n
            if not visited.get(j) and dist.get(j) < best
                best is dist.get(j)
                u is j
            j is j + 1
        if u equals 0 - 1
            i is n
        else
            visited.set(u, true)
            v is 0
            while v < n
                cand is dist.get(u) + g.get(u).get(v)
                if cand < dist.get(v)
                    dist.set(v, cand)
                v is v + 1
            i is i + 1
    for d in dist
        log d
'''

snippets[177] = '''# Karatsuba-ish multiplication test (just a sanity check).
*main
    a is 12345678
    b is 87654321
    log a * b
'''

snippets[178] = '''# Fizzbuzz 1..15.
*main
    i is 1
    while i <= 15
        if i mod 15 equals 0
            log "fizzbuzz"
        if i mod 15 != 0 and i mod 3 equals 0
            log "fizz"
        if i mod 15 != 0 and i mod 5 equals 0 and i mod 3 != 0
            log "buzz"
        if i mod 3 != 0 and i mod 5 != 0
            log i
        i is i + 1
'''

snippets[179] = '''# Number of digits in n.
*ndigits(n as i64) returns i64
    if n equals 0
        return 1
    if n < 0
        n is 0 - n
    c is 0
    while n > 0
        c is c + 1
        n is n / 10
    c

*main
    log ndigits(0)
    log ndigits(9)
    log ndigits(123456789)
'''

snippets[180] = '''# Power of three test.
*is_pow3(n as i64) returns bool
    if n <= 0
        return false
    while n mod 3 equals 0
        n is n / 3
    n equals 1

*main
    log is_pow3(1)
    log is_pow3(9)
    log is_pow3(10)
    log is_pow3(243)
'''

snippets[181] = '''# Convert decimal i64 to binary string.
*to_bin(n as i64) returns String
    if n equals 0
        return "0"
    s is ""
    while n > 0
        if n mod 2 equals 0
            s is "0" + s
        else
            s is "1" + s
        n is n / 2
    s

*main
    log to_bin(0)
    log to_bin(13)
    log to_bin(255)
'''

snippets[182] = '''# Parse a small base-10 integer from a string.
*parse(s as String) returns i64
    n is 0
    i is 0
    while i < s.length
        c is s.char_at(i)
        if c < 48 or c > 57
            return 0
        n is n * 10 + (c - 48)
        i is i + 1
    n

*main
    log parse("12345")
    log parse("0")
    log parse("99999")
'''

snippets[183] = '''# Coin change minimum coins (DP).
*coin_change(coins, amt as i64) returns i64
    INF is 1000000
    dp is vec()
    i is 0
    while i <= amt
        dp.push(INF)
        i is i + 1
    dp.set(0, 0)
    a is 1
    while a <= amt
        for c in coins
            if c <= a and dp.get(a - c) + 1 < dp.get(a)
                dp.set(a, dp.get(a - c) + 1)
        a is a + 1
    if dp.get(amt) equals INF
        return 0 - 1
    dp.get(amt)

*main
    coins is vec(1, 5, 10, 25)
    log coin_change(coins, 47)
'''

snippets[184] = '''# Validate balanced parentheses.
*balanced(s as String) returns bool
    stack is vec()
    i is 0
    while i < s.length
        c is s.char_at(i)
        if c equals 40
            stack.push(41)
        if c equals 91
            stack.push(93)
        if c equals 123
            stack.push(125)
        if c equals 41 or c equals 93 or c equals 125
            if stack.length equals 0
                return false
            top is stack.pop()
            if top != c
                return false
        i is i + 1
    stack.length equals 0

*main
    log balanced("(()[])")
    log balanced("(]")
    log balanced("{[()]}")
'''

snippets[185] = '''# Frequency of each digit in n.
*main
    n is 1224488800
    cnt is vec(0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
    while n > 0
        d is n mod 10
        cnt.set(d, cnt.get(d) + 1)
        n is n / 10
    i is 0
    while i < 10
        log cnt.get(i)
        i is i + 1
'''

snippets[186] = '''# Multiplication table.
*main
    i is 1
    while i <= 5
        j is 1
        while j <= 5
            log i * j
            j is j + 1
        i is i + 1
'''

snippets[187] = '''# Find max value & index in a vec.
*main
    a is vec(7, 1, 9, 3, 11, 5, 8)
    bi is 0
    bv is a.get(0)
    i is 1
    while i < a.length
        if a.get(i) > bv
            bv is a.get(i)
            bi is i
        i is i + 1
    log bv
    log bi
'''

snippets[188] = '''# Convert a vec to a sorted unique vec.
*main
    a is vec(3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5)
    n is a.length
    i is 1
    while i < n
        key is a.get(i)
        j is i - 1
        while j >= 0 and a.get(j) > key
            a.set(j + 1, a.get(j))
            j is j - 1
        a.set(j + 1, key)
        i is i + 1
    out is vec()
    i is 0
    while i < a.length
        if i equals 0
            out.push(a.get(i))
        if i > 0 and a.get(i) != a.get(i - 1)
            out.push(a.get(i))
        i is i + 1
    for x in out
        log x
'''

snippets[189] = '''# Generate first n triangular numbers.
*main
    n is 10
    i is 1
    while i <= n
        log i * (i + 1) / 2
        i is i + 1
'''

snippets[190] = '''# Approximate pi via Leibniz series.
*main
    n is 10000
    s is 0.0
    i is 0
    while i < n
        sign is 1.0
        if i mod 2 equals 1
            sign is 0 - 1.0
        s is s + sign / (2.0 * (i as f64) + 1.0)
        i is i + 1
    log 4.0 * s
'''

snippets[191] = '''# Approximate e via 1/n! summation.
*main
    s is 0.0
    f is 1.0
    i is 0
    while i < 20
        if i > 0
            f is f * (i as f64)
        s is s + 1.0 / f
        i is i + 1
    log s
'''

snippets[192] = '''# Compute median of a vec (insertion sort then mid).
*sort(v)
    n is v.length
    i is 1
    while i < n
        key is v.get(i)
        j is i - 1
        while j >= 0 and v.get(j) > key
            v.set(j + 1, v.get(j))
            j is j - 1
        v.set(j + 1, key)
        i is i + 1

*main
    a is vec(7, 1, 9, 3, 11, 5, 8)
    sort(a)
    log a.get(a.length / 2)
'''

snippets[193] = '''# Shuffle is hard without rng — instead, log a deterministic permutation.
*main
    n is 8
    perm is vec()
    i is 0
    while i < n
        perm.push((i * 5 + 3) mod n)
        i is i + 1
    for x in perm
        log x
'''

snippets[194] = '''# Recursive sum of digits.
*ds(n as i64) returns i64
    if n < 10
        return n
    n mod 10 + ds(n / 10)

*main
    log ds(123456)
'''

snippets[195] = '''# Tail-style accumulator factorial.
*fact_acc(n as i64, acc as i64) returns i64
    if n <= 1
        return acc
    fact_acc(n - 1, acc * n)

*main
    log fact_acc(12, 1)
'''

snippets[196] = '''# Mutual recursion: even/odd.
*is_even(n as i64) returns bool
    if n equals 0
        return true
    is_odd(n - 1)

*is_odd(n as i64) returns bool
    if n equals 0
        return false
    is_even(n - 1)

*main
    log is_even(10)
    log is_odd(7)
'''

snippets[197] = '''# Generate all subsets of {0,1,2,3} using bitmask.
*main
    n is 4
    mask is 0
    limit is 1 << n
    while mask < limit
        i is 0
        log mask
        while i < n
            if (mask >> i) & 1 equals 1
                log i
            i is i + 1
        mask is mask + 1
'''

snippets[198] = '''# Compute checksum: sum of bytes mod 256.
*checksum(s as String) returns i64
    s2 is 0
    i is 0
    while i < s.length
        s2 is (s2 + s.char_at(i)) mod 256
        i is i + 1
    s2

*main
    log checksum("hello world")
'''

snippets[199] = '''# Naive substring search (returns -1 if not found).
*find_sub(h as String, n as String) returns i64
    if n.length equals 0
        return 0
    i is 0
    last is h.length - n.length
    while i <= last
        j is 0
        ok is true
        while j < n.length and ok
            if h.char_at(i + j) != n.char_at(j)
                ok is false
            j is j + 1
        if ok
            return i
        i is i + 1
    0 - 1

*main
    log find_sub("the quick brown fox", "brown")
    log find_sub("hello", "world")
'''

snippets[200] = '''# Final cap: an actor that counts ticks.
actor Counter
    n as i64
    @tick
        n is n + 1
    @final
        log n

*main
    c is spawn Counter
    i is 0
    while i < 5
        c.tick()
        i is i + 1
    c.final()
'''

# Existing s101..s105 already on disk
for n in [101, 102, 103, 104, 105]:
    snippets.setdefault(n, None)

count = 0
for n, body in sorted(snippets.items()):
    if body is None:
        continue
    path = f"{out}/s{n:03d}.jn"
    with open(path, "w") as f:
        f.write(body)
    count += 1
print(f"wrote {count} snippets")
