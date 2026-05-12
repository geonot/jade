# SNIPPET2 — Batch 2 (s101..s200)

100 additional, more-complex Jinn snippets.
**Final result:** `Total=100 OK=100 CompileErr=0 RuntimeErr=0`.

Run via `bash /tmp/jinn_snippets2/run_all.sh`. Output captured at `/tmp/jinn_snippets2/results.txt`.

This batch was added on top of the green batch 1 (s001..s100, see [SNIPPET.md](SNIPPET.md)).
The two harnesses are independent (different output directories) and may be run side-by-side.

## Coverage matrix

| # | Snippet | Intent |
|---|---------|--------|
| s101 | iterative `fact` | imperative loop, return-from-fn |
| s102 | memoized `fib` (vec) | mutable vec + indexed dp |
| s103 | sieve of Eratosthenes | bool vec, nested loops |
| s104 | insertion sort (in-place) | mutating-vec parameter |
| s105 | binary search | divide-and-conquer loop |
| s106 | quicksort (Lomuto) | recursive in-place sort, requires `Vec of i64` annotations |
| s107 | merge sort | recursion + helper concat |
| s108 | gcd (Euclid) | mod loop |
| s109 | lcm via gcd | composition of helper fns |
| s110 | `is_pow2` | bit-and trick |
| s111 | popcount (Kernighan) | bitwise loop |
| s112 | reverse bits 64-bit | shift/or in a loop |
| s113 | digit sum | mod-10 loop |
| s114 | reverse digits | accumulator pattern |
| s115 | palindrome int | digit reverse compare |
| s116 | Collatz step count | branch on parity |
| s117 | Newton-Raphson sqrt (f64) | float arithmetic loop |
| s118 | `ipow` (squaring) | bit-iteration on exponent |
| s119 | `modpow` | modular exponentiation |
| s120 | two-sum | nested loop, early return |
| s121 | Kadane (max subarray) | running-best dp |
| s122 | counting sort (bytes) | histogram-based sort |
| s123 | reverse vec in place | two-pointer swap |
| s124 | rotate vec by k | modular indexing into output vec |
| s125 | linked list via parallel vecs | indexed traversal (no struct, since `next` field caused parser issue) |
| s126 | stack via vec | `vec.push/pop` cycle |
| s127 | queue from two stacks | classic CS pattern |
| s128 | Horner polynomial eval | float vec, negative literal |
| s129 | matmul 2×2 (flat) | flattened 2D index, triple loop |
| s130 | counter via single-cell vec | mutable cell stand-in for closure (lambda-block syntax not yet stable) |
| s131 | `vec.map(|x| x*x)` | higher-order method, lambda |
| s132 | `vec.filter(|x| ...)` | predicate lambda |
| s133 | manual product fold | for-in accumulation |
| s134 | mean of f64 vec | float fold + division |
| s135 | word frequency (parallel vecs) | linear scan key lookup |
| s136 | `MinMax` record return | type-record value |
| s137 | `Res = Ok|Err` enum | pattern match on enum |
| s138 | `String.split` token count | std-string method |
| s139 | string reverse via slice | byte-wise iteration |
| s140 | Caesar cipher (lowercase) | `char_at` + arithmetic |
| s141 | left-trim spaces | `slice` method |
| s142 | count vowels | dispatch via `if`/`return` chain |
| s143 | run-length encoding | run-detection scan |
| s144 | longest common prefix | bounded scan, slice |
| s145 | trial-division primality | early-return loop |
| s146 | Hanoi step count | recursion |
| s147 | Pascal triangle row | combinatorial recurrence |
| s148 | Catalan numbers | iterative formula |
| s149 | Heron's triangle area | float helper composition |
| s150 | compound interest | float accumulator |
| s151 | greatest digit | mod-10 max scan |
| s152 | spiral matrix walk | flat 2D walker, four shrinking edges |
| s153 | 0/1 knapsack DP | flat 2D dp |
| s154 | Levenshtein distance | flat 2D dp, `char_at` compare |
| s155 | gcd-of-vec fold | composition with helper |
| s156 | `fib mod m` | modular running pair |
| s157 | prefix sums | `pre[j]-pre[i]` queries |
| s158 | sliding-window max sum | running window arithmetic |
| s159 | two-pointer pair sum | sorted-vec scan |
| s160 | filter with captured threshold | closure capture (lambda `|x|`) |
| s161 | string concat fold | for-in over `Vec of String` |
| s162 | sum of squares 1..n | trivial loop |
| s163 | Project Euler 1 | branchy filter |
| s164 | Pythagorean triple <1000 | brute search |
| s165 | trailing zeros of n! | divide-by-5 loop |
| s166 | Hamming distance via popcount | XOR + popcount |
| s167 | mean & stddev | sqrt helper, two-pass |
| s168 | bubble sort | nested swap loop |
| s169 | selection sort | min-find inner loop |
| s170 | sum of first n odd | n² identity check |
| s171 | Manhattan distance | abs via `0 - x`, parallel xs/ys vecs |
| s172 | matrix transpose 3×3 | nested vec indexing |
| s173 | DFS via CSR (heads/dest) | manual stack walk |
| s174 | BFS via CSR | dist[] BFS (replaces -1 sentinel) |
| s175 | Kahn's topological sort | indeg-zero queue |
| s176 | Dijkstra (dense, 4 nodes) | classic min-pick relax |
| s177 | i64 product sanity | wrap-around check |
| s178 | FizzBuzz 1..15 | nested if/else (chained-or avoided) |
| s179 | digit count `ndigits` | edge cases |
| s180 | `is_pow3` | divide-by-3 loop |
| s181 | int → binary string | string concat in loop |
| s182 | parse small base-10 int | char arithmetic |
| s183 | coin-change min coins | DP fill table |
| s184 | balanced parentheses | helper-fn `is_close` (avoids chained-or) + `not equals` |
| s185 | digit frequency vec | mod-10 hist |
| s186 | multiplication table | nested loop output |
| s187 | max value & index | running-best track |
| s188 | sort + de-dup | insertion sort then linear de-dup |
| s189 | first-n triangulars | direct formula |
| s190 | Leibniz π | float series, alternating sign |
| s191 | `e ≈ Σ 1/n!` | factorial accumulator |
| s192 | median via sort | sort then index |
| s193 | deterministic permutation | mod-N stride |
| s194 | recursive digit sum | non-tail recursion |
| s195 | accumulator factorial | tail-style recursion |
| s196 | mutual recursion `is_even/odd` | order-independent fn refs |
| s197 | bitmask subsets of {0..3} | nested bit loop |
| s198 | byte checksum mod 256 | running mod sum |
| s199 | naive substring search | brute window match, returns -1 |
| s200 | actor `Counter` | spawn + tick handler + final |

