//! Union-Find based unification engine for type inference.
//!
//! Provides `InferCtx` — a context that manages inference variables (TypeVar)
//! and unifies them into concrete types using a weighted quick-union with
//! path compression.

use crate::ast::Span;
use crate::types::Type;

/// Records the origin of a type constraint for diagnostic purposes.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ConstraintOrigin {
    pub span: Span,
    pub reason: &'static str,
}

/// Inference context: manages unification variables and their solutions.
pub(crate) struct InferCtx {
    /// Union-find parent pointers. `parent[i] == i` means `i` is a root.
    parent: Vec<u32>,
    /// Rank for union-by-rank.
    rank: Vec<u8>,
    /// Resolved type for a root variable (None = still unconstrained).
    types: Vec<Option<Type>>,
    /// Origin span for each variable's binding (set when unified with concrete).
    origins: Vec<Option<ConstraintOrigin>>,
}

impl InferCtx {
    pub(crate) fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            types: Vec::new(),
            origins: Vec::new(),
        }
    }

    /// Create a fresh inference variable and return `Type::TypeVar(id)`.
    pub(crate) fn fresh_var(&mut self) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        Type::TypeVar(id)
    }

    /// Find the root representative of variable `v` with path compression.
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

    /// Unify two types with constraint origin tracking.
    pub(crate) fn unify_at(&mut self, a: &Type, b: &Type, span: Span, reason: &'static str) -> Result<(), String> {
        // Record origin for any TypeVar being bound
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
        self.unify(a, b)
    }

    /// Get the constraint origin for a type variable (if any).
    #[allow(dead_code)]
    pub(crate) fn origin_of(&mut self, ty: &Type) -> Option<ConstraintOrigin> {
        if let Type::TypeVar(v) = ty {
            let root = self.find(*v);
            self.origins[root as usize].clone()
        } else {
            None
        }
    }

    /// Unify two types. Returns Ok(()) on success, Err(msg) on conflict.
    pub(crate) fn unify(&mut self, a: &Type, b: &Type) -> Result<(), String> {
        let a = self.shallow_resolve(a);
        let b = self.shallow_resolve(b);

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
                // Merge: if one root has a type, keep that one as root
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
                if let Some(existing) = self.types[root as usize].clone() {
                    self.unify(&existing, concrete)?;
                } else {
                    self.types[root as usize] = Some(concrete.clone());
                }
                Ok(())
            }
            // Structural unification
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
                    return Err(format!("tuple arity mismatch: {} vs {}", ta.len(), tb.len()));
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
            // Inferred acts like a wildcard — unify with anything silently
            (Type::Inferred, _) | (_, Type::Inferred) => Ok(()),
            // Concrete type mismatch
            _ => Err(format!("type mismatch: expected `{a}`, found `{b}`")),
        }
    }

    /// Union two roots by rank.
    fn union(&mut self, a: u32, b: u32) {
        let ra = self.rank[a as usize];
        let rb = self.rank[b as usize];
        if ra < rb {
            self.parent[a as usize] = b;
        } else if ra > rb {
            self.parent[b as usize] = a;
        } else {
            self.parent[b as usize] = a;
            self.rank[a as usize] += 1;
        }
    }

    /// Occurs check: does variable `v` appear in `ty`?
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
            Type::Array(inner, _) | Type::Vec(inner) | Type::Ptr(inner)
            | Type::Rc(inner) | Type::Weak(inner) | Type::Coroutine(inner)
            | Type::Channel(inner) => self.occurs_in(v, inner),
            Type::Map(k, val) => self.occurs_in(v, k) || self.occurs_in(v, val),
            Type::Tuple(tys) => tys.iter().any(|t| self.occurs_in(v, t)),
            Type::Fn(params, ret) => {
                params.iter().any(|t| self.occurs_in(v, t)) || self.occurs_in(v, ret)
            }
            _ => false,
        }
    }

    /// Shallow resolve: if `ty` is a TypeVar, follow the chain to its current binding.
    /// Does NOT recurse into compound types.
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

    /// Deep resolve: recursively replace all TypeVars with their solutions.
    /// Unsolved vars get defaulted by the `default` closure.
    pub(crate) fn resolve(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.resolve(&resolved)
                } else {
                    // Unsolved — default to i64 (matches existing behavior)
                    Type::I64
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.resolve(inner)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.resolve(inner))),
            Type::Map(k, v) => Type::Map(Box::new(self.resolve(k)), Box::new(self.resolve(v))),
            Type::Tuple(tys) => Type::Tuple(tys.iter().map(|t| self.resolve(t)).collect()),
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.resolve(t)).collect(),
                Box::new(self.resolve(ret)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve(inner))),
            Type::Rc(inner) => Type::Rc(Box::new(self.resolve(inner))),
            Type::Weak(inner) => Type::Weak(Box::new(self.resolve(inner))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.resolve(inner))),
            Type::Channel(inner) => Type::Channel(Box::new(self.resolve(inner))),
            Type::Inferred => Type::I64,
            _ => ty.clone(),
        }
    }

    /// Resolve a type, but return None for unsolved vars instead of defaulting.
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
            Type::Inferred => None,
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
        // Vec(Map(String, ?v)) ~ Vec(Map(String, Bool))
        let a = Type::Vec(Box::new(Type::Map(Box::new(Type::String), Box::new(v.clone()))));
        let b = Type::Vec(Box::new(Type::Map(Box::new(Type::String), Box::new(Type::Bool))));
        ctx.unify(&a, &b).unwrap();
        assert_eq!(ctx.resolve(&v), Type::Bool);
    }

    #[test]
    fn test_unify_at_records_origin() {
        let mut ctx = InferCtx::new();
        let v = ctx.fresh_var();
        let span = crate::ast::Span { start: 0, end: 1, line: 10, col: 5 };
        ctx.unify_at(&v, &Type::String, span, "test constraint").unwrap();
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
}
