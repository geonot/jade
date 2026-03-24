use std::collections::HashMap;

use crate::ast;
use crate::hir::{self, DefId, Ownership};
use crate::types::Type;

use super::{Typer, VarInfo};

impl Typer {
    pub(crate) fn substitute_type(ty: &Type, type_map: &HashMap<String, Type>) -> Type {
        match ty {
            Type::Param(n) => type_map.get(n).cloned().unwrap_or_else(|| ty.clone()),
            Type::Array(inner, sz) => {
                Type::Array(Box::new(Self::substitute_type(inner, type_map)), *sz)
            }
            Type::Tuple(tys) => Type::Tuple(
                tys.iter()
                    .map(|t| Self::substitute_type(t, type_map))
                    .collect(),
            ),
            Type::Fn(ptys, ret) => Type::Fn(
                ptys.iter()
                    .map(|t| Self::substitute_type(t, type_map))
                    .collect(),
                Box::new(Self::substitute_type(ret, type_map)),
            ),
            Type::Ptr(inner) => Type::Ptr(Box::new(Self::substitute_type(inner, type_map))),
            Type::Rc(inner) => Type::Rc(Box::new(Self::substitute_type(inner, type_map))),
            _ => ty.clone(),
        }
    }

    pub(crate) fn mangle_generic(
        base: &str,
        type_map: &HashMap<String, Type>,
        type_params: &[String],
    ) -> String {
        let mut name = base.to_string();
        for tp in type_params {
            if let Some(ty) = type_map.get(tp) {
                name = format!("{name}_{ty}");
            }
        }
        name
    }

    pub(crate) fn effective_type_params(f: &ast::Fn) -> Vec<String> {
        if !f.type_params.is_empty() {
            return f.type_params.clone();
        }
        let mut tps = Vec::new();
        for (i, p) in f.params.iter().enumerate() {
            if p.ty.is_none() {
                let name = format!("__{i}");
                if !tps.contains(&name) {
                    tps.push(name);
                }
            }
            if let Some(ty) = &p.ty {
                Self::collect_type_params_from(ty, &mut tps);
            }
        }
        if let Some(ret) = &f.ret {
            Self::collect_type_params_from(ret, &mut tps);
        }
        tps
    }