## Workarounds applied while authoring batch 2

These reflect real, known compiler limitations (already cataloged from batch 1):

- **`!=` is not lexed** — the language uses `not equals` instead. ~11 snippets were rewritten.
- **`struct` keyword is not accepted at top level** — `type` is the spelling Jinn uses. s125, s136 changed to `type` (or to parallel vecs where the `next as i64` field tripped a separate issue).
- **2D `Vec of Vec of T` segfaults at runtime** in several index patterns (s152/153/154/171/173/174). Rewrote each to use a flattened 1D vec or "CSR" (heads + dest arrays).
- **`map`/`filter` lambdas use `|x| body`**, not `$($)` — corrected in s131/s132/s160.
- **Lambda-block as a value (`|| { ... }`) is not parsed everywhere** — replaced "make_counter" closure (s130) with a one-cell vec.
- **Quicksort's polymorphic `v` parameter** could not be inferred to `Vec of i64` from a deep `v.get/v.set` use; required explicit `v as Vec of i64` annotation in s106.
- **Chained `or` lowering bug** (3+ terms) hit s178 (FizzBuzz `i mod 3 != 0 and i mod 5 != 0` after `!=` rewrite became chained `and` of comparisons that exposed the issue) and s184 (`c equals 41 or c equals 93 or c equals 125`). Rewritten as nested `if/else` and a helper fn respectively.
- **Reserved word `by`** can't be used as an identifier — renamed `by` → `base_y` in s171.

## Cross-cutting compiler bugs uncovered (batch-2 specific)

These extend the list in `/memories/repo/jade_*` notes:

1. **Vec-of-Vec runtime corruption.** Pushing a constructed `vec(...)` into another `vec()` and later reading via `.get(i).get(j)` segfaults even on tiny 3×3 grids (`/tmp/jinn_snippets2/test_2d.jn`). Suspect `vec(...)` literal storage of vec-typed elements is dropped/freed before the outer push completes. Workaround: flatten or use parallel vecs.
2. **Generic-element type inference for `Vec` parameters with no annotation** — calling `.get`/`.set` deep inside a recursive helper does not propagate the i64 element type, surfacing as `unknown method 'get'`. Annotation `as Vec of i64` is required at every level.
3. **Chained `or` (≥3 terms) PHI bug** still present (already known). New repro: `c equals 41 or c equals 93 or c equals 125`.
4. **`struct Foo` at top level**: keyword `struct` is parsed in some contexts (e.g. C-side comparison code) but is rejected by the snippet parser; only `type Foo` works.
5. **Closure-as-value with block body (`|| { ... }`)** — `unexpected character: '{'` — only single-expression closures parse.

All workarounds above are *snippet-side*; no compiler/std changes were needed for batch 2.
The compiler bugs above remain pending and are tracked in repo memory.

## Files

- `/tmp/jinn_snippets2/s101.jn` … `/tmp/jinn_snippets2/s200.jn`
- `/tmp/jinn_snippets2/run_all.sh` — runner (`set -u`, 10 s timeout per binary)
- `/tmp/jinn_snippets2/results.txt` — last run output
- `/tmp/jinn_snippets2/gen.py`, `/tmp/jinn_snippets2/patch.py` — generators (kept for reproducibility)
