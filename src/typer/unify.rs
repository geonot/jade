use crate::ast::Span;
use crate::types::Type;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ConstraintOrigin {
    pub span: Span,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum TypeConstraint {
    None,
    Numeric,  // I8-U64, F32-F64
    Integer,  // I8-U64 only
    Float,    // F32-F64 only
}

pub(crate) struct InferCtx {
    parent: Vec<u32>,
    rank: Vec<u8>,
    types: Vec<Option<Type>>,
    origins: Vec<Option<ConstraintOrigin>>,
    constraints: Vec<TypeConstraint>,
    pub(crate) debug: bool,
    collect_default_warnings: bool,
    default_warnings: Vec<String>,
}

impl InferCtx {
    pub(crate) fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            types: Vec::new(),
            origins: Vec::new(),
            constraints: Vec::new(),
            debug: false,
            collect_default_warnings: false,
            default_warnings: Vec::new(),
        }
    }

    pub(crate) fn enable_default_warnings(&mut self) {
        self.collect_default_warnings = true;
    }

    pub(crate) fn drain_default_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.default_warnings)
    }

    pub(crate) fn fresh_var(&mut self) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        self.constraints.push(TypeConstraint::None);
        Type::TypeVar(id)
    }

    #[allow(dead_code)]
    pub(crate) fn fresh_numeric_var(&mut self) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        self.constraints.push(TypeConstraint::Numeric);
        Type::TypeVar(id)
    }

    pub(crate) fn fresh_integer_var(&mut self) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        self.constraints.push(TypeConstraint::Integer);
        Type::TypeVar(id)
    }

    pub(crate) fn fresh_float_var(&mut self) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        self.constraints.push(TypeConstraint::Float);
        Type::TypeVar(id)
    }

    fn find(&mut self, v: u32) -> u32 {
        let p = self.parent[v as usize];
        if p != v {
            let root = self.find(p);
            self.parent[v as usize] = root;
            root
        } else {
            v
        }
    }

    pub(crate) fn unify_at(
        &mut self,
        a: &Type,
        b: &Type,
        span: Span,
        reason: &'static str,
    ) -> Result<(), String> {
        if self.debug {
            let ra = self.shallow_resolve(a);
            let rb = self.shallow_resolve(b);
            if ra != rb {
                eprintln!(
                    "[type:unify] {} ~ {} (line {}, {})",
                    ra, rb, span.line, reason
                );
            }
        }
        if let Type::TypeVar(v) = self.shallow_resolve(a) {
            let root = self.find(v);
            if self.origins[root as usize].is_none() {
                self.origins[root as usize] = Some(ConstraintOrigin { span, reason });
            }
        }
        if let Type::TypeVar(v) = self.shallow_resolve(b) {
            let root = self.find(v);
            if self.origins[root as usize].is_none() {
                self.origins[root as usize] = Some(ConstraintOrigin { span, reason });
            }
        }
        self.unify(a, b).map_err(|e| {
            // Enrich error with origin context when both sides have origins
            let a_origin = self.origin_of(a);
            let b_origin = self.origin_of(b);
            let mut msg = format!("line {}:{}: {} ({})", span.line, span.col, e, reason);
            if let Some(origin) = a_origin {
                if origin.span.line != span.line {
                    msg.push_str(&format!("\n  note: first type established at line {} ({})", origin.span.line, origin.reason));
                }
            }
            if let Some(origin) = b_origin {
                if origin.span.line != span.line {
                    msg.push_str(&format!("\n  note: second type established at line {} ({})", origin.span.line, origin.reason));
                }
            }
            msg
        })
    }

    #[allow(dead_code)]
    pub(crate) fn origin_of(&mut self, ty: &Type) -> Option<ConstraintOrigin> {
        if let Type::TypeVar(v) = ty {
            let root = self.find(*v);
            self.origins[root as usize].clone()
        } else {
            None
        }
    }

    pub(crate) fn unify(&mut self, a: &Type, b: &Type) -> Result<(), String> {
        let a = a.clone();
        let b = b.clone();
        let a = self.shallow_resolve(&a);
        let b = self.shallow_resolve(&b);

        if a == b {
            return Ok(());
        }

        match (&a, &b) {
            (Type::TypeVar(va), Type::TypeVar(vb)) => {
                let ra = self.find(*va);
                let rb = self.find(*vb);
                if ra == rb {
                    return Ok(());
                }
                let ta = self.types[ra as usize].clone();
                let tb = self.types[rb as usize].clone();
                self.union(ra, rb);
                let root = self.find(ra);
                match (ta, tb) {
                    (Some(ta), Some(tb)) => {
                        self.types[root as usize] = Some(ta.clone());
                        self.unify(&ta, &tb)?;
                    }
                    (Some(t), None) | (None, Some(t)) => {
                        self.types[root as usize] = Some(t);
                    }
                    (None, None) => {}
                }
                Ok(())
            }
            (Type::TypeVar(v), concrete) | (concrete, Type::TypeVar(v)) => {
                let root = self.find(*v);
                if self.occurs_in(root, concrete) {
                    return Err(format!("infinite type: ?{root} occurs in {concrete}"));
                }
                // Validate type constraint
                match &self.constraints[root as usize] {
                    TypeConstraint::Integer if !concrete.is_int() && !matches!(concrete, Type::TypeVar(_)) => {
                        return Err(format!("type mismatch: expected integer type (i8..u64), found `{concrete}`"));
                    }
                    TypeConstraint::Float if !concrete.is_float() && !matches!(concrete, Type::TypeVar(_)) => {
                        return Err(format!("type mismatch: expected float type (f32/f64), found `{concrete}`"));
                    }
                    TypeConstraint::Numeric if !concrete.is_num() && !matches!(concrete, Type::TypeVar(_)) => {
                        return Err(format!("type mismatch: expected numeric type, found `{concrete}`; consider using a conversion function"));
                    }
                    _ => {}
                }
                if let Some(existing) = self.types[root as usize].clone() {
                    self.unify(&existing, concrete)?;
                } else {
                    self.types[root as usize] = Some(concrete.clone());
                }
                Ok(())
            }
            (Type::Array(ea, la), Type::Array(eb, lb)) => {
                if la != lb {
                    return Err(format!("array length mismatch: {la} vs {lb}"));
                }
                self.unify(ea, eb)
            }
            (Type::Vec(ea), Type::Vec(eb)) => self.unify(ea, eb),
            (Type::Map(ka, va), Type::Map(kb, vb)) => {
                self.unify(ka, kb)?;
                self.unify(va, vb)
            }
            (Type::Tuple(ta), Type::Tuple(tb)) => {
                if ta.len() != tb.len() {
                    return Err(format!(
                        "tuple arity mismatch: {} vs {}",
                        ta.len(),
                        tb.len()
                    ));
                }
                for (a, b) in ta.iter().zip(tb.iter()) {
                    self.unify(a, b)?;
                }
                Ok(())
            }
            (Type::Fn(pa, ra), Type::Fn(pb, rb)) => {
                if pa.len() != pb.len() {
                    return Err(format!(
                        "function arity mismatch: {} vs {}",
                        pa.len(),
                        pb.len()
                    ));
                }
                for (a, b) in pa.iter().zip(pb.iter()) {
                    self.unify(a, b)?;
                }
                self.unify(ra, rb)
            }
            (Type::Ptr(a), Type::Ptr(b)) => self.unify(a, b),
            (Type::Rc(a), Type::Rc(b)) => self.unify(a, b),
            (Type::Weak(a), Type::Weak(b)) => self.unify(a, b),
            (Type::Channel(a), Type::Channel(b)) => self.unify(a, b),
            (Type::Coroutine(a), Type::Coroutine(b)) => self.unify(a, b),
            _ => Err(format!("type mismatch: expected `{a}`, found `{b}`")),
        }
    }

    fn union(&mut self, a: u32, b: u32) {
        let ra = self.rank[a as usize];
        let rb = self.rank[b as usize];
        // Merge constraints: more specific wins
        let merged = match (&self.constraints[a as usize], &self.constraints[b as usize]) {
            (TypeConstraint::None, c) | (c, TypeConstraint::None) => c.clone(),
            (TypeConstraint::Integer, _) | (_, TypeConstraint::Integer) => TypeConstraint::Integer,
            (TypeConstraint::Float, _) | (_, TypeConstraint::Float) => TypeConstraint::Float,
            (TypeConstraint::Numeric, TypeConstraint::Numeric) => TypeConstraint::Numeric,
        };
        if ra < rb {
            self.parent[a as usize] = b;
            self.constraints[b as usize] = merged;
        } else if ra > rb {
            self.parent[b as usize] = a;
            self.constraints[a as usize] = merged;
        } else {
            self.parent[b as usize] = a;
            self.rank[a as usize] += 1;
            self.constraints[a as usize] = merged;
        }
    }

    fn occurs_in(&mut self, v: u32, ty: &Type) -> bool {
        match ty {
            Type::TypeVar(u) => {
                let root = self.find(*u);
                if root == v {
                    return true;
                }
                if let Some(resolved) = self.types[root as usize].clone() {
                    return self.occurs_in(v, &resolved);
                }
                false
            }
            Type::Array(inner, _)
            | Type::Vec(inner)
            | Type::Ptr(inner)
            | Type::Rc(inner)
            | Type::Weak(inner)
            | Type::Coroutine(inner)
            | Type::Channel(inner) => self.occurs_in(v, inner),
            Type::Map(k, val) => self.occurs_in(v, k) || self.occurs_in(v, val),
            Type::Tuple(tys) => tys.iter().any(|t| self.occurs_in(v, t)),
            Type::Fn(params, ret) => {
                params.iter().any(|t| self.occurs_in(v, t)) || self.occurs_in(v, ret)
            }
            _ => false,
        }
    }

    pub(crate) fn shallow_resolve(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.shallow_resolve(&resolved)
                } else {
                    Type::TypeVar(root)
                }
            }
            _ => ty.clone(),
        }
    }

    pub(crate) fn resolve(&mut self, ty: &Type) -> Type {
        if self.collect_default_warnings {
            let mut warnings = Vec::new();
            let resolved = self.resolve_inner_warn(ty, &mut warnings);
            self.default_warnings.extend(warnings);
            resolved
        } else {
            self.resolve_inner(ty, false)
        }
    }

    /// Resolve with optional warning for unsolved TypeVars that default.
    pub(crate) fn resolve_with_warnings(&mut self, ty: &Type, warnings: &mut Vec<String>) -> Type {
        self.resolve_inner_warn(ty, warnings)
    }

    fn resolve_inner(&mut self, ty: &Type, _tracking: bool) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.resolve_inner(&resolved, _tracking)
                } else {
                    // Constraint-aware defaulting
                    match &self.constraints[root as usize] {
                        TypeConstraint::Float => Type::F64,
                        _ => Type::I64, // Numeric, Integer, None all → I64
                    }
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.resolve_inner(inner, _tracking)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.resolve_inner(inner, _tracking))),
            Type::Map(k, v) => Type::Map(Box::new(self.resolve_inner(k, _tracking)), Box::new(self.resolve_inner(v, _tracking))),
            Type::Tuple(tys) => Type::Tuple(tys.iter().map(|t| self.resolve_inner(t, _tracking)).collect()),
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.resolve_inner(t, _tracking)).collect(),
                Box::new(self.resolve_inner(ret, _tracking)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_inner(inner, _tracking))),
            Type::Rc(inner) => Type::Rc(Box::new(self.resolve_inner(inner, _tracking))),
            Type::Weak(inner) => Type::Weak(Box::new(self.resolve_inner(inner, _tracking))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.resolve_inner(inner, _tracking))),
            Type::Channel(inner) => Type::Channel(Box::new(self.resolve_inner(inner, _tracking))),

            _ => ty.clone(),
        }
    }

    fn resolve_inner_warn(&mut self, ty: &Type, warnings: &mut Vec<String>) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.resolve_inner_warn(&resolved, warnings)
                } else {
                    let constraint = &self.constraints[root as usize];
                    let default_ty = match constraint {
                        TypeConstraint::Float => Type::F64,
                        _ => Type::I64,
                    };
                    if let Some(origin) = &self.origins[root as usize] {
                        match constraint {
                            TypeConstraint::None => {
                                warnings.push(format!(
                                    "line {}:{}: unsolved type variable defaulted to i64 ({})",
                                    origin.span.line, origin.span.col, origin.reason
                                ));
                            }
                            TypeConstraint::Numeric => {
                                warnings.push(format!(
                                    "line {}:{}: ambiguous numeric type defaulted to i64 ({})",
                                    origin.span.line, origin.span.col, origin.reason
                                ));
                            }
                            // Integer→I64 and Float→F64 are unambiguous — no warning
                            _ => {}
                        }
                    }
                    default_ty
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.resolve_inner_warn(inner, warnings)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.resolve_inner_warn(inner, warnings))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.resolve_inner_warn(k, warnings)),
                Box::new(self.resolve_inner_warn(v, warnings)),
            ),
            Type::Tuple(tys) => Type::Tuple(tys.iter().map(|t| self.resolve_inner_warn(t, warnings)).collect()),
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.resolve_inner_warn(t, warnings)).collect(),
                Box::new(self.resolve_inner_warn(ret, warnings)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_inner_warn(inner, warnings))),
            Type::Rc(inner) => Type::Rc(Box::new(self.resolve_inner_warn(inner, warnings))),
            Type::Weak(inner) => Type::Weak(Box::new(self.resolve_inner_warn(inner, warnings))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.resolve_inner_warn(inner, warnings))),
            Type::Channel(inner) => Type::Channel(Box::new(self.resolve_inner_warn(inner, warnings))),

            _ => ty.clone(),
        }
    }

    /// Instantiate a type scheme: create fresh TypeVars for each quantified variable
    /// and substitute them into the type. Returns the instantiated type.
    pub(crate) fn instantiate(&mut self, scheme: &crate::types::Scheme) -> Type {
        if scheme.quantified.is_empty() {
            return scheme.ty.clone();
        }
        let subst: std::collections::HashMap<u32, Type> = scheme.quantified.iter()
            .map(|&v| (v, self.fresh_var()))
            .collect();
        self.substitute(&scheme.ty, &subst)
    }

    /// Substitute TypeVars according to a mapping.
    fn substitute(&self, ty: &Type, subst: &std::collections::HashMap<u32, Type>) -> Type {
        match ty {
            Type::TypeVar(v) => {
                if let Some(replacement) = subst.get(v) {
                    replacement.clone()
                } else {
                    ty.clone()
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.substitute(inner, subst)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.substitute(inner, subst))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.substitute(k, subst)),
                Box::new(self.substitute(v, subst)),
            ),
            Type::Tuple(tys) => Type::Tuple(tys.iter().map(|t| self.substitute(t, subst)).collect()),
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.substitute(t, subst)).collect(),
                Box::new(self.substitute(ret, subst)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.substitute(inner, subst))),
            Type::Rc(inner) => Type::Rc(Box::new(self.substitute(inner, subst))),
            Type::Weak(inner) => Type::Weak(Box::new(self.substitute(inner, subst))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.substitute(inner, subst))),
            Type::Channel(inner) => Type::Channel(Box::new(self.substitute(inner, subst))),
            _ => ty.clone(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn try_resolve(&mut self, ty: &Type) -> Option<Type> {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.try_resolve(&resolved)
                } else {
                    None
                }
            }

            _ => Some(ty.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_var() {
        let mut ctx = InferCtx::new();
        let v0 = ctx.fresh_var();
        let v1 = ctx.fresh_var();
        assert_eq!(v0, Type::TypeVar(0));
        assert_eq!(v1, Type::TypeVar(1));
    }

    #[test]
    fn test_unify_var_concrete() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        ctx.unify(&v, &Type::I64).unwrap();
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_unify_two_vars() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();
        ctx.unify(&a, &b).unwrap();
        ctx.unify(&b, &Type::String).unwrap();
        assert_eq!(ctx.resolve(&a), Type::String);
        assert_eq!(ctx.resolve(&b), Type::String);
    }

    #[test]
    fn test_structural_unify() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let arr_a = Type::Vec(Box::new(v.clone()));
        let arr_b = Type::Vec(Box::new(Type::F64));
        ctx.unify(&arr_a, &arr_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::F64);
    }

    #[test]
    fn test_occurs_check() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let circular = Type::Vec(Box::new(v.clone()));
        assert!(ctx.unify(&v, &circular).is_err());
    }

    #[test]
    fn test_unsolved_defaults_to_i64() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_fn_unify() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let fn_a = Type::Fn(vec![v.clone()], Box::new(Type::Bool));
        let fn_b = Type::Fn(vec![Type::String], Box::new(Type::Bool));
        ctx.unify(&fn_a, &fn_b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::String);
    }

    #[test]
    fn test_transitive_unification() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();
        let c = ctx.fresh_var();
        ctx.unify(&a, &b).unwrap();
        ctx.unify(&b, &c).unwrap();
        ctx.unify(&c, &Type::F64).unwrap();
        assert_eq!(ctx.resolve(&a), Type::F64);
    }

    #[test]
    fn test_concrete_mismatch_errors() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&Type::I64, &Type::String).is_err());
        assert!(ctx.unify(&Type::Bool, &Type::F64).is_err());
        assert!(ctx.unify(&Type::I32, &Type::I64).is_err());
    }

    #[test]
    fn test_concrete_same_ok() {
        let mut ctx = InferCtx::new();
        assert!(ctx.unify(&Type::I64, &Type::I64).is_ok());
        assert!(ctx.unify(&Type::String, &Type::String).is_ok());
        assert!(ctx.unify(&Type::Bool, &Type::Bool).is_ok());
    }

    #[test]
    fn test_structural_vec_mismatch() {
        let mut ctx = InferCtx::new();
        let va = Type::Vec(Box::new(Type::I64));
        let vb = Type::Vec(Box::new(Type::String));
        assert!(ctx.unify(&va, &vb).is_err());
    }

    #[test]
    fn test_tuple_arity_mismatch() {
        let mut ctx = InferCtx::new();
        let ta = Type::Tuple(vec![Type::I64]);
        let tb = Type::Tuple(vec![Type::I64, Type::Bool]);
        assert!(ctx.unify(&ta, &tb).is_err());
    }

    #[test]
    fn test_tuple_unify_with_vars() {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_var();
        let b = ctx.fresh_var();
        let ta = Type::Tuple(vec![a.clone(), b.clone()]);
        let tb = Type::Tuple(vec![Type::String, Type::Bool]);
        ctx.unify(&ta, &tb).unwrap();
        assert_eq!(ctx.resolve(&a), Type::String);
        assert_eq!(ctx.resolve(&b), Type::Bool);
    }

    #[test]
    fn test_map_unify() {
        let mut ctx = InferCtx::new();
        let k = ctx.fresh_var();
        let v = ctx.fresh_var();
        let ma = Type::Map(Box::new(k.clone()), Box::new(v.clone()));
        let mb = Type::Map(Box::new(Type::String), Box::new(Type::I64));
        ctx.unify(&ma, &mb).unwrap();
        assert_eq!(ctx.resolve(&k), Type::String);
        assert_eq!(ctx.resolve(&v), Type::I64);
    }

    #[test]
    fn test_channel_unify() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let ca = Type::Channel(Box::new(v.clone()));
        let cb = Type::Channel(Box::new(Type::String));
        ctx.unify(&ca, &cb).unwrap();
        assert_eq!(ctx.resolve(&v), Type::String);
    }

    #[test]
    fn test_fn_arity_mismatch() {
        let mut ctx = InferCtx::new();
        let fa = Type::Fn(vec![Type::I64], Box::new(Type::Void));
        let fb = Type::Fn(vec![Type::I64, Type::Bool], Box::new(Type::Void));
        assert!(ctx.unify(&fa, &fb).is_err());
    }

    #[test]
    fn test_array_length_mismatch() {
        let mut ctx = InferCtx::new();
        let aa = Type::Array(Box::new(Type::I64), 3);
        let ab = Type::Array(Box::new(Type::I64), 5);
        assert!(ctx.unify(&aa, &ab).is_err());
    }

    #[test]
    fn test_deeply_nested_unification() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let a = Type::Vec(Box::new(Type::Map(
            Box::new(Type::String),
            Box::new(v.clone()),
        )));
        let b = Type::Vec(Box::new(Type::Map(
            Box::new(Type::String),
            Box::new(Type::Bool),
        )));
        ctx.unify(&a, &b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::Bool);
    }

    #[test]
    fn test_unify_at_records_origin() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span {
            start: 0,
            end: 1,
            line: 10,
            col: 5,
        };
        ctx.unify_at(&v, &Type::String, span, "test constraint")
            .unwrap();
        let origin = ctx.origin_of(&v).unwrap();
        assert_eq!(origin.span.line, 10);
        assert_eq!(origin.reason, "test constraint");
    }

    #[test]
    fn test_try_resolve_unsolved() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        assert!(ctx.try_resolve(&v).is_none());
        ctx.unify(&v, &Type::Bool).unwrap();
        assert_eq!(ctx.try_resolve(&v), Some(Type::Bool));
    }

    #[test]
    fn test_default_warnings_disabled_by_default() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let _ = ctx.resolve(&v);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_default_warnings_collected_when_enabled() {
        let mut ctx = InferCtx::new();
        ctx.enable_default_warnings();
        let span = Span { start: 0, end: 0, line: 5, col: 3 };
        let v = ctx.fresh_var();
        let v2 = ctx.fresh_var();
        // Set origin by unifying two vars at a span
        let _ = ctx.unify_at(&v, &v2, span, "test param");
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64);
        let warnings = ctx.drain_default_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("unsolved type variable defaulted to i64"));
        assert!(warnings[0].contains("test param"));
    }

    #[test]
    fn test_default_warnings_not_emitted_for_solved_vars() {
        let mut ctx = InferCtx::new();
        ctx.enable_default_warnings();
        let v = ctx.fresh_var();
        ctx.unify(&v, &Type::String).unwrap();
        let _ = ctx.resolve(&v);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_default_warnings_not_emitted_for_constrained_numeric() {
        let mut ctx = InferCtx::new();
        ctx.enable_default_warnings();
        let v = ctx.fresh_integer_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64);
        let warnings = ctx.drain_default_warnings();
        // Integer-constrained vars default to I64 without warning (constraint is clear)
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_default_warnings_float_constraint_no_warning() {
        let mut ctx = InferCtx::new();
        ctx.enable_default_warnings();
        let v = ctx.fresh_float_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::F64);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }
}
