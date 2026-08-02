use crate::intern::Symbol;

use crate::ast::{self};
use crate::hir::{self};
use crate::types::Type;

use super::Typer;
use resolve::type_references_name;

mod block;
mod decl;
mod deferred;
mod display;
mod exhaust;
mod iter;
mod resolve;

impl Typer {
    pub fn lower_program(&mut self, prog: &ast::Program) -> Result<hir::Program, String> {
        if self.debug_types {
            tracing::debug!(target: "jinnc::type", "starting type inference and HIR lowering");
        }
        self.register_prelude_types();

        for d in &prog.decls {
            let name = match d {
                ast::Decl::Type(td) if td.type_params.is_empty() => Some(td.name),
                ast::Decl::Enum(ed) if ed.type_params.is_empty() => Some(ed.name),
                ast::Decl::ErrDef(ed) => Some(ed.name),
                ast::Decl::Actor(ad) => Some(ad.name),
                ast::Decl::Store(sd) => Some(sd.name),
                _ => None,
            };
            if let Some(name) = name {
                self.declared_type_names.insert(name);
            }
        }

        let mut alias_map: std::collections::HashMap<Symbol, Type> =
            std::collections::HashMap::new();
        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if self.is_generic_fn(f) => {
                    if !f.type_bounds.is_empty() {
                        self.generic_bounds.insert(f.name, f.type_bounds.clone());
                    }
                    self.generic_fns
                        .insert(f.name, self.normalize_generic_fn(f));
                }
                ast::Decl::Fn(f) => {
                    let has_untyped_params = f.params.iter().any(|p| p.ty.is_none());
                    let all_untyped =
                        !f.params.is_empty() && f.params.iter().all(|p| p.ty.is_none());
                    if has_untyped_params {
                        self.inferable_fns.insert(f.name, f.clone());
                    }
                    if all_untyped && !f.params.is_empty() {
                        let normalized = Self::normalize_inferable_fn(f);
                        self.generic_fns.insert(f.name, normalized);
                    }
                    self.declare_fn_sig(f);

                    if has_untyped_params
                        && let Some((_, ptys, ret)) = self.fns.get(&f.name).cloned()
                    {
                        let mut ftvs = std::collections::HashSet::new();
                        for pt in &ptys {
                            pt.free_type_vars(&mut ftvs);
                        }
                        ret.free_type_vars(&mut ftvs);
                        let roots: Vec<u32> = ftvs.into_iter().collect();
                        self.infer_ctx.mark_quantified(&roots);
                    }
                }
                ast::Decl::Type(td) if !td.type_params.is_empty() => {
                    self.generic_types.insert(td.name, td.clone());
                }
                ast::Decl::Type(td) => {
                    for m in &td.methods {
                        self.methods.entry(td.name).or_default().push(m.clone());
                    }
                    self.declare_type_def(td);
                    for m in &td.methods {
                        self.declare_method_sig_by_ptr(&td.name.as_str(), m);
                    }
                }
                ast::Decl::Enum(ed) if !ed.type_params.is_empty() => {
                    self.generic_enums.insert(ed.name, ed.clone());
                }
                ast::Decl::Enum(ed) => {
                    self.declare_enum_def(ed);
                }
                ast::Decl::Extern(ef) => {
                    self.declare_extern_sig(ef);
                }
                ast::Decl::Use(u) => {
                    let mod_name = u
                        .alias
                        .unwrap_or_else(|| u.path.last().cloned().unwrap_or_default());
                    if !mod_name.is_empty() {
                        self.modules.insert(mod_name);
                    }
                }
                ast::Decl::ErrDef(ed) => {
                    if ed.name.as_str() == "StoreError" {
                        self.store_error_def = None;
                    }
                    self.declare_err_def_sig(ed);
                }
                ast::Decl::Test(_) => {}
                ast::Decl::Actor(ad) => {
                    self.declare_actor_def(ad);
                }
                ast::Decl::Store(sd) => {
                    let is_simple = sd.decorators.contains(&ast::StoreDecorator::Simple);
                    let mut fields: Vec<(Symbol, Type)> = Vec::new();

                    if !is_simple {
                        fields.push(("sid".into(), Type::I64));
                        fields.push(("uuid".into(), Type::String));
                        fields.push(("hash".into(), Type::String));
                        fields.push(("created".into(), Type::I64));
                        fields.push(("updated".into(), Type::I64));
                        fields.push(("deleted".into(), Type::I64));
                    }
                    let mut relations: Vec<(Symbol, Symbol, bool)> = Vec::new();
                    for f in &sd.fields {
                        if f.is_relation {
                            let target = match &f.ty {
                                Some(Type::Struct(n, _)) => *n,
                                Some(Type::Row(n)) => *n,
                                Some(Type::Enum(n)) => *n,
                                Some(Type::Param(n)) => *n,
                                Some(Type::Alias(n, _)) => *n,
                                _ => f.name,
                            };
                            relations.push((f.name, target, f.is_has_many));
                            if !f.is_has_many {
                                fields.push((f.name, Type::I64));
                            }
                        } else {
                            fields.push((f.name, f.ty.clone().unwrap_or(Type::I64)));
                        }
                    }
                    self.structs.insert(
                        Symbol::intern(&format!("__store_{}", sd.name)),
                        fields.clone(),
                    );
                    self.store_schemas.insert(sd.name, fields);
                    self.store_relations.insert(sd.name, relations);
                    self.store_decorators.insert(sd.name, sd.decorators.clone());
                }
                ast::Decl::Trait(td) => {
                    self.declare_trait_def(td);
                }
                ast::Decl::Impl(_) => {}
                ast::Decl::Const(name, expr, _) => {
                    self.consts.insert(*name, expr.clone());
                }
                ast::Decl::Global(name, expr, span) => {
                    self.globals.insert(*name, (expr.clone(), *span));
                }
                ast::Decl::Supervisor(sup) => {
                    for child in &sup.children {
                        if !self.fns.contains_key(child) && !self.actors.contains_key(child) {
                            self.type_errors.push(format!(
                                "supervisor '{}': unknown child '{}'",
                                sup.name, child
                            ));
                        }
                    }

                    let sup_name = sup.name.as_str();
                    let start_name = Symbol::intern(&format!("{}_start", sup_name));
                    if !self.fns.contains_key(&start_name) {
                        let id = self.fresh_id();
                        self.fns.insert(start_name, (id, vec![], Type::I64));
                    }
                    let rc_name = Symbol::intern(&format!("{}_restart_count", sup_name));
                    if !self.fns.contains_key(&rc_name) {
                        let id = self.fresh_id();
                        self.fns.insert(rc_name, (id, vec![], Type::I64));
                    }
                }
                ast::Decl::TypeAlias(name, ty, _span) => {
                    if type_references_name(ty, *name) {
                        self.type_errors.push(format!(
                            "type alias '{}' is cyclic (references itself)",
                            name
                        ));
                    }
                    alias_map.insert(*name, ty.clone());
                }
                ast::Decl::Newtype(_, _, _) => {}
                ast::Decl::TopStmt(_) => {}
                ast::Decl::Migration(_) => {}
                ast::Decl::View(vd) => {
                    self.view_defs
                        .insert(vd.name, (vd.source, vd.clauses.clone()));
                }
            }
        }

        for name in alias_map.keys() {
            let mut visited = std::collections::HashSet::new();
            let mut cur = *name;
            while let Some(ty) = alias_map.get(&cur) {
                if !visited.insert(cur) {
                    let cycle: Vec<_> = visited.into_iter().collect();
                    self.type_errors.push(format!(
                        "type alias '{}' participates in a cycle: {}",
                        name,
                        Symbol::join_vec(&cycle, " -> ")
                    ));
                    break;
                }

                let mut next = None;
                for other in alias_map.keys() {
                    if other != &cur && type_references_name(ty, *other) {
                        next = Some(*other);
                        break;
                    }
                }
                match next {
                    Some(n) => cur = n,
                    None => break,
                }
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                self.declare_impl_block(ib)?;
            }
        }

        self.infer_ctx.set_trait_impls(self.trait_impls.clone());

        let actor_names: std::collections::HashSet<Symbol> = self.actors.keys().cloned().collect();
        if !actor_names.is_empty() {
            let fn_keys: Vec<Symbol> = self.fns.keys().cloned().collect();
            for k in fn_keys {
                if let Some((id, ptys, ret)) = self.fns.shift_remove(&k) {
                    let ptys: Vec<Type> = ptys
                        .into_iter()
                        .map(|t| Self::normalize_actor_refs(t, &actor_names))
                        .collect();
                    let ret = Self::normalize_actor_refs(ret, &actor_names);
                    self.fns.insert(k, (id, ptys, ret));
                }
            }

            let actor_keys: Vec<Symbol> = self.actors.keys().cloned().collect();
            for k in actor_keys {
                if let Some((id, fields, handlers)) = self.actors.shift_remove(&k) {
                    let handlers = handlers
                        .into_iter()
                        .map(|(hn, ptys, tag)| {
                            let ptys = ptys
                                .into_iter()
                                .map(|t| Self::normalize_actor_refs(t, &actor_names))
                                .collect();
                            (hn, ptys, tag)
                        })
                        .collect();
                    let fields = fields
                        .into_iter()
                        .map(|(fn_name, ft)| {
                            (fn_name, Self::normalize_actor_refs(ft, &actor_names))
                        })
                        .collect();
                    self.actors.insert(k, (id, fields, handlers));
                }
            }
        }

        {
            let fn_keys: Vec<Symbol> = self.fns.keys().cloned().collect();
            for k in &fn_keys {
                if let Some((id, ptys, ret)) = self.fns.get(k).cloned() {
                    let ptys: Vec<Type> = ptys
                        .iter()
                        .map(|t| self.monomorphize_named_annotation(t))
                        .collect();
                    let ret = self.monomorphize_named_annotation(&ret);
                    if let Some(slot) = self.fns.get_mut(k) {
                        *slot = (id, ptys, ret);
                    }
                }
            }
        }

        if self.debug_types {
            tracing::debug!(target: "jinnc::type", "running bidirectional parameter inference");
        }
        self.infer_param_types(prog);

        {
            let all_fns: Vec<&ast::Fn> = prog
                .decls
                .iter()
                .filter_map(|d| match d {
                    ast::Decl::Fn(f) => Some(f),
                    _ => None,
                })
                .collect();
            self.infer_consuming_params(&all_fns);
        }

        if self.debug_types {
            tracing::debug!(target: "jinnc::type", "lowering declarations to HIR");
        }
        let mut hir_fns = Vec::new();
        let mut hir_types = Vec::new();
        let mut hir_enums = Vec::new();
        let mut hir_externs = Vec::new();
        let mut hir_err_defs = Vec::new();
        if let Some(sed) = self.store_error_def.clone() {
            hir_err_defs.push(self.lower_err_def(&sed));
        }
        let mut hir_actors = Vec::new();
        let mut hir_stores = Vec::new();
        let mut test_fns: Vec<(String, String)> = Vec::new();

        let non_generic_fns: Vec<&ast::Fn> = prog
            .decls
            .iter()
            .filter_map(|d| {
                if let ast::Decl::Fn(f) = d
                    && !self.is_generic_fn(f)
                    && !(self.test_mode && f.name == "main")
                {
                    return Some(f);
                }
                None
            })
            .collect();

        let call_graph = super::scc::build_call_graph(&non_generic_fns);
        let sccs = super::scc::tarjan_scc(&call_graph);

        let fn_lookup: std::collections::HashMap<Symbol, &ast::Fn> =
            non_generic_fns.iter().map(|f| (f.name, *f)).collect();

        self.seed_inferred_fallibility(&non_generic_fns);

        super::caps::analyze(&non_generic_fns)?;

        let mut lowered_fn_names = std::collections::HashSet::new();
        for scc in &sccs {
            if scc.len() > 1 {
                let mut scc_fns = Vec::new();
                let mut scc_fn_names = Vec::new();
                for name in scc {
                    if let Some(f) = fn_lookup.get(name) {
                        let hfn = self.lower_fn_deferred(f).map_err(|e| {
                            if scc.len() > 1 {
                                let peers = scc
                                    .iter()
                                    .filter(|n| f.name != **n)
                                    .map(|n| n.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!("{e}\n  note: in mutually recursive group with: {peers}")
                            } else {
                                e
                            }
                        })?;
                        scc_fns.push((f.ret.is_none() && f.name != "main", f.span, hfn, f.name));
                        scc_fn_names.push(f.name);
                        lowered_fn_names.insert(*name);
                    }
                }
                for (needs_resolve, _span, hfn, fname) in &mut scc_fns {
                    if *needs_resolve && !self.inferable_fns.contains_key(&*fname) {
                        let _ = hfn;
                    }
                }
                for (_, _, hfn, fname) in &scc_fns {
                    if self.inferable_fns.contains_key(fname) {
                        self.build_fn_scheme(*fname, hfn);
                    }
                }
                for (_, _, hfn, fname) in scc_fns {
                    if self.fn_schemes.get(&fname).is_some_and(|s| !s.0.is_empty()) {
                        continue;
                    }
                    hir_fns.push(hfn);
                }
            } else {
                for name in scc {
                    if let Some(f) = fn_lookup.get(name) {
                        let hfn = self.lower_fn(f)?;
                        if self.inferable_fns.contains_key(&f.name) {
                            self.build_fn_scheme(f.name, &hfn);
                        }
                        if self
                            .fn_schemes
                            .get(&f.name)
                            .is_some_and(|s| !s.0.is_empty())
                        {
                            lowered_fn_names.insert(*name);
                            continue;
                        }
                        hir_fns.push(hfn);
                        lowered_fn_names.insert(*name);
                    }
                }
            }
        }

        for f in &non_generic_fns {
            if !lowered_fn_names.contains(&f.name) {
                let hfn = self.lower_fn(f)?;
                if self.inferable_fns.contains_key(&f.name) {
                    self.build_fn_scheme(f.name, &hfn);
                }
                if self
                    .fn_schemes
                    .get(&f.name)
                    .is_some_and(|s| !s.0.is_empty())
                {
                    continue;
                }
                hir_fns.push(hfn);
            }
        }

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(_) => {}
                ast::Decl::Type(td) if td.type_params.is_empty() => {
                    let htd = self.lower_type_def(td)?;
                    hir_types.push(htd);
                }
                ast::Decl::Enum(ed) if ed.type_params.is_empty() => {
                    let hed = self.lower_enum_def(ed);
                    hir_enums.push(hed);
                }
                ast::Decl::Extern(ef) => {
                    if !hir_externs
                        .iter()
                        .any(|e: &hir::ExternFn| e.name == ef.name)
                    {
                        let hef = self.lower_extern(ef);
                        hir_externs.push(hef);
                    }
                }
                ast::Decl::ErrDef(ed) => {
                    let hed = self.lower_err_def(ed);
                    hir_err_defs.push(hed);
                }
                ast::Decl::Test(tb) if self.test_mode => {
                    let fn_name = format!("__test_{}", test_fns.len());
                    let test_fn = self.lower_test_block(tb, &fn_name)?;
                    let test_id = test_fn.def_id;
                    self.fns
                        .insert(Symbol::intern(&fn_name), (test_id, vec![], Type::Void));
                    hir_fns.push(test_fn);
                    test_fns.push((tb.name.as_str(), fn_name));
                }
                _ => {}
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Actor(ad) = d {
                let ha = self.lower_actor_def(ad)?;
                hir_actors.push(ha);
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Store(sd) = d {
                let hs = self.lower_store_def(sd)?;
                hir_stores.push(hs);
            }
        }

        let mut hir_trait_impls = Vec::new();
        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                let hi = self.lower_impl_block(ib)?;
                hir_trait_impls.push(hi);
            }
        }

        for hfn in hir_fns.iter_mut() {
            hfn.ret = self.infer_ctx.resolve(&hfn.ret);
        }
        for ht in hir_types.iter_mut() {
            for m in ht.methods.iter_mut() {
                m.ret = self.infer_ctx.resolve(&m.ret);
            }
        }
        for hs in hir_stores.iter_mut() {
            for m in hs.methods.iter_mut() {
                m.ret = self.infer_ctx.resolve(&m.ret);
            }
        }
        for hi in hir_trait_impls.iter_mut() {
            for m in hi.methods.iter_mut() {
                m.ret = self.infer_ctx.resolve(&m.ret);
            }
        }

        if self.test_mode && !test_fns.is_empty() {
            let main_fn = self.build_test_runner(&test_fns);
            self.fns
                .insert("main".into(), (main_fn.def_id, vec![], Type::I32));
            hir_fns.push(main_fn);
        }

        hir_fns.append(&mut self.mono_fns);
        hir_enums.append(&mut self.mono_enums);
        hir_types.append(&mut self.mono_types);

        let mut hir_supervisors = Vec::new();
        for d in &prog.decls {
            if let ast::Decl::Supervisor(sup) = d {
                let strat = match sup.strategy {
                    ast::SupervisorStrategy::OneForOne => hir::SupervisorStrategy::OneForOne,
                    ast::SupervisorStrategy::OneForAll => hir::SupervisorStrategy::OneForAll,
                    ast::SupervisorStrategy::RestForOne => hir::SupervisorStrategy::RestForOne,
                };
                hir_supervisors.push(hir::SupervisorDef {
                    def_id: self.fresh_id(),
                    name: sup.name,
                    strategy: strat,
                    children: sup.children.clone(),
                    span: sup.span,
                });
            }
        }

        let mut program = hir::Program {
            fns: hir_fns,
            types: hir_types,
            enums: hir_enums,
            externs: hir_externs,
            err_defs: hir_err_defs,
            actors: hir_actors,
            stores: hir_stores,
            trait_impls: hir_trait_impls,
            supervisors: hir_supervisors,
            type_aliases: Vec::new(),
            newtypes: Vec::new(),
            migrations: Vec::new(),
            globals: Vec::new(),
            pkg_id: self.root_pkg_id,
            item_pkgs: std::collections::HashMap::new(),
        };

        let global_entries: Vec<_> = self.globals.clone().into_iter().collect();
        for (name, (ast_expr, span)) in global_entries {
            let hir_expr = self.lower_expr(&ast_expr)?;
            let ty = hir_expr.ty.clone();
            program.globals.push(hir::Global {
                name,
                init: hir_expr,
                ty,
                span,
            });
        }

        for d in &prog.decls {
            if let ast::Decl::Migration(m) = d {
                program.migrations.push(m.clone());
            }
        }
        self.resolve_deferred_methods();
        self.resolve_deferred_fields();
        self.resolve_trait_constrained_vars();
        if self.infer_ctx.is_strict() {
            let mut struct_field_errors = Vec::new();
            for (struct_name, field_name, ty, span) in &self.unannotated_struct_fields {
                let resolved = self.infer_ctx.shallow_resolve(ty);
                if let Type::TypeVar(v) = resolved {
                    let root = self.infer_ctx.find(v);
                    let constraint = self.infer_ctx.constraint(root);
                    if matches!(constraint, super::unify::TypeConstraint::None) {
                        struct_field_errors.push(format!(
                            "{}: struct `{}` field `{}` has no type annotation and was never constrained",
                            span.loc(), struct_name, field_name
                        ));
                    }
                }
            }
            if !struct_field_errors.is_empty() {
                let combined = struct_field_errors.join("\n");
                return Err(format!("strict type checking failed:\n{combined}"));
            }
        }
        self.resolve_all_types(&mut program);

        if !self.mono_fns.is_empty() {
            let mut new_fns: Vec<hir::Fn> = self.mono_fns.drain(..).collect();
            for f in &mut new_fns {
                self.resolve_fn(f);
            }
            program.fns.extend(new_fns);
        }
        self.auto_derive_display(&mut program);
        let default_warnings = self.infer_ctx.drain_default_warnings();

        let mut seen_warn = std::collections::HashSet::new();
        self.warnings.extend(
            default_warnings
                .into_iter()
                .filter(|w| seen_warn.insert(w.clone())),
        );

        let strict_errors = self.infer_ctx.drain_strict_errors();
        let unify_errors = self.infer_ctx.drain_unify_errors();
        let type_errors = std::mem::take(&mut self.type_errors);
        let mut seen = std::collections::HashSet::new();
        let mut unique_errors: Vec<&String> = Vec::new();
        for e in unify_errors
            .iter()
            .chain(type_errors.iter())
            .chain(strict_errors.iter())
        {
            if seen.insert(e) {
                unique_errors.push(e);
            }
        }
        if !unique_errors.is_empty() {
            let combined = unique_errors
                .iter()
                .map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!("type checking failed:\n{combined}"));
        }

        program.item_pkgs = {
            let names = program
                .fns
                .iter()
                .map(|f| &f.name)
                .chain(program.types.iter().map(|t| &t.name))
                .chain(program.enums.iter().map(|e| &e.name))
                .chain(program.externs.iter().map(|e| &e.name))
                .chain(program.err_defs.iter().map(|e| &e.name))
                .chain(program.actors.iter().map(|a| &a.name))
                .chain(program.stores.iter().map(|s| &s.name))
                .chain(program.supervisors.iter().map(|s| &s.name))
                .chain(program.globals.iter().map(|g| &g.name));
            crate::pkgid::build_item_pkgs(&self.dep_pkg_ids, names)
        };
        for w in &self.warnings {
            eprintln!("warning: {w}");
        }
        if self.debug_types {
            tracing::debug!(
                target: "jinnc::type",
                "complete: {} fns, {} types, {} enums",
                program.fns.len(),
                program.types.len(),
                program.enums.len()
            );
        }
        Ok(program)
    }
}
