use std::collections::HashMap;

use crate::ast::{self, Span};
use crate::hir::{self, DefId, Ownership};
use crate::types::Type;

use super::{Typer, VarInfo};

impl Typer {
    pub fn lower_program(&mut self, prog: &ast::Program) -> Result<hir::Program, String> {
        if self.debug_types {
            eprintln!("[type:pipeline] starting type inference and HIR lowering");
        }
        self.register_prelude_types();

        for d in &prog.decls {
            match d {
                ast::Decl::Fn(f) if Self::is_generic_fn(f) => {
                    if !f.type_bounds.is_empty() {
                        self.generic_bounds
                            .insert(f.name.clone(), f.type_bounds.clone());
                    }
                    self.generic_fns
                        .insert(f.name.clone(), Self::normalize_generic_fn(f));
                }
                ast::Decl::Fn(f) => {
                    let has_untyped_params = f.params.iter().any(|p| p.ty.is_none());
                    let all_untyped =
                        !f.params.is_empty() && f.params.iter().all(|p| p.ty.is_none());
                    if has_untyped_params {
                        self.inferable_fns.insert(f.name.clone(), f.clone());
                    }
                    if all_untyped && !f.params.is_empty() {
                        let normalized = Self::normalize_inferable_fn(f);
                        self.generic_fns.insert(f.name.clone(), normalized);
                    }
                    self.declare_fn_sig(f);
                }
                ast::Decl::Type(td) if !td.type_params.is_empty() => {
                    self.generic_types.insert(td.name.clone(), td.clone());
                }
                ast::Decl::Type(td) => {
                    for m in &td.methods {
                        self.methods
                            .entry(td.name.clone())
                            .or_default()
                            .push(m.clone());
                    }
                    self.declare_type_def(td);
                    for m in &td.methods {
                        self.declare_method_sig_by_ptr(&td.name, m);
                    }
                }
                ast::Decl::Enum(ed) if !ed.type_params.is_empty() => {
                    self.generic_enums.insert(ed.name.clone(), ed.clone());
                }
                ast::Decl::Enum(ed) => {
                    self.declare_enum_def(ed);
                }
                ast::Decl::Extern(ef) => {
                    self.declare_extern_sig(ef);
                }
                ast::Decl::Use(_) => {}
                ast::Decl::ErrDef(ed) => {
                    self.declare_err_def_sig(ed);
                }
                ast::Decl::Test(_) => {}
                ast::Decl::Actor(ad) => {
                    self.declare_actor_def(ad);
                }
                ast::Decl::Store(sd) => {
                    let fields: Vec<(String, Type)> = sd
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), f.ty.clone().unwrap_or(Type::I64)))
                        .collect();
                    self.structs
                        .insert(format!("__store_{}", sd.name), fields.clone());
                    self.store_schemas.insert(sd.name.clone(), fields);
                }
                ast::Decl::Trait(td) => {
                    self.declare_trait_def(td);
                }
                ast::Decl::Impl(_) => {}
                ast::Decl::Const(name, expr, _) => {
                    self.consts.insert(name.clone(), expr.clone());
                }
                ast::Decl::Supervisor(_) => {}
                ast::Decl::TypeAlias(_, _, _) => {}
                ast::Decl::Newtype(_, _, _) => {}
            }
        }

        for d in &prog.decls {
            if let ast::Decl::Impl(ib) = d {
                self.declare_impl_block(ib)?;
            }
        }

        // Sync trait_impls to InferCtx for constraint enforcement during unification
        self.infer_ctx.set_trait_impls(self.trait_impls.clone());

        if self.debug_types {
            eprintln!("[type:pipeline] running bidirectional parameter inference");
        }
        self.infer_param_types(prog);

        if self.debug_types {
            eprintln!("[type:pipeline] lowering declarations to HIR");
        }
        let mut hir_fns = Vec::new();
        let mut hir_types = Vec::new();
        let mut hir_enums = Vec::new();
        let mut hir_externs = Vec::new();
        let mut hir_err_defs = Vec::new();
        let mut hir_actors = Vec::new();
        let mut hir_stores = Vec::new();
        let mut test_fns: Vec<(String, String)> = Vec::new();

        let non_generic_fns: Vec<&ast::Fn> = prog
            .decls
            .iter()
            .filter_map(|d| {
                if let ast::Decl::Fn(f) = d {
                    if !Self::is_generic_fn(f) && !(self.test_mode && f.name == "main") {
                        return Some(f);
                    }
                }
                None
            })
            .collect();

        let call_graph = super::scc::build_call_graph(&non_generic_fns);
        let sccs = super::scc::tarjan_scc(&call_graph);

        let fn_lookup: std::collections::HashMap<&str, &ast::Fn> = non_generic_fns
            .iter()
            .map(|f| (f.name.as_str(), *f))
            .collect();

        let mut lowered_fn_names = std::collections::HashSet::new();
        for scc in &sccs {
            if scc.len() > 1 {
                let mut scc_fns = Vec::new();
                let mut scc_fn_names = Vec::new();
                for name in scc {
                    if let Some(f) = fn_lookup.get(name.as_str()) {
                        let hfn = self.lower_fn_deferred(f).map_err(|e| {
                            if scc.len() > 1 {
                                let peers = scc
                                    .iter()
                                    .filter(|n| n.as_str() != f.name)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                format!("{e}\n  note: in mutually recursive group with: {peers}")
                            } else {
                                e
                            }
                        })?;
                        scc_fns.push((
                            f.ret.is_none() && f.name != "main",
                            f.span,
                            hfn,
                            f.name.clone(),
                        ));
                        scc_fn_names.push(f.name.clone());
                        lowered_fn_names.insert(name.clone());
                    }
                }
                for (needs_resolve, _span, hfn, fname) in &mut scc_fns {
                    if *needs_resolve && !self.inferable_fns.contains_key(fname.as_str()) {
                        hfn.ret = self.infer_ctx.resolve(&hfn.ret);
                    }
                }
                for (_, _, hfn, fname) in &scc_fns {
                    if self.inferable_fns.contains_key(fname) {
                        self.build_fn_scheme(fname, hfn);
                    }
                }
                for (_, _, hfn, fname) in scc_fns {
                    if self
                        .fn_schemes
                        .get(&fname)
                        .map_or(false, |s| !s.0.is_empty())
                    {
                        continue;
                    }
                    hir_fns.push(hfn);
                }
            } else {
                for name in scc {
                    if let Some(f) = fn_lookup.get(name.as_str()) {
                        let hfn = self.lower_fn(f)?;
                        if self.inferable_fns.contains_key(&f.name) {
                            self.build_fn_scheme(&f.name, &hfn);
                        }
                        if self
                            .fn_schemes
                            .get(&f.name)
                            .map_or(false, |s| !s.0.is_empty())
                        {
                            lowered_fn_names.insert(name.clone());
                            continue;
                        }
                        hir_fns.push(hfn);
                        lowered_fn_names.insert(name.clone());
                    }
                }
            }
        }

        for f in &non_generic_fns {
            if !lowered_fn_names.contains(&f.name) {
                let hfn = self.lower_fn(f)?;
                if self.inferable_fns.contains_key(&f.name) {
                    self.build_fn_scheme(&f.name, &hfn);
                }
                if self
                    .fn_schemes
                    .get(&f.name)
                    .map_or(false, |s| !s.0.is_empty())
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
                    let hef = self.lower_extern(ef);
                    hir_externs.push(hef);
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
                        .insert(fn_name.clone(), (test_id, vec![], Type::Void));
                    hir_fns.push(test_fn);
                    test_fns.push((tb.name.clone(), fn_name));
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

        if self.test_mode && !test_fns.is_empty() {
            let main_fn = self.build_test_runner(&test_fns);
            self.fns
                .insert("main".into(), (main_fn.def_id, vec![], Type::I32));
            hir_fns.push(main_fn);
        }

        hir_fns.extend(self.mono_fns.drain(..));
        hir_enums.extend(self.mono_enums.drain(..));
        hir_types.extend(self.mono_types.drain(..));

        let mut program = hir::Program {
            fns: hir_fns,
            types: hir_types,
            enums: hir_enums,
            externs: hir_externs,
            err_defs: hir_err_defs,
            actors: hir_actors,
            stores: hir_stores,
            trait_impls: hir_trait_impls,
            supervisors: Vec::new(),
            type_aliases: Vec::new(),
            newtypes: Vec::new(),
        };
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
                            "line {}:{}: struct `{}` field `{}` has no type annotation and was never constrained",
                            span.line, span.col, struct_name, field_name
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
        self.auto_derive_display(&mut program);
        let default_warnings = self.infer_ctx.drain_default_warnings();
        self.warnings.extend(default_warnings);
        let strict_errors = self.infer_ctx.drain_strict_errors();
        if !strict_errors.is_empty() {
            let mut seen = std::collections::HashSet::new();
            let mut unique_errors: Vec<&String> = Vec::new();
            for e in &strict_errors {
                if seen.insert(e) {
                    unique_errors.push(e);
                }
            }
            if !self.type_errors.is_empty() {
                let mut type_seen = std::collections::HashSet::new();
                for te in &self.type_errors {
                    if type_seen.insert(te) {
                        unique_errors.push(te);
                    }
                }
            }
            let combined = unique_errors
                .iter()
                .map(|e| e.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(format!("strict type checking failed:\n{combined}"));
        }
        for w in &self.warnings {
            eprintln!("warning: {w}");
        }
        if self.debug_types {
            eprintln!(
                "[type:pipeline] complete: {} fns, {} types, {} enums",
                program.fns.len(),
                program.types.len(),
                program.enums.len()
            );
        }
        Ok(program)
    }

    fn resolve_deferred_methods(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_methods);
        for dm in &deferred {
            let recv_ty = self.infer_ctx.shallow_resolve(&dm.receiver_ty);
            match &recv_ty {
                Type::Vec(elem_ty) => {
                    let elem = elem_ty.as_ref().clone();
                    if dm.method == "push" {
                        if let Some(arg_ty) = dm.arg_tys.first() {
                            let _ = self
                                .infer_ctx
                                .unify_at(&elem, arg_ty, dm.span, "vec.push arg");
                        }
                    } else if dm.method == "set" {
                        if let Some(idx_ty) = dm.arg_tys.first() {
                            let _ = self.infer_ctx.unify_at(
                                &Type::I64,
                                idx_ty,
                                dm.span,
                                "vec.set index",
                            );
                        }
                        if let Some(val_ty) = dm.arg_tys.get(1) {
                            let _ =
                                self.infer_ctx
                                    .unify_at(&elem, val_ty, dm.span, "vec.set value");
                        }
                    }
                    let actual_ret = match Self::vec_method_ret_ty(&dm.method, &elem) {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred vec method return",
                    );
                }
                Type::Map(key_ty, val_ty) => {
                    let key = key_ty.as_ref().clone();
                    let val = val_ty.as_ref().clone();
                    match dm.method.as_str() {
                        "set" => {
                            if let Some(k) = dm.arg_tys.first() {
                                let _ = self.infer_ctx.unify_at(&key, k, dm.span, "map.set key");
                            }
                            if let Some(v) = dm.arg_tys.get(1) {
                                let _ = self.infer_ctx.unify_at(&val, v, dm.span, "map.set value");
                            }
                        }
                        "get" | "has" => {
                            if let Some(k) = dm.arg_tys.first() {
                                let reason = if dm.method == "get" {
                                    "map.get key"
                                } else {
                                    "map.has key"
                                };
                                let _ = self.infer_ctx.unify_at(&key, k, dm.span, reason);
                            }
                        }
                        _ => {}
                    }
                    let actual_ret = match Self::map_method_ret_ty(&dm.method, &key, &val) {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred map method return",
                    );
                }
                Type::String => {
                    let actual_ret = match Self::string_method_ret_ty(&dm.method) {
                        Some(ty) => ty,
                        None => continue,
                    };
                    let _ = self.infer_ctx.unify_at(
                        &dm.ret_ty,
                        &actual_ret,
                        dm.span,
                        "deferred string method return",
                    );
                }
                Type::Struct(type_name, _) => {
                    let method_name = format!("{}_{}", type_name, dm.method);
                    if let Some((_, param_tys, ret)) = self.fns.get(&method_name).cloned() {
                        for (i, arg_ty) in dm.arg_tys.iter().enumerate() {
                            if let Some(expected) = param_tys.get(i + 1) {
                                let _ = self.infer_ctx.unify_at(
                                    expected,
                                    arg_ty,
                                    dm.span,
                                    "deferred struct method arg",
                                );
                            }
                        }
                        let _ = self.infer_ctx.unify_at(
                            &dm.ret_ty,
                            &ret,
                            dm.span,
                            "deferred struct method return",
                        );
                    }
                }
                Type::Coroutine(yield_ty) => {
                    if dm.method == "next" {
                        let _ = self.infer_ctx.unify_at(
                            &dm.ret_ty,
                            yield_ty,
                            dm.span,
                            "deferred coroutine.next return",
                        );
                    }
                }
                _ => {
                    if matches!(recv_ty, Type::TypeVar(_)) {
                        // If the method is exclusive to String, resolve immediately
                        if Self::is_string_exclusive_method(&dm.method) {
                            let _ = self.infer_ctx.unify_at(
                                &recv_ty,
                                &Type::String,
                                dm.span,
                                "deferred string-exclusive method implies String",
                            );
                            if let Some(actual_ret) = Self::string_method_ret_ty(&dm.method) {
                                let _ = self.infer_ctx.unify_at(
                                    &dm.ret_ty,
                                    &actual_ret,
                                    dm.span,
                                    "deferred string method return",
                                );
                            }
                            continue;
                        }

                        let suffix = format!("_{}", dm.method);
                        let mut candidates: Vec<(String, Vec<Type>, Type)> = self
                            .fns
                            .iter()
                            .filter(|(name, _)| name.ends_with(&suffix))
                            .map(|(name, (_, ptys, ret))| {
                                let type_name = name[..name.len() - suffix.len()].to_string();
                                (type_name, ptys.clone(), ret.clone())
                            })
                            .filter(|(type_name, _, _)| self.structs.contains_key(type_name))
                            .collect();

                        if let Type::TypeVar(v) = recv_ty {
                            let constraint = self.infer_ctx.constraint(v);
                            if let super::unify::TypeConstraint::Trait(ref required_traits) =
                                constraint
                            {
                                let narrowed: Vec<(String, Vec<Type>, Type)> = candidates
                                    .iter()
                                    .filter(|(type_name, _, _)| {
                                        self.trait_impls.get(type_name).map_or(false, |impls| {
                                            required_traits.iter().all(|rt| impls.contains(rt))
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                if !narrowed.is_empty() {
                                    candidates = narrowed;
                                }
                            }
                        }

                        if candidates.len() > 1 {
                            let defining_traits: Vec<&String> = self
                                .traits
                                .iter()
                                .filter(|(_, sigs)| sigs.iter().any(|s| s.name == dm.method))
                                .map(|(tname, _)| tname)
                                .collect();
                            if !defining_traits.is_empty() {
                                let narrowed: Vec<(String, Vec<Type>, Type)> = candidates
                                    .iter()
                                    .filter(|(type_name, _, _)| {
                                        self.trait_impls.get(type_name).map_or(false, |impls| {
                                            impls.iter().any(|i| defining_traits.contains(&i))
                                        })
                                    })
                                    .cloned()
                                    .collect();
                                if !narrowed.is_empty() {
                                    candidates = narrowed;
                                }
                            }
                        }

                        if candidates.len() == 1 {
                            let (type_name, param_tys, ret) = &candidates[0];
                            let struct_ty = Type::Struct(type_name.clone(), vec![]);
                            let _ = self.infer_ctx.unify_at(
                                &recv_ty,
                                &struct_ty,
                                dm.span,
                                "deferred method candidate",
                            );
                            for (i, arg_ty) in dm.arg_tys.iter().enumerate() {
                                if let Some(expected) = param_tys.get(i + 1) {
                                    let _ = self.infer_ctx.unify_at(
                                        expected,
                                        arg_ty,
                                        dm.span,
                                        "deferred method arg",
                                    );
                                }
                            }
                            let _ = self.infer_ctx.unify_at(
                                &dm.ret_ty,
                                &ret,
                                dm.span,
                                "deferred method return",
                            );
                        }
                    }
                }
            }
        }
    }

    fn resolve_deferred_fields(&mut self) {
        let deferred = std::mem::take(&mut self.deferred_fields);

        let mut by_receiver: HashMap<u32, Vec<&super::DeferredField>> = HashMap::new();
        let mut resolved_concrete: Vec<&super::DeferredField> = Vec::new();
        for df in &deferred {
            let recv_ty = self.infer_ctx.shallow_resolve(&df.receiver_ty);
            match recv_ty {
                Type::TypeVar(v) => {
                    let root = self.infer_ctx.find(v);
                    by_receiver.entry(root).or_default().push(df);
                }
                _ => resolved_concrete.push(df),
            }
        }

        for df in resolved_concrete {
            let recv_ty = self.infer_ctx.shallow_resolve(&df.receiver_ty);
            if let Type::Struct(ref name, _) = recv_ty {
                if let Some(fields) = self.structs.get(name) {
                    if let Some((_, fty)) = fields.iter().find(|(n, _)| n == &df.field_name) {
                        let fty = fty.clone();
                        let _ = self.infer_ctx.unify_at(
                            &df.field_ty,
                            &fty,
                            df.span,
                            "deferred field access",
                        );
                    }
                }
            } else if matches!(recv_ty, Type::String) && df.field_name == "length" {
                let _ = self.infer_ctx.unify_at(
                    &df.field_ty,
                    &Type::I64,
                    df.span,
                    "deferred string.length",
                );
            }
        }

        for (_root, fields) in by_receiver {
            let required_fields: Vec<(&str, &Type)> = fields
                .iter()
                .map(|df| (df.field_name.as_str(), &df.field_ty))
                .collect();

            let extra_constraints: Vec<(String, Type)> = self
                .field_constraints
                .get(&_root)
                .cloned()
                .unwrap_or_default();

            let candidates: Vec<String> = self
                .structs
                .iter()
                .filter(|(_, struct_fields)| {
                    required_fields
                        .iter()
                        .all(|(req, _)| struct_fields.iter().any(|(n, _)| n == req))
                        && extra_constraints
                            .iter()
                            .all(|(req, _)| struct_fields.iter().any(|(n, _)| n == req))
                })
                .map(|(name, _)| name.clone())
                .collect();

            if candidates.len() == 1 {
                let sname = &candidates[0];
                let struct_ty = Type::Struct(sname.clone(), vec![]);
                let span = fields[0].span;
                let _ = self.infer_ctx.unify_at(
                    &fields[0].receiver_ty,
                    &struct_ty,
                    span,
                    "deferred field constraints imply struct type",
                );
                if let Some(struct_fields) = self.structs.get(sname).cloned() {
                    for df in &fields {
                        if let Some((_, fty)) =
                            struct_fields.iter().find(|(n, _)| n == &df.field_name)
                        {
                            let _ = self.infer_ctx.unify_at(
                                &df.field_ty,
                                fty,
                                df.span,
                                "deferred field access",
                            );
                        }
                    }
                }
            }
        }
    }

    fn resolve_trait_constrained_vars(&mut self) {
        let n = self.infer_ctx.num_vars();
        for v in 0..n {
            let root = self.infer_ctx.find(v);
            if root != v {
                continue;
            }
            let resolved = self.infer_ctx.shallow_resolve(&Type::TypeVar(root));
            if !matches!(resolved, Type::TypeVar(_)) {
                continue;
            }
            let constraint = self.infer_ctx.constraint(root);
            if let super::unify::TypeConstraint::Trait(ref required_traits) = constraint {
                if required_traits.is_empty() {
                    continue;
                }
                let mut candidates: Vec<String> = Vec::new();
                for (type_name, impl_traits) in &self.trait_impls {
                    if required_traits.iter().all(|rt| impl_traits.contains(rt)) {
                        candidates.push(type_name.clone());
                    }
                }
                if candidates.len() == 1 {
                    let ty = match candidates[0].as_str() {
                        "i8" => Type::I8,
                        "i16" => Type::I16,
                        "i32" => Type::I32,
                        "i64" => Type::I64,
                        "u8" => Type::U8,
                        "u16" => Type::U16,
                        "u32" => Type::U32,
                        "u64" => Type::U64,
                        "f32" => Type::F32,
                        "f64" => Type::F64,
                        "bool" => Type::Bool,
                        "String" => Type::String,
                        name => Type::Struct(name.to_string(), vec![]),
                    };
                    let _ = self.infer_ctx.unify(&Type::TypeVar(root), &ty);
                }
            }
        }
    }

    fn reclassify_method_call(&mut self, expr: &mut hir::Expr) {
        let (recv_ty, method) = match &expr.kind {
            hir::ExprKind::StringMethod(recv, m, _) => (recv.ty.clone(), m.clone()),
            _ => return,
        };
        match &recv_ty {
            Type::Vec(_) => {
                if let hir::ExprKind::StringMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::VecMethod(recv, method, args);
                }
            }
            Type::Map(_, _) => {
                if let hir::ExprKind::StringMethod(recv, method, args) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::MapMethod(recv, method, args);
                }
            }
            Type::Struct(type_name, _) => {
                let method_name = format!("{}_{}", type_name, method);
                if self.fns.contains_key(&method_name) {
                    if let hir::ExprKind::StringMethod(recv, _method_str, args) =
                        std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                    {
                        expr.kind = hir::ExprKind::Method(recv, method_name, method, args);
                    }
                }
            }
            Type::Coroutine(_) if method == "next" => {
                if let hir::ExprKind::StringMethod(recv, _, _) =
                    std::mem::replace(&mut expr.kind, hir::ExprKind::Void)
                {
                    expr.kind = hir::ExprKind::CoroutineNext(recv);
                }
            }
            _ => {}
        }
    }

    fn resolve_all_types(&mut self, prog: &mut hir::Program) {
        for f in &mut prog.fns {
            self.resolve_fn(f);
        }
        for td in &mut prog.types {
            for field in &mut td.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
                if let Some(def) = &mut field.default {
                    self.resolve_expr(def);
                }
            }
            for m in &mut td.methods {
                self.resolve_fn(m);
            }
            if let Some(sfields) = self.structs.get_mut(&td.name) {
                for (i, field) in td.fields.iter().enumerate() {
                    if let Some(sf) = sfields.get_mut(i) {
                        sf.1 = field.ty.clone();
                    }
                }
            }
        }
        for ed in &mut prog.enums {
            for v in &mut ed.variants {
                for vf in &mut v.fields {
                    vf.ty = self.infer_ctx.resolve(&vf.ty);
                }
            }
        }
        for ef in &mut prog.externs {
            ef.ret = self.infer_ctx.resolve(&ef.ret);
            for (_, ty) in &mut ef.params {
                *ty = self.infer_ctx.resolve(ty);
            }
        }
        for errdef in &mut prog.err_defs {
            for v in &mut errdef.variants {
                for ft in &mut v.fields {
                    *ft = self.infer_ctx.resolve(ft);
                }
            }
        }
        for ad in &mut prog.actors {
            for field in &mut ad.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
                if let Some(def) = &mut field.default {
                    self.resolve_expr(def);
                }
            }
            for h in &mut ad.handlers {
                for p in &mut h.params {
                    p.ty = self.infer_ctx.resolve(&p.ty);
                }
                self.resolve_block(&mut h.body);
            }
        }
        for sd in &mut prog.stores {
            for field in &mut sd.fields {
                field.ty = self.infer_ctx.resolve(&field.ty);
            }
        }
        for ti in &mut prog.trait_impls {
            for m in &mut ti.methods {
                self.resolve_fn(m);
            }
        }
    }

    /// Auto-derive Display for structs that are passed to log/to_string
    /// but don't have an explicit display method.
    fn auto_derive_display(&mut self, prog: &mut hir::Program) {
        // Collect struct names that need Display
        let mut needs_display: std::collections::HashSet<String> = std::collections::HashSet::new();
        for f in &prog.fns {
            Self::collect_display_usage(&f.body, &mut needs_display);
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                Self::collect_display_usage(&m.body, &mut needs_display);
            }
        }
        // Remove structs that already have a display method
        needs_display
            .retain(|name| !self.fns.contains_key(&format!("{name}_display")));

        // Generate display methods for structs
        for type_name in &needs_display {
            if let Some(fields) = self.structs.get(type_name).cloned() {
                let method_name = format!("{type_name}_display");
                let self_id = self.fresh_id();
                let self_ty = Type::Struct(type_name.clone(), vec![]);
                let span = crate::ast::Span::dummy();

                // Build a single nested concat expression:
                // "TypeName(" + field1_label + to_string(field1) + ... + ")"
                let mk_str = |s: String| hir::Expr {
                    kind: hir::ExprKind::Str(s),
                    ty: Type::String,
                    span,
                };
                let concat = |a: hir::Expr, b: hir::Expr| hir::Expr {
                    kind: hir::ExprKind::BinOp(
                        Box::new(a),
                        crate::ast::BinOp::Add,
                        Box::new(b),
                    ),
                    ty: Type::String,
                    span,
                };

                let mut result = mk_str(format!("{type_name}("));

                for (i, (fname, fty)) in fields.iter().enumerate() {
                    let label = if i == 0 {
                        format!("{fname}: ")
                    } else {
                        format!(", {fname}: ")
                    };
                    result = concat(result, mk_str(label));

                    let field_val = hir::Expr {
                        kind: hir::ExprKind::Field(
                            Box::new(hir::Expr {
                                kind: hir::ExprKind::Var(self_id, "__self".into()),
                                ty: self_ty.clone(),
                                span,
                            }),
                            fname.clone(),
                            i,
                        ),
                        ty: fty.clone(),
                        span,
                    };
                    let to_string = hir::Expr {
                        kind: hir::ExprKind::Builtin(
                            hir::BuiltinFn::ToString,
                            vec![field_val],
                        ),
                        ty: Type::String,
                        span,
                    };
                    result = concat(result, to_string);
                }

                result = concat(result, mk_str(")".into()));
                let body = vec![hir::Stmt::Expr(result)];

                let hir_fn = hir::Fn {
                    def_id: self.fresh_id(),
                    name: method_name.clone(),
                    params: vec![hir::Param {
                        def_id: self_id,
                        name: "__self".into(),
                        ty: self_ty.clone(),
                        ownership: hir::Ownership::Owned,
                        span,
                    }],
                    ret: Type::String,
                    body,
                    span,
                    generic_origin: None,
                    is_generator: false,
                };
                self.fns.insert(
                    method_name,
                    (hir_fn.def_id, vec![self_ty], Type::String),
                );
                prog.fns.push(hir_fn);
            }
        }
    }

    fn collect_display_usage(block: &[hir::Stmt], needs: &mut std::collections::HashSet<String>) {
        for stmt in block {
            Self::collect_display_usage_stmt(stmt, needs);
        }
    }

    fn collect_display_usage_stmt(
        stmt: &hir::Stmt,
        needs: &mut std::collections::HashSet<String>,
    ) {
        match stmt {
            hir::Stmt::Bind(b) => Self::collect_display_usage_expr(&b.value, needs),
            hir::Stmt::TupleBind(_, e, _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::Assign(l, r, _) => {
                Self::collect_display_usage_expr(l, needs);
                Self::collect_display_usage_expr(r, needs);
            }
            hir::Stmt::Expr(e) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::If(i) => {
                Self::collect_display_usage_expr(&i.cond, needs);
                Self::collect_display_usage(&i.then, needs);
                for (c, b) in &i.elifs {
                    Self::collect_display_usage_expr(c, needs);
                    Self::collect_display_usage(b, needs);
                }
                if let Some(b) = &i.els {
                    Self::collect_display_usage(b, needs);
                }
            }
            hir::Stmt::While(w) => {
                Self::collect_display_usage_expr(&w.cond, needs);
                Self::collect_display_usage(&w.body, needs);
            }
            hir::Stmt::For(f) => {
                Self::collect_display_usage_expr(&f.iter, needs);
                Self::collect_display_usage(&f.body, needs);
            }
            hir::Stmt::Loop(l) => Self::collect_display_usage(&l.body, needs),
            hir::Stmt::Match(m) => {
                Self::collect_display_usage_expr(&m.subject, needs);
                for a in &m.arms {
                    Self::collect_display_usage(&a.body, needs);
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::Break(Some(e), _) => Self::collect_display_usage_expr(e, needs),
            hir::Stmt::ErrReturn(e, _, _) => Self::collect_display_usage_expr(e, needs),
            _ => {}
        }
    }

    fn collect_display_usage_expr(
        expr: &hir::Expr,
        needs: &mut std::collections::HashSet<String>,
    ) {
        match &expr.kind {
            hir::ExprKind::Builtin(hir::BuiltinFn::Log, args) => {
                for a in args {
                    if let Type::Struct(name, _) = &a.ty {
                        needs.insert(name.clone());
                    }
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Builtin(hir::BuiltinFn::ToString, args) => {
                for a in args {
                    if let Type::Struct(name, _) = &a.ty {
                        needs.insert(name.clone());
                    }
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_display_usage_expr(l, needs);
                Self::collect_display_usage_expr(r, needs);
            }
            hir::ExprKind::Call(_, _, args)
            | hir::ExprKind::Builtin(_, args)
            | hir::ExprKind::VecNew(args)
            | hir::ExprKind::Array(args)
            | hir::ExprKind::Tuple(args) => {
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                Self::collect_display_usage_expr(callee, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Method(recv, _, _, args)
            | hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::VecMethod(recv, _, args)
            | hir::ExprKind::MapMethod(recv, _, args)
            | hir::ExprKind::SetMethod(recv, _, args)
            | hir::ExprKind::PQMethod(recv, _, args)
            | hir::ExprKind::DynDispatch(recv, _, _, args) => {
                Self::collect_display_usage_expr(recv, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Pipe(l, _, _, args) => {
                Self::collect_display_usage_expr(l, needs);
                for a in args {
                    Self::collect_display_usage_expr(a, needs);
                }
            }
            hir::ExprKind::Lambda(_, body) | hir::ExprKind::Block(body) => {
                Self::collect_display_usage(body, needs);
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_display_usage_expr(&i.cond, needs);
                Self::collect_display_usage(&i.then, needs);
                for (c, b) in &i.elifs {
                    Self::collect_display_usage_expr(c, needs);
                    Self::collect_display_usage(b, needs);
                }
                if let Some(b) = &i.els {
                    Self::collect_display_usage(b, needs);
                }
            }
            _ => {}
        }
    }

    fn resolve_fn(&mut self, f: &mut hir::Fn) {
        f.ret = self.infer_ctx.resolve(&f.ret);
        for p in &mut f.params {
            p.ty = self.infer_ctx.resolve(&p.ty);
        }
        self.resolve_block(&mut f.body);
    }

    fn resolve_block(&mut self, block: &mut hir::Block) {
        for stmt in block {
            self.resolve_stmt(stmt);
        }
    }

    fn resolve_stmt(&mut self, stmt: &mut hir::Stmt) {
        match stmt {
            hir::Stmt::Bind(b) => {
                b.ty = self.infer_ctx.resolve(&b.ty);
                self.resolve_expr(&mut b.value);
            }
            hir::Stmt::TupleBind(bindings, expr, _) => {
                for (_, _, ty) in bindings {
                    *ty = self.infer_ctx.resolve(ty);
                }
                self.resolve_expr(expr);
            }
            hir::Stmt::Assign(lhs, rhs, _) => {
                self.resolve_expr(lhs);
                self.resolve_expr(rhs);
            }
            hir::Stmt::Expr(e) => self.resolve_expr(e),
            hir::Stmt::If(if_stmt) => {
                self.resolve_expr(&mut if_stmt.cond);
                self.resolve_block(&mut if_stmt.then);
                for (cond, block) in &mut if_stmt.elifs {
                    self.resolve_expr(cond);
                    self.resolve_block(block);
                }
                if let Some(els) = &mut if_stmt.els {
                    self.resolve_block(els);
                }
            }
            hir::Stmt::While(w) => {
                self.resolve_expr(&mut w.cond);
                self.resolve_block(&mut w.body);
            }
            hir::Stmt::For(f) => {
                f.bind_ty = self.infer_ctx.resolve(&f.bind_ty);
                self.resolve_expr(&mut f.iter);
                if let Some(end) = &mut f.end {
                    self.resolve_expr(end);
                }
                if let Some(step) = &mut f.step {
                    self.resolve_expr(step);
                }
                self.resolve_block(&mut f.body);
            }
            hir::Stmt::Loop(l) => {
                self.resolve_block(&mut l.body);
            }
            hir::Stmt::Ret(expr, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
                if let Some(e) = expr {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::Break(expr, _) => {
                if let Some(e) = expr {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::Continue(_) => {}
            hir::Stmt::Match(m) => {
                self.resolve_expr(&mut m.subject);
                m.ty = self.infer_ctx.resolve(&m.ty);
                for arm in &mut m.arms {
                    self.resolve_pat(&mut arm.pat);
                    if let Some(g) = &mut arm.guard {
                        self.resolve_expr(g);
                    }
                    self.resolve_block(&mut arm.body);
                }
            }
            hir::Stmt::Asm(_) => {}
            hir::Stmt::Drop(_, _, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::Stmt::ErrReturn(e, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
                self.resolve_expr(e);
            }
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.resolve_expr(e);
                }
            }
            hir::Stmt::StoreDelete(_, filter, _) => {
                self.resolve_filter(filter);
            }
            hir::Stmt::StoreSet(_, updates, filter, _) => {
                for (_, e) in updates {
                    self.resolve_expr(e);
                }
                self.resolve_filter(filter);
            }
            hir::Stmt::Transaction(block, _) => {
                self.resolve_block(block);
            }
            hir::Stmt::ChannelClose(e, _) => self.resolve_expr(e),
            hir::Stmt::Stop(e, _) => self.resolve_expr(e),
            hir::Stmt::SimFor(f, _) => {
                f.bind_ty = self.infer_ctx.resolve(&f.bind_ty);
                self.resolve_expr(&mut f.iter);
                if let Some(end) = &mut f.end { self.resolve_expr(end); }
                if let Some(step) = &mut f.step { self.resolve_expr(step); }
                self.resolve_block(&mut f.body);
            }
            hir::Stmt::UseLocal(_, _, _, _) => {}
        }
    }

    fn resolve_expr(&mut self, expr: &mut hir::Expr) {
        expr.ty = self.infer_ctx.resolve(&expr.ty);
        match &mut expr.kind {
            hir::ExprKind::Int(_)
            | hir::ExprKind::Float(_)
            | hir::ExprKind::Str(_)
            | hir::ExprKind::Bool(_)
            | hir::ExprKind::None
            | hir::ExprKind::Void
            | hir::ExprKind::MapNew
            | hir::ExprKind::SetNew
            | hir::ExprKind::PQNew
            | hir::ExprKind::NDArrayNew(_)
            | hir::ExprKind::SIMDNew(_)
            | hir::ExprKind::StoreCount(_)
            | hir::ExprKind::StoreAll(_) => {}
            hir::ExprKind::Var(_, _)
            | hir::ExprKind::FnRef(_, _)
            | hir::ExprKind::VariantRef(_, _, _) => {}
            hir::ExprKind::BinOp(l, _, r) => {
                self.resolve_expr(l);
                self.resolve_expr(r);
            }
            hir::ExprKind::UnaryOp(_, e) => self.resolve_expr(e),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.resolve_expr(callee);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Builtin(_, args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Method(recv, _, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::StringMethod(recv, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
                self.reclassify_method_call(expr);
            }
            hir::ExprKind::VecMethod(recv, _, args) | hir::ExprKind::MapMethod(recv, _, args) | hir::ExprKind::SetMethod(recv, _, args) | hir::ExprKind::PQMethod(recv, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::VecNew(args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Field(e, _field_name, _idx) => {
                self.resolve_expr(e);
                if let hir::ExprKind::Field(ref inner, ref fname, ref mut field_idx) = expr.kind {
                    let recv_ty = &inner.ty;
                    if let Type::Struct(name, _) = recv_ty {
                        if let Some(fields) = self.structs.get(name) {
                            if let Some((i, _)) =
                                fields.iter().enumerate().find(|(_, (n, _))| n == fname)
                            {
                                *field_idx = i;
                            }
                        }
                    }
                }
            }
            hir::ExprKind::Index(arr, idx) => {
                self.resolve_expr(arr);
                self.resolve_expr(idx);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                self.resolve_expr(c);
                self.resolve_expr(t);
                self.resolve_expr(f);
            }
            hir::ExprKind::Coerce(e, _) => self.resolve_expr(e),
            hir::ExprKind::Cast(e, ty) => {
                self.resolve_expr(e);
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::ExprKind::Array(elems) | hir::ExprKind::Tuple(elems) => {
                for e in elems {
                    self.resolve_expr(e);
                }
            }
            hir::ExprKind::Struct(_, fields) | hir::ExprKind::VariantCtor(_, _, _, fields) => {
                for fi in fields {
                    self.resolve_expr(&mut fi.value);
                }
            }
            hir::ExprKind::IfExpr(if_stmt) => {
                self.resolve_expr(&mut if_stmt.cond);
                self.resolve_block(&mut if_stmt.then);
                for (cond, block) in &mut if_stmt.elifs {
                    self.resolve_expr(cond);
                    self.resolve_block(block);
                }
                if let Some(els) = &mut if_stmt.els {
                    self.resolve_block(els);
                }
            }
            hir::ExprKind::Pipe(e, _, _, args) => {
                self.resolve_expr(e);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Block(block) => self.resolve_block(block),
            hir::ExprKind::Lambda(params, body) => {
                for p in params {
                    p.ty = self.infer_ctx.resolve(&p.ty);
                }
                self.resolve_block(body);
            }
            hir::ExprKind::Ref(e) | hir::ExprKind::Deref(e) => self.resolve_expr(e),
            hir::ExprKind::ListComp(body, _, _, iter, cond, map) => {
                self.resolve_expr(body);
                self.resolve_expr(iter);
                if let Some(c) = cond {
                    self.resolve_expr(c);
                }
                if let Some(m) = map {
                    self.resolve_expr(m);
                }
            }
            hir::ExprKind::Syscall(args) => {
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::Spawn(_) => {}
            hir::ExprKind::Send(recv, _, _, _, args) => {
                self.resolve_expr(recv);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::CoroutineCreate(_, stmts) => {
                self.resolve_block(stmts);
            }
            hir::ExprKind::CoroutineNext(e) | hir::ExprKind::Yield(e) => {
                self.resolve_expr(e);
            }
            hir::ExprKind::DynDispatch(obj, _, _, args) => {
                self.resolve_expr(obj);
                for a in args {
                    self.resolve_expr(a);
                }
            }
            hir::ExprKind::DynCoerce(e, _, _) => self.resolve_expr(e),
            hir::ExprKind::StoreQuery(_, filter) => self.resolve_filter(filter),
            hir::ExprKind::IterNext(_, _, _) => {}
            hir::ExprKind::ChannelCreate(ty, cap) => {
                *ty = self.infer_ctx.resolve(ty);
                self.resolve_expr(cap);
            }
            hir::ExprKind::ChannelSend(ch, val) => {
                self.resolve_expr(ch);
                self.resolve_expr(val);
            }
            hir::ExprKind::ChannelRecv(ch) => self.resolve_expr(ch),
            hir::ExprKind::Unreachable => {}
            hir::ExprKind::StrictCast(e, ty) => {
                self.resolve_expr(e);
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::ExprKind::AsFormat(e, _) | hir::ExprKind::AtomicLoad(e) => self.resolve_expr(e),
            hir::ExprKind::AtomicStore(a, b) | hir::ExprKind::AtomicAdd(a, b) | hir::ExprKind::AtomicSub(a, b) => {
                self.resolve_expr(a);
                self.resolve_expr(b);
            }
            hir::ExprKind::AtomicCas(p, e, n) => {
                self.resolve_expr(p);
                self.resolve_expr(e);
                self.resolve_expr(n);
            }
            hir::ExprKind::Slice(obj, start, end) => {
                self.resolve_expr(obj);
                self.resolve_expr(start);
                self.resolve_expr(end);
            }
            hir::ExprKind::Select(arms, default) => {
                for arm in arms {
                    arm.elem_ty = self.infer_ctx.resolve(&arm.elem_ty);
                    self.resolve_expr(&mut arm.chan);
                    if let Some(v) = &mut arm.value {
                        self.resolve_expr(v);
                    }
                    self.resolve_block(&mut arm.body);
                }
                if let Some(block) = default {
                    self.resolve_block(block);
                }
            }
            hir::ExprKind::DequeNew => {}
            hir::ExprKind::DequeMethod(obj, _, args) => {
                self.resolve_expr(obj);
                for a in args { self.resolve_expr(a); }
            }
            hir::ExprKind::Grad(e) | hir::ExprKind::CowWrap(e) | hir::ExprKind::CowClone(e) | hir::ExprKind::GeneratorNext(e) => {
                self.resolve_expr(e);
            }
            hir::ExprKind::Einsum(_, args) => {
                for a in args { self.resolve_expr(a); }
            }
            hir::ExprKind::Builder(_, fields) => {
                for (_, v) in fields { self.resolve_expr(v); }
            }
            hir::ExprKind::GeneratorCreate(_, _, stmts) => {
                for s in stmts { self.resolve_stmt(s); }
            }
        }
    }

    fn resolve_pat(&mut self, pat: &mut hir::Pat) {
        match pat {
            hir::Pat::Wild(_) => {}
            hir::Pat::Bind(_, _, ty, _) => {
                *ty = self.infer_ctx.resolve(ty);
            }
            hir::Pat::Lit(e) => self.resolve_expr(e),
            hir::Pat::Ctor(_, _, pats, _)
            | hir::Pat::Tuple(pats, _)
            | hir::Pat::Array(pats, _)
            | hir::Pat::Or(pats, _) => {
                for p in pats {
                    self.resolve_pat(p);
                }
            }
            hir::Pat::Range(lo, hi, _) => {
                self.resolve_expr(lo);
                self.resolve_expr(hi);
            }
        }
    }

    fn resolve_filter(&mut self, filter: &mut hir::StoreFilter) {
        self.resolve_expr(&mut filter.value);
        for (_, cond) in &mut filter.extra {
            self.resolve_expr(&mut cond.value);
        }
    }

    pub(crate) fn lower_actor_def(&mut self, ad: &ast::ActorDef) -> Result<hir::ActorDef, String> {
        let (id, ref declared_fields, ref handler_info) = self
            .actors
            .get(&ad.name)
            .ok_or_else(|| format!("undeclared actor: {}", ad.name))?
            .clone();

        let fields: Vec<hir::Field> = ad
            .fields
            .iter()
            .map(|f| {
                let ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let default = f.default.as_ref().map(|e| {
                    self.lower_expr_expected(e, Some(&ty))
                        .unwrap_or_else(|_| hir::Expr {
                            kind: hir::ExprKind::Int(0),
                            ty: Type::I64,
                            span: e.span(),
                        })
                });
                hir::Field {
                    name: f.name.clone(),
                    ty,
                    default,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_handlers = Vec::new();
        for (i, h) in ad.handlers.iter().enumerate() {
            self.push_scope();
            for f in &fields {
                let fid = self.fresh_id();
                self.define_var(
                    &f.name,
                    VarInfo {
                        def_id: fid,
                        ty: f.ty.clone(),
                        ownership: Ownership::Owned,
                        scheme: None,
                    },
                );
            }
            let mut params = Vec::new();
            let declared_ptys = &handler_info[i].1;
            for (pi, p) in h.params.iter().enumerate() {
                let pid = self.fresh_id();
                let ty = p.ty.clone().unwrap_or_else(|| {
                    declared_ptys
                        .get(pi)
                        .map(|t| self.infer_ctx.resolve(t))
                        .unwrap_or(Type::I64)
                });
                let ownership = Self::ownership_for_type(&ty);
                self.define_var(
                    &p.name,
                    VarInfo {
                        def_id: pid,
                        ty: ty.clone(),
                        ownership,
                        scheme: None,
                    },
                );
                params.push(hir::Param {
                    def_id: pid,
                    name: p.name.clone(),
                    ty,
                    ownership,
                    span: p.span,
                });
            }
            let body = self.lower_block(&h.body, &Type::Void)?;
            self.pop_scope();
            hir_handlers.push(hir::HandlerDef {
                name: h.name.clone(),
                params,
                body,
                tag: handler_info[i].2,
                span: h.span,
            });
        }

        Ok(hir::ActorDef {
            def_id: id,
            name: ad.name.clone(),
            fields,
            handlers: hir_handlers,
            span: ad.span,
        })
    }

    pub(crate) fn lower_store_def(&mut self, sd: &ast::StoreDef) -> Result<hir::StoreDef, String> {
        let id = self.fresh_id();
        let fields: Vec<hir::Field> = sd
            .fields
            .iter()
            .map(|f| hir::Field {
                name: f.name.clone(),
                ty: f.ty.clone().unwrap_or(Type::I64),
                default: None,
                span: f.span,
            })
            .collect();
        Ok(hir::StoreDef {
            def_id: id,
            name: sd.name.clone(),
            fields,
            span: sd.span,
        })
    }

    pub(crate) fn lower_impl_block(
        &mut self,
        ib: &ast::ImplBlock,
    ) -> Result<hir::TraitImpl, String> {
        let mut hir_methods = Vec::new();
        for m in &ib.methods {
            let hm = self.lower_method_by_ptr(&ib.type_name, m)?;
            hir_methods.push(hm);
        }
        Ok(hir::TraitImpl {
            trait_name: ib.trait_name.clone(),
            trait_type_args: ib.trait_type_args.clone(),
            type_name: ib.type_name.clone(),
            methods: hir_methods,
            span: ib.span,
        })
    }

    pub(crate) fn hir_tail_type(&self, body: &[hir::Stmt]) -> Option<Type> {
        let last = body
            .iter()
            .rev()
            .find(|s| !matches!(s, hir::Stmt::Drop(..)))?;
        match last {
            hir::Stmt::Expr(e) if e.ty != Type::Void => Some(e.ty.clone()),
            hir::Stmt::If(i) => {
                if i.els.is_some() {
                    self.hir_tail_type(&i.then)
                } else {
                    None
                }
            }
            hir::Stmt::Match(m) => {
                if let Some(arm) = m.arms.first() {
                    self.hir_tail_type(&arm.body)
                } else {
                    None
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Some(e.ty.clone()),
            _ => None,
        }
    }

    fn build_fn_scheme(&mut self, name: &str, hfn: &hir::Fn) {
        let param_tys: Vec<crate::types::Type> = hfn
            .params
            .iter()
            .map(|p| self.infer_ctx.canonicalize_type(&p.ty))
            .collect();
        let ret_ty = self.infer_ctx.canonicalize_type(&hfn.ret);
        let fn_ty = crate::types::Type::Fn(param_tys.clone(), Box::new(ret_ty.clone()));
        let scheme = self.generalize(&fn_ty);
        if scheme.is_poly() {
            self.infer_ctx.mark_quantified(&scheme.quantified);
        }
        if self.debug_types {
            if scheme.is_poly() {
                eprintln!(
                    "[type:scheme] {} :: ∀{:?}. ({}) -> {}",
                    name,
                    scheme.quantified,
                    param_tys
                        .iter()
                        .map(|t| format!("{t}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    ret_ty
                );
            }
        }
        self.fn_schemes
            .insert(name.to_string(), (scheme.quantified, param_tys, ret_ty));
    }

    pub(crate) fn lower_fn(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let mut hfn = self.lower_fn_deferred(f)?;
        if f.ret.is_none() && f.name != "main" {
            if !self.inferable_fns.contains_key(&f.name) {
                hfn.ret = self.infer_ctx.resolve(&hfn.ret);
            }
        }
        Ok(hfn)
    }

    fn lower_fn_deferred(&mut self, f: &ast::Fn) -> Result<hir::Fn, String> {
        let (id, ptys, ret) = self
            .fns
            .get(&f.name)
            .ok_or_else(|| format!("undeclared function: {}", f.name))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }
        let body = self.lower_block(&f.body, &ret)?;
        self.pop_scope();

        if f.ret.is_none() && f.name != "main" {
            if let Some(tail_ty) = self.hir_tail_type(&body) {
                let r = self
                    .infer_ctx
                    .unify_at(&ret, &tail_ty, f.span, "function tail expression");
                self.collect_unify_error(r);
            } else {
                let _ = self.infer_ctx.unify(&ret, &Type::Void);
            }
        }

        Ok(hir::Fn {
            def_id: id,
            name: f.name.clone(),
            params,
            ret,
            body,
            span: f.span,
            generic_origin: None,
            is_generator: f.is_generator,
        })
    }

    fn lower_test_block(&mut self, tb: &ast::TestBlock, fn_name: &str) -> Result<hir::Fn, String> {
        let id = self.fresh_id();
        self.push_scope();
        let body = self.lower_block(&tb.body, &Type::Void)?;
        self.pop_scope();
        Ok(hir::Fn {
            def_id: id,
            name: fn_name.to_string(),
            params: vec![],
            ret: Type::Void,
            body,
            span: tb.span,
            generic_origin: None,
            is_generator: false,
        })
    }

    fn build_test_runner(&mut self, tests: &[(String, String)]) -> hir::Fn {
        let id = self.fresh_id();
        let s = Span::dummy();
        let mut body: hir::Block = Vec::new();
        for (display_name, fn_name) in tests {
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str(format!("test {display_name} ...")),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
            let test_id = self.fns.get(fn_name).unwrap().0;
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Call(test_id, fn_name.clone(), vec![]),
                ty: Type::Void,
                span: s,
            }));
            body.push(hir::Stmt::Expr(hir::Expr {
                kind: hir::ExprKind::Builtin(
                    hir::BuiltinFn::Log,
                    vec![hir::Expr {
                        kind: hir::ExprKind::Str("  ok".into()),
                        ty: Type::String,
                        span: s,
                    }],
                ),
                ty: Type::Void,
                span: s,
            }));
        }
        hir::Fn {
            def_id: id,
            name: "main".into(),
            params: vec![],
            ret: Type::I32,
            body,
            span: s,
            generic_origin: None,
            is_generator: false,
        }
    }

    pub(crate) fn lower_type_def(&mut self, td: &ast::TypeDef) -> Result<hir::TypeDef, String> {
        let id = self.fresh_id();
        let declared_fields = self.structs.get(&td.name).cloned().unwrap_or_default();
        let fields: Vec<hir::Field> = td
            .fields
            .iter()
            .map(|f| {
                let raw_ty = declared_fields
                    .iter()
                    .find(|(n, _)| n == &f.name)
                    .map(|(_, t)| t.clone())
                    .unwrap_or_else(|| f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)));
                let ty = self.infer_ctx.resolve(&raw_ty);
                let default = f.default.as_ref().map(|e| {
                    let lowered =
                        self.lower_expr_expected(e, Some(&ty))
                            .unwrap_or_else(|_| hir::Expr {
                                kind: hir::ExprKind::Int(0),
                                ty: Type::I64,
                                span: e.span(),
                            });
                    let _ =
                        self.infer_ctx
                            .unify_at(&ty, &lowered.ty, f.span, "field default value");
                    lowered
                });
                hir::Field {
                    name: f.name.clone(),
                    ty,
                    default,
                    span: f.span,
                }
            })
            .collect();

        let mut hir_methods = Vec::new();
        for m in &td.methods {
            let method_name = format!("{}_{}", td.name, m.name);
            if self.fns.contains_key(&method_name) {
                let hm = self.lower_method_by_ptr(&td.name, m)?;
                hir_methods.push(hm);
            }
        }

        Ok(hir::TypeDef {
            def_id: id,
            name: td.name.clone(),
            fields,
            methods: hir_methods,
            layout: td.layout.clone(),
            span: td.span,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn lower_method(&mut self, type_name: &str, m: &ast::Fn) -> Result<hir::Fn, String> {
        self.lower_method_impl(type_name, m, false)
    }

    pub(crate) fn lower_method_by_ptr(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
    ) -> Result<hir::Fn, String> {
        self.lower_method_impl(type_name, m, true)
    }

    fn lower_method_impl(
        &mut self,
        type_name: &str,
        m: &ast::Fn,
        by_ptr: bool,
    ) -> Result<hir::Fn, String> {
        let method_name = format!("{type_name}_{}", m.name);
        let (id, ptys, ret) = self
            .fns
            .get(&method_name)
            .ok_or_else(|| format!("undeclared method: {method_name}"))?
            .clone();

        self.push_scope();
        let mut params = Vec::new();

        let self_id = self.fresh_id();
        let self_ty = ptys[0].clone();
        self.define_var(
            "self",
            VarInfo {
                def_id: self_id,
                ty: self_ty.clone(),
                ownership: Ownership::Borrowed,
                scheme: None,
            },
        );
        params.push(hir::Param {
            def_id: self_id,
            name: "self".to_string(),
            ty: self_ty,
            ownership: Ownership::Borrowed,
            span: m.span,
        });

        let param_iter: Box<dyn Iterator<Item = &ast::Param>> = if by_ptr {
            Box::new(m.params.iter().filter(|p| p.name != "self"))
        } else {
            Box::new(m.params.iter())
        };
        for (i, p) in param_iter.enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i + 1].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
                    scheme: None,
                },
            );
            params.push(hir::Param {
                def_id: pid,
                name: p.name.clone(),
                ty,
                ownership,
                span: p.span,
            });
        }

        let body = self.lower_block(&m.body, &ret)?;
        self.pop_scope();

        let reason = if by_ptr {
            "ptr method tail expression"
        } else {
            "method tail expression"
        };
        let ret = if m.ret.is_none() {
            if let Some(tail_ty) = self.hir_tail_type(&body) {
                let r = self.infer_ctx.unify_at(&ret, &tail_ty, m.span, reason);
                self.collect_unify_error(r);
            } else {
                let _ = self.infer_ctx.unify(&ret, &Type::Void);
            }
            self.infer_ctx.resolve(&ret)
        } else {
            ret
        };

        Ok(hir::Fn {
            def_id: id,
            name: method_name,
            params,
            ret,
            body,
            span: m.span,
            generic_origin: None,
            is_generator: false,
        })
    }

    pub(crate) fn lower_enum_def(&mut self, ed: &ast::EnumDef) -> hir::EnumDef {
        let id = self.fresh_id();
        let variants: Vec<hir::Variant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::Variant {
                name: v.name.clone(),
                fields: v
                    .fields
                    .iter()
                    .map(|f| hir::VField {
                        name: f.name.clone(),
                        ty: f.ty.clone(),
                    })
                    .collect(),
                tag: v.discriminant.map(|d| d as u32).unwrap_or(tag as u32),
                discriminant: v.discriminant,
                span: v.span,
            })
            .collect();
        hir::EnumDef {
            def_id: id,
            name: ed.name.clone(),
            variants,
            span: ed.span,
        }
    }

    pub(crate) fn lower_extern(&self, ef: &ast::ExternFn) -> hir::ExternFn {
        let (id, _, _) = self
            .fns
            .get(&ef.name)
            .cloned()
            .unwrap_or_else(|| (DefId::BUILTIN, vec![], Type::Void));
        hir::ExternFn {
            def_id: id,
            name: ef.name.clone(),
            params: ef.params.clone(),
            ret: ef.ret.clone(),
            variadic: ef.variadic,
            span: ef.span,
        }
    }

    pub(crate) fn lower_err_def(&mut self, ed: &ast::ErrDef) -> hir::ErrDef {
        let id = self.fresh_id();
        let variants: Vec<hir::ErrVariant> = ed
            .variants
            .iter()
            .enumerate()
            .map(|(tag, v)| hir::ErrVariant {
                name: v.name.clone(),
                fields: v.fields.clone(),
                tag: tag as u32,
                span: v.span,
            })
            .collect();
        hir::ErrDef {
            def_id: id,
            name: ed.name.clone(),
            variants,
            span: ed.span,
        }
    }

    pub(crate) fn type_implements_trait(&self, type_name: &str, trait_name: &str) -> bool {
        self.trait_impls
            .get(type_name)
            .map(|impls| impls.contains(&trait_name.to_string()))
            .unwrap_or(false)
    }

    pub(crate) fn iter_element_type(&self, type_name: &str) -> Type {
        if let Some(args) = self
            .trait_impl_type_args
            .get(&(type_name.into(), "Iter".into()))
        {
            if let Some(t) = args.first() {
                return t.clone();
            }
        }
        let fn_name = format!("{type_name}_next");
        if let Some((_, _, ret)) = self.fns.get(&fn_name) {
            if let Type::Enum(ename) = ret {
                if let Some(stripped) = ename.strip_prefix("Option_") {
                    return match stripped {
                        "i64" => Type::I64,
                        "f64" => Type::F64,
                        "bool" => Type::Bool,
                        "String" => Type::String,
                        other => Type::Struct(other.into(), vec![]),
                    };
                }
            }
        }
        Type::I64
    }

    pub(crate) fn desugar_for_iter(
        &mut self,
        f: &ast::For,
        iter_expr: hir::Expr,
        type_name: String,
        elem_ty: Type,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        let span = f.span;

        let mut option_type_map = HashMap::new();
        option_type_map.insert("T".into(), elem_ty.clone());
        let option_enum_name = self.monomorphize_enum("Option", &option_type_map)?;

        let some_tag = self.variant_tags.get("Some").map(|(_, t)| *t).unwrap_or(0);
        let nothing_tag = self
            .variant_tags
            .get("Nothing")
            .map(|(_, t)| *t)
            .unwrap_or(1);

        let iter_bind_id = self.fresh_id();
        let iter_var_name = format!("__iter_{}", f.bind);

        self.define_var(
            &iter_var_name,
            VarInfo {
                def_id: iter_bind_id,
                ty: iter_expr.ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );

        let bind_stmt = hir::Stmt::Bind(hir::Bind {
            def_id: iter_bind_id,
            name: iter_var_name.clone(),
            value: iter_expr.clone(),
            ty: iter_expr.ty.clone(),
            ownership: Ownership::Owned,
            span,
        });

        let method_name = format!("{type_name}_next");
        let ret = Type::Enum(option_enum_name.clone());
        if let Some(entry) = self.fns.get_mut(&method_name) {
            entry.2 = ret.clone();
        }

        let next_call = hir::Expr {
            kind: hir::ExprKind::IterNext(iter_var_name.clone(), type_name.clone(), "next".into()),
            ty: ret,
            span,
        };

        let bind_id = self.fresh_id();
        let some_pat = hir::Pat::Ctor(
            "Some".into(),
            some_tag,
            vec![hir::Pat::Bind(
                bind_id,
                f.bind.clone(),
                elem_ty.clone(),
                span,
            )],
            span,
        );
        let nothing_pat = hir::Pat::Ctor("Nothing".into(), nothing_tag, vec![], span);

        self.push_scope();
        self.define_var(
            &f.bind,
            VarInfo {
                def_id: bind_id,
                ty: elem_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let body = self.lower_block_no_scope(&f.body, ret_ty)?;
        self.pop_scope();

        let some_arm = hir::Arm {
            pat: some_pat,
            guard: None,
            body,
            span,
        };
        let nothing_arm = hir::Arm {
            pat: nothing_pat,
            guard: None,
            body: vec![hir::Stmt::Break(None, span)],
            span,
        };

        let match_stmt = hir::Stmt::Match(hir::Match {
            subject: next_call,
            arms: vec![some_arm, nothing_arm],
            ty: Type::Void,
            span,
        });

        let loop_stmt = hir::Stmt::Loop(hir::Loop {
            body: vec![match_stmt],
            span,
        });

        Ok(hir::Stmt::Expr(hir::Expr {
            kind: hir::ExprKind::Block(vec![bind_stmt, loop_stmt]),
            ty: Type::Void,
            span,
        }))
    }

    /// Desugar `for k, v in map` into keys-based iteration:
    /// `__keys = map.keys(); for __i from 0 to __keys.len() { k = __keys.get(__i); v = map.get(k); ...body }`
    pub(crate) fn desugar_for_map(
        &mut self,
        f: &ast::For,
        val_bind: &str,
        map_expr: hir::Expr,
        key_ty: &Type,
        val_ty: &Type,
        ret_ty: &Type,
    ) -> Result<hir::Stmt, String> {
        let span = f.span;
        let key_ty = key_ty.clone();
        let val_ty = val_ty.clone();

        // Bind the map to a temp variable
        let map_id = self.fresh_id();
        let map_var = "__map_iter".to_string();
        let map_ty = map_expr.ty.clone();
        self.define_var(
            &map_var,
            VarInfo {
                def_id: map_id,
                ty: map_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let map_bind = hir::Stmt::Bind(hir::Bind {
            def_id: map_id,
            name: map_var.clone(),
            value: map_expr,
            ty: map_ty.clone(),
            ownership: Ownership::Owned,
            span,
        });

        // __keys = map.keys()
        let keys_id = self.fresh_id();
        let keys_var = "__map_keys".to_string();
        let keys_ty = Type::Vec(Box::new(key_ty.clone()));
        let keys_call = hir::Expr {
            kind: hir::ExprKind::MapMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(map_id, map_var.clone()),
                    ty: map_ty.clone(),
                    span,
                }),
                "keys".into(),
                vec![],
            ),
            ty: keys_ty.clone(),
            span,
        };
        self.define_var(
            &keys_var,
            VarInfo {
                def_id: keys_id,
                ty: keys_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let keys_bind = hir::Stmt::Bind(hir::Bind {
            def_id: keys_id,
            name: keys_var.clone(),
            value: keys_call,
            ty: keys_ty.clone(),
            ownership: Ownership::Owned,
            span,
        });

        // for __i from 0 to __keys.len() { k = __keys.get(__i); v = map.get(k); ...body }
        let i_id = self.fresh_id();
        let i_var = "__map_i".to_string();
        self.push_scope();
        self.define_var(
            &i_var,
            VarInfo {
                def_id: i_id,
                ty: Type::I64,
                ownership: Ownership::Owned,
                scheme: None,
            },
        );

        // k = __keys.get(__i)
        let k_id = self.fresh_id();
        let k_get = hir::Expr {
            kind: hir::ExprKind::VecMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(keys_id, keys_var.clone()),
                    ty: keys_ty.clone(),
                    span,
                }),
                "get".into(),
                vec![hir::Expr {
                    kind: hir::ExprKind::Var(i_id, i_var.clone()),
                    ty: Type::I64,
                    span,
                }],
            ),
            ty: key_ty.clone(),
            span,
        };
        self.define_var(
            &f.bind,
            VarInfo {
                def_id: k_id,
                ty: key_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let k_bind = hir::Stmt::Bind(hir::Bind {
            def_id: k_id,
            name: f.bind.clone(),
            value: k_get,
            ty: key_ty.clone(),
            ownership: Ownership::Owned,
            span,
        });

        // v = map.get(k)
        let v_id = self.fresh_id();
        let v_get = hir::Expr {
            kind: hir::ExprKind::MapMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(map_id, map_var.clone()),
                    ty: map_ty,
                    span,
                }),
                "get".into(),
                vec![hir::Expr {
                    kind: hir::ExprKind::Var(k_id, f.bind.clone()),
                    ty: key_ty,
                    span,
                }],
            ),
            ty: val_ty.clone(),
            span,
        };
        self.define_var(
            val_bind,
            VarInfo {
                def_id: v_id,
                ty: val_ty.clone(),
                ownership: Ownership::Owned,
                scheme: None,
            },
        );
        let v_bind = hir::Stmt::Bind(hir::Bind {
            def_id: v_id,
            name: val_bind.to_string(),
            value: v_get,
            ty: val_ty,
            ownership: Ownership::Owned,
            span,
        });

        let user_body = self.lower_block_no_scope(&f.body, ret_ty)?;
        self.pop_scope();

        let mut for_body = vec![k_bind, v_bind];
        for_body.extend(user_body);

        // __keys.len() as the end expression
        let keys_len = hir::Expr {
            kind: hir::ExprKind::VecMethod(
                Box::new(hir::Expr {
                    kind: hir::ExprKind::Var(keys_id, keys_var),
                    ty: keys_ty,
                    span,
                }),
                "len".into(),
                vec![],
            ),
            ty: Type::I64,
            span,
        };

        let for_stmt = hir::Stmt::For(hir::For {
            bind_id: i_id,
            bind: i_var,
            bind_ty: Type::I64,
            iter: hir::Expr {
                kind: hir::ExprKind::Int(0),
                ty: Type::I64,
                span,
            },
            end: Some(keys_len),
            step: None,
            body: for_body,
            span,
        });

        Ok(hir::Stmt::Expr(hir::Expr {
            kind: hir::ExprKind::Block(vec![map_bind, keys_bind, for_stmt]),
            ty: Type::Void,
            span,
        }))
    }

    pub(crate) fn lower_block(
        &mut self,
        block: &ast::Block,
        ret_ty: &Type,
    ) -> Result<hir::Block, String> {
        self.push_scope();
        let mut stmts = self.lower_block_no_scope(block, ret_ty)?;
        let ends_with_jump = stmts.last().map_or(false, |s| {
            matches!(
                s,
                hir::Stmt::Ret(..) | hir::Stmt::Break(..) | hir::Stmt::Continue(..)
            )
        });
        if ends_with_jump {
            let jump = stmts.pop().unwrap();
            self.emit_scope_drops(&mut stmts);
            stmts.push(jump);
        } else {
            self.emit_scope_drops(&mut stmts);
        }
        self.pop_scope();
        Ok(stmts)
    }

    fn emit_scope_drops(&self, stmts: &mut Vec<hir::Stmt>) {
        let scope = match self.scopes.last() {
            Some(s) => s,
            None => return,
        };
        let mut drops: Vec<_> = scope
            .iter()
            .filter(|(_, info)| Self::needs_drop(&info.ty))
            .collect();
        drops.sort_by_key(|(_, info)| std::cmp::Reverse(info.def_id.0));
        for (name, info) in drops {
            stmts.push(hir::Stmt::Drop(
                info.def_id,
                name.clone(),
                info.ty.clone(),
                crate::ast::Span::dummy(),
            ));
        }
    }

    pub(crate) fn needs_drop(ty: &Type) -> bool {
        matches!(
            ty,
            Type::String | Type::Vec(_) | Type::Map(_, _) | Type::Rc(_) | Type::Weak(_)
        )
    }

    pub(crate) fn check_exhaustiveness(
        &self,
        subject_ty: &Type,
        arms: &[hir::Arm],
        _span: Span,
    ) -> Result<(), String> {
        let pats: Vec<&hir::Pat> = arms
            .iter()
            .filter(|a| a.guard.is_none())
            .map(|a| &a.pat)
            .collect();

        let missing = self.find_missing_patterns(&pats, subject_ty);
        if !missing.is_empty() {
            let missing_str = missing.join(", ");
            let ty_name = match subject_ty {
                Type::Enum(n) => format!("`{n}`"),
                Type::Bool => "Bool".to_string(),
                _ => format!("{:?}", subject_ty),
            };
            return Err(format!(
                "non-exhaustive match on {ty_name}: missing {missing_str}"
            ));
        }

        if let Type::Enum(_) = subject_ty {
            let mut seen: Vec<&str> = Vec::new();
            for arm in arms {
                if let hir::Pat::Ctor(n, _, subs, _) = &arm.pat {
                    if subs.is_empty() && seen.contains(&n.as_str()) {
                        eprintln!("warning: unreachable pattern `{n}` — already matched above");
                    }
                    if subs.is_empty() {
                        seen.push(n.as_str());
                    }
                }
            }
        }

        Ok(())
    }

    fn find_missing_patterns(&self, pats: &[&hir::Pat], ty: &Type) -> Vec<String> {
        let mut flat: Vec<&hir::Pat> = Vec::new();
        for p in pats {
            Self::flatten_or_pat(p, &mut flat);
        }

        if flat
            .iter()
            .any(|p| matches!(p, hir::Pat::Wild(_) | hir::Pat::Bind(..)))
        {
            return vec![];
        }

        let ty = self.resolve_ty(ty.clone());

        match &ty {
            Type::Enum(name) => {
                let variants = match self.enums.get(name) {
                    Some(v) => v,
                    None => return vec![],
                };
                let mut missing = Vec::new();
                for (vname, field_tys) in variants {
                    let sub_lists: Vec<&Vec<hir::Pat>> = flat
                        .iter()
                        .filter_map(|p| match p {
                            hir::Pat::Ctor(n, _, subs, _) if n == vname => Some(subs),
                            _ => None,
                        })
                        .collect();

                    if sub_lists.is_empty() {
                        if field_tys.is_empty() {
                            missing.push(vname.clone());
                        } else {
                            let fields = vec!["_"; field_tys.len()].join(", ");
                            missing.push(format!("{}({})", vname, fields));
                        }
                    } else if !field_tys.is_empty() {
                        for (i, ft) in field_tys.iter().enumerate() {
                            let col: Vec<&hir::Pat> =
                                sub_lists.iter().filter_map(|subs| subs.get(i)).collect();
                            let sub_missing = self.find_missing_patterns(&col, ft);
                            for sm in &sub_missing {
                                let fields: Vec<String> = field_tys
                                    .iter()
                                    .enumerate()
                                    .map(|(j, _)| if j == i { sm.clone() } else { "_".to_string() })
                                    .collect();
                                missing.push(format!("{}({})", vname, fields.join(", ")));
                            }
                        }
                    }
                }
                missing
            }
            Type::Bool => {
                let has_true = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(true)),
                    _ => false,
                });
                let has_false = flat.iter().any(|p| match p {
                    hir::Pat::Lit(e) => matches!(e.kind, hir::ExprKind::Bool(false)),
                    _ => false,
                });
                let mut missing = Vec::new();
                if !has_true {
                    missing.push("true".to_string());
                }
                if !has_false {
                    missing.push("false".to_string());
                }
                missing
            }
            Type::I64 | Type::F64 | Type::String => {
                vec!["_".to_string()]
            }
            _ => vec![],
        }
    }

    fn flatten_or_pat<'a>(pat: &'a hir::Pat, out: &mut Vec<&'a hir::Pat>) {
        match pat {
            hir::Pat::Or(pats, _) => {
                for p in pats {
                    Self::flatten_or_pat(p, out);
                }
            }
            _ => out.push(pat),
        }
    }
}
