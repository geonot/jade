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
    Test(TestBlock),
    Actor(ActorDef),
    Store(StoreDef),
    Trait(TraitDef),
    Impl(ImplBlock),
    Const(String, Expr, Span),
    Supervisor(SupervisorDef),
    TypeAlias(String, Type, Span),
    Newtype(String, Type, Span),
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
    StoreInsert(String, Vec<Expr>, Span),
    StoreDelete(String, StoreFilter, Span),
    StoreSet(String, Vec<(String, Expr)>, StoreFilter, Span),
    Transaction(Block, Span),
    ChannelClose(Expr, Span),
    Stop(Expr, Span),
    SimFor(For, Span),
    UseLocal(UseDecl),
}

#[derive(Debug, Clone)]
pub enum Pat {
    Wild(Span),
    Ident(String, Span),
    Lit(Expr),
    Ctor(String, Vec<Pat>, Span),
    Or(Vec<Pat>, Span),
    Range(Expr, Expr, Span),
    Tuple(Vec<Pat>, Span),
    Array(Vec<Pat>, Span),
}

impl Pat {
    pub fn span(&self) -> Span {
        match self {
            Pat::Wild(s)
            | Pat::Ident(_, s)
            | Pat::Ctor(_, _, s)
            | Pat::Or(_, s)
            | Pat::Range(_, _, s)
            | Pat::Tuple(_, s)
            | Pat::Array(_, s) => *s,
            Pat::Lit(e) => e.span(),
        }
    }
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
    Embed(String, Span),
    ListComp(
        Box<Expr>,
        String,
        Box<Expr>,
        Option<Box<Expr>>,
        Option<Box<Expr>>,
        Span,
    ),
    Syscall(Vec<Expr>, Span),
    Query(Box<Expr>, Vec<QueryClause>, Span),
    StoreQuery(String, Box<StoreFilter>, Span),
    StoreCount(String, Span),
    StoreAll(String, Span),
    Spawn(String, Span),
    Send(Box<Expr>, String, Vec<Expr>, Span),
    Receive(Vec<ReceiveArm>, Span),
    Yield(Box<Expr>, Span),
    DispatchBlock(String, Block, Span),
    ChannelCreate(Option<Type>, Box<Expr>, Span),
    ChannelSend(Box<Expr>, Box<Expr>, Span),
    ChannelRecv(Box<Expr>, Span),
    Select(Vec<SelectArm>, Option<Block>, Span),
    Unreachable(Span),
    AsFormat(Box<Expr>, String, Span),
    StrictCast(Box<Expr>, Type, Span),
    Slice(Box<Expr>, Box<Expr>, Box<Expr>, Span),
    NamedArg(String, Box<Expr>, Span),
    Spread(Box<Expr>, Span),
    NDArray(Vec<Expr>, Span),
    SIMDLit(Type, usize, Vec<Expr>, Span),
    Grad(Box<Expr>, Span),
    Einsum(String, Vec<Expr>, Span),
    Builder(String, Vec<BuilderField>, Span),
    Deque(Vec<Expr>, Span),
    OfCall(Box<Expr>, Box<Expr>, Span),
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
            | Self::Embed(_, s)
            | Self::ListComp(_, _, _, _, _, s)
            | Self::Syscall(_, s)
            | Self::Query(_, _, s)
            | Self::StoreQuery(_, _, s)
            | Self::StoreCount(_, s)
            | Self::StoreAll(_, s)
            | Self::Spawn(_, s)
            | Self::Send(_, _, _, s)
            | Self::Receive(_, s)
            | Self::Yield(_, s)
            | Self::DispatchBlock(_, _, s)
            | Self::ChannelCreate(_, _, s)
            | Self::ChannelSend(_, _, s)
            | Self::ChannelRecv(_, s)
            | Self::Select(_, _, s)
            | Self::Unreachable(s)
            | Self::AsFormat(_, _, s)
            | Self::StrictCast(_, _, s)
            | Self::Slice(_, _, _, s)
            | Self::NamedArg(_, _, s)
            | Self::Spread(_, s)
            | Self::NDArray(_, s)
            | Self::SIMDLit(_, _, _, s) => *s,
            Self::IfExpr(i) => i.span,
            Self::Grad(_, s)
            | Self::Einsum(_, _, s)
            | Self::Builder(_, _, s)
            | Self::Deque(_, s)
            | Self::OfCall(_, _, s) => *s,
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
    pub type_bounds: Vec<(String, Vec<String>)>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Block,
    pub is_generator: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
    pub literal: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct LayoutAttrs {
    pub packed: bool,
    pub strict: bool,
    pub align: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct TypeDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<Field>,
    pub methods: Vec<Fn>,
    pub layout: LayoutAttrs,
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
    pub discriminant: Option<i64>,
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
    pub label: Option<String>,
    pub bind: String,
    pub bind2: Option<String>,
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
    pub guard: Option<Expr>,
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
pub enum QueryClause {
    Where(Expr, Span),
    Limit(Expr, Span),
    Sort(String, bool, Span),
    Take(Expr, Span),
    Skip(Expr, Span),
    Set(String, Expr, Span),
    Delete(Span),
}

#[derive(Debug, Clone)]
pub struct UseDecl {
    pub path: Vec<String>,
    pub imports: Option<Vec<String>>,
    pub alias: Option<String>,
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

#[derive(Debug, Clone)]
pub struct TestBlock {
    pub name: String,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ActorDef {
    pub name: String,
    pub fields: Vec<Field>,
    pub handlers: Vec<Handler>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Handler {
    pub name: String,
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ReceiveArm {
    pub handler: String,
    pub bindings: Vec<String>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct SelectArm {
    pub is_send: bool,
    pub chan: Expr,
    pub value: Option<Expr>,
    pub binding: Option<String>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreDef {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitDef {
    pub name: String,
    pub type_params: Vec<String>,
    pub assoc_types: Vec<String>,
    pub methods: Vec<TraitMethod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub default_body: Option<Block>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub trait_name: Option<String>,
    pub trait_type_args: Vec<Type>,
    pub type_name: String,
    pub assoc_type_bindings: Vec<(String, Type)>,
    pub methods: Vec<Fn>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorStrategy {
    OneForOne,
    OneForAll,
    RestForOne,
}

#[derive(Debug, Clone)]
pub struct SupervisorDef {
    pub name: String,
    pub strategy: SupervisorStrategy,
    pub children: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct BuilderField {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StoreFilter {
    pub field: String,
    pub op: BinOp,
    pub value: Expr,
    pub span: Span,
    pub extra: Vec<(LogicalOp, StoreFilterCond)>,
}

#[derive(Debug, Clone)]
pub struct StoreFilterCond {
    pub field: String,
    pub op: BinOp,
    pub value: Expr,
}
