//! High-level Intermediate Representation (HIR)
//!
//! The HIR mirrors the AST but carries resolved semantic information:
//! - Every expression has a resolved type
//! - Every name is resolved to a definition site (DefId)
//! - Generic instantiations are monomorphized
//! - Coercions are explicit nodes
//! - Ownership categories annotate every binding
//!
//! Pipeline: AST → Typer → HIR → Codegen → LLVM IR

use crate::ast::Span;
use crate::types::Type;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Definition IDs
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Unique identifier for every definition in the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

impl DefId {
    pub const BUILTIN: DefId = DefId(0);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Ownership
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// Ownership category for a binding or value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// Value is moved on use, dropped at scope end.
    Owned,
    /// Immutable borrow — no drop responsibility.
    Borrowed,
    /// Exclusive mutable borrow.
    BorrowMut,
    /// Reference-counted shared ownership.
    Rc,
    /// Raw pointer — unmanaged, no automatic drop.
    Raw,
}

impl Default for Ownership {
    fn default() -> Self {
        Ownership::Owned
    }
}

impl std::fmt::Display for Ownership {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ownership::Owned => f.write_str("owned"),
            Ownership::Borrowed => f.write_str("&"),
            Ownership::BorrowMut => f.write_str("&mut"),
            Ownership::Rc => f.write_str("rc"),
            Ownership::Raw => f.write_str("*raw"),
        }
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Program
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<Fn>,
    pub types: Vec<TypeDef>,
    pub enums: Vec<EnumDef>,
    pub externs: Vec<ExternFn>,
    pub err_defs: Vec<ErrDef>,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Functions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct Fn {
    pub def_id: DefId,
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Block,
    pub span: Span,
    /// If this was monomorphized from a generic, the original name.
    pub generic_origin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub def_id: DefId,
    pub name: String,
    pub ty: Type,
    pub ownership: Ownership,
    pub span: Span,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Type definitions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub def_id: DefId,
    pub name: String,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
    pub span: Span,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Enums
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub def_id: DefId,
    pub name: String,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<VField>,
    pub tag: u32,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VField {
    pub name: Option<String>,
    pub ty: Type,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Externs and errors
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct ExternFn {
    pub def_id: DefId,
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub variadic: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrDef {
    pub def_id: DefId,
    pub name: String,
    pub variants: Vec<ErrVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrVariant {
    pub name: String,
    pub fields: Vec<Type>,
    pub tag: u32,
    pub span: Span,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Blocks and statements
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind(Bind),
    TupleBind(Vec<(DefId, String, Type)>, Expr, Span),
    Assign(Expr, Expr, Span),
    Expr(Expr),
    If(If),
    While(While),
    For(For),
    Loop(Loop),
    Ret(Option<Expr>, Type, Span),
    Break(Option<Expr>, Span),
    Continue(Span),
    Match(Match),
    Asm(AsmBlock),
    /// Drop a value at this point (inserted by ownership pass).
    Drop(DefId, Type, Span),
    ErrReturn(Expr, Type, Span),
}

#[derive(Debug, Clone)]
pub struct Bind {
    pub def_id: DefId,
    pub name: String,
    pub value: Expr,
    pub ty: Type,
    pub ownership: Ownership,
    pub span: Span,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Expressions — every variant carries its resolved Type
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

use crate::ast::{BinOp, UnaryOp};

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    /// Integer literal.
    Int(i64),
    /// Floating-point literal.
    Float(f64),
    /// String literal.
    Str(String),
    /// Boolean literal.
    Bool(bool),
    /// The `none` value.
    None,
    /// Void expression (no value).
    Void,
    /// Resolved variable reference.
    Var(DefId, String),
    /// Resolved function reference (function pointer).
    FnRef(DefId, String),
    /// Enum variant reference (unit variant, no fields).
    VariantRef(String, String, u32),
    /// Binary operation with typed operands.
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    /// Unary operation.
    UnaryOp(UnaryOp, Box<Expr>),
    /// Direct function call with resolved callee.
    Call(DefId, String, Vec<Expr>),
    /// Indirect call through a function pointer.
    IndirectCall(Box<Expr>, Vec<Expr>),
    /// Builtin call (log, to_string, rc, rc_retain, rc_release, popcount, etc.)
    Builtin(BuiltinFn, Vec<Expr>),
    /// Method call — resolved to a concrete function name.
    Method(Box<Expr>, String, String, Vec<Expr>),
    /// String method call — remains method dispatch.
    StringMethod(Box<Expr>, String, Vec<Expr>),
    /// Struct field access.
    Field(Box<Expr>, String, usize),
    /// Array/pointer index.
    Index(Box<Expr>, Box<Expr>),
    /// Ternary conditional.
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Explicit type coercion (inserted by typer).
    Coerce(Box<Expr>, CoercionKind),
    /// Explicit `as` cast from source.
    Cast(Box<Expr>, Type),
    /// Array literal.
    Array(Vec<Expr>),
    /// Tuple literal.
    Tuple(Vec<Expr>),
    /// Struct construction.
    Struct(String, Vec<FieldInit>),
    /// Enum variant construction.
    VariantCtor(String, String, u32, Vec<FieldInit>),
    /// If-expression.
    IfExpr(Box<If>),
    /// Pipe expression — desugared to a call.
    Pipe(Box<Expr>, DefId, String, Vec<Expr>),
    /// Block expression.
    Block(Block),
    /// Lambda / closure.
    Lambda(Vec<Param>, Block),
    /// Address-of.
    Ref(Box<Expr>),
    /// Pointer/Rc dereference.
    Deref(Box<Expr>),
    /// List comprehension.
    ListComp(Box<Expr>, DefId, String, Box<Expr>, Option<Box<Expr>>, Option<Box<Expr>>),
    /// Inline syscall.
    Syscall(Vec<Expr>),
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Builtins
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinFn {
    Log,
    ToString,
    RcAlloc,
    RcRetain,
    RcRelease,
    Popcount,
    Clz,
    Ctz,
    RotateLeft,
    RotateRight,
    Bswap,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Coercions
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone, PartialEq)]
pub enum CoercionKind {
    /// Widen an integer (zero-extend or sign-extend).
    IntWiden { from_bits: u32, to_bits: u32, signed: bool },
    /// Truncate an integer.
    IntTrunc { from_bits: u32, to_bits: u32 },
    /// Float widening (f32 → f64).
    FloatWiden,
    /// Float narrowing (f64 → f32).
    FloatNarrow,
    /// Int to float.
    IntToFloat { signed: bool },
    /// Float to int.
    FloatToInt { signed: bool },
    /// Bool to int (i1 → target int).
    BoolToInt,
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Control flow
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct If {
    pub cond: Expr,
    pub then: Block,
    pub elifs: Vec<(Expr, Block)>,
    pub els: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct While {
    pub cond: Expr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct For {
    pub bind_id: DefId,
    pub bind: String,
    pub bind_ty: Type,
    pub iter: Expr,
    pub end: Option<Expr>,
    pub step: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Loop {
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Match {
    pub subject: Expr,
    pub arms: Vec<Arm>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pat,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild(Span),
    Bind(DefId, String, Type, Span),
    Lit(Expr),
    Ctor(String, u32, Vec<Pat>, Span),
    /// Or-pattern: matches if any alternative matches.
    Or(Vec<Pat>, Span),
    /// Range pattern: matches if value is in [lo, hi].
    Range(Box<Expr>, Box<Expr>, Span),
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Misc
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[derive(Debug, Clone)]
pub struct FieldInit {
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct AsmBlock {
    pub template: String,
    pub outputs: Vec<(String, String)>,
    pub inputs: Vec<(String, Expr)>,
    pub clobbers: Vec<String>,
    pub span: Span,
}
