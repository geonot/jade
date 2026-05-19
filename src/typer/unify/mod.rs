use crate::ast::Span;
use crate::intern::Symbol;
use crate::types::Type;
use indexmap::IndexMap;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct ConstraintOrigin {
    pub span: Span,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub(crate) enum TypeConstraint {
    None,
    Numeric,

    Addable,
    Integer,
    Float,
    Trait(Vec<String>),
}

pub(crate) struct InferCtx {
    parent: Vec<u32>,
    rank: Vec<u8>,
    types: Vec<Option<Type>>,
    origins: Vec<Option<ConstraintOrigin>>,
    constraints: Vec<TypeConstraint>,

    usage_sites: Vec<Vec<(Span, &'static str)>>,
    pub(crate) debug: bool,
    collect_default_warnings: bool,
    default_warnings: Vec<String>,
    strict_types: bool,
    strict_errors: Vec<String>,
    pedantic: bool,
    quantified_vars: std::collections::HashSet<u32>,

    trait_impls: IndexMap<Symbol, Vec<String>>,
}

impl InferCtx {
    pub(crate) fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            types: Vec::new(),
            origins: Vec::new(),
            constraints: Vec::new(),
            usage_sites: Vec::new(),
            debug: false,
            collect_default_warnings: false,
            default_warnings: Vec::new(),
            strict_types: true,
            strict_errors: Vec::new(),
            pedantic: false,
            quantified_vars: std::collections::HashSet::new(),
            trait_impls: IndexMap::new(),
        }
    }

    pub(crate) fn set_trait_impls(&mut self, impls: IndexMap<Symbol, Vec<String>>) {
        self.trait_impls = impls;
    }

    pub(crate) fn enable_default_warnings(&mut self) {
        self.collect_default_warnings = true;
    }

    pub(crate) fn drain_default_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.default_warnings)
    }

    pub(crate) fn enable_strict_types(&mut self) {
        self.strict_types = true;
    }

    pub(crate) fn disable_strict_types(&mut self) {
        self.strict_types = false;
    }

    pub(crate) fn default_quantified_vars(&mut self, quantified: &[u32]) {
        for &v in quantified {
            let root = self.find(v);
            if self.types[root as usize].is_some() {
                continue;
            }
            let default_ty = match self.constraints[root as usize] {
                TypeConstraint::Float => Type::F64,
                _ => Type::I64,
            };
            self.types[root as usize] = Some(default_ty);
        }
    }

    pub(crate) fn drain_strict_errors(&mut self) -> Vec<String> {
        std::mem::take(&mut self.strict_errors)
    }

    pub(crate) fn mark_quantified(&mut self, vars: &[u32]) {
        for &v in vars {
            let root = self.find(v);
            self.quantified_vars.insert(root);
        }
    }

    pub(crate) fn is_strict(&self) -> bool {
        self.strict_types
    }

    pub(crate) fn set_strict(&mut self, strict: bool) {
        self.strict_types = strict;
    }

    pub(crate) fn set_pedantic(&mut self, pedantic: bool) {
        self.pedantic = pedantic;
        if pedantic {
            self.strict_types = true;
        }
    }

    pub(crate) fn num_vars(&self) -> u32 {
        self.parent.len() as u32
    }

    fn fresh_var_with(&mut self, constraint: TypeConstraint) -> Type {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.rank.push(0);
        self.types.push(None);
        self.origins.push(None);
        self.constraints.push(constraint);
        self.usage_sites.push(Vec::new());
        Type::TypeVar(id)
    }

    pub(crate) fn fresh_var(&mut self) -> Type {
        self.fresh_var_with(TypeConstraint::None)
    }

    pub(crate) fn fresh_var_at(&mut self, span: Span, reason: &'static str) -> Type {
        let v = self.fresh_var_with(TypeConstraint::None);
        if let Type::TypeVar(id) = v {
            self.origins[id as usize] = Some(ConstraintOrigin { span, reason });
        }
        v
    }

    #[allow(dead_code)]
    pub(crate) fn fresh_numeric_var(&mut self) -> Type {
        self.fresh_var_with(TypeConstraint::Numeric)
    }

    pub(crate) fn fresh_integer_var(&mut self) -> Type {
        self.fresh_var_with(TypeConstraint::Integer)
    }

    pub(crate) fn fresh_float_var(&mut self) -> Type {
        self.fresh_var_with(TypeConstraint::Float)
    }

    pub(crate) fn constraint(&mut self, id: u32) -> TypeConstraint {
        let root = self.find(id);
        self.constraints
            .get(root as usize)
            .cloned()
            .unwrap_or(TypeConstraint::None)
    }

    fn merge_constraints(
        a: &TypeConstraint,
        b: &TypeConstraint,
    ) -> Result<TypeConstraint, &'static str> {
        match (a, b) {
            (TypeConstraint::None, c) | (c, TypeConstraint::None) => Ok(c.clone()),
            (TypeConstraint::Integer, TypeConstraint::Float)
            | (TypeConstraint::Float, TypeConstraint::Integer) => {
                Err("integer and float are mutually exclusive")
            }
            (TypeConstraint::Trait(_), TypeConstraint::Numeric)
            | (TypeConstraint::Trait(_), TypeConstraint::Integer)
            | (TypeConstraint::Trait(_), TypeConstraint::Float)
            | (TypeConstraint::Trait(_), TypeConstraint::Addable)
            | (TypeConstraint::Numeric, TypeConstraint::Trait(_))
            | (TypeConstraint::Integer, TypeConstraint::Trait(_))
            | (TypeConstraint::Float, TypeConstraint::Trait(_))
            | (TypeConstraint::Addable, TypeConstraint::Trait(_)) => {
                Err("trait bound and numeric constraint are mutually exclusive")
            }
            (TypeConstraint::Trait(ta), TypeConstraint::Trait(tb)) => {
                let mut merged = ta.clone();
                for t in tb {
                    if !merged.contains(t) {
                        merged.push(t.clone());
                    }
                }
                Ok(TypeConstraint::Trait(merged))
            }

            (TypeConstraint::Addable, TypeConstraint::Numeric)
            | (TypeConstraint::Numeric, TypeConstraint::Addable) => Ok(TypeConstraint::Numeric),
            (TypeConstraint::Addable, TypeConstraint::Integer)
            | (TypeConstraint::Integer, TypeConstraint::Addable) => Ok(TypeConstraint::Integer),
            (TypeConstraint::Addable, TypeConstraint::Float)
            | (TypeConstraint::Float, TypeConstraint::Addable) => Ok(TypeConstraint::Float),
            (TypeConstraint::Addable, TypeConstraint::Addable) => Ok(TypeConstraint::Addable),

            (TypeConstraint::Numeric, TypeConstraint::Integer)
            | (TypeConstraint::Integer, TypeConstraint::Numeric) => Ok(TypeConstraint::Integer),
            (TypeConstraint::Numeric, TypeConstraint::Float)
            | (TypeConstraint::Float, TypeConstraint::Numeric) => Ok(TypeConstraint::Float),
            (TypeConstraint::Integer, TypeConstraint::Integer) => Ok(TypeConstraint::Integer),
            (TypeConstraint::Float, TypeConstraint::Float) => Ok(TypeConstraint::Float),
            (TypeConstraint::Numeric, TypeConstraint::Numeric) => Ok(TypeConstraint::Numeric),
        }
    }

    pub(crate) fn constraints_conflict(a: &TypeConstraint, b: &TypeConstraint) -> bool {
        Self::merge_constraints(a, b).is_err()
    }

    fn record_usage(&mut self, ty: &Type, span: Span, reason: &'static str) {
        if let Type::TypeVar(v) = self.shallow_resolve(ty) {
            let root = self.find(v) as usize;
            if root < self.usage_sites.len() {
                self.usage_sites[root].push((span, reason));
            }
        }
    }

    pub(crate) fn constrain(
        &mut self,
        ty: &Type,
        constraint: TypeConstraint,
        span: Span,
        reason: &'static str,
    ) -> Result<(), String> {
        self.record_usage(ty, span, reason);
        let resolved = self.shallow_resolve(ty);
        match resolved {
            Type::TypeVar(v) => {
                let root = self.find(v);
                let merged = Self::merge_constraints(&self.constraints[root as usize], &constraint)
                    .map_err(|e| {
                        format!(
                            "{}: conflicting constraints for {}: {e}",
                            span.loc(),
                            reason
                        )
                    })?;
                self.constraints[root as usize] = merged;
                if self.origins[root as usize].is_none() {
                    self.origins[root as usize] = Some(ConstraintOrigin { span, reason });
                }
                Ok(())
            }
            ref concrete => match constraint {
                TypeConstraint::Integer if !concrete.is_int() => Err(format!(
                    "{}: expected integer type for {}, found `{concrete}`",
                    span.loc(),
                    reason
                )),
                TypeConstraint::Float if !concrete.is_float() => Err(format!(
                    "{}: expected float type for {}, found `{concrete}`",
                    span.loc(),
                    reason
                )),
                TypeConstraint::Numeric if !concrete.is_num() => Err(format!(
                    "{}: expected numeric type for {}, found `{concrete}`",
                    span.loc(),
                    reason
                )),
                _ => Ok(()),
            },
        }
    }

    pub(crate) fn find(&mut self, v: u32) -> u32 {
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
            self.usage_sites[root as usize].push((span, reason));
        }
        if let Type::TypeVar(v) = self.shallow_resolve(b) {
            let root = self.find(v);
            if self.origins[root as usize].is_none() {
                self.origins[root as usize] = Some(ConstraintOrigin { span, reason });
            }
            self.usage_sites[root as usize].push((span, reason));
        }
        self.unify(a, b).map_err(|e| {
            let ra = self.shallow_resolve(a);
            let rb = self.shallow_resolve(b);
            let a_origin = self.origin_of(a);
            let b_origin = self.origin_of(b);

            let mut msg = format!("{}: {} ({})", span.loc(), e, reason);

            if let Some(origin) = &a_origin {
                if origin.span.line != span.line {
                    msg.push_str(&format!(
                        "\n  note: expected `{}` because of line {} ({})",
                        ra, origin.span.line, origin.reason
                    ));
                }
            }
            if let Some(origin) = &b_origin {
                if origin.span.line != span.line {
                    msg.push_str(&format!(
                        "\n  note: found `{}` because of line {} ({})",
                        rb, origin.span.line, origin.reason
                    ));
                }
            }

            let suggestion = self.suggest_fix(reason, &ra, &rb);
            if let Some(s) = suggestion {
                msg.push_str(&format!("\n  help: {s}"));
            }

            msg
        })
    }

    fn suggest_fix(&self, reason: &str, expected: &Type, found: &Type) -> Option<String> {
        if expected.is_int() && found.is_float() {
            return Some(format!(
                "use `{found} as {expected}` to convert float to integer"
            ));
        }
        if expected.is_float() && found.is_int() {
            return Some(format!(
                "use `{found} as {expected}` to convert integer to float"
            ));
        }
        if expected.is_int() && found.is_int() && expected != found {
            return Some(format!(
                "use `{found} as {expected}` to convert between integer types"
            ));
        }
        if matches!(expected, Type::String) && found.is_num() {
            return Some("use `to_string(value)` to convert a number to a string".into());
        }
        if expected.is_num() && matches!(found, Type::String) {
            return Some("strings cannot be used as numbers directly".into());
        }
        if reason.contains("argument") {
            return Some("check that the argument type matches the parameter type".into());
        }
        if reason.contains("return") || reason.contains("tail") {
            return Some("ensure all return paths produce the same type".into());
        }
        if reason.contains("binary operand") || reason.contains("operands") {
            return Some("binary operators require both operands to have the same type".into());
        }
        if reason.contains("assign") {
            return Some("the assigned value must match the variable's type".into());
        }
        None
    }

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

        // Canonicalize at the unification entry (P0-12). This collapses
        // `Type::Struct("String", _)` ↔ `Type::String`, transparently
        // unwraps `Type::Alias`, and recurses into composites. After this
        // step the unifier sees a single canonical representation per
        // semantic type.
        let a = a.canonical();
        let b = b.canonical();

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
                self.union(ra, rb)?;
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
                match &self.constraints[root as usize] {
                    TypeConstraint::Integer
                        if !concrete.is_int() && !matches!(concrete, Type::TypeVar(_)) =>
                    {
                        return Err(format!(
                            "type mismatch: expected integer type (i8..u64), found `{concrete}`"
                        ));
                    }
                    TypeConstraint::Float
                        if !concrete.is_float() && !matches!(concrete, Type::TypeVar(_)) =>
                    {
                        return Err(format!(
                            "type mismatch: expected float type (f32/f64), found `{concrete}`"
                        ));
                    }
                    TypeConstraint::Numeric
                        if !concrete.is_num() && !matches!(concrete, Type::TypeVar(_)) =>
                    {
                        return Err(format!(
                            "type mismatch: expected numeric type, found `{concrete}`; consider using a conversion function"
                        ));
                    }
                    TypeConstraint::Addable
                        if !concrete.is_num()
                            && !matches!(concrete, Type::String | Type::TypeVar(_)) =>
                    {
                        return Err(format!(
                            "type mismatch: expected numeric or String type for `+`, found `{concrete}`"
                        ));
                    }
                    TypeConstraint::Trait(required_traits)
                        if !matches!(concrete, Type::TypeVar(_))
                            && !required_traits.is_empty()
                            && !self.trait_impls.is_empty() =>
                    {
                        let type_name = Self::type_to_impl_name(concrete);
                        if let Some(name) = type_name {
                            let impl_traits = self.trait_impls.get(&name);
                            let missing: Vec<&String> = required_traits
                                .iter()
                                .filter(|rt| impl_traits.map_or(true, |impls| !impls.contains(rt)))
                                .collect();
                            if !missing.is_empty() {
                                let missing_str = missing
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                return Err(format!(
                                    "type mismatch: `{concrete}` does not implement required trait(s): {missing_str}"
                                ));
                            }
                        }
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
            (Type::Channel(a), Type::Channel(b)) => self.unify(a, b),
            (Type::Coroutine(a), Type::Coroutine(b)) => self.unify(a, b),
            (Type::Generator(a), Type::Generator(b)) => self.unify(a, b),
            (Type::Struct(na, aa), Type::Struct(nb, ab)) if na == nb && aa.len() == ab.len() => {
                for (x, y) in aa.iter().zip(ab.iter()) {
                    self.unify(x, y)?;
                }
                Ok(())
            }
            // P0-12 compatibility shim: until every site that produces
            // `Type::Enum(n)` is migrated to produce `Type::Struct(n, args)`,
            // accept `Struct(n, _) ↔ Enum(n)` when names match. Generic args
            // on the struct side are not unified (the enum side carries no
            // arg info), which preserves existing behaviour.
            (Type::Struct(na, _), Type::Enum(nb)) | (Type::Enum(nb), Type::Struct(na, _))
                if na == nb =>
            {
                Ok(())
            }
            (Type::Newtype(na, ia), Type::Newtype(nb, ib)) if na == nb => self.unify(ia, ib),
            (Type::Row(sa), Type::Row(sb)) => {
                if sa == sb {
                    Ok(())
                } else {
                    Err(format!(
                        "row type mismatch: expected `Row<{sa}>`, found `Row<{sb}>`"
                    ))
                }
            }
            _ => Err(format!("type mismatch: expected `{a}`, found `{b}`")),
        }
    }

    fn union(&mut self, a: u32, b: u32) -> Result<(), String> {
        let ra = self.rank[a as usize];
        let rb = self.rank[b as usize];
        let merged =
            Self::merge_constraints(&self.constraints[a as usize], &self.constraints[b as usize])
                .map_err(|e| format!("type mismatch: {e}"))?;

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
        Ok(())
    }
}
mod resolve;
#[cfg(test)]
mod tests;
