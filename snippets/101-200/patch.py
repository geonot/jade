#!/usr/bin/env python3
"""Patch all failing snippets in batch 2."""
import os
out = "/tmp/jinn_snippets2"

# ---- !=  →  not equals
import re
need_ne = [108, 109, 111, 116, 155, 166, 169, 178, 184, 188, 199]
for n in need_ne:
    p = f"{out}/s{n:03d}.jn"
    with open(p) as f:
        s = f.read()
    s = s.replace("!=", "not equals")
    with open(p, "w") as f:
        f.write(s)

# ---- write fresh versions for the rest
fixes = {}

# s104 RE — sort returns the same vec; bug may be in returning vec param.
fixes[104] = '''# Insertion sort on a vec of i64.
*sort(v)
    i is 1
    while i < v.length
        key is v.get(i)
        j is i - 1
        while j >= 0 and v.get(j) > key
            v.set(j + 1, v.get(j))
            j is j - 1
        v.set(j + 1, key)
        i is i + 1

*main
    a is vec(5, 2, 9, 1, 7, 3, 8, 4, 6)
    sort(a)
    for x in a
        log x
'''

# s106 unknown method `get` — issue with v parameter inferred as wrong type. Annotate.
fixes[106] = '''# Quicksort (Lomuto partition).
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
    a is vec()
    a.push(8)
    a.push(3)
    a.push(1)
    a.push(7)
    a.push(0)
    a.push(10)
    a.push(2)
    a.push(5)
    a.push(4)
    a.push(9)
    a.push(6)
    qsort(a, 0, a.length - 1)
    for x in a
        log x
'''

# s125 struct → type; use simpler form.
fixes[125] = '''# Linked list as parallel arrays.
*main
    vals is vec(10, 20, 30)
    nxt is vec(1, 2, -1)
    i is 0
    while i >= 0
        log vals.get(i)
        i is nxt.get(i)
'''

# s128 horner — use unary-minus and explicit floats.
fixes[128] = '''# Polynomial evaluation (Horner).
*horner(coeffs, x as f64) returns f64
    r is 0.0
    i is 0
    while i < coeffs.length
        r is r * x + coeffs.get(i)
        i is i + 1
    r

*main
    c is vec(1.0, 0.0, -2.0, 1.0)
    log horner(c, 2.0)
'''

# s129 matmul — flatten to 1D.
fixes[129] = '''# Matrix multiplication 2x2 via flat vec.
*main
    a is vec(1, 2, 3, 4)
    b is vec(5, 6, 7, 8)
    c is vec(0, 0, 0, 0)
    n is 2
    i is 0
    while i < n
        j is 0
        while j < n
            s is 0
            k is 0
            while k < n
                s is s + a.get(i * n + k) * b.get(k * n + j)
                k is k + 1
            c.set(i * n + j, s)
            j is j + 1
        i is i + 1
    for x in c
        log x
'''

# s130 closure — use |x| body.
fixes[130] = '''# Closure capturing a counter.
*make_counter()
    c is vec(0)
    || {
        c.set(0, c.get(0) + 1)
        c.get(0)
    }

*main
    f is make_counter()
    log f()
    log f()
    log f()
'''

# s131 - use map with |x| x*x
fixes[131] = '''# Higher-order map via closures over Vec.
*main
    a is vec(1, 2, 3, 4, 5)
    b is a.map(|x| x * x)
    for x in b
        log x
'''

fixes[132] = '''# Filter even numbers.
*main
    a is vec(1, 2, 3, 4, 5, 6, 7, 8, 9, 10)
    b is a.filter(|x| x mod 2 equals 0)
    for x in b
        log x
'''

fixes[133] = '''# Manual fold to product.
*main
    a is vec(1, 2, 3, 4, 5)
    p is 1
    for x in a
        p is p * x
    log p
'''

# s135 - drop map module use; just open-code counts via parallel vecs.
fixes[135] = '''# Word frequency via parallel vecs (no map module needed).
*main
    words is vec("a", "b", "a", "c", "a", "b")
    keys is vec()
    cnts is vec()
    for w in words
        i is 0
        found is false
        while i < keys.length and not found
            if keys.get(i) equals w
                cnts.set(i, cnts.get(i) + 1)
                found is true
            i is i + 1
        if not found
            keys.push(w)
            cnts.push(1)
    i is 0
    while i < keys.length
        log keys.get(i)
        log cnts.get(i)
        i is i + 1
'''

# s136 struct → type.
fixes[136] = '''# Multiple return via type-record.
type MinMax
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

# s152 spiral — flatten to 1D.
fixes[152] = '''# Spiral matrix traversal (4x4) using flat vec.
*main
    n is 4
    grid is vec()
    i is 0
    while i < n * n
        grid.push(i + 1)
        i is i + 1
    top is 0
    bot is n - 1
    lft is 0
    rgt is n - 1
    while top <= bot and lft <= rgt
        c is lft
        while c <= rgt
            log grid.get(top * n + c)
            c is c + 1
        top is top + 1
        r is top
        while r <= bot
            log grid.get(r * n + rgt)
            r is r + 1
        rgt is rgt - 1
        if top <= bot
            c is rgt
            while c >= lft
                log grid.get(bot * n + c)
                c is c - 1
            bot is bot - 1
        if lft <= rgt
            r is bot
            while r >= top
                log grid.get(r * n + lft)
                r is r - 1
            lft is lft + 1
