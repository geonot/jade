# fibonacci — 1.15× vs gcc -O3 (profiled 2026-07-30)

Naive recursive `fib(42)`. Measured 708ms (jinn) vs 614ms (gcc), ratio
**1.15×** on i7-8650U. The July review measured 2.59× (1703ms vs 658ms);
that gap no longer reproduces from current sources — the jinn side more
than halved. The old `results.csv` row (340ms, ratio 1.0) matched neither
measurement and was regenerated away by task 8-26.

## Where the remaining 15% comes from

jinn+LLVM compile `fib` to a compact 24-instruction function: LLVM's
accumulator transformation turns the second recursive call into a loop,
leaving one call per tree node, and marks the function
`nounwind memory(none)`. The IR is clean — there is no jinn-side overhead
(no allocation, no checks) in the hot path.

gcc instead inlines several levels of the recursion into a 243-instruction
body with 78 call sites, trading code size for fewer call/return pairs per
tree node. On a call-dominated microbenchmark that wins ~15%.

## Follow-up

None filed. The residual delta is an LLVM-vs-GCC recursive-inlining
policy difference, reachable only by inliner tuning, with bounded payoff.
Watch it via the CI ratio gate (`ci/bench_regression.py`); revisit if it
regresses past the 10% threshold.
