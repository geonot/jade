# tight_loop — 1.48× vs gcc -O3 (profiled 2026-07-30)

2-billion-iteration loop, body `sum ^= i; sum += i`. Measured 1.57s (jinn)
vs 1.06s (gcc), ratio 1.48×, on i7-8650U. `perf` is unavailable on the
measurement machine; since the program is a single hot loop, the profile
below is annotated disassembly plus `llvm-mca` pipeline analysis, which
fully accounts for the measured delta.

## Root cause: one extra op on the loop-carried dependency chain

gcc emits the obvious 2-op chain (loop control off-chain):

```asm
loop:  xor  %rax,%rsi        ; sum ^= i     — chain op 1
       add  %rax,%rsi        ; sum += i     — chain op 2
       inc  %rax
       cmp  $2000000000,%rax
       jne  loop
```

LLVM (via jinnc) unrolls 16× but rewrites each step as
`sum = (sum ^ (base+k)) + base + k`, rematerializing `i` from the block
base instead of keeping it live, which puts **three** ops on the serial
chain per source iteration:

```asm
       lea  k(%rcx),%rsi      ; i = base+k   — off chain
       xor  %rdx,%rsi         ;              — chain op 1
       lea  (%rcx,%rsi,1),%rdx;              — chain op 2
       add  $k,%rdx           ;              — chain op 3
```

The loop is dependency-bound, not throughput-bound, so unrolling doesn't
help: the chain length per iteration is the whole story.

## llvm-mca confirmation (skylake model, 200 iterations)

| kernel | cycles / source iteration |
|---|---|
| gcc loop | 403/200 ≈ **2.01** |
| jinn loop (16 src-iters per block) | 9604/(200·16) ≈ **3.00** |

Predicted ratio 3.00/2.01 = 1.49×; measured 1.48×. Nothing else in the
binary contributes measurably.

## Follow-up

The IR jinnc emits for `for i in 0 to n` materializes the induction
variable as `base + offset` after unrolling, which invites this
reassociation. Worth checking whether emitting a plain increment IV (or
`-fno-reassociate`-style tuning on the hot function) restores the 2-op
chain. This is an LLVM canonicalization interaction, not a front-end
correctness issue; the gap is bounded (1.5×) and understood.
