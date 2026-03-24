#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Void,
    String,
    Array(Box<Type>, usize),
    Vec(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Struct(String),
    Enum(String),
    Fn(Vec<Type>, Box<Type>),
    Param(String),
    Ptr(Box<Type>),
    Rc(Box<Type>),
    Weak(Box<Type>),
    ActorRef(String),
    Coroutine(Box<Type>),
    Channel(Box<Type>),
    DynTrait(String),
    Inferred,
    TypeVar(u32),
}

impl Type {
    pub fn is_int(&self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
        )
    }
    pub fn is_signed(&self) -> bool {
        matches!(self, Self::I8 | Self::I16 | Self::I32 | Self::I64)
    }
    pub fn is_float(&self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }
    pub fn is_num(&self) -> bool {
        self.is_int() || self.is_float()
    }
    pub fn bits(&self) -> u32 {
        match self {
            Self::I8 | Self::U8 => 8,
            Self::I16 | Self::U16 => 16,
            Self::I32 | Self::U32 | Self::F32 => 32,
            Self::I64 | Self::U64 | Self::F64 => 64,
            Self::Bool => 1,
            _ => 64,
        }
    }

    pub fn is_rc(&self) -> bool {
        matches!(self, Self::Rc(_))
    }

    pub fn is_weak(&self) -> bool {
        matches!(self, Self::Weak(_))
    }

    pub fn is_trivially_droppable(&self) -> bool {
        match self {
            Self::I8
            | Self::I16
            | Self::I32
            | Self::I64
            | Self::U8
            | Self::U16
            | Self::U32
            | Self::U64
            | Self::F32
            | Self::F64
            | Self::Bool
            | Self::Void
            | Self::Inferred
            | Self::TypeVar(_)
            | Self::Ptr(_)
            | Self::ActorRef(_)
            | Self::Channel(_) => true,
            Self::Array(inner, _) => inner.is_trivially_droppable(),
            Self::Vec(_) | Self::Map(_, _) => false,
            Self::Tuple(tys) => tys.iter().all(|t| t.is_trivially_droppable()),
            _ => false,
        }
    }

    pub fn default_ownership(&self) -> crate::hir::Ownership {
        match self {
            Self::Rc(_) => crate::hir::Ownership::Rc,
            Self::Weak(_) => crate::hir::Ownership::Weak,
            Self::Ptr(_) => crate::hir::Ownership::Raw,
            _ => crate::hir::Ownership::Owned,
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::I8 => f.write_str("i8"),
            Self::I16 => f.write_str("i16"),
            Self::I32 => f.write_str("i32"),
            Self::I64 => f.write_str("i64"),
            Self::U8 => f.write_str("u8"),
            Self::U16 => f.write_str("u16"),
            Self::U32 => f.write_str("u32"),
            Self::U64 => f.write_str("u64"),
            Self::F32 => f.write_str("f32"),
            Self::F64 => f.write_str("f64"),
            Self::Bool => f.write_str("bool"),
            Self::Void => f.write_str("void"),
            Self::String => f.write_str("String"),
            Self::Inferred => f.write_str("_"),
            Self::TypeVar(n) => write!(f, "?{n}"),
            Self::Struct(n) | Self::Enum(n) => f.write_str(n),
            Self::Array(e, l) => write!(f, "[{e}; {l}]"),
            Self::Vec(e) => write!(f, "Vec of {e}"),
            Self::Map(k, v) => write!(f, "Map of {k}, {v}"),
            Self::Tuple(ts) => {
                f.write_str("(")?;
                for (i, t) in ts.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str(")")
            }
            Self::Param(n) => f.write_str(n),
            Self::Ptr(inner) => write!(f, "&{inner}"),
            Self::Rc(inner) => write!(f, "rc {inner}"),
            Self::Weak(inner) => write!(f, "weak {inner}"),
            Self::ActorRef(name) => write!(f, "ActorRef<{name}>"),
            Self::Coroutine(inner) => write!(f, "Coroutine of {inner}"),
            Self::Channel(inner) => write!(f, "Channel of {inner}"),
            Self::DynTrait(name) => write!(f, "dyn {name}"),
            Self::Fn(ps, r) => {
                f.write_str("(")?;
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{p}")?;
                }
                write!(f, ") -> {r}")
            }
        }
    }
}

impl Type {
    pub fn has_type_var(&self) -> bool {
        match self {
            Self::TypeVar(_) => true,
            Self::Array(inner, _)
            | Self::Vec(inner)
            | Self::Ptr(inner)
            | Self::Rc(inner)
            | Self::Weak(inner)
            | Self::Coroutine(inner)
            | Self::Channel(inner) => inner.has_type_var(),
            Self::Map(k, v) => k.has_type_var() || v.has_type_var(),
            Self::Tuple(tys) => tys.iter().any(|t| t.has_type_var()),
            Self::Fn(params, ret) => params.iter().any(|t| t.has_type_var()) || ret.has_type_var(),
            _ => false,
        }
    }

    /// Collect all TypeVar ids that appear in this type.
    pub fn free_type_vars(&self, out: &mut std::collections::HashSet<u32>) {
        match self {
            Self::TypeVar(v) => { out.insert(*v); }
            Self::Array(inner, _)
            | Self::Vec(inner)
            | Self::Ptr(inner)
            | Self::Rc(inner)
            | Self::Weak(inner)
            | Self::Coroutine(inner)
            | Self::Channel(inner) => inner.free_type_vars(out),
            Self::Map(k, v) => { k.free_type_vars(out); v.free_type_vars(out); }
            Self::Tuple(tys) => { for t in tys { t.free_type_vars(out); } }
            Self::Fn(params, ret) => {
                for t in params { t.free_type_vars(out); }
                ret.free_type_vars(out);
            }
            _ => {}
        }
    }
}

/// A polymorphic type scheme: ∀ quantified. ty
/// Used for let-generalization (Algorithm J).
#[derive(Debug, Clone)]
pub struct Scheme {
    /// TypeVar ids that are universally quantified
    pub quantified: Vec<u32>,
    /// The underlying type (may contain the quantified TypeVars)
    pub ty: Type,
}

impl Scheme {
    /// A monomorphic scheme (no quantified variables).
    pub fn mono(ty: Type) -> Self {
        Scheme { quantified: vec![], ty }
    }

    /// Whether this scheme is polymorphic (has quantified variables).
    pub fn is_poly(&self) -> bool {
        !self.quantified.is_empty()
    }
}
