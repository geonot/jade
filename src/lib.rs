pub mod ast;
pub mod cache;
pub mod codegen;
pub mod comptime;
pub mod diagnostic;
pub mod hir;
pub mod hir_validate;
pub mod lexer;
pub mod lock;
pub mod ownership;
pub mod parser;
pub mod perceus;
pub mod pkg;
pub mod typer;
pub mod types;

pub use codegen::Compiler;
