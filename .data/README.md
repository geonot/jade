# Scratch data directory

Working directory for ad-hoc program runs. A Jinn `store` resolves its
`.store` / `.wal` files relative to the process cwd, so running a program
with persistent state from the repo root litters the root.

Run such programs from here instead:

    cd .data && ../target/release/my_program

Everything in this directory except this file is gitignored. Tests run
their binaries in a per-test temp dir and benchmarks use
`benchmarks/_build/`, so neither depends on this directory existing.
