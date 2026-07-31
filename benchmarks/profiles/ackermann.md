# ackermann — 1.91× vs gcc -O3 (profiled 2026-07-30)

`ack(3, 10)`, call-tree recursion. Measured 213ms (jinn) vs 112ms (gcc),
ratio **1.91×** on i7-8650U. The old `results.csv` claimed ratio 1.0; the
honest gap appears once both sides are measured from current sources.

## Analysis

Same shape as `fibonacci`, amplified. jinn+LLVM emit an extremely tight
22-instruction `ack`: the `m-1` recursion is converted to a loop
(tail-call elimination), one register spill, one recursive call site.
gcc emits 130 instructions — it inlines multiple levels of the recursion
and partially specializes small `m` values, roughly halving the number of
real call/return pairs per tree node. Ackermann is nothing but calls, so
inlining depth dominates and the smaller/cleaner code loses.

## Follow-up

Same disposition as `fibonacci`: LLVM-vs-GCC inlining policy, clean IR
from the front end, bounded payoff for tuning. Not filed as a jinnc
defect. Gated against regression by `ci/bench_regression.py`; revisit
under an inliner-tuning task if recursive-call-heavy workloads matter for
alpha.
