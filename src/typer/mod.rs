//! Typer root: orchestrates inference, name resolution, ownership, and HIR lowering.

use indexmap::IndexMap;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::intern::Symbol;
use crate::types::{Scheme, Type};

#[derive(Debug, Clone)]
pub(crate) struct VarInfo {
    pub(crate) def_id: DefId,
    pub(crate) ty: Type,
    #[allow(dead_code)]
    pub(crate) ownership: Ownership,
    pub(crate) scheme: Option<Scheme>,
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredMethod {
    pub(crate) receiver_ty: Type,
    pub(crate) method: Symbol,
    pub(crate) arg_tys: Vec<Type>,
    pub(crate) ret_ty: Type,
    pub(crate) span: Span,
}

#[derive(Debug, Clone)]
pub(crate) struct DeferredField {
    pub(crate) receiver_ty: Type,
    pub(crate) field_name: Symbol,
    pub(crate) field_ty: Type,
    pub(crate) span: Span,
}

mod mono;
mod resolve;
pub(crate) mod unify;

pub struct Typer {
    pub(crate) next_id: u32,
    pub(crate) scopes: Vec<HashMap<Symbol, VarInfo>>,
    pub(crate) fns: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,
    pub(crate) structs: IndexMap<Symbol, Vec<(Symbol, Type)>>,
    /// Per-struct layout/annotation attributes (`@packed`, `@strict`,
    /// `@align`, `@resource`, `@atomic`, `@weakable`). Populated when the
    /// declaration is registered; consulted by access-mode inference
    /// (§3 of `docs/access-semantics.md`) and codegen.
    pub(crate) struct_attrs: IndexMap<Symbol, crate::ast::LayoutAttrs>,
    pub(crate) enums: IndexMap<Symbol, Vec<(Symbol, Vec<Type>)>>,
    /// Names of enums declared with the `err` keyword. Used to classify
    /// `! Variant` returns and infer per-function error unions.
    pub(crate) err_enum_names: std::collections::HashSet<Symbol>,
    pub(crate) variant_tags: IndexMap<Symbol, (Symbol, u32)>,
    pub(crate) generic_fns: IndexMap<Symbol, ast::Fn>,
    pub(crate) generic_enums: IndexMap<Symbol, ast::EnumDef>,
    pub(crate) generic_types: IndexMap<Symbol, ast::TypeDef>,
    pub(crate) methods: IndexMap<Symbol, Vec<ast::Fn>>,
    pub(crate) mono_fns: Vec<hir::Fn>,
    pub(crate) mono_enums: Vec<hir::EnumDef>,
    pub(crate) mono_types: Vec<hir::TypeDef>,
    pub(crate) inferred_field_structs: std::collections::HashSet<Symbol>,
    pub(crate) source_dir: Option<PathBuf>,
    pub(crate) test_mode: bool,
    pub(crate) actors:
        IndexMap<Symbol, (DefId, Vec<(Symbol, Type)>, Vec<(Symbol, Vec<Type>, u32)>)>,
    pub(crate) store_schemas: IndexMap<Symbol, Vec<(Symbol, Type)>>,
    pub(crate) store_decorators: IndexMap<Symbol, Vec<crate::ast::StoreDecorator>>,
    pub(crate) view_defs: IndexMap<Symbol, (Symbol, Vec<crate::ast::QueryClause>)>,
    pub(crate) mono_depth: u32,
    pub(crate) traits: IndexMap<Symbol, Vec<TraitMethodSig>>,
    pub(crate) trait_impls: IndexMap<Symbol, Vec<String>>,
    pub(crate) generic_bounds: IndexMap<Symbol, Vec<(Symbol, Vec<Symbol>)>>,
    pub(crate) trait_impl_type_args: IndexMap<(Symbol, Symbol), Vec<Type>>,
    pub(crate) assoc_types: IndexMap<(Symbol, Symbol), Type>,
    pub(crate) trait_assoc_types: IndexMap<Symbol, Vec<String>>,
    pub(crate) consts: IndexMap<Symbol, ast::Expr>,
    pub(crate) globals: IndexMap<Symbol, (ast::Expr, ast::Span)>,
    pub(crate) infer_ctx: unify::InferCtx,
    pub(crate) debug_types: bool,
    pub(crate) warnings: Vec<String>,
    pub(crate) deferred_methods: Vec<DeferredMethod>,
    pub(crate) deferred_fields: Vec<DeferredField>,
    pub(crate) deferred_quantified_vars: Vec<u32>,
    pub(crate) field_constraints: IndexMap<u32, Vec<(Symbol, Type)>>,
    pub(crate) inferable_fns: IndexMap<Symbol, ast::Fn>,
    pub(crate) fn_schemes: IndexMap<Symbol, (Vec<u32>, Vec<Type>, Type)>,
    pub(crate) unannotated_struct_fields: Vec<(String, String, Type, Span)>,
    pub(crate) poly_lambda_asts:
        IndexMap<Symbol, (Vec<ast::Param>, Option<Type>, ast::Block, Span)>,
    pub(crate) type_errors: Vec<String>,
    pub(crate) fn_param_names: IndexMap<Symbol, Vec<String>>,
    pub(crate) fn_defaults: IndexMap<Symbol, Vec<Option<ast::Expr>>>,
    pub(crate) current_method_type: Option<String>,
    pub(crate) modules: std::collections::HashSet<Symbol>,
    /// Extern functions tracked separately from Jinn functions.
    /// Key: C symbol name. Value: (DefId, param types, return type).
    /// Externs are NOT module-prefixed — they keep their C symbol names.
    pub(crate) externs: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,
    /// Current function's return type, used by `try` desugaring.
    pub(crate) current_fn_ret_ty: Option<Type>,
    /// Set of err-enum names that the current function actually early-returns
    /// via `! Variant`. Populated during `lower_fn`; consumed to populate
    /// `hir::Fn.error_types`.
    pub(crate) current_fn_error_types: std::collections::BTreeSet<Symbol>,
    /// Declared `! E1 ! E2` after `returns T` for the current function, if
    /// any. When non-empty, `! Variant` is validated to belong to one of
    /// these enums; when empty, the union is inferred.
    pub(crate) current_fn_declared_errors: Vec<Symbol>,
}

#[derive(Debug, Clone)]
pub(crate) struct TraitMethodSig {
    pub(crate) name: Symbol,
    pub(crate) _params: Vec<(String, Option<Type>)>,
    pub(crate) _ret: Option<Type>,
    pub(crate) has_default: bool,
}

impl Typer {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            scopes: Vec::new(),
            fns: IndexMap::new(),
            structs: IndexMap::new(),
            struct_attrs: IndexMap::new(),
            enums: IndexMap::new(),
            err_enum_names: std::collections::HashSet::new(),
            variant_tags: IndexMap::new(),
            generic_fns: IndexMap::new(),
            generic_enums: IndexMap::new(),
            generic_types: IndexMap::new(),
            methods: IndexMap::new(),
            mono_fns: Vec::new(),
            mono_enums: Vec::new(),
            mono_types: Vec::new(),
            inferred_field_structs: std::collections::HashSet::new(),
            source_dir: None,
            test_mode: false,
            actors: IndexMap::new(),
            store_schemas: IndexMap::new(),
            store_decorators: IndexMap::new(),
            view_defs: IndexMap::new(),
            mono_depth: 0,
            traits: IndexMap::new(),
            trait_impls: IndexMap::new(),
            generic_bounds: IndexMap::new(),
            trait_impl_type_args: IndexMap::new(),
            assoc_types: IndexMap::new(),
            trait_assoc_types: IndexMap::new(),
            consts: IndexMap::new(),
            globals: IndexMap::new(),
            infer_ctx: unify::InferCtx::new(),
            debug_types: false,
            warnings: Vec::new(),
            deferred_methods: Vec::new(),
            deferred_fields: Vec::new(),
            deferred_quantified_vars: Vec::new(),
            field_constraints: IndexMap::new(),
            inferable_fns: IndexMap::new(),
            fn_schemes: IndexMap::new(),
            unannotated_struct_fields: Vec::new(),
            poly_lambda_asts: IndexMap::new(),
            type_errors: Vec::new(),
            fn_param_names: IndexMap::new(),
            fn_defaults: IndexMap::new(),
            current_method_type: None,
            modules: std::collections::HashSet::new(),
            externs: IndexMap::new(),
            current_fn_ret_ty: None,
            current_fn_error_types: std::collections::BTreeSet::new(),
            current_fn_declared_errors: Vec::new(),
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = Some(dir);
    }

