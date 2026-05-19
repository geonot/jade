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

    pub(crate) struct_attrs: IndexMap<Symbol, crate::ast::LayoutAttrs>,
    pub(crate) enums: IndexMap<Symbol, Vec<(Symbol, Vec<Type>)>>,

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

    pub(crate) fn_param_access: IndexMap<Symbol, Vec<Option<ast::AccessMod>>>,

    pub(crate) moved_fields: std::collections::HashMap<DefId, std::collections::HashSet<Symbol>>,

    pub(crate) const_vars: std::collections::HashSet<DefId>,

    pub(crate) suppress_moved_field_check: u32,
    pub(crate) current_method_type: Option<String>,
    pub(crate) modules: std::collections::HashSet<Symbol>,

    pub(crate) externs: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,

    pub(crate) current_fn_ret_ty: Option<Type>,

    pub(crate) current_fn_error_types: std::collections::BTreeSet<Symbol>,

    pub(crate) current_fn_declared_errors: Vec<Symbol>,

    pub(crate) escape_tiers: std::collections::HashMap<DefId, crate::escape::Tier>,
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
            fn_param_access: IndexMap::new(),
            moved_fields: std::collections::HashMap::new(),
            const_vars: std::collections::HashSet::new(),
            suppress_moved_field_check: 0,
            current_method_type: None,
            modules: std::collections::HashSet::new(),
            externs: IndexMap::new(),
            current_fn_ret_ty: None,
            current_fn_error_types: std::collections::BTreeSet::new(),
            current_fn_declared_errors: Vec::new(),
            escape_tiers: std::collections::HashMap::new(),
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

    pub(crate) fn mark_field_moved(&mut self, parent: DefId, field: Symbol) {
        self.moved_fields.entry(parent).or_default().insert(field);
    }

    pub(crate) fn clear_field_moved(&mut self, parent: DefId, field: &Symbol) {
        if let Some(set) = self.moved_fields.get_mut(&parent) {
            set.remove(field);
            if set.is_empty() {
                self.moved_fields.remove(&parent);
            }
        }
    }

    pub(crate) fn clear_all_moved_for(&mut self, parent: DefId) {
        self.moved_fields.remove(&parent);
    }

    pub(crate) fn snapshot_moved_fields(
        &self,
    ) -> std::collections::HashMap<DefId, std::collections::HashSet<Symbol>> {
        self.moved_fields.clone()
    }

    pub(crate) fn restore_moved_fields(
        &mut self,
        snap: std::collections::HashMap<DefId, std::collections::HashSet<Symbol>>,
    ) {
        self.moved_fields = snap;
    }

    pub(crate) fn merge_moved_fields_union(
        &mut self,
        branches: &[std::collections::HashMap<DefId, std::collections::HashSet<Symbol>>],
    ) {
        let mut out = self.moved_fields.clone();
        for br in branches {
            for (id, fields) in br {
                out.entry(*id).or_default().extend(fields.iter().cloned());
            }
        }
        self.moved_fields = out;
    }

    fn ownership_for_type(ty: &Type) -> Ownership {
        match ty {
            Type::Ptr(_) => Ownership::Raw,
            _ => Ownership::Owned,
        }
    }

    pub(crate) fn ownership_with_mod(
        &self,
        ty: &Type,
        access_mod: Option<crate::ast::AccessMod>,
    ) -> Result<Ownership, String> {
        use crate::ast::AccessMod::*;
        let resource = self.type_has_resource_annotation(ty);
        let promote_owned = || -> Ownership {
            if matches!(ty, Type::Ptr(_)) {
                Ownership::Raw
            } else {
                Ownership::Owned
            }
        };
        let ow = match access_mod {
            Some(Copy) => {
                if resource {
                    return Err(format!(
                        "cannot `copy` a @resource type ({ty}): use `take` (move) instead"
                    ));
                }
                promote_owned()
            }
            Some(Take) => promote_owned(),
            Some(Const) => promote_owned(),
            None => promote_owned(),
        };
        Ok(ow)
    }

    pub(crate) fn param_ownership_with_mod(
        &self,
        ty: &Type,
        access_mod: Option<crate::ast::AccessMod>,
    ) -> Result<Ownership, String> {
        if access_mod.is_none() {
            if self.type_param_default_borrows(ty) {
                return Ok(Ownership::Borrowed);
            }
        }
        self.ownership_with_mod(ty, access_mod)
    }

    fn type_param_default_borrows(&self, ty: &Type) -> bool {
        match ty {
            Type::String
            | Type::Vec(_)
            | Type::Map(_, _)
            | Type::Coroutine(_)
            | Type::Generator(_) => true,

            Type::Struct(_, _) | Type::Enum(_) | Type::Tuple(_) | Type::Array(_, _) => {
                self.needs_drop(ty)
            }

            Type::Alias(_, inner) | Type::Newtype(_, inner) => {
                self.type_param_default_borrows(inner)
            }

            _ => false,
        }
    }

    pub(crate) fn type_has_resource_annotation(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct(name, _) => self
                .struct_attrs
                .get(name)
                .map(|a| a.resource)
                .unwrap_or(false),

            Type::Coroutine(_) | Type::Generator(_) => true,

            Type::Row(_) => true,
            Type::Newtype(_, inner) | Type::Alias(_, inner) => {
                self.type_has_resource_annotation(inner)
            }
            _ => false,
        }
    }

    pub(crate) fn enforce_cross_thread_safe(
        &self,
        ty: &Type,
        span: crate::ast::Span,
        context: &str,
    ) -> Result<(), String> {
        if self.type_has_resource_annotation(ty) {
            return Err(format!(
                "{}: resource type `{}` cannot cross thread boundaries ({})",
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
        quantified.sort_unstable();
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