    pub(crate) fn collect_type_params_from(ty: &Type, out: &mut Vec<String>) {
        match ty {
            Type::Param(n) => {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Type::Array(inner, _) | Type::Ptr(inner) | Type::Rc(inner) => {
                Self::collect_type_params_from(inner, out);
            }
            Type::Tuple(tys) => {
                for t in tys {
                    Self::collect_type_params_from(t, out);
                }
            }
            Type::Fn(ptys, ret) => {
                for t in ptys {
                    Self::collect_type_params_from(t, out);
                }
                Self::collect_type_params_from(ret, out);
            }
            _ => {}
        }
    }

    pub(crate) fn is_generic_fn(f: &ast::Fn) -> bool {
        !Self::effective_type_params(f).is_empty()
    }

    pub(crate) fn normalize_generic_fn(f: &ast::Fn) -> ast::Fn {
        let mut gf = f.clone();
        gf.type_params = Self::effective_type_params(f);
        for (i, p) in gf.params.iter_mut().enumerate() {
            if p.ty.is_none() {
                p.ty = Some(Type::Param(format!("__{i}")));
            }
        }
        gf
    }

    pub(crate) fn monomorphize_fn(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        if self.mono_depth >= 64 {
            return Err(format!(
                "monomorphization depth limit exceeded for '{name}' (possible infinite recursion in generics)"
            ));
        }
        self.mono_depth += 1;
        let result = self.monomorphize_fn_inner(name, type_map);
        self.mono_depth -= 1;
        result
    }

    fn monomorphize_fn_inner(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        let gf = self
            .generic_fns
            .get(name)
            .ok_or_else(|| format!("no generic fn: {name}"))?
            .clone();
        if let Some(bounds) = self.generic_bounds.get(name).cloned() {
            for (param, required_traits) in &bounds {
                if let Some(concrete_ty) = type_map.get(param) {
                    let type_name = Self::type_name_for_bound_check(concrete_ty);
                    for trait_name in required_traits {
                        if !self.type_satisfies_trait(&type_name, trait_name) {
                            return Err(format!(
                                "type `{type_name}` does not satisfy trait bound `{trait_name}` \
                                 required by type parameter `{param}` in `{name}`"
                            ));
                        }
                    }
                }
            }
        }

        let mangled = Self::mangle_generic(name, type_map, &gf.type_params);
        if self.fns.contains_key(&mangled) {
            return Ok(mangled);
        }
        let ptys: Vec<Type> = gf
            .params
            .iter()
            .map(|p| {
                let base = p.ty.clone().unwrap_or(Type::I64);
                Self::substitute_type(&base, type_map)
            })
            .collect();
        self.push_scope();
        for (i, p) in gf.params.iter().enumerate() {
            if i < ptys.len() {
                let id = self.fresh_id();
                self.define_var(
                    &p.name,
                    VarInfo {
                        def_id: id,
                        ty: ptys[i].clone(),
                        ownership: Ownership::Owned,
                    },
                );
            }
        }
        let ret = gf
            .ret
            .clone()
            .map(|r| Self::substitute_type(&r, type_map))
            .unwrap_or_else(|| {
                let inferred = self.infer_ret_ast(&gf);
                Self::substitute_type(&inferred, type_map)
            });
        self.pop_scope();
        let id = self.fresh_id();
        self.fns
            .insert(mangled.clone(), (id, ptys.clone(), ret.clone()));

        let mono_fn = self.lower_generic_fn_body(&gf, &mangled, id, &ptys, &ret, name)?;
        self.mono_fns.push(mono_fn);
        Ok(mangled)
    }

    fn lower_generic_fn_body(
        &mut self,
        gf: &ast::Fn,
        mangled: &str,
        def_id: DefId,
        ptys: &[Type],
        ret: &Type,
        origin: &str,
    ) -> Result<hir::Fn, String> {
        let saved_scopes = std::mem::take(&mut self.scopes);
        self.push_scope();

        let mut params = Vec::new();
        for (i, p) in gf.params.iter().enumerate() {
            let pid = self.fresh_id();
            let ty = ptys[i].clone();
            let ownership = Self::ownership_for_type(&ty);
            self.define_var(
                &p.name,
                VarInfo {
                    def_id: pid,
                    ty: ty.clone(),
                    ownership,
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

        let body = self.lower_block(&gf.body, ret)?;

        self.pop_scope();
        self.scopes = saved_scopes;

        Ok(hir::Fn {
            def_id,
            name: mangled.to_string(),
            params,
            ret: ret.clone(),
            body,
            span: gf.span,
            generic_origin: Some(origin.to_string()),
        })
    }

    pub(crate) fn monomorphize_enum(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        let ge = self
            .generic_enums
            .get(name)
            .ok_or_else(|| format!("no generic enum: {name}"))?
            .clone();
        let mangled = Self::mangle_generic(name, type_map, &ge.type_params);
        if self.enums.contains_key(&mangled) {
            return Ok(mangled);
        }
        let mut variants = Vec::new();
        let mut hir_variants = Vec::new();
        for (tag, v) in ge.variants.iter().enumerate() {
            let ftys: Vec<Type> = v
                .fields
                .iter()
                .map(|f| Self::substitute_type(&f.ty, type_map))
                .collect();
            self.variant_tags
                .insert(v.name.clone(), (mangled.clone(), tag as u32));
            let hv = hir::Variant {
                name: v.name.clone(),
                fields: ftys
                    .iter()
                    .enumerate()
                    .map(|(fi, fty)| hir::VField {
                        name: v.fields.get(fi).and_then(|f| f.name.clone()),
                        ty: fty.clone(),
                    })
                    .collect(),
                tag: tag as u32,
                span: v.span,
            };
            hir_variants.push(hv);
            variants.push((v.name.clone(), ftys));
        }
        self.enums.insert(mangled.clone(), variants);
        let hed = hir::EnumDef {
            def_id: self.fresh_id(),
            name: mangled.clone(),
            variants: hir_variants,
            span: ge.span,
        };
        self.mono_enums.push(hed);
        Ok(mangled)
    }

    pub(crate) fn try_monomorphize_generic_variant(
        &mut self,
        variant_name: &str,
        inits: &[ast::FieldInit],
    ) -> Result<Option<String>, String> {
        let found = self.generic_enums.iter().find_map(|(ename, edef)| {
            edef.variants
                .iter()
                .find(|v| v.name == variant_name)
                .map(|v| (ename.clone(), edef.clone(), v.clone()))
        });
        let (enum_name, edef, variant) = match found {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut type_map = HashMap::new();
        for (i, field) in variant.fields.iter().enumerate() {
            if let Type::Param(ref p) = field.ty {
                if let Some(init) = inits.get(i) {
                    type_map.insert(p.clone(), self.expr_ty_ast(&init.value));
                }
            }
        }
        if type_map.is_empty() && !edef.type_params.is_empty() {
            for tp in &edef.type_params {
                type_map.entry(tp.clone()).or_insert(Type::I64);
            }
        }
        let mangled = self.monomorphize_enum(&enum_name, &type_map)?;
        Ok(Some(mangled))
    }

    pub(crate) fn try_monomorphize_generic_variant_bare(
        &mut self,
        variant_name: &str,
    ) -> Result<Option<String>, String> {
        let found = self.generic_enums.iter().find_map(|(ename, edef)| {
            edef.variants
                .iter()
                .find(|v| v.name == variant_name)
                .map(|v| (ename.clone(), edef.clone(), v.clone()))
        });
        let (enum_name, edef, _variant) = match found {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut type_map = HashMap::new();
        for tp in &edef.type_params {
            type_map.entry(tp.clone()).or_insert(Type::I64);
        }
        let mangled = self.monomorphize_enum(&enum_name, &type_map)?;
        Ok(Some(mangled))
    }

    fn type_name_for_bound_check(ty: &Type) -> String {
        match ty {
            Type::I64 => "i64".into(),
            Type::F64 => "f64".into(),
            Type::Bool => "bool".into(),
            Type::String => "String".into(),
            Type::Struct(n) => n.clone(),
            Type::Enum(n) => n.clone(),
            Type::Vec(inner) => format!("Vec_{}", Self::type_name_for_bound_check(inner)),
            Type::Fn(params, ret) => {
                let ps: Vec<_> = params
                    .iter()
                    .map(|p| Self::type_name_for_bound_check(p))
                    .collect();
                format!(
                    "Fn_{}_{}",
                    ps.join("_"),
                    Self::type_name_for_bound_check(ret)
                )
            }
            Type::Param(name) => name.clone(),
            _ => format!("{ty:?}"),
        }
    }

    fn type_satisfies_trait(&self, type_name: &str, trait_name: &str) -> bool {
        if let Some(impls) = self.trait_impls.get(type_name) {
            if impls.contains(&trait_name.to_string()) {
                return true;
            }
        }
        Self::builtin_trait_satisfied(type_name, trait_name)
    }

    fn builtin_trait_satisfied(type_name: &str, trait_name: &str) -> bool {
        match trait_name {
            "Iter" => matches!(type_name, "String") || type_name.starts_with("Vec_"),
            "Display" => matches!(type_name, "i64" | "f64" | "bool" | "String"),
            "Eq" => matches!(type_name, "i64" | "f64" | "bool" | "String"),
            "Ord" => matches!(type_name, "i64" | "f64" | "String"),
            "Add" | "Sub" | "Mul" | "Div" => matches!(type_name, "i64" | "f64"),
            _ => false,
        }
    }
}
