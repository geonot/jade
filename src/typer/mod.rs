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

mod caps;
mod consume_infer;
mod errset;
mod mono;
mod mutate_infer;
pub(crate) mod place;
mod resolve;
pub(crate) mod unify;

pub(crate) type MoveState = place::MoveSet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BorrowKind {
    Iter,
    ViewBind { binder: DefId, scope_depth: usize },
    FrozenShare,
}

#[derive(Debug, Clone)]
pub(crate) struct BorrowEntry {
    pub(crate) place: place::Place,
    pub(crate) span: Span,
    pub(crate) kind: BorrowKind,
}

impl BorrowEntry {
    pub(crate) fn clause(&self, at: &place::Place) -> String {
        let what = if self.place.proj == at.proj {
            "it".to_string()
        } else {
            format!("`{}`", self.place.render())
        };
        match self.kind {
            BorrowKind::Iter => {
                format!(
                    "the `for` loop at {} is iterating {}",
                    self.span.loc(),
                    what
                )
            }
            BorrowKind::ViewBind { .. } => {
                format!("the view bound at {} borrows {}", self.span.loc(), what)
            }
            BorrowKind::FrozenShare => format!(
                "the `together` at {} shares it frozen with its tasks",
                self.span.loc()
            ),
        }
    }

    pub(crate) fn help(&self) -> &'static str {
        match self.kind {
            BorrowKind::Iter => {
                "iterate by index, or collect the changes and apply them after the loop"
            }
            BorrowKind::ViewBind { .. } => {
                "the view lives to the end of its block; do this after the block, or \
                 copy the data instead (`slice` copies)"
            }
            BorrowKind::FrozenShare => {
                "a frozen value shared with tasks stays readable until the `together` \
                 joins them; do this after the `together`"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MoveReason {
    TakeExplicit,

    ConsumingCall(Symbol, usize),

    AssignMove(Symbol, crate::ast::Span),

    Sent(crate::ast::Span),

    TaskCapture(crate::ast::Span),

    ContainerInsert(Symbol, crate::ast::Span),

    CtorCapture(crate::ast::Span),

    ClosureCapture(crate::ast::Span),

    Freeze(crate::ast::Span),
}

#[allow(clippy::type_complexity)]
pub struct Typer {
    pub(crate) next_id: u32,
    pub(crate) scopes: Vec<HashMap<Symbol, VarInfo>>,
    pub(crate) scope_names: Vec<Symbol>,
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
    pub(crate) store_relations: IndexMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    pub(crate) store_error_def: Option<crate::ast::ErrDef>,
    pub(crate) view_defs: IndexMap<Symbol, (Symbol, Vec<crate::ast::QueryClause>)>,
    pub(crate) mono_depth: u32,
    pub(crate) traits: IndexMap<Symbol, Vec<TraitMethodSig>>,
    pub(crate) trait_defs: IndexMap<Symbol, ast::TraitDef>,
    pub(crate) trait_default_methods: IndexMap<(Symbol, Symbol), Vec<ast::Fn>>,
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

    pub(crate) moves: place::MoveSet,

    pub(crate) current_fn_param_ids: std::collections::HashSet<DefId>,

    pub(crate) suppress_whole_struct_check: u32,

    pub(crate) const_vars: std::collections::HashSet<DefId>,

    pub(crate) defer_read_vars: std::collections::HashMap<DefId, crate::ast::Span>,
    pub(crate) payload_bind_subjects: std::collections::HashMap<DefId, place::Place>,

    pub(crate) suppress_moved_field_check: u32,
    pub(crate) current_method_type: Option<String>,
    pub(crate) modules: std::collections::HashSet<Symbol>,

    pub(crate) externs: IndexMap<Symbol, (DefId, Vec<Type>, Type)>,

    pub(crate) current_fn_ret_ty: Option<Type>,
    pub(crate) current_fn_is_main: bool,

    pub(crate) current_fn_error_types: std::collections::BTreeSet<Symbol>,

    pub(crate) current_fn_declared_errors: Vec<Symbol>,

    pub(crate) last_inferred_errors: std::collections::BTreeSet<Symbol>,

    pub(crate) escape_tiers: std::collections::HashMap<DefId, crate::escape::Tier>,

    pub(crate) dollar_stack: Vec<(DefId, Type)>,

    pub(crate) root_pkg_id: Option<crate::pkgid::PkgId>,
    pub(crate) dep_pkg_ids: std::collections::HashMap<crate::intern::Symbol, crate::pkgid::PkgId>,
    pub(crate) scoped_use_map: crate::pkgid::ScopedUseMap,
    pub(crate) declared_type_names: std::collections::HashSet<Symbol>,

    pub(crate) const_expansion_stack: Vec<Symbol>,

    pub(crate) instantiated_generics: std::collections::HashSet<Symbol>,

    pub(crate) iter_borrowed: Vec<BorrowEntry>,

    pub(crate) view_roots: std::collections::HashMap<DefId, place::Place>,

    pub(crate) together_outer_ids: Vec<std::collections::HashSet<DefId>>,

    pub(crate) suppress_move_marking: u32,

    pub(crate) fn_param_mutates: IndexMap<Symbol, Vec<bool>>,

    pub(crate) fn_param_consume_sites: IndexMap<Symbol, Vec<Option<(crate::ast::Span, bool)>>>,

    pub(crate) pending_mono_methods: Vec<(Symbol, ast::Fn)>,

    pub(crate) mono_methods_done: std::collections::HashSet<Symbol>,
    pub(crate) called_mono_methods: std::collections::HashSet<Symbol>,

    pub(crate) std_files: std::collections::HashSet<Symbol>,
}

#[derive(Debug, Clone)]
pub(crate) struct TraitMethodSig {
    pub(crate) name: Symbol,
    pub(crate) _params: Vec<(String, Option<Type>)>,
    pub(crate) _ret: Option<Type>,
    pub(crate) has_default: bool,
}

impl Default for Typer {
    fn default() -> Self {
        Self::new()
    }
}

impl Typer {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            scopes: Vec::new(),
            scope_names: Vec::new(),
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
            store_relations: IndexMap::new(),
            store_error_def: None,
            view_defs: IndexMap::new(),
            mono_depth: 0,
            traits: IndexMap::new(),
            trait_defs: IndexMap::new(),
            trait_default_methods: IndexMap::new(),
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
            moves: place::MoveSet::default(),
            current_fn_param_ids: std::collections::HashSet::new(),
            suppress_whole_struct_check: 0,
            declared_type_names: std::collections::HashSet::new(),
            const_expansion_stack: Vec::new(),
            instantiated_generics: std::collections::HashSet::new(),
            const_vars: std::collections::HashSet::new(),
            defer_read_vars: std::collections::HashMap::new(),
            payload_bind_subjects: std::collections::HashMap::new(),
            suppress_moved_field_check: 0,
            current_method_type: None,
            modules: std::collections::HashSet::new(),
            externs: IndexMap::new(),
            current_fn_ret_ty: None,
            current_fn_is_main: false,
            current_fn_error_types: std::collections::BTreeSet::new(),
            current_fn_declared_errors: Vec::new(),
            last_inferred_errors: std::collections::BTreeSet::new(),
            escape_tiers: std::collections::HashMap::new(),
            dollar_stack: Vec::new(),
            root_pkg_id: None,
            dep_pkg_ids: std::collections::HashMap::new(),
            scoped_use_map: crate::pkgid::ScopedUseMap::new(),
            iter_borrowed: Vec::new(),
            view_roots: std::collections::HashMap::new(),
            together_outer_ids: Vec::new(),
            suppress_move_marking: 0,
            fn_param_mutates: IndexMap::new(),
            fn_param_consume_sites: IndexMap::new(),
            pending_mono_methods: Vec::new(),
            mono_methods_done: std::collections::HashSet::new(),
            called_mono_methods: std::collections::HashSet::new(),
            std_files: std::collections::HashSet::new(),
        }
    }

    pub fn set_source_dir(&mut self, dir: PathBuf) {
        self.source_dir = Some(dir);
    }

    pub fn set_std_files(&mut self, files: std::collections::HashSet<Symbol>) {
        self.std_files = files;
    }

    pub fn set_root_pkg_id(&mut self, id: crate::pkgid::PkgId) {
        self.root_pkg_id = Some(id);
    }

    pub fn set_dep_pkg_ids(
        &mut self,
        ids: std::collections::HashMap<crate::intern::Symbol, crate::pkgid::PkgId>,
    ) {
        self.dep_pkg_ids = ids;
    }

    pub fn set_scoped_use_map(&mut self, map: crate::pkgid::ScopedUseMap) {
        self.scoped_use_map = map;
    }

    pub(crate) fn resolve_scoped_use(
        &self,
        name: Symbol,
    ) -> Result<Option<crate::pkgid::PkgId>, String> {
        let Some(consumer) = self.root_pkg_id else {
            return Ok(None);
        };
        if self.scoped_use_map.is_empty() {
            return Ok(None);
        }
        match crate::pkgid::resolve_use(&self.scoped_use_map, consumer, name) {
            Ok(id) => Ok(Some(id)),
            Err(_) => {
                if self.scoped_use_map.keys().any(|(_, n)| *n == name) {
                    Err(format!(
                        "unresolved import 'use {name}' in package '{}': '{name}' is a \
                         dependency of another scope but not of this one; add it to this \
                         package's project.jn requires (resolution is local, scope.md §2.1)",
                        consumer.fully_qualified()
                    ))
                } else {
                    Ok(None)
                }
            }
        }
    }

    pub(crate) fn resolve_scoped_path_use(
        &self,
        path: &[Symbol],
    ) -> Result<Option<crate::pkgid::PkgId>, String> {
        let Some(consumer) = self.root_pkg_id else {
            return Ok(None);
        };
        if self.scoped_use_map.is_empty() {
            return Ok(None);
        }
        match crate::pkgid::resolve_path_use(&self.scoped_use_map, consumer, path) {
            Ok(id) => Ok(Some(id)),
            Err(crate::pkgid::UseResolveError::Unresolved { .. }) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
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
            self.infer_ctx.set_strict_unsolved(true);
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
        let depth = self.scopes.len();
        self.iter_borrowed.retain(
            |e| !matches!(e.kind, BorrowKind::ViewBind { scope_depth, .. } if scope_depth > depth),
        );
    }

    pub(crate) fn release_view_binds(&mut self, binder: DefId) {
        self.iter_borrowed
            .retain(|e| !matches!(e.kind, BorrowKind::ViewBind { binder: b, .. } if b == binder));
        self.view_roots.remove(&binder);
    }

    pub(crate) fn register_view_bind(
        &mut self,
        binder: DefId,
        root: Option<place::Place>,
        span: Span,
    ) {
        self.release_view_binds(binder);
        if let Some(root) = root {
            self.iter_borrowed.push(BorrowEntry {
                place: root.clone(),
                span,
                kind: BorrowKind::ViewBind {
                    binder,
                    scope_depth: self.scopes.len(),
                },
            });
            self.view_roots.insert(binder, root);
        }
    }

    fn define_var(&mut self, name: &str, info: VarInfo) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.into(), info);
        }
    }

    pub(crate) fn in_scope_def_ids(&self) -> std::collections::HashSet<DefId> {
        self.scopes
            .iter()
            .flat_map(|s| s.values().map(|v| v.def_id))
            .collect()
    }

    pub(crate) fn check_loop_body_moves(
        &self,
        pre: &MoveState,
        outer_ids: &std::collections::HashSet<DefId>,
        span: crate::ast::Span,
    ) -> Result<(), String> {
        for (id, entries) in self.moves.roots() {
            if !outer_ids.contains(id) {
                continue;
            }
            for e in entries {
                let pre_has = pre
                    .entries_for(*id)
                    .iter()
                    .any(|p| p.place.proj == e.place.proj);
                if pre_has {
                    continue;
                }
                if e.place.is_root() {
                    return Err(format!(
                        "{}: value moved inside a loop body (by `take`, an aggregate \
                         assignment, a consuming call, or a channel send) would be moved \
                         again on the next iteration; move a fresh value each iteration or \
                         reassign it before the loop repeats",
                        span.loc(),
                    ));
                }
                return Err(format!(
                    "{}: field moved out inside a loop body would be moved \
                     again on the next iteration; reassign it before the loop repeats",
                    span.loc(),
                ));
            }
        }
        Ok(())
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
            if let std::collections::hash_map::Entry::Occupied(mut e) = scope.entry(sym) {
                e.insert(info);
                return;
            }
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(sym, info);
        }
    }

    fn resolve_ty(&self, ty: Type) -> Type {
        match &ty {
            Type::Struct(n, _) if self.enums.contains_key(n) => Type::Enum(*n),

            Type::Param(n) if self.enums.contains_key(n) => Type::Enum(*n),
            Type::Param(n) if self.structs.contains_key(n) => Type::Struct(*n, vec![]),
            _ => ty,
        }
    }

    fn collect_unify_error(&mut self, result: Result<(), String>) {
        if let Err(e) = result {
            self.type_errors.push(e);
        }
    }

    pub(crate) fn check_extern_arg(
        &mut self,
        pty: &Type,
        aty: &Type,
        span: crate::ast::Span,
        reason: &'static str,
    ) {
        let rp = self.infer_ctx.shallow_resolve(pty);
        let ra = self.infer_ctx.shallow_resolve(aty);

        let param_is_opaque_ptr =
            matches!(&rp, Type::Ptr(inner) if matches!(**inner, Type::I8 | Type::U8 | Type::Void));
        if param_is_opaque_ptr && matches!(ra, Type::String | Type::Ptr(_)) {
            return;
        }
        let _ = self.infer_ctx.unify_at(pty, aty, span, reason);
    }

    pub(crate) fn unify_call_result(
        &mut self,
        expected: &Type,
        result_ty: &Type,
        span: crate::ast::Span,
        ctx: &'static str,
    ) {
        let _ = (span, ctx);
        let resolved = self.infer_ctx.shallow_resolve(result_ty);
        if !matches!(resolved, Type::TypeVar(_)) {
            let _ = self.infer_ctx.unify(expected, result_ty);
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

    pub(crate) fn importable_module_exists(&self, name: &str) -> bool {
        if let Some(dir) = &self.source_dir
            && dir.join(format!("{name}.jn")).exists()
        {
            return true;
        }
        let std_name = format!("{name}.jn");
        if let Ok(exe) = std::env::current_exe()
            && let Some(exe_dir) = exe.parent()
        {
            for base in [
                Some(exe_dir.to_path_buf()),
                exe_dir.parent().map(|p| p.to_path_buf()),
                exe_dir
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf()),
            ]
            .into_iter()
            .flatten()
            {
                if base.join("std").join(&std_name).exists() {
                    return true;
                }
            }
        }
        if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR")
            && std::path::PathBuf::from(manifest)
                .join("std")
                .join(&std_name)
                .exists()
        {
            return true;
        }
        false
    }

    pub fn unresolved_exported_generics(&self) -> Vec<String> {
        self.inferable_fns
            .keys()
            .filter(|n| {
                self.fn_schemes
                    .get(*n)
                    .is_some_and(|(q, _, _)| !q.is_empty())
                    && !self.instantiated_generics.contains(*n)
            })
            .map(|n| n.to_string())
            .collect()
    }

    pub(crate) fn display_fn_name(name: &str) -> &str {
        name.split("__G_").next().unwrap_or(name)
    }

    pub(crate) fn iter_borrow_conflict(&self, place: &place::Place) -> Option<&BorrowEntry> {
        self.iter_borrowed
            .iter()
            .rev()
            .find(|e| e.place.overlaps(place))
    }

    pub(crate) fn mark_place_moved_checked(
        &mut self,
        pl: place::Place,
        reason: MoveReason,
        at: crate::ast::Span,
    ) -> Result<(), String> {
        if self.suppress_move_marking == 0
            && let Some(entry) = self.iter_borrow_conflict(&pl)
        {
            return Err(format!(
                "{}: cannot move `{}` while {}; {}",
                at.loc(),
                pl.render(),
                entry.clause(&pl),
                entry.help(),
            ));
        }
        if let Some(defer_span) = self.defer_read_vars.get(&pl.root) {
            return Err(format!(
                "{}: cannot move `{}`: it is read by the `defer` registered at {}; \
                 move it before the defer is registered, or clone it into the defer",
                at.loc(),
                pl.render(),
                defer_span.loc(),
            ));
        }
        if !pl.is_root()
            && self.suppress_move_marking == 0
            && let Some(root_ty) = self.find_var_by_id(pl.root).map(|v| v.ty.clone())
            && matches!(self.infer_ctx.resolve(&root_ty), Type::View(_))
        {
            return Err(format!(
                "{}: cannot move `{}` out of `{}`: it is a view, a borrowed window \
                 that owns nothing; copy the data instead (`copy {}`)",
                at.loc(),
                pl.render(),
                pl.root_name,
                pl.render(),
            ));
        }
        if !pl.is_root()
            && self.suppress_move_marking == 0
            && let Some(root_ty) = self.find_var_by_id(pl.root).map(|v| v.ty.clone())
            && matches!(self.infer_ctx.resolve(&root_ty), Type::Frozen(_))
        {
            return Err(format!(
                "{}: cannot move `{}` out of `{}`: it is frozen, and a frozen value can \
                 never be written — not even by moving a part out; copy the part instead \
                 (`copy {}`)",
                at.loc(),
                pl.render(),
                pl.root_name,
                pl.render(),
            ));
        }
        if self.suppress_move_marking == 0 {
            let subject = self.payload_bind_subjects.get(&pl.root).cloned();
            self.moves.record(pl, reason);
            if let Some(subj_pl) = subject {
                self.mark_place_moved_checked(subj_pl, reason, at)?;
            }
        }
        Ok(())
    }

    pub(crate) fn mark_var_moved_checked(
        &mut self,
        id: DefId,
        name: Symbol,
        reason: MoveReason,
        at: crate::ast::Span,
    ) -> Result<(), String> {
        self.mark_place_moved_checked(place::Place::var(id, name), reason, at)
    }

    pub(crate) fn clear_all_moved_for(&mut self, parent: DefId) {
        self.moves.clear_root(parent);
    }

    pub(crate) fn snapshot_moved_fields(&self) -> MoveState {
        self.moves.clone()
    }

    pub(crate) fn restore_moved_fields(&mut self, snap: MoveState) {
        self.moves = snap;
    }

    pub(crate) fn merge_moved_fields_union(&mut self, branches: &[MoveState]) {
        for br in branches {
            self.moves.union(br);
        }
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
        if access_mod.is_none() && self.type_param_default_borrows(ty) {
            return Ok(Ownership::Borrowed);
        }
        self.ownership_with_mod(ty, access_mod)
    }

    fn type_param_default_borrows(&self, ty: &Type) -> bool {
        match ty {
            Type::String
            | Type::Vec(_)
            | Type::Map(_, _)
            | Type::Coroutine(_)
            | Type::Generator(_)
            | Type::Fn(_, _) => true,

            Type::Struct(_, _) | Type::Enum(_) | Type::Tuple(_) | Type::Array(_, _) => {
                self.needs_drop(ty)
            }

            Type::Alias(_, inner) | Type::Newtype(_, inner) | Type::Frozen(inner) => {
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
            Type::Newtype(_, inner) | Type::Alias(_, inner) | Type::Frozen(inner) => {
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
        if crate::typer::expr::views::type_contains_view(ty) {
            return Err(format!(
                "{}: a view cannot cross a task boundary ({}): it borrows memory owned \
                 by the sending frame; send the owning container or a copied slice",
                span.loc(),
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
        crate::builtin_methods::StrMethod::from_name(method).map(|m| m.ret_ty())
    }

    pub(crate) fn is_string_exclusive_method(method: &str) -> bool {
        crate::builtin_methods::StrMethod::from_name(method).is_some_and(|m| m.is_exclusive())
    }

    pub(crate) fn vec_method_ret_ty(method: &str, elem_ty: &Type) -> Option<Type> {
        crate::builtin_methods::VecMethod::from_name(method).map(|m| m.ret_ty(elem_ty))
    }

    pub(crate) fn map_method_ret_ty(method: &str, key_ty: &Type, val_ty: &Type) -> Option<Type> {
        crate::builtin_methods::MapMethod::from_name(method).map(|m| m.ret_ty(key_ty, val_ty))
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