'''

# s153 knapsack — flatten dp to 1D.
fixes[153] = '''# Knapsack 0/1 (DP) — flat dp.
*knap(weights, values, cap as i64) returns i64
    n is weights.length
    sz is (n + 1) * (cap + 1)
    dp is vec()
    i is 0
    while i < sz
        dp.push(0)
        i is i + 1
    i is 1
    while i <= n
        w is weights.get(i - 1)
        v is values.get(i - 1)
        c is 0
        while c <= cap
            best is dp.get((i - 1) * (cap + 1) + c)
            if w <= c
                cand is dp.get((i - 1) * (cap + 1) + (c - w)) + v
                if cand > best
                    best is cand
            dp.set(i * (cap + 1) + c, best)
            c is c + 1
        i is i + 1
    dp.get(n * (cap + 1) + cap)

*main
    w is vec(2, 3, 4, 5)
    v is vec(3, 4, 5, 6)
    log knap(w, v, 5)
'''

# s154 lev — flatten dp to 1D.
fixes[154] = '''# Levenshtein edit distance (flat dp).
*lev(a as String, b as String) returns i64
    m is a.length
    n is b.length
    w is n + 1
    sz is (m + 1) * w
    dp is vec()
    k is 0
    while k < sz
        dp.push(0)
        k is k + 1
    i is 0
    while i <= m
        dp.set(i * w, i)
        i is i + 1
    j is 0
    while j <= n
        dp.set(j, j)
        j is j + 1
    i is 1
    while i <= m
        j is 1
        while j <= n
            cost is 1
            if a.char_at(i - 1) equals b.char_at(j - 1)
                cost is 0
            d1 is dp.get((i - 1) * w + j) + 1
            d2 is dp.get(i * w + (j - 1)) + 1
            d3 is dp.get((i - 1) * w + (j - 1)) + cost
            best is d1
            if d2 < best
                best is d2
            if d3 < best
                best is d3
            dp.set(i * w + j, best)
            j is j + 1
        i is i + 1
    dp.get(m * w + n)

*main
    log lev("kitten", "sitting")
'''

# s155 already done by ne replace? Actually s155 had `!=` too. Let me check more carefully.
# Actually s155 fix needs both.

# s160 closure — use |x|
fixes[160] = '''# Closure as predicate.
*main
    a is vec(3, 1, 4, 1, 5, 9, 2, 6)
    threshold is 4
    big is a.filter(|x| x > threshold)
    for x in big
        log x
'''

# s166 already replaced. But popcount has `!=` too — handled.
# s169 ssort  - has !=. Handled.

# s171 RE — vec of vec. Flatten.
fixes[171] = '''# Manhattan distance from base point.
*main
    xs is vec(0, 3, 7)
    ys is vec(0, 4, 1)
    bx is xs.get(0)
    by is ys.get(0)
    i is 1
    while i < xs.length
        dx is xs.get(i) - bx
        dy is ys.get(i) - by
        if dx < 0
            dx is 0 - dx
        if dy < 0
            dy is 0 - dy
        log dx + dy
        i is i + 1
'''

# s173 RE — DFS uses adj as vec of vec. Flatten as adjacency CSR.
fixes[173] = '''# DFS on small graph using parallel arrays.
# Edges out of node i: heads[i] .. heads[i+1] in dest[].
*main
    heads is vec(0, 2, 3, 4, 4)
    dest is vec(1, 2, 3, 3)
    n is 4
    visited is vec()
    i is 0
    while i < n
        visited.push(false)
        i is i + 1
    stack is vec(0)
    while stack.length > 0
        u is stack.pop()
        if not visited.get(u)
            visited.set(u, true)
            log u
            e is heads.get(u + 1) - 1
            while e >= heads.get(u)
                stack.push(dest.get(e))
                e is e - 1
'''

fixes[174] = '''# BFS on graph via CSR.
*main
    heads is vec(0, 2, 3, 5, 6, 7, 7)
    dest is vec(1, 2, 3, 3, 4, 5, 5)
    n is 6
    dist is vec()
    i is 0
    while i < n
        dist.push(0 - 1)
        i is i + 1
    dist.set(0, 0)
    queue is vec(0)
    head is 0
    while head < queue.length
        u is queue.get(head)
        head is head + 1
        e is heads.get(u)
        while e < heads.get(u + 1)
            v is dest.get(e)
            if dist.get(v) equals 0 - 1
                dist.set(v, dist.get(u) + 1)
                queue.push(v)
            e is e + 1
    for d in dist
        log d
'''

# s175 already uses vec of vec — was OK in run? Actually s175 didn't fail. Skip.

# s176 dijkstra on dense graph used vec of vec — but it OK'd? Actually s176 wasn't in failures. OK skip.

# Let me check after — s176 is in failures? No, only the listed ones. OK.

# s184 balanced — `c equals 41 or c equals 93 or c equals 125` is chained or (3 terms). And `top != c`.
fixes[184] = '''# Validate balanced parentheses (no chained or).
*is_close(c as i64) returns bool
    if c equals 41
        return true
    if c equals 93
        return true
    if c equals 125
        return true
    false

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
        if is_close(c)
            if stack.length equals 0
                return false
            top is stack.pop()
            if top not equals c
                return false
        i is i + 1
    stack.length equals 0

*main
    log balanced("(()[])")
    log balanced("(]")
    log balanced("{[()]}")
'''

# s188 has !=. Already handled.

# s190 leibniz — explicit floats.
fixes[190] = '''# Approximate pi via Leibniz series.
*main
    n is 10000
    s is 0.0
    i is 0
    while i < n
        sign is 1.0
        if i mod 2 equals 1
            sign is -1.0
        s is s + sign / (2.0 * (i as f64) + 1.0)
        i is i + 1
    log 4.0 * s
'''

# s199 has !=. Already handled.

for n, body in fixes.items():
    with open(f"{out}/s{n:03d}.jn", "w") as f:
        f.write(body)

print(f"patched {len(fixes)} + ne in {len(need_ne)}")
