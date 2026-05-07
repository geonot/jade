# Jade Reserved Keywords

Auto-generated overview of every reserved word the lexer recognises.

Total **90** keyword spellings → **85** distinct tokens.

Generated from [`src/lexer.rs`](../../src/lexer.rs); regenerate with `cargo run --example dump_keywords` (TODO).


## Comparison & boolean operators

| Keyword(s) | Token | Notes |
|---|---|---|
| `is` | `Token::Is` |  |
| `eq`, `equals` | `Token::Equals` | 2 spellings (alias) |
| `neq` | `Token::Neq` |  |
| `lt`, `ngte` | `Token::Lt` | 2 spellings (alias) |
| `gt`, `nlte` | `Token::Gt` | 2 spellings (alias) |
| `lte`, `ngt` | `Token::LtEq` | 2 spellings (alias) |
| `gte`, `nlt` | `Token::GtEq` | 2 spellings (alias) |
| `and` | `Token::And` |  |
| `or` | `Token::Or` |  |
| `not` | `Token::Not` |  |
| `xor` | `Token::Xor` |  |
| `in` | `Token::In` |  |
| `pow` | `Token::StarStar` |  |
| `mod` | `Token::Percent` |  |

## Control flow

| Keyword(s) | Token | Notes |
|---|---|---|
| `if` | `Token::If` |  |
| `elif` | `Token::Elif` |  |
| `else` | `Token::Else` |  |
| `unless` | `Token::Unless` |  |
| `until` | `Token::Until` |  |
| `while` | `Token::While` |  |
| `for` | `Token::For` |  |
| `loop` | `Token::Loop` |  |
| `break` | `Token::Break` |  |
| `continue` | `Token::Continue` |  |
| `return` | `Token::Return` |  |
| `match` | `Token::Match` |  |
| `when` | `Token::When` |  |
| `do` | `Token::Do` |  |
| `end` | `Token::End` |  |
| `defer` | `Token::Defer` |  |
| `yield` | `Token::Yield` |  |

## Declarations & types

| Keyword(s) | Token | Notes |
|---|---|---|
| `type` | `Token::Type` |  |
| `enum` | `Token::Enum` |  |
| `trait` | `Token::Trait` |  |
| `impl` | `Token::Impl` |  |
| `dispatch` | `Token::Dispatch` |  |
| `pub` | `Token::Pub` |  |
| `use` | `Token::Use` |  |
| `as` | `Token::As` |  |
| `from` | `Token::From` |  |
| `to` | `Token::To` |  |
| `by` | `Token::By` |  |
| `of` | `Token::Of` |  |
| `extern` | `Token::Extern` |  |
| `asm` | `Token::Asm` |  |
| `embed` | `Token::Embed` |  |
| `alias` | `Token::Alias` |  |
| `global` | `Token::Global` |  |
| `atomic` | `Token::Atomic` |  |
| `strict` | `Token::Strict` |  |
| `array` | `Token::Array` |  |
| `deque` | `Token::Deque` |  |
| `returns` | `Token::Returns` |  |
| `at` | `Token::AtKw` |  |

## Concurrency

| Keyword(s) | Token | Notes |
|---|---|---|
| `actor` | `Token::Actor` |  |
| `spawn` | `Token::Spawn` |  |
| `send` | `Token::Send` |  |
| `receive` | `Token::Receive` |  |
| `channel` | `Token::Channel` |  |
| `close` | `Token::Close` |  |
| `select` | `Token::Select` |  |
| `sim` | `Token::Sim` |  |
| `supervisor` | `Token::Supervisor` |  |
| `stop` | `Token::Stop` |  |
| `default` | `Token::Default` |  |

## Persistence & store

| Keyword(s) | Token | Notes |
|---|---|---|
| `store` | `Token::Store` |  |
| `migration` | `Token::Migration` |  |
| `insert` | `Token::Insert` |  |
| `delete` | `Token::Delete` |  |
| `set` | `Token::Set` |  |
| `transaction` | `Token::Transaction` |  |
| `view` | `Token::View` |  |
| `query` | `Token::Query` |  |
| `err` | `Token::Err` |  |

## Diagnostics & meta

| Keyword(s) | Token | Notes |
|---|---|---|
| `test` | `Token::Test` |  |
| `assert` | `Token::Assert` |  |
| `log` | `Token::Log` |  |
| `unreachable` | `Token::Unreachable` |  |
| `build` | `Token::Build` |  |
| `syscall` | `Token::Syscall` |  |
| `grad` | `Token::Grad` |  |
| `einsum` | `Token::Einsum` |  |

## Literals

| Keyword(s) | Token | Notes |
|---|---|---|
| `true` | `Token::True` |  |
| `false` | `Token::False` |  |
| `none` | `Token::None` |  |
