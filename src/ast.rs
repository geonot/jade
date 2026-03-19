use crate::types::Type;

pub type Block = Vec<Stmt>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: u32,
    pub col: u32,
}

impl Span {
    pub fn new(start: usize, end: usize, line: u32, col: u32) -> Self {
        Self {
            start,
            end,
            line,
            col,
        }
    }
    pub fn dummy() -> Self {
        Self {
            start: 0,
            end: 0,
            line: 0,
            col: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Decl {
    Fn(Fn),
    Type(TypeDef),
    Enum(EnumDef),
    Extern(ExternFn),
    Use(UseDecl),
    ErrDef(ErrDef),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    BitOr,
    BitXor,
    BitAnd,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Bind(Bind),
    TupleBind(Vec<String>, Expr, Span),
    Assign(Expr, Expr, Span),
    Expr(Expr),
    If(If),
    While(While),
    For(For),
    Loop(Loop),
    Ret(Option<Expr>, Span),
    Break(Option<Expr>, Span),
    Continue(Span),
    Match(Match),
    Asm(AsmBlock),
    ErrReturn(Expr, Span),
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild(Span),
    Ident(String, Span),
    Lit(Expr),
    Ctor(String, Vec<Pat>, Span),
}

#[derive(Debug, Clone)]
pub enum Expr {
    None(Span),
    Void(Span),
    Int(i64, Span),
    Float(f64, Span),
    Str(String, Span),
    Bool(bool, Span),
    Ident(String, Span),
    BinOp(Box<Expr>, BinOp, Box<Expr>, Span),
    UnaryOp(UnaryOp, Box<Expr>, Span),
    Call(Box<Expr>, Vec<Expr>, Span),
    Method(Box<Expr>, String, Vec<Expr>, Span),
    Field(Box<Expr>, String, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    As(Box<Expr>, Type, Span),
    Array(Vec<Expr>, Span),
    Tuple(Vec<Expr>, Span),
    Struct(String, Vec<FieldInit>, Span),
    IfExpr(Box<If>),
    Pipe(Box<Expr>, Box<Expr>, Vec<Expr>, Span),
    Block(Block, Span),
    Lambda(Vec<Param>, Option<Type>, Block, Span),
    Placeholder(Span),
    Ref(Box<Expr>, Span),
    Deref(Box<Expr>, Span),
    ListComp(
        Box<Expr>,
        String,
        Box<Expr>,
        Option<Box<Expr>>,
        Option<Box<Expr>>,
        Span,
    ),
    Syscall(Vec<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Self::None(s)
            | Self::Void(s)
            | Self::Int(_, s)
            | Self::Float(_, s)
            | Self::Str(_, s)
            | Self::Bool(_, s)
            | Self::Ident(_, s)
            | Self::BinOp(_, _, _, s)
            | Self::UnaryOp(_, _, s)
            | Self::Call(_, _, s)
            | Self::Method(_, _, _, s)
            | Self::Field(_, _, s)
            | Self::Index(_, _, s)
            | Self::Ternary(_, _, _, s)
            | Self::As(_, _, s)
            | Self::Array(_, s)
            | Self::Tuple(_, s)
            | Self::Struct(_, _, s)
            | Self::Pipe(_, _, _, s)
            | Self::Block(_, s)
            | Self::Lambda(_, _, _, s)
            | Self::Placeholder(s)
            | Self::Ref(_, s)
            | Self::Deref(_, s)
            | Self::ListComp(_, _, _, _, _, s)
            | Self::Syscall(_, s) => *s,
            Self::IfExpr(i) => i.span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Program {
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Fn {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<Variant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<VField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VField {
    pub name: Option<String>,
    pub ty: Type,
}

#[derive(Debug, Clone)]
pub struct Bind {
    pub name: String,
    pub value: Expr,
    pub ty: Option<Type>,
    pub span: Span,
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
    pub bind: String,
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
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pat,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ExternFn {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub variadic: bool,
    pub span: Span,
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

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrDef {
    pub name: String,
    pub variants: Vec<ErrVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ErrVariant {
    pub name: String,
    pub fields: Vec<Type>,
    pub span: Span,
}
