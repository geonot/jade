//! Library facade for the Jinn compiler. Re-exports the pipeline modules so the CLI driver and LSP can share types.

pub mod ast;
pub mod bind;
pub mod cache;
pub mod codegen;
pub mod comptime;
pub mod diagnostic;
pub mod driver;
pub mod fmt;
pub mod hir;
pub mod hir_validate;
pub mod incr;
pub mod interface;
pub mod intern;
pub mod lexer;
pub mod lock;
pub mod lsp;
pub mod mir;
pub mod ownership;
pub mod parser;
pub mod perceus;
pub mod pkg;
pub mod resolve;
pub mod runtime_ffi;
pub mod typer;
pub mod types;

pub use codegen::Compiler;
