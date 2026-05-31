use crate::intern::Symbol;
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
    Struct(Symbol, Vec<Type>),
    Enum(Symbol),
    Fn(Vec<Type>, Box<Type>),
    Param(Symbol),
    Ptr(Box<Type>),
    ActorRef(Symbol),
    Coroutine(Box<Type>),
    Channel(Box<Type>),
    TypeVar(u32),
    Alias(Symbol, Box<Type>),
    Newtype(Symbol, Box<Type>),
    Generator(Box<Type>),
    Row(Symbol),
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

    pub fn is_ptr_represented(&self) -> bool {
        matches!(
            self,
            Self::Ptr(_)
                | Self::ActorRef(_)
                | Self::Coroutine(_)
                | Self::Channel(_)
                | Self::Vec(_)
                | Self::Map(_, _)
                | Self::Generator(_)
        )
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

    pub fn is_value_clonable(&self) -> bool {
        if self.is_trivially_droppable() {
            return true;
        }
        match self {
            Self::String => true,
            Self::Vec(elem) | Self::Array(elem, _) => elem.is_value_clonable(),
            Self::Tuple(tys) => tys.iter().all(|t| t.is_value_clonable()),
            Self::Struct(_, _) => true,
            Self::Alias(_, inner) | Self::Newtype(_, inner) => inner.is_value_clonable(),
            _ => false,
        }
    }

    pub fn default_ownership(&self) -> crate::hir::Ownership {
        match self {
            Self::Ptr(_) => crate::hir::Ownership::Raw,
            _ => crate::hir::Ownership::Owned,
        }
    }

    pub fn needs_atomic_rc(&self) -> bool {
        match self {
            Self::ActorRef(_) | Self::Channel(_) | Self::Coroutine(_) | Self::Generator(_) => true,
            Self::Vec(inner) | Self::Ptr(inner) => inner.needs_atomic_rc(),
            Self::Map(k, v) => k.needs_atomic_rc() || v.needs_atomic_rc(),
            Self::Array(inner, _) => inner.needs_atomic_rc(),
            Self::Tuple(tys) => tys.iter().any(|t| t.needs_atomic_rc()),
            Self::Fn(params, ret) => {
                params.iter().any(|t| t.needs_atomic_rc()) || ret.needs_atomic_rc()
            }
            Self::Alias(_, inner) | Self::Newtype(_, inner) => inner.needs_atomic_rc(),
            _ => false,
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
            Self::String => f.write_str("string"),
            Self::TypeVar(n) => write!(f, "?{n}"),
            Self::Struct(n, params) => {
                write!(f, "{n}")?;
                if !params.is_empty() {
                    f.write_str("<")?;
                    for (i, p) in params.iter().enumerate() {
                        if i > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{p}")?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
            Self::Enum(n) => write!(f, "{n}"),
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
            Self::Param(n) => write!(f, "{n}"),
            Self::Ptr(inner) => write!(f, "&{inner}"),
            Self::ActorRef(name) => write!(f, "ActorRef<{name}>"),
            Self::Coroutine(inner) => write!(f, "Coroutine of {inner}"),
            Self::Channel(inner) => write!(f, "Channel of {inner}"),
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
            Self::Alias(name, inner) => write!(f, "alias {name} is {inner}"),
            Self::Newtype(name, inner) => write!(f, "newtype {name} is {inner}"),
            Self::Generator(inner) => write!(f, "Generator of {inner}"),
            Self::Row(name) => write!(f, "Row<{name}>"),
        }
    }
}

impl Type {
    pub fn canonical(&self) -> Type {
        match self {
            Type::Struct(n, _) if n.as_str() == "String" || n.as_str() == "string" => Type::String,
            Type::Alias(_, inner) => inner.canonical(),
            Type::Array(inner, n) => Type::Array(Box::new(inner.canonical()), *n),
            Type::Vec(inner) => Type::Vec(Box::new(inner.canonical())),
            Type::Map(k, v) => Type::Map(Box::new(k.canonical()), Box::new(v.canonical())),
            Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| t.canonical()).collect()),
            Type::Struct(n, args) => {
                Type::Struct(n.clone(), args.iter().map(|t| t.canonical()).collect())
            }
            Type::Fn(ps, r) => Type::Fn(
                ps.iter().map(|t| t.canonical()).collect(),
                Box::new(r.canonical()),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(inner.canonical())),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(inner.canonical())),
            Type::Channel(inner) => Type::Channel(Box::new(inner.canonical())),
            Type::Generator(inner) => Type::Generator(Box::new(inner.canonical())),
            Type::Newtype(n, inner) => Type::Newtype(n.clone(), Box::new(inner.canonical())),
            other => other.clone(),
        }
    }

    pub fn eq_canonical(&self, other: &Type) -> bool {
        self.canonical() == other.canonical()
    }

    pub fn is_string(&self) -> bool {
        matches!(self.canonical(), Type::String)
    }

    /// Debug-only invariant check: returns `true` if any nested type is a
    /// `Struct("String"|"string")`, i.e. a non-canonical spelling of
    /// [`Type::String`] that should have been normalized at the resolution
    /// boundary (parser `ident_to_type`, typer `ident_to_type`, iterator
    /// element inference). Such a type is canonicalized away by
    /// [`Type::canonical`], but its presence means a birth site forgot to
    /// canonicalize, so equality/`matches!` checks that bypass `canonical`
    /// could silently mis-compare. Used by the `debug_assert!` at the
    /// unification entry to catch regressions.
    pub(crate) fn has_string_struct(&self) -> bool {
        match self {
            Type::Struct(n, args) => {
                n.as_str() == "String"
                    || n.as_str() == "string"
                    || args.iter().any(Type::has_string_struct)
            }
            Type::Array(inner, _)
            | Type::Vec(inner)
            | Type::Ptr(inner)
            | Type::Coroutine(inner)
            | Type::Channel(inner)
            | Type::Generator(inner)
            | Type::Alias(_, inner)
            | Type::Newtype(_, inner) => inner.has_string_struct(),
            Type::Map(k, v) => k.has_string_struct() || v.has_string_struct(),
            Type::Tuple(ts) => ts.iter().any(Type::has_string_struct),
            Type::Fn(ps, r) => ps.iter().any(Type::has_string_struct) || r.has_string_struct(),
            _ => false,
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
            | Self::Coroutine(inner)
            | Self::Channel(inner) => inner.has_type_var(),
            Self::Map(k, v) => k.has_type_var() || v.has_type_var(),
            Self::Tuple(tys) => tys.iter().any(|t| t.has_type_var()),
            Self::Fn(params, ret) => params.iter().any(|t| t.has_type_var()) || ret.has_type_var(),
            _ => false,
        }
    }

    pub fn free_type_vars(&self, out: &mut std::collections::HashSet<u32>) {
        match self {
            Self::TypeVar(v) => {
                out.insert(*v);
            }
            Self::Array(inner, _)
            | Self::Vec(inner)
            | Self::Ptr(inner)
            | Self::Coroutine(inner)
            | Self::Channel(inner) => inner.free_type_vars(out),
            Self::Map(k, v) => {
                k.free_type_vars(out);
                v.free_type_vars(out);
            }
            Self::Tuple(tys) => {
                for t in tys {
                    t.free_type_vars(out);
                }
            }
            Self::Fn(params, ret) => {
                for t in params {
                    t.free_type_vars(out);
                }
                ret.free_type_vars(out);
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scheme {
    pub quantified: Vec<u32>,
    pub ty: Type,
}

impl Scheme {
    pub fn mono(ty: Type) -> Self {
        Scheme {
            quantified: vec![],
            ty,
        }
    }

    pub fn is_poly(&self) -> bool {
        !self.quantified.is_empty()
    }
}
