# Security Policy

## Supported versions

Jinn is pre-1.0 software under active development. During the alpha series,
security fixes are applied to the `main` branch only. There is not yet a
released version line with backported patches; once `1.0` ships, this section
will enumerate the supported version ranges.

| Version | Supported |
| --- | --- |
| `main` (development) | ✅ |
| Pre-release tags | ⚠️ best-effort, no backports |

## Reporting a vulnerability

**Please do not open public issues for security vulnerabilities.**

If you believe you have found a security vulnerability in the Jinn compiler
(`jinnc`), the runtime (`runtime/`), the language server (`jinnc-lsp`), or the
standard library (`std/`, `libjn/`), report it privately:

- Open a [GitHub security advisory](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)
  ("Report a vulnerability") on the repository, **or**
- Contact the maintainers privately through the repository's listed contact
  channel.

Please include:

- A description of the issue and its impact.
- A minimal reproduction: a `.jn` program and the exact `jinnc` invocation, or a
  Rust test, that triggers the problem.
- The affected component (compiler, runtime, LSP, stdlib) and commit hash.
- Any suggested remediation, if known.

## Scope

Security-relevant issues include, but are not limited to:

- **Memory safety in the runtime** (`runtime/*.c`): out-of-bounds access, use
  after free, double free, data races, or signal-handler unsafety.
- **Unsoundness in the compiler** that allows safe-looking source to produce
  memory-unsafe machine code (miscompilation of bounds checks, reference
  counting, or aliasing).
- **Standard-library functions** that mishandle untrusted input (parsers,
  codecs, crypto helpers) in a way that leads to memory corruption or incorrect
  security guarantees.
- **Crashes on untrusted input** that are exploitable beyond a clean abort.

The following are generally **not** treated as vulnerabilities:

- A program that intentionally invokes undefined behaviour via `unsafe`-style
  primitives (e.g. `volatile_load`/`volatile_store` with bogus addresses).
- Resource exhaustion from deliberately pathological input (e.g. unbounded
  recursion) that terminates with a clean, diagnosed abort. Jinn already detects
  coroutine and native-thread stack overflow and exits with status `134` and a
  specific diagnostic.

## Disclosure process

1. We acknowledge the report and begin assessment.
2. We develop and validate a fix on a private branch with a regression test.
3. We coordinate a disclosure timeline with the reporter.
4. We publish the fix and credit the reporter (unless anonymity is requested).

We aim to handle reports promptly and will keep reporters informed of progress.
