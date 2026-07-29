# §19 Performance observations

I did not run formal benchmarks for this review (the in-tree
`benchmarks/` already exist and `run_benchmarks.py` is present).
Some informal numbers from the probe set:

| Probe | What it does | Time |
| ----- | ------------ | ---- |
| `p32_alloc_churn.jn` | 100,000 × { vec(); push; read } | **14 ms** |
| `p33_tight_loop.jn` | sum 0..100,000,000 | **2 ms** |
| `p41_tco.jn` | 10,000,000-deep tail-call | **2 ms** |
| `p20_deep_recur.jn` (v1) | 100,000-deep recursion | succeeded |
| `apps/iot_telemetry` | 2000 samples sim | sub-second |

These are tiny, but the *shape* of the numbers is reassuring:
- 100 M tight-loop summation in 2 ms means LLVM is doing its job
  through `--opt 3`.
- 100 K vec churn in 14 ms is consistent with Perceus reuse pairing
  working (otherwise we'd see ~ 10× more time spent in malloc).
- 10 M-deep tail call in 2 ms means TCO is firing and the stack is
  not growing.

The benchmarks themselves all **compile and run** (§ apps sweep);
their reported numbers should be trusted to the same degree the rest
of the project is. The "store_perf_baselines.txt" file is the right
artifact to gate regressions on in CI.

## 19.1 Smell: missing PGO / LTO defaults

The CLI exposes `--lto` but it's not on by default. For alpha
binaries shipped to users, LTO at least on the runtime would be
worth measuring.

## 19.2 Smell: codegen LOC scale suggests slow compile times

29 kLOC of codegen logic compiled at every `jinnc` invocation
is not free. I did not measure end-to-end compile time, but the per-
probe `~30-60 ms` times above are dominated by the LLVM phase, not
the rest of the pipeline (verified by `jinnc check` timing in `bulk_tests`).

---

# §20 Comparison to existing languages

Jinn's positioning, distilled from what works and what doesn't:

| Axis | Jinn (today) | Rust | Zig | Go | C | Swift | Mojo | Kotlin | Scala | Python |
| ---- | ----- | ---- | --- | -- | - | ----- | ---- | ------ | ----- | ------ |
| Memory model | Perceus RC + borrow check | Lifetimes | Manual | GC | Manual | ARC + escape | RC | GC | GC | GC |
| Mem safety (today) | **Partial** | Safe | Manual | Safe | Unsafe | Safe | Safe | Safe | Safe | Safe |
| Compile to native | LLVM | LLVM | LLVM/own | own | (gcc/clang) | LLVM | MLIR | JVM/LLVM | JVM | – |
| Stackful coroutines | **Yes** | No | No | Yes (goroutine) | No | No | – | No | No | – |
| Actor model in stdlib | **Yes** | No | No | No | No | No | – | Akka (lib) | Akka | – |
| Built-in WAL store | **Yes (unique)** | No | No | No | No | No | – | No | No | – |
| Channels in stdlib | Yes | std (sync mpsc) | – | Yes | – | Concurrency | – | Coroutines | – | – |
| Pattern matching | Yes | Yes | switch | switch | switch | Yes | – | when | match | match (3.10+) |
| Significant indent | Yes | No | No | No | No | No | Yes | No | No | Yes |
| Tail-call opt | **Yes (verified)** | No (impl-def) | No | No | No (impl-def) | No | – | No | No | No |
| Generics | Yes (mono) | Yes (mono) | comptime | Yes (1.18+) | – | Yes | Yes | Yes | Yes | duck |
| Comptime | TBD | const_fn | yes (powerful) | – | – | – | yes | – | yes (macro) | – |
| Tooling: LSP | Yes | rust-analyzer | zls | gopls | clangd | sourcekit-lsp | – | kotlin-lsp | metals | pylsp |
| Tooling: pkg mgr | Yes | cargo | zigmod | go mod | – | spm | – | gradle | sbt | pip |
| Tooling: fmt | Yes | rustfmt | zig fmt | gofmt | clang-fmt | swift-fmt | – | ktlint | scalafmt | black |
| WAL/store built-in | **Yes (unique)** | – | – | – | – | – | – | – | – | – |
| Persistent stores keyword | **Yes (unique)** | – | – | – | – | – | – | – | – | – |

**Distinctive bets that pay off (if execution is finished):**
1. **Built-in persistent stores + WAL.** Nobody else in this list
   has it as a first-class language feature. Genuinely novel.
2. **Stackful coroutines + actor mailbox** in the language with
   syntactic sugar (`@handler`, `spawn`, `channel of T (cap)`,
   `select`). Erlang-class concurrency model on an LLVM backend.
3. **Perceus RC** — Lean 4's strategy in a non-academic language.
4. **`*name` function syntax** plus significant indentation gives a
   Python-shaped surface with a Rust-shaped middle.

**Distinctive bets that hurt today:**
1. Enormous keyword set (≈ 80) blocks idiomatic method names like
   `.send()`, `.close()`, `.count()`.
2. Significant indentation means tabs vs spaces is a hard lexer
   error — onboarding friction.
3. The grammar in `jinn.ebnf` and the parser in `src/parser/` are
   not in lock-step.

**Honest positioning:** Jinn is closer to Erlang+Rust+SQLite in a
trench coat than to any of the comparators alone. That is a unique
and defensible niche. The execution needs the safety floor of §13 in
place before it can carry that positioning.
