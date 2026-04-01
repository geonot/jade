use crate::ast::Span;
use crate::types::Type;
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
    /// Numeric OR String — used for `+` which supports both arithmetic and concatenation.
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
    /// All locations where each TypeVar was mentioned/constrained.
    usage_sites: Vec<Vec<(Span, &'static str)>>,
    pub(crate) debug: bool,
    collect_default_warnings: bool,
    default_warnings: Vec<String>,
    strict_types: bool,
    strict_errors: Vec<String>,
    pedantic: bool,
    quantified_vars: std::collections::HashSet<u32>,
    /// Maps type_name -> list of trait names it implements.
    /// Used to enforce Trait constraints during unification.
    trait_impls: HashMap<String, Vec<String>>,
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
            trait_impls: HashMap::new(),
        }
    }

    /// Update the trait implementation map (called from Typer after trait registration).
    pub(crate) fn set_trait_impls(&mut self, impls: HashMap<String, Vec<String>>) {
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
            // Addable + Numeric/Integer/Float narrows to the stricter constraint
            (TypeConstraint::Addable, TypeConstraint::Numeric)
            | (TypeConstraint::Numeric, TypeConstraint::Addable) => Ok(TypeConstraint::Numeric),
            (TypeConstraint::Addable, TypeConstraint::Integer)
            | (TypeConstraint::Integer, TypeConstraint::Addable) => Ok(TypeConstraint::Integer),
            (TypeConstraint::Addable, TypeConstraint::Float)
            | (TypeConstraint::Float, TypeConstraint::Addable) => Ok(TypeConstraint::Float),
            (TypeConstraint::Addable, TypeConstraint::Addable) => Ok(TypeConstraint::Addable),
            (TypeConstraint::Integer, _) | (_, TypeConstraint::Integer) => {
                Ok(TypeConstraint::Integer)
            }
            (TypeConstraint::Float, _) | (_, TypeConstraint::Float) => Ok(TypeConstraint::Float),
            (TypeConstraint::Numeric, TypeConstraint::Numeric) => Ok(TypeConstraint::Numeric),
        }
    }

    /// Check if two constraints are fundamentally incompatible (would fail merge).
    pub(crate) fn constraints_conflict(a: &TypeConstraint, b: &TypeConstraint) -> bool {
        Self::merge_constraints(a, b).is_err()
    }

    /// Record that a TypeVar was used at the given span/reason.
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
                            "line {}:{}: conflicting constraints for {}: {e}",
                            span.line, span.col, reason
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
                    "line {}:{}: expected integer type for {}, found `{concrete}`",
                    span.line, span.col, reason
                )),
                TypeConstraint::Float if !concrete.is_float() => Err(format!(
                    "line {}:{}: expected float type for {}, found `{concrete}`",
                    span.line, span.col, reason
                )),
                TypeConstraint::Numeric if !concrete.is_num() => Err(format!(
                    "line {}:{}: expected numeric type for {}, found `{concrete}`",
                    span.line, span.col, reason
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

            let mut msg = format!("line {}:{}: {} ({})", span.line, span.col, e, reason);

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
                                .filter(|rt| {
                                    impl_traits.map_or(true, |impls| !impls.contains(rt))
                                })
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
            (Type::Rc(a), Type::Rc(b)) => self.unify(a, b),
            (Type::Weak(a), Type::Weak(b)) => self.unify(a, b),
            (Type::Channel(a), Type::Channel(b)) => self.unify(a, b),
            (Type::Coroutine(a), Type::Coroutine(b)) => self.unify(a, b),
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

    /// Convert a concrete Type to the name used in trait_impls lookups.
    fn type_to_impl_name(ty: &Type) -> Option<String> {
        match ty {
            Type::I8 => Some("i8".into()),
            Type::I16 => Some("i16".into()),
            Type::I32 => Some("i32".into()),
            Type::I64 => Some("i64".into()),
            Type::U8 => Some("u8".into()),
            Type::U16 => Some("u16".into()),
            Type::U32 => Some("u32".into()),
            Type::U64 => Some("u64".into()),
            Type::F32 => Some("f32".into()),
            Type::F64 => Some("f64".into()),
            Type::Bool => Some("bool".into()),
            Type::String => Some("String".into()),
            Type::Struct(name, _) => Some(name.clone()),
            _ => None,
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

    pub(crate) fn canonicalize_type(&mut self, ty: &Type) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    self.canonicalize_type(&resolved)
                } else {
                    Type::TypeVar(root)
                }
            }
            Type::Array(inner, len) => Type::Array(Box::new(self.canonicalize_type(inner)), *len),
            Type::Vec(inner) => Type::Vec(Box::new(self.canonicalize_type(inner))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.canonicalize_type(k)),
                Box::new(self.canonicalize_type(v)),
            ),
            Type::Tuple(tys) => {
                Type::Tuple(tys.iter().map(|t| self.canonicalize_type(t)).collect())
            }
            Type::Fn(params, ret) => Type::Fn(
                params.iter().map(|t| self.canonicalize_type(t)).collect(),
                Box::new(self.canonicalize_type(ret)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.canonicalize_type(inner))),
            Type::Rc(inner) => Type::Rc(Box::new(self.canonicalize_type(inner))),
            Type::Weak(inner) => Type::Weak(Box::new(self.canonicalize_type(inner))),
            Type::Coroutine(inner) => Type::Coroutine(Box::new(self.canonicalize_type(inner))),
            Type::Channel(inner) => Type::Channel(Box::new(self.canonicalize_type(inner))),
            _ => ty.clone(),
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
        self.resolve_core(ty, self.collect_default_warnings)
    }

    /// Resolve a container element type. If the element type is an unsolved TypeVar
    /// with no constraint (e.g., from an empty `vec()`), default to I64 silently
    /// without triggering strict-mode errors.
    fn resolve_container_elem(&mut self, ty: &Type) -> Type {
        if let Type::TypeVar(v) = ty {
            let root = self.find(*v);
            if let Some(resolved) = self.types[root as usize].clone() {
                return self.resolve_core(&resolved, self.collect_default_warnings);
            }
            let constraint = &self.constraints[root as usize];
            match constraint {
                TypeConstraint::Float => Type::F64,
                TypeConstraint::None | TypeConstraint::Numeric | TypeConstraint::Addable => Type::I64,
                TypeConstraint::Integer => Type::I64,
                TypeConstraint::Trait(_) => {
                    // Trait-constrained container elements should still error
                    self.resolve_core(ty, self.collect_default_warnings)
                }
            }
        } else {
            self.resolve_core(ty, self.collect_default_warnings)
        }
    }

    fn resolve_core(&mut self, ty: &Type, warn_only: bool) -> Type {
        match ty {
            Type::TypeVar(v) => {
                let root = self.find(*v);
                if let Some(resolved) = self.types[root as usize].clone() {
                    return self.resolve_core(&resolved, warn_only);
                }
                let constraint = &self.constraints[root as usize];
                let default_ty = match constraint {
                    TypeConstraint::Float => Type::F64,
                    _ => Type::I64,
                };
                if warn_only && !self.pedantic {
                    if let Some(origin) = &self.origins[root as usize] {
                        match constraint {
                            TypeConstraint::None => {
                                self.default_warnings.push(format!(
                                    "line {}:{}: unsolved type variable defaulted to i64 ({}). Consider adding `: i64` or the appropriate type annotation.",
                                    origin.span.line, origin.span.col, origin.reason
                                ));
                            }
                            TypeConstraint::Numeric => {
                                self.default_warnings.push(format!(
                                    "line {}:{}: numeric type defaults to i64 ({}). Add `: i64` for integer or `: f64` for float.",
                                    origin.span.line, origin.span.col, origin.reason
                                ));
                            }
                            _ => {}
                        }
                    }
                } else if self.strict_types && !self.quantified_vars.contains(&root) {
                    // Collect usage site notes for enhanced diagnostics
                    let usage_notes = {
                        let sites = &self.usage_sites[root as usize];
                        if sites.len() > 1 {
                            let mut notes = String::new();
                            for (site_span, site_reason) in sites.iter().take(5) {
                                notes.push_str(&format!(
                                    "\n  note: used at line {}:{} ({})",
                                    site_span.line, site_span.col, site_reason
                                ));
                            }
                            if sites.len() > 5 {
                                notes.push_str(&format!(
                                    "\n  note: ... and {} more usage(s)",
                                    sites.len() - 5
                                ));
                            }
                            notes
                        } else {
                            String::new()
                        }
                    };
                    match constraint {
                        TypeConstraint::None => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "line {}:{}: ambiguous type: cannot infer type for this expression ({})\n  help: consider adding a type annotation, e.g. `: i64` or `: String`{}",
                                    origin.span.line, origin.span.col, origin.reason, usage_notes
                                )
                            } else {
                                format!(
                                    "ambiguous type: unsolved type variable ?{root}\n  help: add a type annotation to resolve the ambiguity{usage_notes}"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Numeric => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "line {}:{}: numeric type defaults to i64 ({})\n  help: add `: i64` for integer or `: f64` for float",
                                    origin.span.line, origin.span.col, origin.reason
                                )
                            } else {
                                format!(
                                    "numeric type defaults to i64 for ?{root}\n  help: add `: i64` for integer or `: f64` for float"
                                )
                            };
                            self.default_warnings.push(msg);
                        }
                        TypeConstraint::Trait(traits) => {
                            let traits_str = traits.join(", ");
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "line {}:{}: ambiguous type: cannot infer concrete type for trait-constrained variable (requires: {}) ({})\n  help: add a type annotation for a type that implements {}{}",
                                    origin.span.line,
                                    origin.span.col,
                                    traits_str,
                                    origin.reason,
                                    traits_str,
                                    usage_notes
                                )
                            } else {
                                format!(
                                    "ambiguous type: unsolved type variable ?{root} with trait bound(s) [{traits_str}]\n  help: add a type annotation for a type that implements {traits_str}{usage_notes}"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Integer if self.pedantic => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "line {}:{}: pedantic: integer type defaults to i64 ({})\n  help: add an explicit annotation, e.g. `: i64` or `: i32`",
                                    origin.span.line, origin.span.col, origin.reason
                                )
                            } else {
                                format!(
                                    "pedantic: unsolved integer type variable ?{root} defaults to i64\n  help: add an explicit annotation, e.g. `: i64` or `: i32`"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        TypeConstraint::Float if self.pedantic => {
                            let msg = if let Some(origin) = &self.origins[root as usize] {
                                format!(
                                    "line {}:{}: pedantic: float type defaults to f64 ({})\n  help: add an explicit annotation, e.g. `: f64` or `: f32`",
                                    origin.span.line, origin.span.col, origin.reason
                                )
                            } else {
                                format!(
                                    "pedantic: unsolved float type variable ?{root} defaults to f64\n  help: add an explicit annotation, e.g. `: f64` or `: f32`"
                                )
                            };
                            self.strict_errors.push(msg);
                        }
                        _ => {}
                    }
                }
                default_ty
            }
            Type::Array(inner, len) => {
                Type::Array(Box::new(self.resolve_container_elem(inner)), *len)
            }
            Type::Vec(inner) => Type::Vec(Box::new(self.resolve_container_elem(inner))),
            Type::Map(k, v) => Type::Map(
                Box::new(self.resolve_core(k, warn_only)),
                Box::new(self.resolve_core(v, warn_only)),
            ),
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| self.resolve_core(t, warn_only))
                    .collect(),
            ),
            Type::Fn(params, ret) => Type::Fn(
                params
                    .iter()
                    .map(|t| self.resolve_core(t, warn_only))
                    .collect(),
                Box::new(self.resolve_core(ret, warn_only)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(self.resolve_core(inner, warn_only))),
            Type::Rc(inner) => Type::Rc(Box::new(self.resolve_core(inner, warn_only))),
            Type::Weak(inner) => Type::Weak(Box::new(self.resolve_core(inner, warn_only))),
            Type::Coroutine(inner) => {
                Type::Coroutine(Box::new(self.resolve_core(inner, warn_only)))
            }
            Type::Channel(inner) => Type::Channel(Box::new(self.resolve_core(inner, warn_only))),
            _ => ty.clone(),
        }
    }

    pub(crate) fn instantiate(&mut self, scheme: &crate::types::Scheme) -> Type {
        if scheme.quantified.is_empty() {
            return scheme.ty.clone();
        }
        let subst: std::collections::HashMap<u32, Type> = scheme
            .quantified
            .iter()
            .map(|&v| {
                let root = self.find(v);
                let constraint = self.constraints[root as usize].clone();
                let fresh = match constraint {
                    TypeConstraint::Integer => self.fresh_integer_var(),
                    TypeConstraint::Float => self.fresh_float_var(),
                    TypeConstraint::Numeric => self.fresh_numeric_var(),
                    TypeConstraint::Addable => {
                        let var = self.fresh_var();
                        if let Type::TypeVar(id) = var {
                            let root = self.find(id);
                            self.constraints[root as usize] = TypeConstraint::Addable;
                        }
                        var
                    }
                    TypeConstraint::Trait(ref traits) => {
                        let var = self.fresh_var();
                        if let Type::TypeVar(id) = var {
                            let root = self.find(id);
                            self.constraints[root as usize] = TypeConstraint::Trait(traits.clone());
                        }
                        var
                    }
                    TypeConstraint::None => self.fresh_var(),
                };
                (v, fresh)
            })
            .collect();
        self.substitute(&scheme.ty, &subst)
    }

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
            Type::Tuple(tys) => {
                Type::Tuple(tys.iter().map(|t| self.substitute(t, subst)).collect())
            }
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
        ctx.disable_strict_types();
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
        ctx.disable_strict_types();
        let v = ctx.fresh_var();
        let _ = ctx.resolve(&v);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_default_warnings_collected_when_enabled() {
        let mut ctx = InferCtx::new();
        ctx.disable_strict_types();
        ctx.enable_default_warnings();
        let span = Span {
            start: 0,
            end: 0,
            line: 5,
            col: 3,
        };
        let v = ctx.fresh_var();
        let v2 = ctx.fresh_var();
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
        ctx.disable_strict_types();
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
        ctx.disable_strict_types();
        ctx.enable_default_warnings();
        let v = ctx.fresh_integer_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::I64);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_default_warnings_float_constraint_no_warning() {
        let mut ctx = InferCtx::new();
        ctx.disable_strict_types();
        ctx.enable_default_warnings();
        let v = ctx.fresh_float_var();
        let resolved = ctx.resolve(&v);
        assert_eq!(resolved, Type::F64);
        let warnings = ctx.drain_default_warnings();
        assert!(warnings.is_empty());
    }
}
