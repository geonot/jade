# §26 Perspectives panel — Linus, Geohot, Carmack, Knuth, Antirez

I asked myself how each of the user's invoked engineers would react
to this codebase, going by their public writing and known taste. The
purpose is not impersonation but useful pressure on the design
choices.

## Linus Torvalds

> "I'd merge most of this. The lexer is fine. The MIR side-table
> design — drops in the IR, metadata on the side — that's good taste.
>
> What I would not merge:
>
> - The auto-widen comment in `mir_codegen/helpers/values.rs`. That
>   is somebody papering over a bug in another part of the tree. The
>   correct patch is to *fix* the typer. The code-author knew it,
>   wrote the comment, and shipped anyway. That's how technical debt
>   gets metastatic.
>
> - The double codegen path. Pick one. The fact that
>   `checked_divmod` exists and is correct, and yet the actually-used
>   path doesn't call it, is not a bug — it is a *taste* failure. You
>   have safety code that is dead.
>
> - The `panic!()` in `escape/mod.rs` saying 'expected Bind in slot
>   0'. That is an invariant the caller should have enforced. Move it
>   upstream or make it a recoverable error.
>
> Other than that: stop writing memory-unsafe code in a 'memory-safe
> language'. SIGSEGV on `v[5]` is not a feature."

## George Hotz (geohot)

> "Cool that it builds 16 demo apps including a raft cluster and
> lattice crypto. Most languages at this stage have hello world.
> 
> But: `10 / 0` returns a stack address? `v[5]` segfaults? `map(v,
> $ * 2)` ICEs the compiler? *None of those should happen in any
> language that wants to be taken seriously*. Strip the problem to its
> core — every single one of those is a single LLVM IR instruction
> that needed a check before it. Write the check.
>
> The codegen is 30 kLOC. Why? Because two paths. Delete one. Then it
> is 15 kLOC. Then look at what's left and ask which 5 kLOC of it
> exist to paper over MIR's lack of a verifier — delete those too.
>
> Generator emits `ret i64 0` for a `ptr`-returning function and LLVM
> catches it. That's *good* — the verifier saved you. Now write the
> verifier you don't yet have, *for MIR*, and never let LLVM be the
> first one to notice."

## John Carmack

> "The architecture is clear. The boundaries are real. The
> instrumentation is missing.
>
> Specifically: I see no place where you can ask 'what did the
> compiler do to my program?' beyond `--emit-llvm` / `--emit-mir`. A
> structured `--explain perceus`, `--explain ownership`, `--explain
> tco` would pay back its development cost in the first week of
> users. Carmack's law: the first version of any feature is broken;
> the way you find out is by looking.
>
> The runtime: 64 KB stacks with a guard page is the right model.
> But a guard page that just becomes SIGSEGV without a diagnostic is
> half a feature. `sigaction(SIGSEGV)` + `siginfo->si_addr` check + a
> readable message is one afternoon's work and pays back forever.
>
> 1,570 tests is impressive. Zero fuzz, zero ASan, zero TSan is not.
> Those tools find what your test corpus by definition cannot — they
> sample the space orthogonally to your imagination.
>
> Also: profile first. Number of `.expect()` calls is a metric only
> in aggregate; what matters is which ones are hot."

## Donald Knuth

> "The keyword table is large enough that it deserves a documented
> classification (`statement keywords`, `expression keywords`,
> `type-constructor keywords`, `query keywords`) and a documented
> rule for contextual de-promotion. The current table is a flat list
> and the resulting parsing decisions are correspondingly ad-hoc.
>
> The grammar in `jinn.ebnf` does not match the parser. This is
> always a sign that one of the two is wrong, and the user community
> will discover it by accident. Generate the parser from the grammar,
> or generate the grammar from the parser; do not maintain both by
> hand.
>
> The MIR Perceus side-table is a beautiful piece of separation of
> concerns. The naming (`reuse_save` / `reuse_consume`) is clear.
> Document it in the published reference — it is the kind of detail
> that distinguishes a serious language design.
>
> Finally: the lack of TODO/FIXME markers anywhere is, in my
> experience, evidence either of extraordinary discipline or of
> their having been scrubbed. If the former, congratulations; if the
> latter, please put them back. Marking the work you know is unfinished
> is courtesy to your future self."

## Salvatore Sanfilippo (antirez)

> "I would write a Jinn program today, for real, if the safety floor
> were in place. Built-in WAL stores + actors + channels + a single
> `jinnc init && jinnc build && jinnc run` workflow is what I always
> wanted for Redis-class projects. The fact that the lattice_crypto
> app builds and runs tells me the language is much further along
> than its bug list suggests.
>
> My one critique is that the documentation does not match the
> ambition of the language. There are a dozen markdown files at the
> repo root — VISION, ROADMAP, JINN, JINN_DEV_REPORT, the-way-of-jinn,
> perspectives — and a user has no clear path through them. Strip all
> of that, put it on a single page that says 'Jinn is X, here is how
> to install it, here is a 50-line program', and link the deeper
> material from there.
>
> Also: the apps/ directory is the best documentation in the project.
> Make sure every file in it stays compilable as a CI gate. Those 16
> programs are the language's portfolio."

---

# Closing

Jinn is **closer to a real language than its bug list suggests** — 16
substantial apps and 33 benchmarks compile and run today. But the
**safety floor below the working programs is below the alpha bar**:
trivial arithmetic, trivial indexing, and trivial idioms (`map(v, $ * 2)`)
all produce undefined behaviour or compiler crashes.

The path to alpha is concrete and bounded: Phase A in §24 — perhaps
2–3 engineer-weeks — fixes every P0 finding in §21 by deleting code
as often as adding it (auto-widen patch goes away, double codegen
path collapses, MIR verifier replaces a class of latent ICEs with
build-time errors).

After Phase A the language can be honestly described as alpha-quality.
After Phase B and C it can credibly compete in the niche it has
carved out: **Erlang × Rust × SQLite, with significant indentation, on
LLVM**.

That niche is real. Jinn is the only entrant.