    pub fn set_test_mode(&mut self, enabled: bool) {
        self.test_mode = enabled;
    }

    pub fn set_debug_types(&mut self, enabled: bool) {
        self.debug_types = enabled;
        self.infer_ctx.debug = enabled;
    }

    pub fn set_warn_inferred_defaults(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.enable_default_warnings();
        }
    }

    pub fn set_strict_types(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.enable_strict_types();
        }
    }

    pub fn set_lenient(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.disable_strict_types();
        }
    }

    pub fn set_pedantic(&mut self, enabled: bool) {
        if enabled {
            self.infer_ctx.set_pedantic(true);
        }
    }

    fn fresh_id(&mut self) -> DefId {
        let id = DefId(self.next_id);
        self.next_id += 1;
        id
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_var(&mut self, name: &str, info: VarInfo) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.into(), info);
        }
    }

    fn find_var(&self, name: &str) -> Option<&VarInfo> {
        let sym: Symbol = name.into();
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(&sym) {
                return Some(v);
            }
        }
        None
    }

    fn update_var(&mut self, name: &str, info: VarInfo) {
        let sym: Symbol = name.into();
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(&sym) {
                scope.insert(sym, info);
                return;
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(sym, info);
        }
    }

    fn resolve_ty(&self, ty: Type) -> Type {
        match &ty {
            Type::Struct(n, _) if self.enums.contains_key(n) => Type::Enum(n.clone()),
            _ => ty,
        }
    }

    fn collect_unify_error(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.type_errors.push(e);
        }
    }

    /// Unify expected type with a call/method result type, but only when the
    /// result type has already resolved to a concrete type.  When the result is
    /// still an unresolved TypeVar (i.e. the callee's return type hasn't been
    /// inferred from its body yet), we skip the unification to prevent the
    /// caller's expected type from backward-propagating into the callee's return
    /// type variable.
    pub(crate) fn unify_call_result(
        &mut self,
        expected: &Type,
        result_ty: &Type,
        span: crate::ast::Span,
        ctx: &'static str,
    ) {
        let resolved = self.infer_ctx.shallow_resolve(result_ty);
        if !matches!(resolved, Type::TypeVar(_)) {
            let _ = self.infer_ctx.unify_at(expected, result_ty, span, ctx);
        }
    }

    pub(crate) fn make_coerce(
        expr: hir::Expr,
        coercion: hir::CoercionKind,
        target_ty: Type,
    ) -> hir::Expr {
        let span = expr.span;
        hir::Expr {
            kind: hir::ExprKind::Coerce(Box::new(expr), coercion),
            ty: target_ty,
            span,
        }
    }

    fn ownership_for_type(ty: &Type) -> Ownership {
        match ty {
            Type::Rc(_) => Ownership::Rc,
            Type::Ptr(_) => Ownership::Raw,
            _ => Ownership::Owned,
        }
    }

    /// Compute the HIR `Ownership` tier for a binding/param/field, honoring
    /// any explicit access modifier (`copy`/`ref`/`mut`/`take`) and any
    /// type-level annotations (`@atomic` promotes to Arc tier;
    /// `@resource` rejects `copy`).
    ///
    /// Returns `Err` only when a hard semantic rule is violated (e.g. `copy`
    /// of a `@resource`). When no modifier is present, falls back to the
    /// structural default produced by `ownership_for_type` then promoted
    /// to Arc if the underlying struct is `@atomic`.
    pub(crate) fn ownership_with_mod(
        &self,
        ty: &Type,
        access_mod: Option<crate::ast::AccessMod>,
    ) -> Result<Ownership, String> {
        use crate::ast::AccessMod::*;
        let atomic = self.type_has_atomic_annotation(ty);
        let resource = self.type_has_resource_annotation(ty);
        let promote_owned = || -> Ownership {
            if atomic {
                Ownership::Arc
            } else if matches!(ty, Type::Rc(_)) {
                Ownership::Rc
            } else if matches!(ty, Type::Ptr(_)) {
                Ownership::Raw
            } else {
                Ownership::Owned
            }
        };
        let ow = match access_mod {
            Some(Copy) => {
                if resource {
                    return Err(format!(
                        "cannot `copy` a @resource type ({ty}): use `take` (move), `ref` (alias) or `mut` (mutable alias) instead"
                    ));
                }
                promote_owned()
            }
            Some(Take) => promote_owned(),
            Some(Ref) => Ownership::Borrowed,
            Some(Mut) => Ownership::BorrowMut,
            None => promote_owned(),
        };
        Ok(ow)
    }

    /// True iff `ty` is (or wraps) a user-defined struct annotated with
    /// `@resource`. See `docs/access-semantics.md` §3.
    ///
    /// Walks through `Rc<T>`, `Vec<T>`, `Cow<T>`, `Newtype`, `Alias`, etc.
    /// to find the underlying struct \u2014 a `Vec(Socket)` is still a vector
    /// *of* resources, so the linear discipline propagates.
    ///
    /// Built-in linear types (`Coroutine`, `Generator`) are also resources.
    pub(crate) fn type_has_resource_annotation(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct(name, _) => self
                .struct_attrs
                .get(name)
                .map(|a| a.resource)
                .unwrap_or(false),
            // Built-in linear resource types (close-once / single-run).
            Type::Coroutine(_) | Type::Generator(_) => true,
            Type::Newtype(_, inner)
            | Type::Alias(_, inner)
            | Type::Rc(inner)
            | Type::Weak(inner)
            | Type::Cow(inner) => self.type_has_resource_annotation(inner),
            _ => false,
        }
    }

    /// True iff `ty` is a struct annotated with `@atomic`, requiring
    /// tier-3 (Arc / Arc<Mutex>) lowering for shared bindings.
    /// See `docs/access-semantics.md` \u00a73.
    ///
    /// Built-in cross-thread types (`Channel`, `ActorRef`) are atomic by
    /// construction \u2014 they already carry an atomic refcount header.
    pub(crate) fn type_has_atomic_annotation(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct(name, _) => self
                .struct_attrs
                .get(name)
                .map(|a| a.atomic)
                .unwrap_or(false),
            // Built-in cross-thread atomic types.
            Type::Channel(_) | Type::ActorRef(_) => true,
            Type::Newtype(_, inner) | Type::Alias(_, inner) => {
                self.type_has_atomic_annotation(inner)
            }
            _ => false,
        }
    }

    /// Enforce the `@resource` cross-thread safety rule
    /// (`docs/access-semantics.md` \u00a74.1 final bullet).
    ///
    /// A `@resource` type may cross a thread boundary only when it is also
    /// annotated `@atomic`. The boundary is any value sent on a channel,
    /// passed to an actor handler, or supplied as an actor-spawn init.
    ///
    /// `context` is a short label inserted into the diagnostic
    /// (e.g. `"channel send"`, `"actor handler argument"`,
    /// `"actor spawn init"`).
    pub(crate) fn enforce_cross_thread_safe(
        &self,
        ty: &Type,
        span: crate::ast::Span,
        context: &str,
    ) -> Result<(), String> {
        if self.type_has_resource_annotation(ty) && !self.type_has_atomic_annotation(ty) {
            return Err(format!(
                "{}: resource type `{}` is not `@atomic`; cannot send across threads ({})",
                span.loc(),
                ty,
                context
            ));
        }
        Ok(())
    }

    fn free_type_vars_in_env(&mut self) -> std::collections::HashSet<u32> {
        let mut ftvs = std::collections::HashSet::new();
        for scope in &self.scopes {
            for info in scope.values() {
                let resolved = self.infer_ctx.shallow_resolve(&info.ty);
                resolved.free_type_vars(&mut ftvs);
            }
        }
        ftvs
    }

    fn generalize(&mut self, ty: &Type) -> Scheme {
        let resolved = self.infer_ctx.canonicalize_type(ty);
        if !resolved.has_type_var() {
            return Scheme::mono(resolved);
        }
        let env_ftvs = self.free_type_vars_in_env();
        let mut ty_ftvs = std::collections::HashSet::new();
        resolved.free_type_vars(&mut ty_ftvs);
        let mut quantified: Vec<u32> = ty_ftvs.difference(&env_ftvs).copied().collect();
        quantified.sort_unstable(); // deterministic scheme variable ordering
        if quantified.is_empty() {
            Scheme::mono(resolved)
        } else {
            Scheme {
                quantified,
                ty: resolved,
            }
        }
    }

    fn is_syntactic_value(expr: &ast::Expr) -> bool {
        match expr {
            ast::Expr::Lambda(..) => true,
            ast::Expr::Ident(..) => true,
            ast::Expr::Struct(..) => true,
            ast::Expr::Array(elems, _) | ast::Expr::Tuple(elems, _) => {
                elems.iter().all(Self::is_syntactic_value)
            }
            ast::Expr::Ref(inner, _) => Self::is_syntactic_value(inner),
            _ => false,
        }
    }

    pub(crate) fn string_method_ret_ty(method: &str) -> Option<Type> {
        match method {
            "contains" | "starts_with" | "ends_with" => Some(Type::Bool),
            "matches" => Some(Type::Bool),
            "char_at" | "len" | "find" => Some(Type::I64),
            "slice" | "trim" | "trim_left" | "trim_right" | "replace" | "to_upper" | "to_lower"
            | "repeat" | "replace_re" => Some(Type::String),
            "split" | "lines" => Some(Type::Vec(Box::new(Type::String))),
            "find_all" => Some(Type::Vec(Box::new(Type::String))),
            "is_empty" => Some(Type::Bool),
            _ => None,
        }
    }

    /// Returns true if the method name is exclusive to String (not shared with Vec/Map/Struct).
    /// Used to immediately constrain TypeVar receivers to String.
    pub(crate) fn is_string_exclusive_method(method: &str) -> bool {
        matches!(
            method,
            "contains"
                | "starts_with"
                | "ends_with"
                | "char_at"
                | "find"
                | "slice"
                | "trim"
                | "trim_left"
                | "trim_right"
                | "replace"
                | "to_upper"
                | "to_lower"
                | "split"
                | "lines"
                | "repeat"
                | "is_empty"
                | "matches"
                | "find_all"
                | "replace_re"
        )
    }

    pub(crate) fn vec_method_ret_ty(method: &str, elem_ty: &Type) -> Option<Type> {
        match method {
            "push" | "clear" | "set" => Some(Type::Void),
            "pop" | "get" | "remove" | "shift" | "first" | "last" => Some(elem_ty.clone()),
            "len" | "count" => Some(Type::I64),
            "is_empty" => Some(Type::Bool),
            "take" | "skip" | "flatten" | "collect" | "reverse" | "sort" => {
                Some(Type::Vec(Box::new(elem_ty.clone())))
            }
            "sum" => Some(elem_ty.clone()),
            "contains" => Some(Type::Bool),
            "join" => Some(Type::String),
            "enumerate" => Some(Type::Vec(Box::new(Type::Tuple(vec![
                Type::I64,
                elem_ty.clone(),
            ])))),
            _ => None,
        }
    }

    pub(crate) fn map_method_ret_ty(method: &str, key_ty: &Type, val_ty: &Type) -> Option<Type> {
        match method {
            "set" | "remove" | "clear" => Some(Type::Void),
            "get" => Some(val_ty.clone()),
            "has" | "contains" => Some(Type::Bool),
            "len" => Some(Type::I64),
            "keys" => Some(Type::Vec(Box::new(key_ty.clone()))),
            "values" => Some(Type::Vec(Box::new(val_ty.clone()))),
            _ => None,
        }
    }

    pub(crate) fn set_method_ret_ty(method: &str, elem_ty: &Type) -> Option<Type> {
        match method {
            "add" | "remove" | "clear" => Some(Type::Void),
            "contains" => Some(Type::Bool),
            "len" => Some(Type::I64),
            "union" | "difference" | "intersection" => Some(Type::Set(Box::new(elem_ty.clone()))),
            "to_vec" => Some(Type::Vec(Box::new(elem_ty.clone()))),
            _ => None,
        }
    }
}

mod builtins;
mod call;
mod expr;
mod infer;
mod lower;
pub(crate) mod scc;
mod stmt;

#[cfg(test)]
mod tests;
