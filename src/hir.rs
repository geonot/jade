use crate::ast::Span;
use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

impl DefId {
    pub const BUILTIN: DefId = DefId(0);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    Owned,
    Borrowed,
    BorrowMut,
    Rc,
    Weak,
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
            Ownership::Weak => f.write_str("weak"),
            Ownership::Raw => f.write_str("*raw"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub fns: Vec<Fn>,
    pub types: Vec<TypeDef>,
    pub enums: Vec<EnumDef>,
    pub externs: Vec<ExternFn>,
    pub err_defs: Vec<ErrDef>,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub def_id: DefId,
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Type,
    pub body: Block,
    pub span: Span,
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

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub def_id: DefId,
    pub name: String,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub layout: crate::ast::LayoutAttrs,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
    pub span: Span,
}

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
    Drop(DefId, String, Type, Span),
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

use crate::ast::{BinOp, UnaryOp};

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    None,
    Void,
    Var(DefId, String),
    FnRef(DefId, String),
    VariantRef(String, String, u32),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    Call(DefId, String, Vec<Expr>),
    IndirectCall(Box<Expr>, Vec<Expr>),
    Builtin(BuiltinFn, Vec<Expr>),
    Method(Box<Expr>, String, String, Vec<Expr>),
    StringMethod(Box<Expr>, String, Vec<Expr>),
    Field(Box<Expr>, String, usize),
    Index(Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Coerce(Box<Expr>, CoercionKind),
    Cast(Box<Expr>, Type),
    Array(Vec<Expr>),
    Tuple(Vec<Expr>),
    Struct(String, Vec<FieldInit>),
    VariantCtor(String, String, u32, Vec<FieldInit>),
    IfExpr(Box<If>),
    Pipe(Box<Expr>, DefId, String, Vec<Expr>),
    Block(Block),
    Lambda(Vec<Param>, Block),
    Ref(Box<Expr>),
    Deref(Box<Expr>),
    ListComp(
        Box<Expr>,
        DefId,
        String,
        Box<Expr>,
        Option<Box<Expr>>,
        Option<Box<Expr>>,
    ),
    Syscall(Vec<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BuiltinFn {
    Log,
    ToString,
    RcAlloc,
    RcRetain,
    RcRelease,
    WeakAlloc,
    WeakUpgrade,
    WeakDowngrade,
    VolatileLoad,
    VolatileStore,
    WrappingAdd,
    WrappingSub,
    WrappingMul,
    SaturatingAdd,
    SaturatingSub,
    SaturatingMul,
    CheckedAdd,
    CheckedSub,
    CheckedMul,
    SignalHandle,
    SignalRaise,
    SignalIgnore,
    Popcount,
    Clz,
    Ctz,
    RotateLeft,
    RotateRight,
    Bswap,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CoercionKind {
    IntWiden {
        from_bits: u32,
        to_bits: u32,
        signed: bool,
    },
    IntTrunc {
        from_bits: u32,
        to_bits: u32,
    },
    FloatWiden,
    FloatNarrow,
    IntToFloat {
        signed: bool,
    },
    FloatToInt {
        signed: bool,
    },
    BoolToInt,
}

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
    Or(Vec<Pat>, Span),
    Range(Box<Expr>, Box<Expr>, Span),
}

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
