use crate::ast::{self, Span};
use crate::types::Type;

use super::Typer;

impl Typer {
    pub(crate) fn register_prelude_types(&mut self) {
        let s = Span::dummy();
        self.generic_enums
            .entry("Option".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Option".into(),
                type_params: vec!["T".into()],
                variants: vec![
                    ast::Variant {
                        name: "Some".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Nothing".into(),
                        fields: vec![],
                        span: s,
                    },
                ],
                span: s,
            });
        self.generic_enums
            .entry("Result".into())
            .or_insert_with(|| ast::EnumDef {
                name: "Result".into(),
                type_params: vec!["T".into(), "E".into()],
                variants: vec![
                    ast::Variant {
                        name: "Ok".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    ast::Variant {
                        name: "Err".into(),
                        fields: vec![ast::VField {
                            name: None,
                            ty: Type::Param("E".into()),
                        }],
                        span: s,
                    },
                ],
                span: s,
            });

        self.traits.entry("Iter".into()).or_insert_with(|| {
            vec![super::TraitMethodSig {
                name: "next".into(),
                _params: vec![],
                _ret: Some(Type::Enum("Option".into())),
                has_default: false,
            }]
        });
    }

    pub(crate) fn declare_fn_sig(&mut self, f: &ast::Fn) {
        let ptys: Vec<Type> = f
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()))
            .collect();
        let ret = if f.name == "main" {
            Type::I32
        } else if let Some(ref explicit) = f.ret {
            explicit.clone()
        } else {
            // Phase 1.3: Pure TypeVar for unannotated return types.
            // Solved by unify_at on return stmts and tail exprs during lowering.
            self.infer_ctx.fresh_var()
        };
        let id = self.fresh_id();
        if self.debug_types {
            eprintln!(
                "[type:sig] {} :: ({}) -> {}",
                f.name,
                ptys.iter()
                    .map(|t| format!("{t}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                ret
            );
        }
        self.fns.insert(f.name.clone(), (id, ptys, ret));
    }

    pub(crate) fn declare_method_sig(&mut self, type_name: &str, m: &ast::Fn) {
        let method_name = format!("{type_name}_{}", m.name);
        let self_ty = Type::Struct(type_name.to_string());
        let mut ptys = vec![self_ty];
        for p in &m.params {
            ptys.push(p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()));
        }
        let ret = m.ret.clone().unwrap_or_else(|| {
            // Phase 1.3: Pure TypeVar for unannotated return types
            self.infer_ctx.fresh_var()
        });
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));
    }

    pub(crate) fn declare_method_sig_by_ptr(&mut self, type_name: &str, m: &ast::Fn) {
        let method_name = format!("{type_name}_{}", m.name);
        let self_ty = Type::Ptr(Box::new(Type::Struct(type_name.to_string())));
        let mut ptys = vec![self_ty];
        for p in &m.params {
            if p.name == "self" {
                continue;
            }
            ptys.push(p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()));
        }
        let ret = m.ret.clone().unwrap_or_else(|| {
            // Phase 1.3: Pure TypeVar for unannotated return types
            self.infer_ctx.fresh_var()
        });
        let id = self.fresh_id();
        self.fns.insert(method_name, (id, ptys, ret));
    }

    pub(crate) fn declare_type_def(&mut self, td: &ast::TypeDef) {
        let fields: Vec<(String, Type)> = td
            .fields
            .iter()
            .map(|f| {
                let ty = f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f));
                // R1.2: Track unannotated fields for strict-mode enforcement
                if f.ty.is_none() {
                    self.unannotated_struct_fields.push((
                        td.name.clone(),
                        f.name.clone(),
                        ty.clone(),
                        f.span,
                    ));
                }
                (f.name.clone(), ty)
            })
            .collect();
        self.structs.insert(td.name.clone(), fields);
    }

    pub(crate) fn declare_enum_def(&mut self, ed: &ast::EnumDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    pub(crate) fn declare_extern_sig(&mut self, ef: &ast::ExternFn) {
        let ptys: Vec<Type> = ef.params.iter().map(|(_, t)| t.clone()).collect();
        let id = self.fresh_id();
        self.fns.insert(ef.name.clone(), (id, ptys, ef.ret.clone()));
    }

    pub(crate) fn declare_err_def_sig(&mut self, ed: &ast::ErrDef) {
        let mut variants = Vec::new();
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys = v.fields.clone();
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(ed.name.clone(), variants);
    }

    pub(crate) fn declare_actor_def(&mut self, ad: &ast::ActorDef) {
        let id = self.fresh_id();
        let fields: Vec<(String, Type)> = ad
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        let handlers: Vec<(String, Vec<Type>, u32)> = ad
            .handlers
            .iter()
            .enumerate()
            .map(|(tag, h)| {
                let ptys: Vec<Type> = h
                    .params
                    .iter()
                    .map(|p| p.ty.clone().unwrap_or_else(|| self.infer_ctx.fresh_var()))
                    .collect();
                (h.name.clone(), ptys, tag as u32)
            })
            .collect();
        self.actors.insert(ad.name.clone(), (id, fields, handlers));
    }

    pub(crate) fn declare_trait_def(&mut self, td: &ast::TraitDef) {
        let sigs: Vec<super::TraitMethodSig> = td
            .methods
            .iter()
            .map(|m| super::TraitMethodSig {
                name: m.name.clone(),
                _params: m
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), p.ty.clone()))
                    .collect(),
                _ret: m.ret.clone(),
                has_default: m.default_body.is_some(),
            })
            .collect();
        self.traits.insert(td.name.clone(), sigs);
        if !td.assoc_types.is_empty() {
            self.trait_assoc_types
                .insert(td.name.clone(), td.assoc_types.clone());
        }
    }

    pub(crate) fn declare_impl_block(&mut self, ib: &ast::ImplBlock) -> Result<(), String> {
        if !self.structs.contains_key(&ib.type_name) {
            return Err(format!(
                "line {}: impl references unknown type '{}'",
                ib.span.line, ib.type_name
            ));
        }

        let is_iter_impl;

        if let Some(ref trait_name) = ib.trait_name {
            if !self.traits.contains_key(trait_name) {
                return Err(format!(
                    "line {}: impl references unknown trait '{}'",
                    ib.span.line, trait_name
                ));
            }

            is_iter_impl = trait_name == "Iter";

            if let Some(required_assocs) = self.trait_assoc_types.get(trait_name) {
                let provided: Vec<&str> = ib
                    .assoc_type_bindings
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect();
                for required in required_assocs {
                    if !provided.contains(&required.as_str()) {
                        return Err(format!(
                            "line {}: impl {} for {} is missing required associated type '{}'",
                            ib.span.line, trait_name, ib.type_name, required
                        ));
                    }
                }
            }

            let trait_sigs = self.traits.get(trait_name).cloned().unwrap();
            let impl_method_names: Vec<&str> = ib.methods.iter().map(|m| m.name.as_str()).collect();
            for sig in &trait_sigs {
                if !sig.has_default && !impl_method_names.contains(&sig.name.as_str()) {
                    return Err(format!(
                        "line {}: impl {} for {} is missing required method '{}'",
                        ib.span.line, trait_name, ib.type_name, sig.name
                    ));
                }
            }

            self.trait_impls
                .entry(ib.type_name.clone())
                .or_default()
                .push(trait_name.clone());

            if !ib.trait_type_args.is_empty() {
                self.trait_impl_type_args.insert(
                    (ib.type_name.clone(), trait_name.clone()),
                    ib.trait_type_args.clone(),
                );
            }

            for (assoc_name, assoc_ty) in &ib.assoc_type_bindings {
                self.assoc_types
                    .insert((ib.type_name.clone(), assoc_name.clone()), assoc_ty.clone());
            }
        } else {
            is_iter_impl = false;
        }

        for m in &ib.methods {
            self.methods
                .entry(ib.type_name.clone())
                .or_default()
                .push(m.clone());
            if is_iter_impl {
                self.declare_method_sig_by_ptr(&ib.type_name, m);
            } else {
                self.declare_method_sig(&ib.type_name, m);
            }
        }

        Ok(())
    }

    pub(crate) fn infer_param_types(&mut self, _prog: &ast::Program) {
        // Phase 3A: Infer `self` param type for standalone method functions.
        // When a function is named `TypeName_method` and has a `self` param with
        // a TypeVar type, unify the TypeVar with Type::Struct(TypeName). This
        // ensures the function is not overly polymorphic and can be emitted
        // directly without monomorphization.
        let struct_names: Vec<String> = self.structs.keys().cloned().collect();
        let fn_keys: Vec<String> = self.fns.keys().cloned().collect();
        for fname in &fn_keys {
            for sname in &struct_names {
                let prefix = format!("{}_", sname);
                if fname.starts_with(&prefix) && fname.len() > prefix.len() {
                    // Check if first param is a TypeVar (unannotated self)
                    if let Some((_, ptys, _)) = self.fns.get(fname) {
                        if let Some(Type::TypeVar(_)) = ptys.first() {
                            // Verify the AST param is named "self"
                            if let Some(ast_fn) = self.inferable_fns.get(fname) {
                                if ast_fn.params.first().map_or(false, |p| p.name == "self") {
                                    let self_ty = Type::Struct(sname.clone());
                                    let tv = ptys[0].clone();
                                    let _ = self.infer_ctx.unify(&tv, &self_ty);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Normalize: shallow-resolve solved TypeVar chains
        let keys: Vec<String> = self.fns.keys().cloned().collect();
        for k in keys {
            let entry = self.fns.get_mut(&k).unwrap();
            for ty in &mut entry.1 {
                if matches!(ty, Type::TypeVar(_)) {
                    *ty = self.infer_ctx.shallow_resolve(ty);
                }
            }
            if entry.2.has_type_var() {
                entry.2 = self.infer_ctx.shallow_resolve(&entry.2);
            }
        }

        if self.debug_types {
            eprintln!("[type:resolved] final signatures:");
            let mut names: Vec<String> = self.fns.keys().cloned().collect();
            names.sort();
            for name in &names {
                let (_, ptys, ret) = &self.fns[name];
                eprintln!(
                    "  {} :: ({}) -> {}",
                    name,
                    ptys.iter()
                        .map(|t| format!("{t}"))
                        .collect::<Vec<_>>()
                        .join(", "),
                    ret
                );
            }
        }
    }
}
