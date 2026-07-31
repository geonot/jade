# array_ops — 24.66× vs gcc -O3 (profiled 2026-07-30)

50M-iteration loop building a 5-element array literal and summing its
elements. Measured 1.02s (jinn) vs 41ms (gcc), ratio **24.66×** — the
largest comparable gap in the suite, surfaced by task 8-26's regeneration.

Two prior fictions hid it:

1. **The old C source ran a different workload** — 1,500,000,000
   iterations vs the jinn side's 50,000,000 (30× more), left stale by an
   earlier edit of only one side. It produced a different final sum and a
   flattering 0.27× "jinn wins" ratio. The C source now matches the jinn
   program; outputs are byte-identical (`-152121467`).
2. The old `results.csv` reported 0.42ms/0.87× for this row, which
   corresponds to neither source.

## Root cause: heap round-trip per iteration for a non-escaping literal

The jinn loop body contains, every iteration:

```
call jinn_xmalloc      ; array literal allocation
call jinn_xmalloc      ; (second allocation on the same path)
...
call free
call free
```

The array literal `[i xor total, i+1, ...]` never escapes the iteration,
but jinnc lowers array literals to heap-allocated vectors unconditionally,
so the loop pays two malloc/free pairs × 50M. gcc keeps the array virtual
(the additions fold into scalar arithmetic; the array never exists at
runtime).

`alloc_churn` (2.16×) is the same allocator cost measured deliberately —
there the comparison is jinn_xmalloc vs glibc malloc on purpose. Here the
allocation itself is the bug: it shouldn't exist.

## Follow-up

Filed as the top codegen follow-up from 8-26: promote non-escaping array
literals to stack storage (or let them SROA away entirely). Candidate
home: the MIR lowering of `ExprKind::Array` (`src/mir/lower/expr.rs:169`)
— emit an `alloca`-backed aggregate when escape analysis (already
flow-sensitive since task 8-7) shows the value neither escapes nor
outlives the scope. Expected effect:
this row drops from ~24× to ~1×, and every array-literal-in-loop program
benefits.
