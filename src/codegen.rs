use std::collections::{HashMap, HashSet};
use std::path::Path;

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, FloatPredicate, IntPredicate, OptimizationLevel};

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::types::Type;

macro_rules! b {
    ($e:expr) => {
        $e.map_err(|e| e.to_string())?
    };
}

pub struct Compiler<'ctx> {
    ctx: &'ctx Context,
    module: Module<'ctx>,
    bld: Builder<'ctx>,
    cur_fn: Option<FunctionValue<'ctx>>,
    vars: Vec<HashMap<String, (PointerValue<'ctx>, Type)>>,
    fns: HashMap<String, (FunctionValue<'ctx>, Vec<Type>, Type)>,
    structs: HashMap<String, Vec<(String, Type)>>,
    enums: HashMap<String, Vec<(String, Vec<Type>)>>,
    variant_tags: HashMap<String, (String, u32)>,
    loop_stack: Vec<LoopCtx<'ctx>>,
    generic_fns: HashMap<String, Fn>,
    generic_types: HashMap<String, TypeDef>,
    generic_enums: HashMap<String, EnumDef>,
    methods: HashMap<String, Vec<Fn>>,
    source: String,
    filename: String,
}

struct LoopCtx<'ctx> {
    continue_bb: BasicBlock<'ctx>,
    break_bb: BasicBlock<'ctx>,
}

impl<'ctx> Compiler<'ctx> {
    pub fn new(ctx: &'ctx Context, name: &str) -> Self {
        Self {
            module: ctx.create_module(name),
            bld: ctx.create_builder(),
            ctx,
            cur_fn: None,
            vars: vec![HashMap::new()],
            fns: HashMap::new(),
            structs: HashMap::new(),
            enums: HashMap::new(),
            variant_tags: HashMap::new(),
            loop_stack: Vec::new(),
            generic_fns: HashMap::new(),
            generic_types: HashMap::new(),
            generic_enums: HashMap::new(),
            methods: HashMap::new(),
            source: String::new(),
            filename: name.to_string(),
        }
    }

    pub fn set_source(&mut self, src: &str) {
        self.source = src.to_string();
    }

    fn diag_err(&self, msg: &str, span: Span) -> String {
        Diagnostic::error(msg)
            .at(span)
            .render(&self.filename, &self.source)
    }

    fn collect_type_params(ty: &Type, out: &mut Vec<String>) {
        match ty {
            Type::Param(n) => {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
            Type::Array(inner, _) | Type::Ptr(inner) | Type::Rc(inner) => {
                Self::collect_type_params(inner, out)
            }
            Type::Tuple(tys) => {
                for t in tys {
                    Self::collect_type_params(t, out);
                }
            }
            Type::Fn(ptys, ret) => {
                for t in ptys {
                    Self::collect_type_params(t, out);
                }
                Self::collect_type_params(ret, out);
            }
            _ => {}
        }
    }

    fn effective_type_params(f: &Fn) -> Vec<String> {
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
                Self::collect_type_params(ty, &mut tps);
            }
        }
        if let Some(ret) = &f.ret {
            Self::collect_type_params(ret, &mut tps);
        }
        tps
    }

    fn normalize_generic_fn(f: &Fn) -> Fn {
        let mut gf = f.clone();
        gf.type_params = Self::effective_type_params(f);
        for (i, p) in gf.params.iter_mut().enumerate() {
            if p.ty.is_none() {
                p.ty = Some(Type::Param(format!("__{i}")));
            }
        }
        gf
    }

    fn is_generic_fn(f: &Fn) -> bool {
        !Self::effective_type_params(f).is_empty()
    }

    pub fn compile_program(&mut self, prog: &Program) -> Result<(), String> {
        self.setup_target()?;
        self.declare_builtins();
        self.register_prelude_types();
        for d in &prog.decls {
            match d {
                Decl::Fn(f) if Self::is_generic_fn(f) => {
                    self.generic_fns
                        .insert(f.name.clone(), Self::normalize_generic_fn(f));
                }
                Decl::Fn(f) => self.declare_fn(f)?,
                Decl::Type(td) if !td.type_params.is_empty() => {
                    self.generic_types.insert(td.name.clone(), td.clone());
                }
                Decl::Type(td) => {
                    for m in &td.methods {
                        self.methods
                            .entry(td.name.clone())
                            .or_default()
                            .push(m.clone());
                    }
                    self.declare_type(td)?;
                    for m in &td.methods {
                        self.declare_method(&td.name, m)?;
                    }
                }
                Decl::Enum(ed) if !ed.type_params.is_empty() => {
                    self.generic_enums.insert(ed.name.clone(), ed.clone());
                }
                Decl::Enum(ed) => self.declare_enum(ed)?,
                Decl::Extern(ef) => self.declare_extern(ef)?,
                Decl::Use(_) => {}
                Decl::ErrDef(ed) => self.declare_err_def(ed)?,
            }
        }
        for d in &prog.decls {
            if let Decl::Fn(f) = d {
                if !Self::is_generic_fn(f) {
                    self.compile_fn(f)?;
                }
            }
            if let Decl::Type(td) = d {
                if td.type_params.is_empty() {
                    for m in &td.methods {
                        let mn = format!("{}_{}", td.name, m.name);
                        if self.fns.contains_key(&mn) {
                            self.compile_method_body(&td.name, &mn, m)?;
                        }
                    }
                }
            }
        }
        self.module.verify().map_err(|e| e.to_string())
    }

    fn setup_target(&self) -> Result<(), String> {
        Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;
        let triple = TargetMachine::get_default_triple();
        self.module.set_triple(&triple);
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        let tm = target
            .create_target_machine(
                &triple,
                TargetMachine::get_host_cpu_name().to_str().unwrap(),
                TargetMachine::get_host_cpu_features().to_str().unwrap(),
                OptimizationLevel::None,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or("failed to create target machine")?;
        self.module
            .set_data_layout(&tm.get_target_data().get_data_layout());
        Ok(())
    }

    fn attr(&self, name: &str) -> Attribute {
        self.ctx
            .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0)
    }

    fn tag_fn(&self, fv: FunctionValue<'ctx>) {
        for a in ["nounwind", "nosync", "nofree", "mustprogress", "willreturn"] {
            fv.add_attribute(AttributeLoc::Function, self.attr(a));
        }
    }

    fn tag_fn_noreturn_ok(&self, fv: FunctionValue<'ctx>) {
        for a in ["nounwind", "nosync", "nofree", "mustprogress"] {
            fv.add_attribute(AttributeLoc::Function, self.attr(a));
        }
    }

    pub fn emit_ir(&self) -> String {
        self.module.print_to_string().to_string()
    }

    pub fn emit_object(&self, path: &Path, opt: OptimizationLevel) -> Result<(), String> {
        let passes = match opt {
            OptimizationLevel::None => "default<O0>",
            OptimizationLevel::Less => "default<O1>",
            OptimizationLevel::Default => "default<O2>",
            OptimizationLevel::Aggressive => "default<O3>",
        };
        let tm = self.target_machine(opt)?;
        let pb = PassBuilderOptions::create();
        pb.set_loop_vectorization(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
        pb.set_loop_slp_vectorization(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
        pb.set_loop_unrolling(matches!(
            opt,
            OptimizationLevel::Default | OptimizationLevel::Aggressive
        ));
        pb.set_merge_functions(matches!(opt, OptimizationLevel::Aggressive));
        self.module
            .run_passes(passes, &tm, pb)
            .map_err(|e| e.to_string())?;
        tm.write_to_file(&self.module, FileType::Object, path)
            .map_err(|e| e.to_string())
    }

    fn declare_builtins(&mut self) {
        let i32t = self.ctx.i32_type();
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let pf = self
            .module
            .add_function("printf", i32t.fn_type(&[ptr.into()], true), None);
        pf.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        pf.add_attribute(AttributeLoc::Function, self.attr("nofree"));
        let pc = self
            .module
            .add_function("putchar", i32t.fn_type(&[i32t.into()], false), None);
        pc.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        pc.add_attribute(AttributeLoc::Function, self.attr("nofree"));
    }

    fn register_prelude_types(&mut self) {
        let s = Span::dummy();
        self.generic_enums
            .entry("Option".into())
            .or_insert_with(|| EnumDef {
                name: "Option".into(),
                type_params: vec!["T".into()],
                variants: vec![
                    Variant {
                        name: "Some".into(),
                        fields: vec![VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    Variant {
                        name: "Nothing".into(),
                        fields: vec![],
                        span: s,
                    },
                ],
                span: s,
            });
        self.generic_enums
            .entry("Result".into())
            .or_insert_with(|| EnumDef {
                name: "Result".into(),
                type_params: vec!["T".into(), "E".into()],
                variants: vec![
                    Variant {
                        name: "Ok".into(),
                        fields: vec![VField {
                            name: None,
                            ty: Type::Param("T".into()),
                        }],
                        span: s,
                    },
                    Variant {
                        name: "Err".into(),
                        fields: vec![VField {
                            name: None,
                            ty: Type::Param("E".into()),
                        }],
                        span: s,
                    },
                ],
                span: s,
            });
    }

    fn declare_fn(&mut self, f: &Fn) -> Result<(), String> {
        let ptys: Vec<Type> = f
            .params
            .iter()
            .map(|p| p.ty.clone().unwrap_or(Type::I64))
            .collect();
        let ret = if f.name == "main" {
            Type::I32
        } else {
            f.ret.clone().unwrap_or_else(|| self.infer_ret(f))
        };
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret, &lp, false);
        let fv = self.module.add_function(&f.name, ft, None);
        if self.fn_may_recurse(f) {
            self.tag_fn_noreturn_ok(fv);
        } else {
            self.tag_fn(fv);
        }
        if f.name != "main" {
            fv.set_linkage(Linkage::Internal);
        }
        for (i, p) in f.params.iter().enumerate() {
            if let Some(v) = fv.get_nth_param(i as u32) {
                v.set_name(&p.name);
            }
            fv.add_attribute(AttributeLoc::Param(i as u32), self.attr("noundef"));
        }
        self.fns.insert(f.name.clone(), (fv, ptys, ret));
        Ok(())
    }

    fn fn_may_recurse(&self, f: &Fn) -> bool {
        let mut idents = HashSet::new();
        Self::collect_idents_block(&f.body, &mut idents);
        idents.contains(&f.name)
    }

    fn declare_method(&mut self, type_name: &str, m: &Fn) -> Result<(), String> {
        let method_name = format!("{type_name}_{}", m.name);
        let self_ty = Type::Struct(type_name.to_string());
        let mut ptys = vec![self_ty];
        for p in &m.params {
            ptys.push(p.ty.clone().unwrap_or(Type::I64));
        }
        let ret = m.ret.clone().unwrap_or_else(|| self.infer_ret(m));
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret, &lp, false);
        let fv = self.module.add_function(&method_name, ft, None);
        self.tag_fn(fv);
        fv.set_linkage(Linkage::Internal);
        for i in 0..ptys.len() {
            fv.add_attribute(AttributeLoc::Param(i as u32), self.attr("noundef"));
        }
        self.fns.insert(method_name, (fv, ptys, ret));
        Ok(())
    }

    fn emit_body(
        &mut self,
        fv: FunctionValue<'ctx>,
        params: &[(String, Type)],
        body: &Block,
        ret: &Type,
    ) -> Result<(), String> {
        self.cur_fn = Some(fv);
        let entry = self.ctx.append_basic_block(fv, "entry");
        self.bld.position_at_end(entry);
        self.vars.push(HashMap::new());
        for (i, (name, ty)) in params.iter().enumerate() {
            let a = self.entry_alloca(self.llvm_ty(ty), name);
            b!(self.bld.build_store(a, fv.get_nth_param(i as u32).unwrap()));
            self.set_var(name, a, ty.clone());
        }
        let last = self.compile_block(body)?;
        if self.no_term() {
            match ret {
                Type::Void => {
                    b!(self.bld.build_return(None));
                }
                _ => {
                    let rty = self.llvm_ty(ret);
                    let v = match last {
                        Some(v) => self.coerce_val(v, rty),
                        _ => self.default_val(ret),
                    };
                    b!(self.bld.build_return(Some(&v)));
                }
            }
        }
        self.vars.pop();
        self.cur_fn = None;
        Ok(())
    }

    fn compile_fn(&mut self, f: &Fn) -> Result<(), String> {
        let (fv, ptys, ret) = self
            .fns
            .get(&f.name)
            .ok_or_else(|| format!("undeclared: {}", f.name))?
            .clone();
        let params: Vec<_> = f
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), ptys[i].clone()))
            .collect();
        self.emit_body(fv, &params, &f.body, &ret)
    }

    fn declare_type(&mut self, td: &TypeDef) -> Result<(), String> {
        let fields: Vec<(String, Type)> = td
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.ty.clone().unwrap_or_else(|| self.infer_field_ty(f)),
                )
            })
            .collect();
        let ltys: Vec<BasicTypeEnum<'ctx>> = fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
        let st = self.ctx.opaque_struct_type(&td.name);
        st.set_body(&ltys, false);
        self.structs.insert(td.name.clone(), fields);
        Ok(())
    }

    fn declare_enum(&mut self, ed: &EnumDef) -> Result<(), String> {
        let i32t = self.ctx.i32_type();
        let ptr_size = 8usize;
        let mut variants = Vec::new();
        let mut max_payload = 0usize;
        for (tag, v) in ed.variants.iter().enumerate() {
            let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
            let payload_bytes: usize = ftys
                .iter()
                .map(|t| {
                    if Self::is_recursive_field(t, &ed.name) {
                        // Recursive fields are boxed (stored as pointers)
                        ptr_size
                    } else {
                        let lty = self.llvm_ty(t);
                        self.type_store_size(lty) as usize
                    }
                })
                .sum();
            max_payload = max_payload.max(payload_bytes);
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), ftys));
        }
        let payload_ty = self.ctx.i8_type().array_type(max_payload as u32);
        let st = self.ctx.opaque_struct_type(&ed.name);
        st.set_body(&[i32t.into(), payload_ty.into()], false);
        self.enums.insert(ed.name.clone(), variants);
        Ok(())
    }

    fn declare_extern(&mut self, ef: &ExternFn) -> Result<(), String> {
        let ptys: Vec<BasicMetadataTypeEnum<'ctx>> = ef
            .params
            .iter()
            .map(|(_, t)| self.llvm_ty(t).into())
            .collect();
        let ret = &ef.ret;
        let ft = self.mk_fn_type(ret, &ptys, ef.variadic);
        let fv = self
            .module
            .add_function(&ef.name, ft, Some(Linkage::External));
        fv.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        let param_tys: Vec<Type> = ef.params.iter().map(|(_, t)| t.clone()).collect();
        self.fns
            .insert(ef.name.clone(), (fv, param_tys, ret.clone()));
        Ok(())
    }

    fn compile_method_body(
        &mut self,
        _type_name: &str,
        mangled: &str,
        m: &Fn,
    ) -> Result<(), String> {
        let (fv, ptys, ret) = self
            .fns
            .get(mangled)
            .ok_or_else(|| format!("undeclared: {mangled}"))?
            .clone();
        let mut params = vec![("self".to_string(), ptys[0].clone())];
        for (i, p) in m.params.iter().enumerate() {
            params.push((p.name.clone(), ptys[i + 1].clone()));
        }
        self.emit_body(fv, &params, &m.body, &ret)
    }

    fn substitute_type(ty: &Type, type_map: &HashMap<String, Type>) -> Type {
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

    fn mangle_generic(
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

    fn monomorphize_enum(
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
        let specialized = EnumDef {
            name: mangled.clone(),
            type_params: vec![],
            variants: ge
                .variants
                .iter()
                .map(|v| Variant {
                    name: v.name.clone(),
                    fields: v
                        .fields
                        .iter()
                        .map(|f| VField {
                            name: f.name.clone(),
                            ty: Self::substitute_type(&f.ty, type_map),
                        })
                        .collect(),
                    span: v.span,
                })
                .collect(),
            span: ge.span,
        };
        self.declare_enum(&specialized)?;
        Ok(mangled)
    }

    fn try_monomorphize_generic_variant(
        &mut self,
        variant_name: &str,
        inits: &[FieldInit],
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
                    type_map.insert(p.clone(), self.expr_ty(&init.value));
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

    fn monomorphize_fn(
        &mut self,
        name: &str,
        type_map: &HashMap<String, Type>,
    ) -> Result<String, String> {
        let gf = self
            .generic_fns
            .get(name)
            .ok_or_else(|| format!("no generic fn: {name}"))?
            .clone();
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
        let ret = gf
            .ret
            .clone()
            .map(|r| Self::substitute_type(&r, type_map))
            .unwrap_or_else(|| {
                let inferred = self.infer_ret(&gf);
                Self::substitute_type(&inferred, type_map)
            });
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret, &lp, false);
        let fv = self.module.add_function(&mangled, ft, None);
        if self.fn_may_recurse(&gf) {
            self.tag_fn_noreturn_ok(fv);
        } else {
            self.tag_fn(fv);
        }
        fv.set_linkage(Linkage::Internal);
        for (i, _) in gf.params.iter().enumerate() {
            fv.add_attribute(AttributeLoc::Param(i as u32), self.attr("noundef"));
        }
        self.fns
            .insert(mangled.clone(), (fv, ptys.clone(), ret.clone()));
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        let saved_vars = std::mem::replace(&mut self.vars, vec![HashMap::new()]);
        let params: Vec<_> = gf
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), ptys[i].clone()))
            .collect();
        self.emit_body(fv, &params, &gf.body, &ret)?;
        self.vars = saved_vars;
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        Ok(mangled)
    }

    fn compile_block(&mut self, block: &Block) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let mut last = None;
        for s in block {
            last = self.compile_stmt(s)?;
        }
        Ok(last)
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        match stmt {
            Stmt::Bind(b) => {
                let val = self.compile_expr(&b.value)?;
                let ty = self.expr_ty(&b.value);
                if matches!(ty, Type::Array(_, _)) {
                    self.set_var(&b.name, val.into_pointer_value(), ty);
                } else if let Some((ptr, _)) = self.find_var(&b.name).cloned() {
                    b!(self.bld.build_store(ptr, val));
                    self.set_var(&b.name, ptr, ty);
                } else {
                    let a = self.entry_alloca(self.llvm_ty(&ty), &b.name);
                    b!(self.bld.build_store(a, val));
                    self.set_var(&b.name, a, ty);
                }
                Ok(None)
            }
            Stmt::TupleBind(names, value, _) => {
                let val = self.compile_expr(value)?;
                let tup_ty = self.expr_ty(value);
                let tys = match &tup_ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => return Err("tuple bind requires a tuple value".into()),
                };
                if names.len() != tys.len() {
                    return Err(format!(
                        "tuple bind: {} names but tuple has {} elements",
                        names.len(),
                        tys.len()
                    ));
                }
                let st = self.ctx.struct_type(
                    &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                    false,
                );
                let tmp = self.entry_alloca(st.into(), "tup.tmp");
                b!(self.bld.build_store(tmp, val));
                for (i, (name, ety)) in names.iter().zip(tys.iter()).enumerate() {
                    let lty = self.llvm_ty(ety);
                    let gep = b!(self.bld.build_struct_gep(st, tmp, i as u32, "tup.d"));
                    let elem = b!(self.bld.build_load(lty, gep, name));
                    let a = self.entry_alloca(lty, name);
                    b!(self.bld.build_store(a, elem));
                    self.set_var(name, a, ety.clone());
                }
                Ok(None)
            }
            Stmt::Assign(target, value, _) => {
                self.compile_assign(target, value)?;
                Ok(None)
            }
            Stmt::Expr(e) => Ok(Some(self.compile_expr(e)?)),
            Stmt::If(i) => self.compile_if(i),
            Stmt::While(w) => self.compile_while(w),
            Stmt::For(f) => self.compile_for(f),
            Stmt::Loop(l) => self.compile_loop(l),
            Stmt::Ret(v, _) => {
                if let Some(e) = v {
                    let val = self.compile_expr(e)?;
                    let val = if let Some(rt) = self.cur_fn.unwrap().get_type().get_return_type() {
                        self.coerce_val(val, rt)
                    } else {
                        val
                    };
                    b!(self.bld.build_return(Some(&val)));
                } else {
                    b!(self.bld.build_return(None));
                }
                Ok(None)
            }
            Stmt::Break(_, _) => {
                if let Some(lctx) = self.loop_stack.last() {
                    let bb = lctx.break_bb;
                    b!(self.bld.build_unconditional_branch(bb));
                }
                Ok(None)
            }
            Stmt::Continue(_) => {
                if let Some(lctx) = self.loop_stack.last() {
                    let bb = lctx.continue_bb;
                    b!(self.bld.build_unconditional_branch(bb));
                }
                Ok(None)
            }
            Stmt::Match(m) => self.compile_match(m),
            Stmt::Asm(a) => self.compile_asm(a),
            Stmt::ErrReturn(e, _) => {
                let val = self.compile_expr(e)?;
                b!(self.bld.build_return(Some(&val)));
                Ok(None)
            }
        }
    }

    fn compile_if(&mut self, i: &If) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let merge = self.ctx.append_basic_block(fv, "merge");
        let cv = self.compile_expr(&i.cond)?;
        let cond = self.to_bool(cv);
        let then_bb = self.ctx.append_basic_block(fv, "then");
        let mut else_bb = self.ctx.append_basic_block(fv, "else");
        b!(self.bld.build_conditional_branch(cond, then_bb, else_bb));

        let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut all_valued = i.els.is_some();

        // Then
        self.bld.position_at_end(then_bb);
        let then_val = self.compile_block(&i.then)?;
        if self.no_term() {
            let bb = self.bld.get_insert_block().unwrap();
            match then_val {
                Some(v) => phi_in.push((v, bb)),
                None => all_valued = false,
            }
            b!(self.bld.build_unconditional_branch(merge));
        } else {
            all_valued = false;
        }

        // Elifs
        for (elif_cond, elif_body) in &i.elifs {
            self.bld.position_at_end(else_bb);
            let cv = self.compile_expr(elif_cond)?;
            let c = self.to_bool(cv);
            let elif_then = self.ctx.append_basic_block(fv, "elif.then");
            let next_else = self.ctx.append_basic_block(fv, "elif.else");
            b!(self.bld.build_conditional_branch(c, elif_then, next_else));
            self.bld.position_at_end(elif_then);
            let elif_val = self.compile_block(elif_body)?;
            if self.no_term() {
                let bb = self.bld.get_insert_block().unwrap();
                match elif_val {
                    Some(v) => phi_in.push((v, bb)),
                    None => all_valued = false,
                }
                b!(self.bld.build_unconditional_branch(merge));
            } else {
                all_valued = false;
            }
            else_bb = next_else;
        }

        // Else
        self.bld.position_at_end(else_bb);
        if let Some(ref els) = i.els {
            let else_val = self.compile_block(els)?;
            if self.no_term() {
                let bb = self.bld.get_insert_block().unwrap();
                match else_val {
                    Some(v) => phi_in.push((v, bb)),
                    None => all_valued = false,
                }
                b!(self.bld.build_unconditional_branch(merge));
            } else {
                all_valued = false;
            }
        } else {
            if self.no_term() {
                b!(self.bld.build_unconditional_branch(merge));
            }
        }

        self.bld.position_at_end(merge);
        self.build_match_phi(&phi_in, all_valued)
    }

    fn compile_while(&mut self, w: &While) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(fv, "wh.cond");
        let body_bb = self.ctx.append_basic_block(fv, "wh.body");
        let end_bb = self.ctx.append_basic_block(fv, "wh.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let wv = self.compile_expr(&w.cond)?;
        let c = self.to_bool(wv);
        b!(self.bld.build_conditional_branch(c, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(LoopCtx {
            continue_bb: cond_bb,
            break_bb: end_bb,
        });
        self.compile_block(&w.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(cond_bb));
        }
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn compile_for(&mut self, f: &For) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if f.end.is_none() && f.step.is_none() {
            let iter_ty = self.expr_ty(&f.iter);
            if let Type::Array(elem_ty, len) = iter_ty {
                return self.compile_for_array(f, &elem_ty, len);
            }
        }
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let start_val = if f.end.is_some() {
            self.compile_expr(&f.iter)?
        } else {
            i64t.const_int(0, false).into()
        };
        let end_val = if let Some(end) = &f.end {
            self.compile_expr(end)?
        } else {
            self.compile_expr(&f.iter)?
        };
        let step_val = if let Some(step) = &f.step {
            self.compile_expr(step)?
        } else {
            i64t.const_int(1, false).into()
        };
        let a = self.entry_alloca(i64t.into(), &f.bind);
        b!(self.bld.build_store(a, start_val));
        self.set_var(&f.bind, a, Type::I64);
        let cond_bb = self.ctx.append_basic_block(fv, "for.cond");
        let body_bb = self.ctx.append_basic_block(fv, "for.body");
        let inc_bb = self.ctx.append_basic_block(fv, "for.inc");
        let end_bb = self.ctx.append_basic_block(fv, "for.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let cur = b!(self.bld.build_load(i64t, a, "i"));
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::SLT,
            cur.into_int_value(),
            end_val.into_int_value(),
            "for.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }
        self.bld.position_at_end(inc_bb);
        let cur = b!(self.bld.build_load(i64t, a, "i"));
        let next =
            b!(self
                .bld
                .build_int_nsw_add(cur.into_int_value(), step_val.into_int_value(), "inc"));
        b!(self.bld.build_store(a, next));
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn compile_for_array(
        &mut self,
        f: &For,
        elem_ty: &Type,
        len: usize,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let arr_ptr = if let Expr::Ident(ref n, _) = f.iter {
            self.find_var(n)
                .map(|(p, _)| *p)
                .ok_or_else(|| format!("undefined: {n}"))?
        } else {
            self.compile_expr(&f.iter)?.into_pointer_value()
        };
        let lty = self.llvm_ty(elem_ty);
        let arr_ty = lty.array_type(len as u32);
        let idx_alloca = self.entry_alloca(i64t.into(), "__idx");
        b!(self.bld.build_store(idx_alloca, i64t.const_int(0, false)));
        let elem_alloca = self.entry_alloca(lty, &f.bind);
        self.set_var(&f.bind, elem_alloca, elem_ty.clone());
        let cond_bb = self.ctx.append_basic_block(fv, "for.cond");
        let body_bb = self.ctx.append_basic_block(fv, "for.body");
        let inc_bb = self.ctx.append_basic_block(fv, "for.inc");
        let end_bb = self.ctx.append_basic_block(fv, "for.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::ULT,
            idx,
            i64t.const_int(len as u64, false),
            "for.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        let gep = unsafe {
            b!(self.bld.build_gep(
                arr_ty,
                arr_ptr,
                &[i64t.const_int(0, false), idx],
                "elem.ptr"
            ))
        };
        let elem = b!(self.bld.build_load(lty, gep, "elem"));
        b!(self.bld.build_store(elem_alloca, elem));
        self.loop_stack.push(LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });
        self.compile_block(&f.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }
        self.bld.position_at_end(inc_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let next = b!(self
            .bld
            .build_int_nuw_add(idx, i64t.const_int(1, false), "inc"));
        b!(self.bld.build_store(idx_alloca, next));
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn compile_loop(&mut self, l: &Loop) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let body_bb = self.ctx.append_basic_block(fv, "loop");
        let end_bb = self.ctx.append_basic_block(fv, "loop.end");
        b!(self.bld.build_unconditional_branch(body_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(LoopCtx {
            continue_bb: body_bb,
            break_bb: end_bb,
        });
        self.compile_block(&l.body)?;
        self.loop_stack.pop();
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(body_bb));
        }
        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    fn check_exhaustive(&self, m: &Match, subject_ty: &Type) -> Result<(), String> {
        let has_wild = m.arms.iter().any(|a| matches!(&a.pat, Pat::Wild(_) | Pat::Ident(_, _) if !self.variant_tags.contains_key(match &a.pat { Pat::Ident(n, _) => n.as_str(), _ => "" })));
        if has_wild {
            return Ok(());
        }
        let enum_name = match subject_ty {
            Type::Enum(n) | Type::Struct(n) if self.enums.contains_key(n) => n,
            _ => return Ok(()),
        };
        let variants = self
            .enums
            .get(enum_name)
            .ok_or_else(|| format!("undefined enum: {enum_name}"))?;
        let covered: Vec<&str> = m
            .arms
            .iter()
            .filter_map(|a| match &a.pat {
                Pat::Ctor(n, _, _) => Some(n.as_str()),
                Pat::Ident(n, _) if self.variant_tags.contains_key(n) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        let missing: Vec<&str> = variants
            .iter()
            .filter(|(n, _)| !covered.contains(&n.as_str()))
            .map(|(n, _)| n.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(self.diag_err(
                &format!(
                    "non-exhaustive match on {enum_name}: missing {}",
                    missing.join(", ")
                ),
                m.span,
            ));
        }
        Ok(())
    }

    fn compile_match(&mut self, m: &Match) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let subject_val = self.compile_expr(&m.subject)?;
        let subject_ty = self.resolve_ty(self.expr_ty(&m.subject));
        self.check_exhaustive(m, &subject_ty)?;
        let is_enum = matches!(subject_ty, Type::Enum(_))
            || matches!(&subject_ty, Type::Struct(n) if self.enums.contains_key(n));
        if !is_enum {
            let merge_bb = self.ctx.append_basic_block(fv, "match.end");
            let mut default_bb = None;
            let arm_bbs: Vec<_> = m
                .arms
                .iter()
                .enumerate()
                .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.arm{i}")))
                .collect();
            let iv = subject_val.into_int_value();
            let mut cases = Vec::new();
            for (i, arm) in m.arms.iter().enumerate() {
                match &arm.pat {
                    Pat::Lit(Expr::Int(n, _)) => {
                        cases.push((self.ctx.i64_type().const_int(*n as u64, true), arm_bbs[i]));
                    }
                    Pat::Wild(_) | Pat::Ident(_, _) => {
                        default_bb = Some(arm_bbs[i]);
                    }
                    _ => return Err("unsupported match pattern".into()),
                }
            }
            let def =
                default_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "match.unreach"));
            b!(self.bld.build_switch(iv, def, &cases));
            if default_bb.is_none() {
                self.bld.position_at_end(def);
                b!(self.bld.build_unreachable());
            }
            let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
            let mut all_valued = true;
            for (i, arm) in m.arms.iter().enumerate() {
                self.bld.position_at_end(arm_bbs[i]);
                if let Pat::Ident(ref name, _) = arm.pat {
                    let ty = self.expr_ty(&m.subject);
                    let a = self.entry_alloca(self.llvm_ty(&ty), name);
                    b!(self.bld.build_store(a, subject_val));
                    self.set_var(name, a, ty);
                }
                let arm_val = self.compile_block(&arm.body)?;
                let cur_bb = self.bld.get_insert_block().unwrap();
                if self.no_term() {
                    match arm_val {
                        Some(v) => phi_in.push((v, cur_bb)),
                        None => all_valued = false,
                    }
                    b!(self.bld.build_unconditional_branch(merge_bb));
                }
            }
            self.bld.position_at_end(merge_bb);
            return self.build_match_phi(&phi_in, all_valued);
        }
        let enum_name = match &subject_ty {
            Type::Enum(n) | Type::Struct(n) => n.clone(),
            _ => unreachable!(),
        };
        let st = self
            .module
            .get_struct_type(&enum_name)
            .ok_or_else(|| format!("no LLVM type: {enum_name}"))?;
        let sub_ptr = self.entry_alloca(st.into(), "match.sub");
        b!(self.bld.build_store(sub_ptr, subject_val));
        let tag_gep = b!(self.bld.build_struct_gep(st, sub_ptr, 0, "tag.ptr"));
        let tag_val = b!(self.bld.build_load(self.ctx.i32_type(), tag_gep, "tag")).into_int_value();
        let merge_bb = self.ctx.append_basic_block(fv, "match.end");
        let arm_bbs: Vec<_> = m
            .arms
            .iter()
            .enumerate()
            .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.arm{i}")))
            .collect();
        let mut cases = Vec::new();
        let mut default_bb = None;
        for (i, arm) in m.arms.iter().enumerate() {
            match &arm.pat {
                Pat::Ctor(vname, _, _) => {
                    if let Some((_, tag)) = self.variant_tags.get(vname) {
                        cases.push((
                            self.ctx.i32_type().const_int(*tag as u64, false),
                            arm_bbs[i],
                        ));
                    }
                }
                Pat::Ident(name, _) if self.variant_tags.contains_key(name) => {
                    let (_, tag) = self.variant_tags[name];
                    cases.push((self.ctx.i32_type().const_int(tag as u64, false), arm_bbs[i]));
                }
                Pat::Wild(_) | Pat::Ident(_, _) => {
                    default_bb = Some(arm_bbs[i]);
                }
                _ => return Err("unsupported pattern in enum match".into()),
            }
        }
        let def = default_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "match.unreach"));
        b!(self.bld.build_switch(tag_val, def, &cases));
        if default_bb.is_none() {
            self.bld.position_at_end(def);
            b!(self.bld.build_unreachable());
        }
        let variants = self
            .enums
            .get(&enum_name)
            .cloned()
            .ok_or_else(|| format!("undefined enum: {enum_name}"))?;
        let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut all_valued = true;
        for (i, arm) in m.arms.iter().enumerate() {
            self.bld.position_at_end(arm_bbs[i]);
            self.vars.push(HashMap::new());
            if let Pat::Ctor(vname, sub_pats, _) = &arm.pat {
                let ftys: Vec<Type> = variants
                    .iter()
                    .find(|(n, _)| n == vname)
                    .map(|(_, f)| f.clone())
                    .unwrap_or_default();
                if !sub_pats.is_empty() {
                    let payload_gep = b!(self.bld.build_struct_gep(st, sub_ptr, 1, "payload"));
                    let mut offset = 0u64;
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    for (j, pat) in sub_pats.iter().enumerate() {
                        let fty = ftys.get(j).cloned().unwrap_or(Type::I64);
                        let is_rec = Self::is_recursive_field(&fty, &enum_name);
                        let field_ptr = if offset == 0 {
                            payload_gep
                        } else {
                            unsafe {
                                b!(self.bld.build_gep(
                                    self.ctx.i8_type(),
                                    payload_gep,
                                    &[self.ctx.i64_type().const_int(offset, false)],
                                    "fptr"
                                ))
                            }
                        };
                        if is_rec {
                            // Unbox: load pointer from payload, load value through pointer
                            let heap_ptr = b!(self
                                .bld
                                .build_load(ptr_ty, field_ptr, "box.ptr"))
                            .into_pointer_value();
                            let actual_ty = self.llvm_ty(&fty);
                            let fval =
                                b!(self.bld.build_load(actual_ty, heap_ptr, "field"));
                            if let Pat::Ident(bname, _) = pat {
                                let a = self.entry_alloca(actual_ty, bname);
                                b!(self.bld.build_store(a, fval));
                                self.set_var(bname, a, fty);
                            }
                            offset += 8; // pointer size
                        } else {
                            let lty = self.llvm_ty(&fty);
                            let fval = b!(self.bld.build_load(lty, field_ptr, "field"));
                            if let Pat::Ident(bname, _) = pat {
                                let a = self.entry_alloca(lty, bname);
                                b!(self.bld.build_store(a, fval));
                                self.set_var(bname, a, fty);
                            }
                            offset += self.type_store_size(lty);
                        }
                    }
                }
            } else if let Pat::Ident(ref name, _) = arm.pat {
                let a = self.entry_alloca(st.into(), name);
                b!(self.bld.build_store(a, subject_val));
                self.set_var(name, a, subject_ty.clone());
            }
            let arm_val = self.compile_block(&arm.body)?;
            self.vars.pop();
            let cur_bb = self.bld.get_insert_block().unwrap();
            if self.no_term() {
                match arm_val {
                    Some(v) => phi_in.push((v, cur_bb)),
                    None => all_valued = false,
                }
                b!(self.bld.build_unconditional_branch(merge_bb));
            }
        }
        self.bld.position_at_end(merge_bb);
        self.build_match_phi(&phi_in, all_valued)
    }

    fn build_match_phi(
        &self,
        phi_in: &[(BasicValueEnum<'ctx>, BasicBlock<'ctx>)],
        all_valued: bool,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if all_valued && !phi_in.is_empty() {
            let phi = b!(self.bld.build_phi(phi_in[0].0.get_type(), "match.val"));
            for (v, bb) in phi_in {
                phi.add_incoming(&[(v, *bb)]);
            }
            Ok(Some(phi.as_basic_value()))
        } else {
            Ok(None)
        }
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match expr {
            Expr::Int(n, _) => Ok(self.int_const(*n, &Type::I64)),
            Expr::Float(n, _) => Ok(self.ctx.f64_type().const_float(*n).into()),
            Expr::Str(s, _) => self.compile_str_literal(s),
            Expr::Bool(v, _) => Ok(self.ctx.bool_type().const_int(*v as u64, false).into()),
            Expr::None(_) | Expr::Void(_) => Ok(self.ctx.i64_type().const_int(0, false).into()),
            Expr::Ident(n, _) => {
                if let Some((enum_name, tag)) = self.variant_tags.get(n.as_str()).cloned() {
                    return self.compile_variant(&enum_name, tag, n, &[]);
                }
                if self.try_monomorphize_generic_variant(n, &[])?.is_some() {
                    let (enum_name, tag) =
                        self.variant_tags.get(n.as_str()).cloned().ok_or_else(|| {
                            format!("variant {n} not found after monomorphization")
                        })?;
                    return self.compile_variant(&enum_name, tag, n, &[]);
                }
                self.load_var(n)
            }
            Expr::BinOp(l, op, r, _) => self.compile_binop(l, *op, r),
            Expr::UnaryOp(op, e, _) => self.compile_unary(*op, e),
            Expr::Call(callee, args, _) => self.compile_call(callee, args),
            Expr::Method(obj, m, args, _) => self.compile_method(obj, m, args),
            Expr::Field(obj, f, _) => self.compile_field(obj, f),
            Expr::Index(arr, idx, _) => self.compile_index(arr, idx),
            Expr::Ternary(c, t, e, _) => self.compile_ternary(c, t, e),
            Expr::As(e, ty, _) => self.compile_cast(e, ty),
            Expr::Array(elems, _) => self.compile_array(elems),
            Expr::Tuple(elems, _) => self.compile_tuple(elems),
            Expr::Struct(name, inits, _) => self.compile_struct(name, inits),
            Expr::IfExpr(i) => {
                match self.compile_if(i)? {
                    Some(v) => Ok(v),
                    None => Ok(self.ctx.i64_type().const_int(0, false).into()),
                }
            }
            Expr::Pipe(left, right, _, _) => self.compile_pipe(left, right),
            Expr::Block(_, _) => Err("block expressions not yet implemented".into()),
            Expr::Lambda(params, ret, body, _) => self.compile_lambda(params, ret, body),
            Expr::Placeholder(_) => Err("$ placeholder outside pipeline".into()),
            Expr::Ref(inner, _) => self.compile_ref(inner),
            Expr::Deref(inner, _) => self.compile_deref(inner),
            Expr::ListComp(body, bind, start, end, cond, _) => {
                self.compile_list_comp(body, bind, start, end.as_deref(), cond.as_deref())
            }
            Expr::Syscall(args, _) => self.compile_syscall(args),
        }
    }

    fn compile_binop(
        &mut self,
        left: &Expr,
        op: BinOp,
        right: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if matches!(op, BinOp::And) {
            return self.compile_short_circuit(left, right, true);
        }
        if matches!(op, BinOp::Or) {
            return self.compile_short_circuit(left, right, false);
        }
        let lty = self.expr_ty(left);
        let rty = self.expr_ty(right);
        let (lhs, rhs) = if let Expr::Int(n, _) = left {
            if rty.is_int() {
                (self.int_const(*n, &rty), self.compile_expr(right)?)
            } else {
                (self.compile_expr(left)?, self.compile_expr(right)?)
            }
        } else if let Expr::Int(n, _) = right {
            if lty.is_int() {
                (self.compile_expr(left)?, self.int_const(*n, &lty))
            } else {
                (self.compile_expr(left)?, self.compile_expr(right)?)
            }
        } else {
            let (lhs, rhs) = (self.compile_expr(left)?, self.compile_expr(right)?);
            if lty.is_int() && rty.is_int() && lty.bits() != rty.bits() {
                self.coerce_int_width(lhs, rhs, &lty, &rty)
            } else {
                (lhs, rhs)
            }
        };
        let ety = if matches!(left, Expr::Int(..)) && rty.is_int() {
            &rty
        } else {
            &lty
        };
        if matches!(ety, Type::String) && matches!(op, BinOp::Add) {
            return self.string_concat(lhs, rhs);
        }
        if ety.is_float() {
            let (l, r) = (lhs.into_float_value(), rhs.into_float_value());
            Ok(match op {
                BinOp::Add => b!(self.bld.build_float_add(l, r, "fadd")).into(),
                BinOp::Sub => b!(self.bld.build_float_sub(l, r, "fsub")).into(),
                BinOp::Mul => b!(self.bld.build_float_mul(l, r, "fmul")).into(),
                BinOp::Div => b!(self.bld.build_float_div(l, r, "fdiv")).into(),
                BinOp::Mod => b!(self.bld.build_float_rem(l, r, "fmod")).into(),
                BinOp::Exp => {
                    let f64t = self.ctx.f64_type();
                    let pt = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                    let pf = self
                        .module
                        .get_function("llvm.pow.f64")
                        .unwrap_or_else(|| self.module.add_function("llvm.pow.f64", pt, None));
                    b!(self.bld.build_call(pf, &[l.into(), r.into()], "pow"))
                        .try_as_basic_value()
                        .basic()
                        .unwrap()
                }
                BinOp::Eq => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::OEQ, l, r, "feq"))
                .into(),
                BinOp::Ne => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::ONE, l, r, "fne"))
                .into(),
                BinOp::Lt => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::OLT, l, r, "flt"))
                .into(),
                BinOp::Gt => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::OGT, l, r, "fgt"))
                .into(),
                BinOp::Le => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::OLE, l, r, "fle"))
                .into(),
                BinOp::Ge => b!(self
                    .bld
                    .build_float_compare(FloatPredicate::OGE, l, r, "fge"))
                .into(),
                _ => return Err(format!("unsupported float op: {op:?}")),
            })
        } else {
            let (l, r) = (lhs.into_int_value(), rhs.into_int_value());
            let s = ety.is_signed();
            Ok(match op {
                BinOp::Add if s => b!(self.bld.build_int_nsw_add(l, r, "add")).into(),
                BinOp::Add => b!(self.bld.build_int_nuw_add(l, r, "add")).into(),
                BinOp::Sub if s => b!(self.bld.build_int_nsw_sub(l, r, "sub")).into(),
                BinOp::Sub => b!(self.bld.build_int_nuw_sub(l, r, "sub")).into(),
                BinOp::Mul if s => b!(self.bld.build_int_nsw_mul(l, r, "mul")).into(),
                BinOp::Mul => b!(self.bld.build_int_nuw_mul(l, r, "mul")).into(),
                BinOp::Div if s => b!(self.bld.build_int_signed_div(l, r, "sdiv")).into(),
                BinOp::Div => b!(self.bld.build_int_unsigned_div(l, r, "udiv")).into(),
                BinOp::Mod if s => b!(self.bld.build_int_signed_rem(l, r, "srem")).into(),
                BinOp::Mod => b!(self.bld.build_int_unsigned_rem(l, r, "urem")).into(),
                BinOp::Eq => b!(self.bld.build_int_compare(IntPredicate::EQ, l, r, "eq")).into(),
                BinOp::Ne => b!(self.bld.build_int_compare(IntPredicate::NE, l, r, "ne")).into(),
                BinOp::Lt => b!(self.bld.build_int_compare(
                    if s {
                        IntPredicate::SLT
                    } else {
                        IntPredicate::ULT
                    },
                    l,
                    r,
                    "lt"
                ))
                .into(),
                BinOp::Gt => b!(self.bld.build_int_compare(
                    if s {
                        IntPredicate::SGT
                    } else {
                        IntPredicate::UGT
                    },
                    l,
                    r,
                    "gt"
                ))
                .into(),
                BinOp::Le => b!(self.bld.build_int_compare(
                    if s {
                        IntPredicate::SLE
                    } else {
                        IntPredicate::ULE
                    },
                    l,
                    r,
                    "le"
                ))
                .into(),
                BinOp::Ge => b!(self.bld.build_int_compare(
                    if s {
                        IntPredicate::SGE
                    } else {
                        IntPredicate::UGE
                    },
                    l,
                    r,
                    "ge"
                ))
                .into(),
                BinOp::BitAnd => b!(self.bld.build_and(l, r, "and")).into(),
                BinOp::BitOr => b!(self.bld.build_or(l, r, "or")).into(),
                BinOp::BitXor => b!(self.bld.build_xor(l, r, "xor")).into(),
                BinOp::Shl => b!(self.bld.build_left_shift(l, r, "shl")).into(),
                BinOp::Shr => b!(self.bld.build_right_shift(l, r, s, "shr")).into(),
                BinOp::Exp => self.compile_int_pow(l, r)?,
                _ => return Err(format!("unsupported int op: {op:?}")),
            })
        }
    }

    fn compile_int_pow(
        &mut self,
        base: inkwell::values::IntValue<'ctx>,
        exp: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let result_ptr = self.entry_alloca(i64t.into(), "pow.res");
        let base_ptr = self.entry_alloca(i64t.into(), "pow.base");
        let exp_ptr = self.entry_alloca(i64t.into(), "pow.exp");
        b!(self.bld.build_store(result_ptr, i64t.const_int(1, false)));
        b!(self.bld.build_store(base_ptr, base));
        b!(self.bld.build_store(exp_ptr, exp));
        let cond_bb = self.ctx.append_basic_block(fv, "pow.cond");
        let body_bb = self.ctx.append_basic_block(fv, "pow.body");
        let sq_bb = self.ctx.append_basic_block(fv, "pow.sq");
        let end_bb = self.ctx.append_basic_block(fv, "pow.end");
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(
            IntPredicate::SGT,
            e,
            i64t.const_int(0, false),
            "pow.cmp"
        ));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        self.bld.position_at_end(body_bb);
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let odd = b!(self.bld.build_and(e, i64t.const_int(1, false), "odd"));
        let is_odd = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            odd,
            i64t.const_int(0, false),
            "isodd"
        ));
        let mul_bb = self.ctx.append_basic_block(fv, "pow.mul");
        b!(self.bld.build_conditional_branch(is_odd, mul_bb, sq_bb));
        self.bld.position_at_end(mul_bb);
        let r = b!(self.bld.build_load(i64t, result_ptr, "r")).into_int_value();
        let bv = b!(self.bld.build_load(i64t, base_ptr, "b")).into_int_value();
        let nr = b!(self.bld.build_int_nsw_mul(r, bv, "pow.m"));
        b!(self.bld.build_store(result_ptr, nr));
        b!(self.bld.build_unconditional_branch(sq_bb));
        self.bld.position_at_end(sq_bb);
        let bv = b!(self.bld.build_load(i64t, base_ptr, "b")).into_int_value();
        let nb = b!(self.bld.build_int_nsw_mul(bv, bv, "pow.sq"));
        b!(self.bld.build_store(base_ptr, nb));
        let e = b!(self.bld.build_load(i64t, exp_ptr, "e")).into_int_value();
        let ne = b!(self
            .bld
            .build_right_shift(e, i64t.const_int(1, false), false, "pow.shr"));
        b!(self.bld.build_store(exp_ptr, ne));
        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(end_bb);
        Ok(b!(self.bld.build_load(i64t, result_ptr, "pow.result")))
    }

    fn compile_short_circuit(
        &mut self,
        left: &Expr,
        right: &Expr,
        is_and: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let lhs = self.compile_expr(left)?;
        let lbool = self.to_bool(lhs);
        let rhs_bb = self.ctx.append_basic_block(fv, "sc.rhs");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");
        let lhs_bb = self.bld.get_insert_block().unwrap();
        if is_and {
            b!(self.bld.build_conditional_branch(lbool, rhs_bb, merge_bb));
        } else {
            b!(self.bld.build_conditional_branch(lbool, merge_bb, rhs_bb));
        }
        self.bld.position_at_end(rhs_bb);
        let rhs = self.compile_expr(right)?;
        let rbool = self.to_bool(rhs);
        let rhs_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "sc"));
        let short_val = self
            .ctx
            .bool_type()
            .const_int(if is_and { 0 } else { 1 }, false);
        phi.add_incoming(&[(&short_val, lhs_bb), (&rbool, rhs_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_unary(&mut self, op: UnaryOp, expr: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        Ok(match op {
            UnaryOp::Neg => {
                if self.expr_ty(expr).is_float() {
                    b!(self.bld.build_float_neg(val.into_float_value(), "fneg")).into()
                } else {
                    b!(self.bld.build_int_nsw_neg(val.into_int_value(), "neg")).into()
                }
            }
            UnaryOp::Not | UnaryOp::BitNot => {
                b!(self.bld.build_not(val.into_int_value(), "not")).into()
            }
        })
    }

    fn compile_bit_intrinsic(
        &mut self,
        name: &str,
        args: &[Expr],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let intrinsic = match name {
            "popcount" | "clz" | "ctz" | "rotate_left" | "rotate_right" | "bswap" => name,
            _ => return Ok(None),
        };
        if args.is_empty() {
            return Err(format!("{intrinsic}() requires at least one argument"));
        }
        let val = self.compile_expr(&args[0])?;
        let int_val = val.into_int_value();
        let bw = int_val.get_type().get_bit_width();
        let llvm_name = match intrinsic {
            "popcount" => format!("llvm.ctpop.i{bw}"),
            "clz" => format!("llvm.ctlz.i{bw}"),
            "ctz" => format!("llvm.cttz.i{bw}"),
            "rotate_left" => format!("llvm.fshl.i{bw}"),
            "rotate_right" => format!("llvm.fshr.i{bw}"),
            "bswap" => format!("llvm.bswap.i{bw}"),
            _ => unreachable!(),
        };
        let it = int_val.get_type();
        match intrinsic {
            "popcount" | "bswap" => {
                let ft = it.fn_type(&[it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(Some(
                    b!(self.bld.build_call(f, &[int_val.into()], intrinsic))
                        .try_as_basic_value()
                        .basic()
                        .unwrap(),
                ))
            }
            "clz" | "ctz" => {
                let false_val = self.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.ctx.bool_type().into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(Some(
                    b!(self
                        .bld
                        .build_call(f, &[int_val.into(), false_val.into()], intrinsic))
                    .try_as_basic_value()
                    .basic()
                    .unwrap(),
                ))
            }
            "rotate_left" | "rotate_right" => {
                if args.len() < 2 {
                    return Err(format!("{intrinsic}() requires two arguments"));
                }
                let amt = self.compile_expr(&args[1])?.into_int_value();
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self
                    .module
                    .get_function(&llvm_name)
                    .unwrap_or_else(|| self.module.add_function(&llvm_name, ft, None));
                Ok(Some(
                    b!(self.bld.build_call(
                        f,
                        &[int_val.into(), int_val.into(), amt.into()],
                        intrinsic
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap(),
                ))
            }
            _ => Ok(None),
        }
    }

    fn coerced_args(
        &mut self,
        args: &[Expr],
        fv: FunctionValue<'ctx>,
    ) -> Result<Vec<BasicMetadataValueEnum<'ctx>>, String> {
        let ptypes = fv.get_type().get_param_types();
        args.iter()
            .enumerate()
            .map(|(i, e)| {
                let v = self.compile_expr(e)?;
                let v = if let Some(pt) = ptypes.get(i) {
                    self.coerce_val(v, (*pt).try_into().unwrap_or(v.get_type()))
                } else {
                    v
                };
                Ok(v.into())
            })
            .collect()
    }

    fn call_result(&self, csv: inkwell::values::CallSiteValue<'ctx>) -> BasicValueEnum<'ctx> {
        csv.try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into())
    }

    fn mk_fn_type(
        &self,
        ret: &Type,
        params: &[BasicMetadataTypeEnum<'ctx>],
        variadic: bool,
    ) -> inkwell::types::FunctionType<'ctx> {
        match ret {
            Type::Void => self.ctx.void_type().fn_type(params, variadic),
            ty => self.llvm_ty(ty).fn_type(params, variadic),
        }
    }

    fn call_fn(
        &mut self,
        fv: FunctionValue<'ctx>,
        args: &[Expr],
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let a = self.coerced_args(args, fv)?;
        let csv = b!(self.bld.build_call(fv, &a, name));
        Ok(self.call_result(csv))
    }

    fn compile_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let name = match callee {
            Expr::Ident(n, _) => n.clone(),
            _ => {
                let fn_ptr = self.compile_expr(callee)?;
                let fn_ty = self.expr_ty(callee);
                return self.indirect_call(fn_ptr, &fn_ty, args);
            }
        };
        if name == "log" {
            return self.compile_log(args);
        }
        if name == "to_string" && args.len() == 1 {
            return self.compile_to_string(&args[0]);
        }
        if name == "rc" && args.len() == 1 {
            let val = self.compile_expr(&args[0])?;
            let inner_ty = self.expr_ty(&args[0]);
            return self.rc_alloc(&inner_ty, val);
        }
        if name == "rc_retain" && args.len() == 1 {
            let val = self.compile_expr(&args[0])?;
            let arg_ty = self.expr_ty(&args[0]);
            if let Type::Rc(inner) = arg_ty {
                self.rc_retain(val, &inner)?;
                return Ok(val);
            }
        }
        if name == "rc_release" && args.len() == 1 {
            let val = self.compile_expr(&args[0])?;
            let arg_ty = self.expr_ty(&args[0]);
            if let Type::Rc(inner) = arg_ty {
                self.rc_release(val, &inner)?;
                return Ok(self.ctx.i64_type().const_int(0, false).into());
            }
        }
        if let Some(v) = self.compile_bit_intrinsic(&name, args)? {
            return Ok(v);
        }
        if self.generic_fns.contains_key(&name) {
            let arg_tys: Vec<Type> = args.iter().map(|e| self.expr_ty(e)).collect();
            let gf = self.generic_fns.get(&name).unwrap().clone();
            let mut type_map = HashMap::new();
            for (i, p) in gf.params.iter().enumerate() {
                if let Some(Type::Param(tp)) = &p.ty {
                    if i < arg_tys.len() {
                        type_map.insert(tp.clone(), arg_tys[i].clone());
                    }
                }
            }
            let mangled = self.monomorphize_fn(&name, &type_map)?;
            let fv = self.module.get_function(&mangled).unwrap();
            return self.call_fn(fv, args, &mangled);
        }
        if let Some(fv) = self.module.get_function(&name) {
            return self.call_fn(fv, args, &name);
        }
        let fn_ptr = self.load_var(&name)?;
        let fn_ty = self.expr_ty(callee);
        self.indirect_call(fn_ptr, &fn_ty, args)
    }

    fn indirect_call(
        &mut self,
        fn_ptr: BasicValueEnum<'ctx>,
        fn_ty: &Type,
        args: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Fn(ptys, ret) = fn_ty {
            let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
                ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
            let ft = self.mk_fn_type(ret.as_ref(), &lp, false);
            let a: Vec<BasicMetadataValueEnum<'ctx>> = args
                .iter()
                .map(|e| self.compile_expr(e).map(|v| v.into()))
                .collect::<Result<_, _>>()?;
            let csv =
                b!(self
                    .bld
                    .build_indirect_call(ft, fn_ptr.into_pointer_value(), &a, "icall"));
            Ok(self.call_result(csv))
        } else {
            Err(format!("cannot call non-function type: {fn_ty}"))
        }
    }

    fn fmt_for_ty(&self, ty: &Type) -> &'static str {
        match ty {
            Type::I64 => "%ld\n",
            Type::I32 | Type::I16 | Type::I8 => "%d\n",
            Type::U64 => "%lu\n",
            Type::U32 | Type::U16 | Type::U8 => "%u\n",
            Type::F64 | Type::F32 => "%f\n",
            Type::Bool => "%d\n",
            Type::String => "%s\n",
            _ => "%ld\n",
        }
    }

    fn emit_log(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let printf = self.module.get_function("printf").unwrap();
        let fmt = self.fmt_for_ty(ty);
        let fs = b!(self.bld.build_global_string_ptr(fmt, "fmt"));
        let print_val: BasicMetadataValueEnum<'ctx> = if matches!(ty, Type::String) {
            self.string_data(val)?.into()
        } else if matches!(ty, Type::Bool) {
            let iv = val.into_int_value();
            let ext = if iv.get_type().get_bit_width() == 1 {
                b!(self.bld.build_int_z_extend(iv, self.ctx.i32_type(), "bext"))
            } else {
                iv
            };
            ext.into()
        } else {
            val.into()
        };
        b!(self
            .bld
            .build_call(printf, &[fs.as_pointer_value().into(), print_val], "log"));
        Ok(val)
    }

    fn compile_log(&mut self, args: &[Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("log() requires an argument".into());
        }
        let val = self.compile_expr(&args[0])?;
        let ty = self.expr_ty(&args[0]);
        self.emit_log(val, &ty)?;
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    fn compile_to_string(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let ty = self.resolve_ty(self.expr_ty(expr));
        match ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => {
                self.int_to_string(val, false)
            }
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => {
                self.int_to_string(val, true)
            }
            Type::F64 | Type::F32 => self.float_to_string(val),
            Type::Bool => self.bool_to_string(val),
            _ => {
                // Fallback: convert integer representation
                self.int_to_string(val, false)
            }
        }
    }

    fn int_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        unsigned: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        // snprintf(NULL, 0, fmt, val) to get length, then snprintf into buffer
        let fmt_str = if unsigned { "%lu" } else { "%ld" };
        let fmt = b!(self.bld.build_global_string_ptr(fmt_str, "ts.fmt"));
        let snprintf = self.ensure_snprintf();
        // Extend to i64 if narrower
        let iv = val.into_int_value();
        let wide: BasicValueEnum<'ctx> = if iv.get_type().get_bit_width() < 64 {
            if unsigned {
                b!(self.bld.build_int_z_extend(iv, i64t, "zext")).into()
            } else {
                b!(self.bld.build_int_s_extend(iv, i64t, "sext")).into()
            }
        } else {
            iv.into()
        };
        // Query length
        let null = ptr_ty.const_null();
        let len = b!(self.bld.build_call(
            snprintf,
            &[null.into(), i64t.const_int(0, false).into(), fmt.as_pointer_value().into(), wide.into()],
            "ts.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        // snprintf returns i32 — extend to i64
        let len = b!(self.bld.build_int_s_extend(len, i64t, "ts.len64"));
        // Allocate len+1
        let size = b!(self.bld.build_int_nsw_add(len, i64t.const_int(1, false), "ts.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ts.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        // snprintf into buffer
        b!(self.bld.build_call(
            snprintf,
            &[buf.into(), size.into(), fmt.as_pointer_value().into(), wide.into()],
            ""
        ));
        self.build_string(buf, len, size, "ts.val")
    }

    fn float_to_string(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fmt = b!(self.bld.build_global_string_ptr("%g", "ts.ffmt"));
        let snprintf = self.ensure_snprintf();
        let fv = if val.is_float_value() {
            val.into_float_value()
        } else {
            val.into_float_value()
        };
        // Promote to f64 if f32
        let f64t = self.ctx.f64_type();
        let wide: BasicMetadataValueEnum<'ctx> = if fv.get_type() == self.ctx.f32_type() {
            b!(self.bld.build_float_ext(fv, f64t, "fpext")).into()
        } else {
            fv.into()
        };
        let null = ptr_ty.const_null();
        let len = b!(self.bld.build_call(
            snprintf,
            &[null.into(), i64t.const_int(0, false).into(), fmt.as_pointer_value().into(), wide],
            "ts.len"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();
        // snprintf returns i32 — extend to i64
        let len = b!(self.bld.build_int_s_extend(len, i64t, "ts.len64"));
        let size = b!(self.bld.build_int_nsw_add(len, i64t.const_int(1, false), "ts.sz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[size.into()], "ts.buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        b!(self.bld.build_call(
            snprintf,
            &[buf.into(), size.into(), fmt.as_pointer_value().into(), wide],
            ""
        ));
        self.build_string(buf, len, size, "ts.val")
    }

    fn bool_to_string(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let true_str = b!(self.bld.build_global_string_ptr("true", "ts.true"));
        let false_str = b!(self.bld.build_global_string_ptr("false", "ts.false"));
        let cond = self.to_bool(val);
        let true_bb = self.ctx.append_basic_block(fv, "ts.t");
        let false_bb = self.ctx.append_basic_block(fv, "ts.f");
        let merge_bb = self.ctx.append_basic_block(fv, "ts.m");
        b!(self.bld.build_conditional_branch(cond, true_bb, false_bb));

        let i64t = self.ctx.i64_type();
        let zero = i64t.const_int(0, false);
        self.bld.position_at_end(true_bb);
        let tv = self.build_string(true_str.as_pointer_value(), i64t.const_int(4, false), zero, "ts.true")?;
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(false_bb);
        let fv_val = self.build_string(false_str.as_pointer_value(), i64t.const_int(5, false), zero, "ts.false")?;
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.string_type(), "ts.res"));
        phi.add_incoming(&[(&tv, true_bb), (&fv_val, false_bb)]);
        Ok(phi.as_basic_value())
    }

    fn ensure_snprintf(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("snprintf").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[ptr_ty.into(), i64t.into(), ptr_ty.into()], true);
            self.module.add_function("snprintf", ft, Some(Linkage::External))
        })
    }

    fn ensure_malloc(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("malloc").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let ft = ptr_ty.fn_type(&[i64t.into()], false);
            self.module.add_function("malloc", ft, Some(Linkage::External))
        })
    }

    #[allow(dead_code)] // Used once Perceus drop elaboration is wired to codegen
    fn ensure_free(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("free").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
            self.module.add_function("free", ft, Some(Linkage::External))
        })
    }

    fn compile_method(
        &mut self,
        obj: &Expr,
        m: &str,
        args: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_ty = self.expr_ty(obj);
        if matches!(obj_ty, Type::String) {
            return self.compile_string_method(obj, m, args);
        }
        let type_name = match &obj_ty {
            Type::Struct(n) => n.clone(),
            _ => return Err(format!("method calls not supported on type: {obj_ty}")),
        };
        let method_name = format!("{type_name}_{m}");
        if let Some(fv) = self.module.get_function(&method_name) {
            let self_val = self.compile_expr(obj)?;
            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![self_val.into()];
            for arg in args {
                a.push(self.compile_expr(arg)?.into());
            }
            let csv = b!(self.bld.build_call(fv, &a, &method_name));
            return Ok(self.call_result(csv));
        }
        Err(format!("no method '{m}' on type '{type_name}'"))
    }

    fn collect_idents_expr(e: &Expr, out: &mut HashSet<String>) {
        match e {
            Expr::Ident(n, _) => {
                out.insert(n.clone());
            }
            Expr::BinOp(l, _, r, _) | Expr::Pipe(l, r, _, _) => {
                Self::collect_idents_expr(l, out);
                Self::collect_idents_expr(r, out);
            }
            Expr::UnaryOp(_, e, _) | Expr::As(e, _, _) | Expr::Field(e, _, _) => {
                Self::collect_idents_expr(e, out)
            }
            Expr::Call(c, args, _) | Expr::Method(c, _, args, _) => {
                Self::collect_idents_expr(c, out);
                for a in args {
                    Self::collect_idents_expr(a, out);
                }
            }
            Expr::Index(a, b, _) => {
                Self::collect_idents_expr(a, out);
                Self::collect_idents_expr(b, out);
            }
            Expr::Ternary(c, t, f, _) => {
                Self::collect_idents_expr(c, out);
                Self::collect_idents_expr(t, out);
                Self::collect_idents_expr(f, out);
            }
            Expr::Array(es, _) | Expr::Tuple(es, _) => {
                for e in es {
                    Self::collect_idents_expr(e, out);
                }
            }
            Expr::Struct(_, fs, _) => {
                for f in fs {
                    Self::collect_idents_expr(&f.value, out);
                }
            }
            Expr::IfExpr(i) => {
                Self::collect_idents_expr(&i.cond, out);
                Self::collect_idents_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_idents_expr(c, out);
                    Self::collect_idents_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_idents_block(b, out);
                }
            }
            Expr::Block(b, _) => Self::collect_idents_block(b, out),
            Expr::Lambda(_, _, body, _) => Self::collect_idents_block(body, out),
            _ => {}
        }
    }

    fn collect_idents_block(block: &Block, out: &mut HashSet<String>) {
        for stmt in block {
            match stmt {
                Stmt::Expr(e) | Stmt::Ret(Some(e), _) | Stmt::Break(Some(e), _) => {
                    Self::collect_idents_expr(e, out)
                }
                Stmt::Bind(b) => Self::collect_idents_expr(&b.value, out),
                Stmt::TupleBind(_, v, _) => Self::collect_idents_expr(v, out),
                Stmt::Assign(t, v, _) => {
                    Self::collect_idents_expr(t, out);
                    Self::collect_idents_expr(v, out);
                }
                Stmt::If(i) => {
                    Self::collect_idents_expr(&i.cond, out);
                    Self::collect_idents_block(&i.then, out);
                    for (c, b) in &i.elifs {
                        Self::collect_idents_expr(c, out);
                        Self::collect_idents_block(b, out);
                    }
                    if let Some(b) = &i.els {
                        Self::collect_idents_block(b, out);
                    }
                }
                Stmt::While(w) => {
                    Self::collect_idents_expr(&w.cond, out);
                    Self::collect_idents_block(&w.body, out);
                }
                Stmt::For(f) => {
                    Self::collect_idents_expr(&f.iter, out);
                    Self::collect_idents_block(&f.body, out);
                }
                Stmt::Loop(l) => Self::collect_idents_block(&l.body, out),
                Stmt::Match(m) => {
                    Self::collect_idents_expr(&m.subject, out);
                    for arm in &m.arms {
                        Self::collect_idents_block(&arm.body, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn compile_lambda(
        &mut self,
        params: &[Param],
        ret: &Option<Type>,
        body: &Block,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptys: Vec<Type> = params
            .iter()
            .map(|p| p.ty.clone().unwrap_or(Type::I64))
            .collect();
        let ret_ty = ret.clone().unwrap_or_else(|| match body.last() {
            Some(Stmt::Expr(e)) => self.expr_ty(e),
            _ => Type::Void,
        });
        let lambda_name = format!("lambda.{}", self.module.get_functions().count());
        let mut body_ids = HashSet::new();
        Self::collect_idents_block(body, &mut body_ids);
        let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
        let mut cap_globals: Vec<(String, PointerValue<'ctx>, Type)> = Vec::new();
        for id in &body_ids {
            if param_names.contains(id.as_str())
                || self.fns.contains_key(id)
                || self.variant_tags.contains_key(id)
            {
                continue;
            }
            if let Some((ptr, ty)) = self.find_var(id).cloned() {
                let val = b!(self.bld.build_load(self.llvm_ty(&ty), ptr, id));
                let gname = format!("{}.cap.{}", lambda_name, id);
                let lt = self.llvm_ty(&ty);
                let g = self.module.add_global(lt, None, &gname);
                g.set_initializer(&self.default_val(&ty));
                b!(self.bld.build_store(g.as_pointer_value(), val));
                cap_globals.push((id.clone(), g.as_pointer_value(), ty));
            }
        }
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret_ty, &lp, false);
        let lambda_fv = self.module.add_function(&lambda_name, ft, None);
        lambda_fv.add_attribute(AttributeLoc::Function, self.attr("nounwind"));
        lambda_fv.set_linkage(Linkage::Internal);
        self.fns.insert(
            lambda_name.clone(),
            (lambda_fv, ptys.clone(), ret_ty.clone()),
        );
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        self.cur_fn = Some(lambda_fv);
        let entry = self.ctx.append_basic_block(lambda_fv, "entry");
        self.bld.position_at_end(entry);
        self.vars.push(HashMap::new());
        for (name, gptr, ty) in &cap_globals {
            let lt = self.llvm_ty(ty);
            let val = b!(self.bld.build_load(lt, *gptr, name));
            let a = self.entry_alloca(lt, name);
            b!(self.bld.build_store(a, val));
            self.set_var(name, a, ty.clone());
        }
        for (i, p) in params.iter().enumerate() {
            let ty = &ptys[i];
            let a = self.entry_alloca(self.llvm_ty(ty), &p.name);
            b!(self
                .bld
                .build_store(a, lambda_fv.get_nth_param(i as u32).unwrap()));
            self.set_var(&p.name, a, ty.clone());
        }
        let last = self.compile_block(body)?;
        if self.no_term() {
            match &ret_ty {
                Type::Void => {
                    b!(self.bld.build_return(None));
                }
                _ => {
                    let rty = self.llvm_ty(&ret_ty);
                    let v = match last {
                        Some(v) if v.get_type() == rty => v,
                        _ => self.default_val(&ret_ty),
                    };
                    b!(self.bld.build_return(Some(&v)));
                }
            }
        }
        self.vars.pop();
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        Ok(lambda_fv.as_global_value().as_pointer_value().into())
    }

    fn compile_pipe(&mut self, left: &Expr, right: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let left_val = self.compile_expr(left)?;
        match right {
            Expr::Ident(name, _) => {
                if name == "log" {
                    return self.pipe_log(left_val, left);
                }
                if let Some(fv) = self.module.get_function(name) {
                    let csv = b!(self.bld.build_call(fv, &[left_val.into()], "pipe"));
                    return Ok(self.call_result(csv));
                }
                let fn_ptr = self.load_var(name)?;
                let fn_ty = self.expr_ty(right);
                self.pipe_indirect(fn_ptr, &fn_ty, left_val)
            }
            Expr::Call(callee, args, _) => {
                let has_placeholder = args.iter().any(|a| matches!(a, Expr::Placeholder(_)));
                let mut compiled_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                if has_placeholder {
                    for a in args {
                        if matches!(a, Expr::Placeholder(_)) {
                            compiled_args.push(left_val.into());
                        } else {
                            compiled_args.push(self.compile_expr(a)?.into());
                        }
                    }
                } else {
                    compiled_args.push(left_val.into());
                    for a in args {
                        compiled_args.push(self.compile_expr(a)?.into());
                    }
                }
                if let Expr::Ident(name, _) = callee.as_ref() {
                    if let Some(fv) = self.module.get_function(name) {
                        let csv = b!(self.bld.build_call(fv, &compiled_args, "pipe"));
                        return Ok(self.call_result(csv));
                    }
                }
                Err("pipeline: unresolved function in call".into())
            }
            _ => {
                let fn_ptr = self.compile_expr(right)?;
                let fn_ty = self.expr_ty(right);
                self.pipe_indirect(fn_ptr, &fn_ty, left_val)
            }
        }
    }

    fn pipe_indirect(
        &mut self,
        fn_ptr: BasicValueEnum<'ctx>,
        fn_ty: &Type,
        arg: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Fn(ptys, ret) = fn_ty {
            let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
                ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
            let ft = self.mk_fn_type(ret.as_ref(), &lp, false);
            let csv = b!(self.bld.build_indirect_call(
                ft,
                fn_ptr.into_pointer_value(),
                &[arg.into()],
                "pipe.call"
            ));
            Ok(self.call_result(csv))
        } else {
            Err("pipeline: right side must be callable".into())
        }
    }

    fn pipe_log(
        &mut self,
        val: BasicValueEnum<'ctx>,
        expr: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.emit_log(val, &self.expr_ty(expr))
    }

    fn compile_struct(
        &mut self,
        name: &str,
        inits: &[FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((enum_name, tag)) = self.variant_tags.get(name).cloned() {
            return self.compile_variant(&enum_name, tag, name, inits);
        }
        if self
            .try_monomorphize_generic_variant(name, inits)?
            .is_some()
        {
            let (enum_name, tag) = self
                .variant_tags
                .get(name)
                .cloned()
                .ok_or_else(|| format!("variant {name} not found after monomorphization"))?;
            return self.compile_variant(&enum_name, tag, name, inits);
        }
        let fields = self
            .structs
            .get(name)
            .ok_or_else(|| format!("undefined type: {name}"))?
            .clone();
        let st = self
            .module
            .get_struct_type(name)
            .ok_or_else(|| format!("no LLVM struct: {name}"))?;
        let ptr = self.entry_alloca(st.into(), name);
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let val = inits
                .iter()
                .find(|fi| fi.name.as_deref() == Some(fname))
                .or_else(|| inits.get(i))
                .map(|fi| self.compile_expr(&fi.value))
                .transpose()?
                .unwrap_or_else(|| self.default_val(fty));
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, fname));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, name)))
    }

    fn compile_variant(
        &mut self,
        enum_name: &str,
        tag: u32,
        variant_name: &str,
        inits: &[FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self
            .module
            .get_struct_type(enum_name)
            .ok_or_else(|| format!("no LLVM type: {enum_name}"))?;
        let variants = self
            .enums
            .get(enum_name)
            .cloned()
            .ok_or_else(|| format!("undefined enum: {enum_name}"))?;
        let (_, ftys) = variants
            .iter()
            .find(|(n, _)| n == variant_name)
            .ok_or_else(|| format!("no variant {variant_name}"))?;
        let ftys = ftys.clone();
        let ptr = self.entry_alloca(st.into(), variant_name);
        let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "tag"));
        b!(self
            .bld
            .build_store(tag_gep, self.ctx.i32_type().const_int(tag as u64, false)));
        if !ftys.is_empty() {
            let payload_gep = b!(self.bld.build_struct_gep(st, ptr, 1, "payload"));
            let mut offset = 0u64;
            for (i, fty) in ftys.iter().enumerate() {
                let val = inits
                    .get(i)
                    .map(|fi| self.compile_expr(&fi.value))
                    .transpose()?
                    .unwrap_or_else(|| self.default_val(fty));
                let is_rec = Self::is_recursive_field(fty, enum_name);
                let field_ptr = if offset == 0 {
                    payload_gep
                } else {
                    unsafe {
                        b!(self.bld.build_gep(
                            self.ctx.i8_type(),
                            payload_gep,
                            &[self.ctx.i64_type().const_int(offset, false)],
                            "fptr"
                        ))
                    }
                };
                if is_rec {
                    // Box: malloc the recursive value, store it, put pointer in payload
                    let actual_ty = self.llvm_ty(fty);
                    let size = self.type_store_size(actual_ty);
                    let malloc = self.ensure_malloc();
                    let heap = b!(self.bld.build_call(
                        malloc,
                        &[self.ctx.i64_type().const_int(size, false).into()],
                        "box.alloc"
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                    b!(self.bld.build_store(heap, val));
                    b!(self.bld.build_store(field_ptr, heap));
                    offset += 8; // pointer size
                } else {
                    let lty = self.llvm_ty(fty);
                    let coerced = self.coerce_val(val, lty);
                    b!(self.bld.build_store(field_ptr, coerced));
                    offset += self.type_store_size(lty);
                }
            }
        }
        Ok(b!(self.bld.build_load(st, ptr, variant_name)))
    }

    fn compile_field(&mut self, obj: &Expr, field: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_ty = self.expr_ty(obj);
        if matches!(obj_ty, Type::String) && field == "length" {
            let sv = self.compile_expr(obj)?;
            return self.string_len(sv);
        }
        let ty_name = match obj_ty {
            Type::Struct(n) => n,
            other => return Err(format!("field access on non-struct: {other}")),
        };
        let fields = self
            .structs
            .get(&ty_name)
            .ok_or_else(|| format!("undefined type: {ty_name}"))?
            .clone();
        let idx = fields
            .iter()
            .position(|(n, _)| n == field)
            .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
        let fty = fields[idx].1.clone();
        let st = self
            .module
            .get_struct_type(&ty_name)
            .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;
        if let Expr::Ident(n, _) = obj {
            if let Some((ptr, _)) = self.find_var(n).cloned() {
                let gep = b!(self.bld.build_struct_gep(st, ptr, idx as u32, field));
                return Ok(b!(self.bld.build_load(self.llvm_ty(&fty), gep, field)));
            }
        }
        Err("cannot access field on rvalue".into())
    }

    fn compile_array(&mut self, elems: &[Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if elems.is_empty() {
            return Err("empty array literal".into());
        }
        let elem_ty = self.expr_ty(&elems[0]);
        let lty = self.llvm_ty(&elem_ty);
        let arr_ty = lty.array_type(elems.len() as u32);
        let ptr = self.entry_alloca(arr_ty.into(), "arr");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    ptr,
                    &[
                        self.ctx.i64_type().const_int(0, false),
                        self.ctx.i64_type().const_int(i as u64, false)
                    ],
                    "arr.gep"
                ))
            };
            b!(self.bld.build_store(gep, val));
        }
        Ok(ptr.into())
    }

    fn compile_tuple(&mut self, elems: &[Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        let ltys: Vec<BasicTypeEnum<'ctx>> = elems
            .iter()
            .map(|e| self.llvm_ty(&self.expr_ty(e)))
            .collect();
        let st = self.ctx.struct_type(&ltys, false);
        let ptr = self.entry_alloca(st.into(), "tup");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, "tup.gep"));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, "tup")))
    }

    fn compile_assign(&mut self, target: &Expr, value: &Expr) -> Result<(), String> {
        match target {
            Expr::Index(arr_expr, idx_expr, _) => {
                let arr_ty = self.expr_ty(arr_expr);
                let val = self.compile_expr(value)?;
                let idx_val = self.compile_expr(idx_expr)?.into_int_value();
                match &arr_ty {
                    Type::Array(elem_ty, n) => {
                        let lty = self.llvm_ty(elem_ty);
                        let arr_llvm = lty.array_type(*n as u32);
                        let arr_ptr = if let Expr::Ident(name, _) = arr_expr.as_ref() {
                            self.find_var(name)
                                .map(|(ptr, _)| *ptr)
                                .ok_or_else(|| format!("undefined: {name}"))?
                        } else {
                            return Err("cannot assign to rvalue index".into());
                        };
                        let idx_val = self.wrap_negative_index(idx_val, *n as u64)?;
                        let gep = unsafe {
                            b!(self.bld.build_gep(
                                arr_llvm,
                                arr_ptr,
                                &[self.ctx.i64_type().const_int(0, false), idx_val],
                                "idx.assign"
                            ))
                        };
                        b!(self.bld.build_store(gep, val));
                    }
                    _ => return Err("index assignment only supported for arrays".into()),
                }
            }
            Expr::Field(obj_expr, field, _) => {
                let obj_ty = self.expr_ty(obj_expr);
                let val = self.compile_expr(value)?;
                if let Type::Struct(sname) = &obj_ty {
                    let fields = self
                        .structs
                        .get(sname)
                        .ok_or_else(|| format!("unknown type: {sname}"))?
                        .clone();
                    let idx = fields
                        .iter()
                        .position(|(n, _)| n == field)
                        .ok_or_else(|| format!("no field {field} on {sname}"))?;
                    let obj_ptr = if let Expr::Ident(name, _) = obj_expr.as_ref() {
                        self.find_var(name)
                            .map(|(ptr, _)| *ptr)
                            .ok_or_else(|| format!("undefined: {name}"))?
                    } else {
                        return Err("cannot assign field on rvalue".into());
                    };
                    let ftys: Vec<_> = fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
                    let st = self.ctx.struct_type(&ftys, false);
                    let gep =
                        b!(self
                            .bld
                            .build_struct_gep(st, obj_ptr, idx as u32, "field.assign"));
                    b!(self.bld.build_store(gep, val));
                } else {
                    return Err("field assignment only on structs".into());
                }
            }
            _ => return Err("invalid assignment target".into()),
        }
        Ok(())
    }

    fn compile_index(&mut self, arr: &Expr, idx: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let arr_ty = self.expr_ty(arr);
        let idx_val = self.compile_expr(idx)?.into_int_value();
        match &arr_ty {
            Type::Array(elem_ty, n) => {
                let lty = self.llvm_ty(elem_ty);
                let arr_llvm = lty.array_type(*n as u32);
                let arr_ptr = if let Expr::Ident(name, _) = arr {
                    self.find_var(name)
                        .map(|(ptr, _)| *ptr)
                        .ok_or_else(|| format!("undefined: {name}"))?
                } else {
                    self.compile_expr(arr)?.into_pointer_value()
                };
                let idx_val = self.wrap_negative_index(idx_val, *n as u64)?;
                let gep = unsafe {
                    b!(self.bld.build_gep(
                        arr_llvm,
                        arr_ptr,
                        &[self.ctx.i64_type().const_int(0, false), idx_val],
                        "idx"
                    ))
                };
                Ok(b!(self.bld.build_load(lty, gep, "elem")))
            }
            Type::Tuple(tys) => {
                let i = idx_val
                    .get_zero_extended_constant()
                    .ok_or("tuple index must be a constant")?;
                let fty = tys
                    .get(i as usize)
                    .ok_or_else(|| format!("tuple index {i} out of bounds"))?;
                let lty = self.llvm_ty(fty);
                if let Expr::Ident(name, _) = arr {
                    if let Some((ptr, _)) = self.find_var(name).cloned() {
                        let tup_ty = self.ctx.struct_type(
                            &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                            false,
                        );
                        let gep = b!(self.bld.build_struct_gep(tup_ty, ptr, i as u32, "tup.idx"));
                        return Ok(b!(self.bld.build_load(lty, gep, "tup.elem")));
                    }
                }
                Err("tuple indexing on rvalue not supported".into())
            }
            _ => {
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
                let i64t = self.ctx.i64_type();
                let gep = unsafe { b!(self.bld.build_gep(i64t, arr_ptr, &[idx_val], "idx")) };
                Ok(b!(self.bld.build_load(i64t, gep, "elem")))
            }
        }
    }

    fn compile_ternary(
        &mut self,
        cond: &Expr,
        then_e: &Expr,
        else_e: &Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let tv = self.compile_expr(cond)?;
        let cv = self.to_bool(tv);
        let tbb = self.ctx.append_basic_block(fv, "t.then");
        let ebb = self.ctx.append_basic_block(fv, "t.else");
        let mbb = self.ctx.append_basic_block(fv, "t.merge");
        b!(self.bld.build_conditional_branch(cv, tbb, ebb));
        self.bld.position_at_end(tbb);
        let tv = self.compile_expr(then_e)?;
        let tbb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(ebb);
        let ev = self.compile_expr(else_e)?;
        let ebb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(mbb);
        let phi = b!(self
            .bld
            .build_phi(self.llvm_ty(&self.expr_ty(then_e)), "tern"));
        phi.add_incoming(&[(&tv, tbb_end), (&ev, ebb_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_cast(&mut self, expr: &Expr, target: &Type) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let src = self.expr_ty(expr);
        let dst = self.llvm_ty(target);
        if src.is_int() && target.is_float() {
            return Ok(if src.is_signed() {
                b!(self.bld.build_signed_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "sitofp"
                ))
                .into()
            } else {
                b!(self.bld.build_unsigned_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "uitofp"
                ))
                .into()
            });
        }
        if src.is_float() && target.is_int() {
            return Ok(if target.is_signed() {
                b!(self.bld.build_float_to_signed_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptosi"
                ))
                .into()
            } else {
                b!(self.bld.build_float_to_unsigned_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptoui"
                ))
                .into()
            });
        }
        if src.is_int() && target.is_int() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "sext"
                    ))
                    .into()
                } else {
                    b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "zext"
                    ))
                    .into()
                }
            } else if sb > db {
                b!(self
                    .bld
                    .build_int_truncate(val.into_int_value(), dst.into_int_type(), "trunc"))
                .into()
            } else {
                val
            });
        }
        if src.is_float() && target.is_float() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                b!(self
                    .bld
                    .build_float_ext(val.into_float_value(), dst.into_float_type(), "fpext"))
                .into()
            } else if sb > db {
                b!(self.bld.build_float_trunc(
                    val.into_float_value(),
                    dst.into_float_type(),
                    "fptrunc"
                ))
                .into()
            } else {
                val
            });
        }
        if matches!(src, Type::Bool) && target.is_int() {
            return Ok(b!(self.bld.build_int_z_extend(
                val.into_int_value(),
                dst.into_int_type(),
                "boolext"
            ))
            .into());
        }
        Err(format!("unsupported cast: {src} as {target}"))
    }

    fn set_var(&mut self, name: &str, ptr: PointerValue<'ctx>, ty: Type) {
        self.vars
            .last_mut()
            .unwrap()
            .insert(name.to_string(), (ptr, ty));
    }

    fn entry_alloca(&self, ty: BasicTypeEnum<'ctx>, name: &str) -> PointerValue<'ctx> {
        let entry = self.cur_fn.unwrap().get_first_basic_block().unwrap();
        let tmp = self.ctx.create_builder();
        match entry.get_first_instruction() {
            Some(inst) => tmp.position_before(&inst),
            None => tmp.position_at_end(entry),
        }
        tmp.build_alloca(ty, name).unwrap()
    }

    fn find_var(&self, name: &str) -> Option<&(PointerValue<'ctx>, Type)> {
        self.vars.iter().rev().find_map(|s| s.get(name))
    }

    fn load_var(&mut self, name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some((ptr, ty)) = self.find_var(name).cloned() {
            return Ok(b!(self.bld.build_load(self.llvm_ty(&ty), ptr, name)));
        }
        if let Some(fv) = self.module.get_function(name) {
            return Ok(fv.as_global_value().as_pointer_value().into());
        }
        Err(format!("undefined: {name}"))
    }

    fn no_term(&self) -> bool {
        self.bld
            .get_insert_block()
            .unwrap()
            .get_terminator()
            .is_none()
    }

    fn int_const(&self, n: i64, ty: &Type) -> BasicValueEnum<'ctx> {
        match ty.bits() {
            8 => self.ctx.i8_type().const_int(n as u64, true).into(),
            16 => self.ctx.i16_type().const_int(n as u64, true).into(),
            32 => self.ctx.i32_type().const_int(n as u64, true).into(),
            _ => self.ctx.i64_type().const_int(n as u64, true).into(),
        }
    }

    fn coerce_int_width(
        &self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        lty: &Type,
        rty: &Type,
    ) -> (BasicValueEnum<'ctx>, BasicValueEnum<'ctx>) {
        if !lty.is_int() || !rty.is_int() || lty.bits() == rty.bits() {
            return (lhs, rhs);
        }
        if lty.bits() > rty.bits() {
            let ext = if rty.is_signed() {
                self.bld
                    .build_int_s_extend(
                        rhs.into_int_value(),
                        lhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            } else {
                self.bld
                    .build_int_z_extend(
                        rhs.into_int_value(),
                        lhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            };
            (lhs, ext.into())
        } else {
            let ext = if lty.is_signed() {
                self.bld
                    .build_int_s_extend(
                        lhs.into_int_value(),
                        rhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            } else {
                self.bld
                    .build_int_z_extend(
                        lhs.into_int_value(),
                        rhs.into_int_value().get_type(),
                        "widen",
                    )
                    .unwrap()
            };
            (ext.into(), rhs)
        }
    }

    fn coerce_val(
        &self,
        val: BasicValueEnum<'ctx>,
        target: BasicTypeEnum<'ctx>,
    ) -> BasicValueEnum<'ctx> {
        if val.get_type() == target {
            return val;
        }
        if val.is_int_value() && target.is_int_type() {
            let (fb, tb) = (
                val.into_int_value().get_type().get_bit_width(),
                target.into_int_type().get_bit_width(),
            );
            if fb < tb {
                return self
                    .bld
                    .build_int_z_extend(val.into_int_value(), target.into_int_type(), "ext")
                    .unwrap()
                    .into();
            } else if fb > tb {
                return self
                    .bld
                    .build_int_truncate(val.into_int_value(), target.into_int_type(), "trunc")
                    .unwrap()
                    .into();
            }
        }
        val
    }

    fn string_type(&self) -> inkwell::types::StructType<'ctx> {
        self.module.get_struct_type("String").unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type("String");
            st.set_body(
                &[
                    self.ctx.ptr_type(AddressSpace::default()).into(),
                    self.ctx.i64_type().into(),
                    self.ctx.i64_type().into(),
                ],
                false,
            );
            st
        })
    }

    fn wrap_negative_index(&mut self, idx: IntValue<'ctx>, len: u64) -> Result<IntValue<'ctx>, String> {
        if let Some(c) = idx.get_sign_extended_constant() {
            if c < 0 {
                return Ok(self.ctx.i64_type().const_int((len as i64 + c) as u64, false));
            }
            return Ok(idx);
        }
        let i64t = self.ctx.i64_type();
        let zero = i64t.const_int(0, false);
        let is_neg = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::SLT, idx, zero, "neg"));
        let wrapped = b!(self.bld.build_int_add(
            idx,
            i64t.const_int(len, false),
            "wrap"
        ));
        Ok(b!(self.bld.build_select(is_neg, wrapped, idx, "idx.wrap"))
            .into_int_value())
    }

    fn build_string(
        &mut self,
        data: impl Into<BasicValueEnum<'ctx>>,
        len: impl Into<BasicValueEnum<'ctx>>,
        cap: impl Into<BasicValueEnum<'ctx>>,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let out = self.entry_alloca(st.into(), name);
        let dp = b!(self.bld.build_struct_gep(st, out, 0, "s.data"));
        b!(self.bld.build_store(dp, data.into()));
        let lp = b!(self.bld.build_struct_gep(st, out, 1, "s.len"));
        b!(self.bld.build_store(lp, len.into()));
        let cp = b!(self.bld.build_struct_gep(st, out, 2, "s.cap"));
        b!(self.bld.build_store(cp, cap.into()));
        Ok(b!(self.bld.build_load(st, out, name)))
    }

    fn compile_str_literal(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let gstr = b!(self.bld.build_global_string_ptr(s, "str"));
        let i64t = self.ctx.i64_type();
        self.build_string(
            gstr.as_pointer_value(),
            i64t.const_int(s.len() as u64, false),
            i64t.const_int(0, false),
            "slit",
        )
    }

    fn string_data(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));
        let dp = b!(self.bld.build_struct_gep(st, ptr, 0, "s.data"));
        Ok(b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            dp,
            "data"
        )))
    }

    fn ensure_memcmp(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("memcmp").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let i32t = self.ctx.i32_type();
            let ft = i32t.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64t.into()], false);
            self.module.add_function("memcmp", ft, Some(Linkage::External))
        })
    }

    fn ensure_memcpy(&mut self) -> FunctionValue<'ctx> {
        self.module.get_function("memcpy").unwrap_or_else(|| {
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let i64t = self.ctx.i64_type();
            let ft = ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64t.into()], false);
            self.module.add_function("memcpy", ft, Some(Linkage::External))
        })
    }

    fn compile_string_method(
        &mut self,
        obj: &Expr,
        m: &str,
        args: &[Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sv = self.compile_expr(obj)?;
        match m {
            "contains" | "starts_with" | "ends_with" | "char_at" => {
                if args.len() != 1 {
                    return Err(format!("{m}() takes 1 argument"));
                }
                let a = self.compile_expr(&args[0])?;
                match m {
                    "contains" => self.string_contains(sv, a),
                    "starts_with" => self.string_starts_with(sv, a),
                    "ends_with" => self.string_ends_with(sv, a),
                    _ => self.string_char_at(sv, a),
                }
            }
            "slice" => {
                if args.len() != 2 {
                    return Err("slice() takes 2 arguments (start, end)".into());
                }
                let start = self.compile_expr(&args[0])?;
                let end = self.compile_expr(&args[1])?;
                self.string_slice(sv, start, end)
            }
            "length" | "len" => self.string_len(sv),
            _ => Err(format!("no method '{m}' on String")),
        }
    }

    fn string_contains(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        needle: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Linear scan: for i in 0..hlen-nlen+1, if memcmp(hdata+i, ndata, nlen)==0 return true
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let hlen = self.string_len(haystack)?.into_int_value();
        let nlen = self.string_len(needle)?.into_int_value();
        let hdata = self.string_data(haystack)?.into_pointer_value();
        let ndata = self.string_data(needle)?.into_pointer_value();
        let memcmp = self.ensure_memcmp();

        // Empty needle always matches
        let ne_zero = b!(self.bld.build_int_compare(IntPredicate::EQ, nlen, i64t.const_int(0, false), "nz"));
        let check_bb = self.ctx.append_basic_block(fv, "sc.check");
        let loop_bb = self.ctx.append_basic_block(fv, "sc.loop");
        let found_bb = self.ctx.append_basic_block(fv, "sc.found");
        let notfound_bb = self.ctx.append_basic_block(fv, "sc.nf");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");

        b!(self.bld.build_conditional_branch(ne_zero, found_bb, check_bb));

        // check: hlen >= nlen?
        self.bld.position_at_end(check_bb);
        let ok = b!(self.bld.build_int_compare(IntPredicate::SGE, hlen, nlen, "ok"));
        b!(self.bld.build_conditional_branch(ok, loop_bb, notfound_bb));

        // loop
        self.bld.position_at_end(loop_bb);
        let phi_i = b!(self.bld.build_phi(i64t, "i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), check_bb)]);
        let i = phi_i.as_basic_value().into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), hdata, &[i], "hp")) };
        let cmp = b!(self.bld.build_call(memcmp, &[ptr.into(), ndata.into(), nlen.into()], "cmp"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let eq = b!(self.bld.build_int_compare(IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), "eq"));
        let cont_bb = self.ctx.append_basic_block(fv, "sc.cont");
        b!(self.bld.build_conditional_branch(eq, found_bb, cont_bb));

        self.bld.position_at_end(cont_bb);
        let next = b!(self.bld.build_int_add(i, i64t.const_int(1, false), "next"));
        let limit = b!(self.bld.build_int_nsw_sub(hlen, nlen, "lim"));
        let limit1 = b!(self.bld.build_int_add(limit, i64t.const_int(1, false), "lim1"));
        let done = b!(self.bld.build_int_compare(IntPredicate::SGE, next, limit1, "done"));
        phi_i.add_incoming(&[(&next, cont_bb)]);
        b!(self.bld.build_conditional_branch(done, notfound_bb, loop_bb));

        self.bld.position_at_end(found_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(notfound_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "sc.res"));
        phi.add_incoming(&[
            (&self.ctx.bool_type().const_int(1, false), found_bb),
            (&self.ctx.bool_type().const_int(0, false), notfound_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    fn string_starts_with(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.string_affix_match(haystack, part, false)
    }

    fn string_ends_with(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.string_affix_match(haystack, part, true)
    }

    fn string_affix_match(
        &mut self,
        haystack: BasicValueEnum<'ctx>,
        part: BasicValueEnum<'ctx>,
        from_end: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let p = if from_end { "ew" } else { "sw" };
        let hlen = self.string_len(haystack)?.into_int_value();
        let plen = self.string_len(part)?.into_int_value();
        let hdata = self.string_data(haystack)?.into_pointer_value();
        let pdata = self.string_data(part)?.into_pointer_value();
        let memcmp = self.ensure_memcmp();

        let ok = b!(self.bld.build_int_compare(IntPredicate::SGE, hlen, plen, &format!("{p}.ok")));
        let cmp_bb = self.ctx.append_basic_block(fv, &format!("{p}.cmp"));
        let fail_bb = self.ctx.append_basic_block(fv, &format!("{p}.fail"));
        let merge_bb = self.ctx.append_basic_block(fv, &format!("{p}.merge"));
        b!(self.bld.build_conditional_branch(ok, cmp_bb, fail_bb));

        self.bld.position_at_end(cmp_bb);
        let hptr: inkwell::values::PointerValue<'ctx> = if from_end {
            let off = b!(self.bld.build_int_nsw_sub(hlen, plen, &format!("{p}.off")));
            unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), hdata, &[off], &format!("{p}.ptr"))) }
        } else {
            hdata
        };
        let cmp = b!(self.bld.build_call(memcmp, &[hptr.into(), pdata.into(), plen.into()], &format!("{p}.cmp")))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let eq = b!(self.bld.build_int_compare(IntPredicate::EQ, cmp, self.ctx.i32_type().const_int(0, false), &format!("{p}.eq")));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(fail_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), &format!("{p}.res")));
        phi.add_incoming(&[(&eq, cmp_bb), (&self.ctx.bool_type().const_int(0, false), fail_bb)]);
        Ok(phi.as_basic_value())
    }

    fn string_char_at(
        &mut self,
        s: BasicValueEnum<'ctx>,
        idx: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let data = self.string_data(s)?.into_pointer_value();
        let i = idx.into_int_value();
        let ptr = unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), data, &[i], "ca.ptr")) };
        let byte = b!(self.bld.build_load(self.ctx.i8_type(), ptr, "ca.byte"));
        Ok(b!(self.bld.build_int_z_extend(byte.into_int_value(), i64t, "ca.val")).into())
    }

    fn string_slice(
        &mut self,
        s: BasicValueEnum<'ctx>,
        start: BasicValueEnum<'ctx>,
        end: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let data = self.string_data(s)?.into_pointer_value();
        let si = start.into_int_value();
        let ei = end.into_int_value();
        let new_len = b!(self.bld.build_int_nsw_sub(ei, si, "sl.len"));
        let src = unsafe { b!(self.bld.build_gep(self.ctx.i8_type(), data, &[si], "sl.src")) };
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[new_len.into()], "sl.buf"))
            .try_as_basic_value().basic().unwrap();
        let memcpy = self.ensure_memcpy();
        b!(self.bld.build_call(memcpy, &[buf.into(), src.into(), new_len.into()], ""));
        self.build_string(buf, new_len, new_len, "sl.val")
    }

    fn string_len(&mut self, val: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self.string_type();
        let ptr = self.entry_alloca(st.into(), "s.tmp");
        b!(self.bld.build_store(ptr, val));
        let lp = b!(self.bld.build_struct_gep(st, ptr, 1, "s.len"));
        Ok(b!(self.bld.build_load(self.ctx.i64_type(), lp, "len")))
    }

    fn string_concat(
        &mut self,
        l: BasicValueEnum<'ctx>,
        r: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let llen = self.string_len(l)?.into_int_value();
        let rlen = self.string_len(r)?.into_int_value();
        let total = b!(self.bld.build_int_add(llen, rlen, "total"));
        let malloc = self.ensure_malloc();
        let buf = b!(self.bld.build_call(malloc, &[total.into()], "buf"))
            .try_as_basic_value()
            .basic()
            .unwrap();
        let ldata = self.string_data(l)?;
        let rdata = self.string_data(r)?;
        let memcpy = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy, &[buf.into(), ldata.into(), llen.into()], ""));
        let dst = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf.into_pointer_value(), &[llen], "dst"))
        };
        b!(self
            .bld
            .build_call(memcpy, &[dst.into(), rdata.into(), rlen.into()], ""));
        self.build_string(buf, total, total, "cat")
    }

    fn rc_layout_ty(&self, inner: &Type) -> inkwell::types::StructType<'ctx> {
        let name = format!("Rc_{inner}");
        self.module.get_struct_type(&name).unwrap_or_else(|| {
            let st = self.ctx.opaque_struct_type(&name);
            st.set_body(&[self.ctx.i64_type().into(), self.llvm_ty(inner)], false);
            st
        })
    }

    fn rc_alloc(
        &mut self,
        inner: &Type,
        val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let size = layout.size_of().unwrap();
        let malloc = self.ensure_malloc();
        let heap_ptr = b!(self.bld.build_call(malloc, &[size.into()], "rc.alloc"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        b!(self.bld.build_store(rc_gep, i64t.const_int(1, false)));
        let val_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 1, "rc.val"));
        b!(self.bld.build_store(val_gep, val));
        Ok(heap_ptr.into())
    }

    fn rc_retain(&mut self, ptr: BasicValueEnum<'ctx>, inner: &Type) -> Result<(), String> {
        let layout = self.rc_layout_ty(inner);
        let rc_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 0, "rc.cnt"));
        let old = b!(self.bld.build_load(self.ctx.i64_type(), rc_gep, "rc.old")).into_int_value();
        let new =
            b!(self
                .bld
                .build_int_nuw_add(old, self.ctx.i64_type().const_int(1, false), "rc.inc"));
        b!(self.bld.build_store(rc_gep, new));
        Ok(())
    }

    fn rc_release(&mut self, ptr: BasicValueEnum<'ctx>, inner: &Type) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let layout = self.rc_layout_ty(inner);
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let heap_ptr = ptr.into_pointer_value();
        let rc_gep = b!(self.bld.build_struct_gep(layout, heap_ptr, 0, "rc.cnt"));
        let old = b!(self.bld.build_load(i64t, rc_gep, "rc.old")).into_int_value();
        let new = b!(self
            .bld
            .build_int_nsw_sub(old, i64t.const_int(1, false), "rc.dec"));
        b!(self.bld.build_store(rc_gep, new));
        let is_zero = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            new,
            i64t.const_int(0, false),
            "rc.dead"
        ));
        let free_bb = self.ctx.append_basic_block(fv, "rc.free");
        let cont_bb = self.ctx.append_basic_block(fv, "rc.cont");
        b!(self.bld.build_conditional_branch(is_zero, free_bb, cont_bb));
        self.bld.position_at_end(free_bb);
        let free_fn = self.module.get_function("free").unwrap_or_else(|| {
            self.module.add_function(
                "free",
                self.ctx.void_type().fn_type(&[ptr_ty.into()], false),
                Some(Linkage::External),
            )
        });
        b!(self.bld.build_call(free_fn, &[heap_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(cont_bb));
        self.bld.position_at_end(cont_bb);
        Ok(())
    }

    fn rc_deref(
        &mut self,
        ptr: BasicValueEnum<'ctx>,
        inner: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let layout = self.rc_layout_ty(inner);
        let val_gep = b!(self
            .bld
            .build_struct_gep(layout, ptr.into_pointer_value(), 1, "rc.val"));
        Ok(b!(self.bld.build_load(
            self.llvm_ty(inner),
            val_gep,
            "rc.load"
        )))
    }

    fn to_bool(&self, val: BasicValueEnum<'ctx>) -> inkwell::values::IntValue<'ctx> {
        let iv = val.into_int_value();
        if iv.get_type().get_bit_width() == 1 {
            iv
        } else {
            self.bld
                .build_int_compare(
                    IntPredicate::NE,
                    iv,
                    iv.get_type().const_int(0, false),
                    "tobool",
                )
                .unwrap()
        }
    }

    fn llvm_ty(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.ctx.i8_type().into(),
            Type::I16 | Type::U16 => self.ctx.i16_type().into(),
            Type::I32 | Type::U32 => self.ctx.i32_type().into(),
            Type::I64 | Type::U64 => self.ctx.i64_type().into(),
            Type::F32 => self.ctx.f32_type().into(),
            Type::F64 => self.ctx.f64_type().into(),
            Type::Bool => self.ctx.bool_type().into(),
            Type::Void => self.ctx.i8_type().into(),
            Type::String => self.string_type().into(),
            Type::Inferred => self.ctx.i64_type().into(),
            Type::Struct(name) | Type::Enum(name) => self
                .module
                .get_struct_type(name)
                .map(|s| s.into())
                .unwrap_or_else(|| self.ctx.i64_type().into()),
            Type::Array(et, n) => self.llvm_ty(et).array_type(*n as u32).into(),
            Type::Tuple(tys) => self
                .ctx
                .struct_type(
                    &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                    false,
                )
                .into(),
            Type::Fn(_, _) | Type::Ptr(_) | Type::Rc(_) => {
                self.ctx.ptr_type(AddressSpace::default()).into()
            }
            Type::Param(_) => self.ctx.i64_type().into(),
        }
    }

    /// Compute the store size (in bytes) of an LLVM type.
    ///
    /// We cannot use `size_of().get_zero_extended_constant()` because LLVM's
    /// `LLVMSizeOf` always returns a `ConstantExpr` (`ptrtoint(gep(null,1))`),
    /// never a `ConstantInt`. Inkwell's `get_zero_extended_constant()` calls
    /// `LLVMConstIntGetZExtValue` which does an unchecked `cast<ConstantInt>`
    /// on the `ConstantExpr`, producing undefined behavior (returns garbage).
    fn type_store_size(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => ((it.get_bit_width() + 7) / 8) as u64,
            BasicTypeEnum::FloatType(ft) => {
                if ft == self.ctx.f32_type() { 4 } else { 8 }
            }
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(st) => {
                let fields = st.get_field_types();
                let mut offset = 0u64;
                let mut max_align = 1u64;
                for f in &fields {
                    let fs = self.type_store_size(*f);
                    let fa = self.type_abi_align(*f);
                    offset = (offset + fa - 1) & !(fa - 1);
                    offset += fs;
                    max_align = max_align.max(fa);
                }
                (offset + max_align - 1) & !(max_align - 1)
            }
            BasicTypeEnum::ArrayType(at) => {
                if at.len() == 0 {
                    return 0;
                }
                let elem: BasicTypeEnum = at
                    .get_element_type()
                    .try_into()
                    .unwrap_or(self.ctx.i8_type().into());
                self.type_store_size(elem) * at.len() as u64
            }
            _ => 8,
        }
    }

    fn type_abi_align(&self, ty: BasicTypeEnum<'ctx>) -> u64 {
        match ty {
            BasicTypeEnum::IntType(it) => {
                let bytes = ((it.get_bit_width() + 7) / 8) as u64;
                bytes.next_power_of_two().min(8)
            }
            BasicTypeEnum::FloatType(_) => {
                self.type_store_size(ty).min(8)
            }
            BasicTypeEnum::PointerType(_) => 8,
            BasicTypeEnum::StructType(st) => {
                st.get_field_types()
                    .iter()
                    .map(|f| self.type_abi_align(*f))
                    .max()
                    .unwrap_or(1)
            }
            BasicTypeEnum::ArrayType(at) => {
                let elem: BasicTypeEnum = at
                    .get_element_type()
                    .try_into()
                    .unwrap_or(self.ctx.i8_type().into());
                self.type_abi_align(elem)
            }
            _ => 8,
        }
    }

    /// Check if a field type is a recursive reference to the enclosing enum.
    fn is_recursive_field(fty: &Type, enum_name: &str) -> bool {
        match fty {
            Type::Enum(n) | Type::Struct(n) => n == enum_name,
            _ => false,
        }
    }

    fn default_val(&self, ty: &Type) -> BasicValueEnum<'ctx> {
        match ty {
            Type::I8 | Type::U8 => self.ctx.i8_type().const_int(0, false).into(),
            Type::I16 | Type::U16 => self.ctx.i16_type().const_int(0, false).into(),
            Type::I32 | Type::U32 => self.ctx.i32_type().const_int(0, false).into(),
            Type::I64 | Type::U64 => self.ctx.i64_type().const_int(0, false).into(),
            Type::F32 => self.ctx.f32_type().const_float(0.0).into(),
            Type::F64 => self.ctx.f64_type().const_float(0.0).into(),
            Type::Bool => self.ctx.bool_type().const_int(0, false).into(),
            Type::String => self.string_type().const_zero().into(),
            _ => self.ctx.i64_type().const_int(0, false).into(),
        }
    }

    fn resolve_ty(&self, ty: Type) -> Type {
        match &ty {
            Type::Struct(n) if self.enums.contains_key(n) => Type::Enum(n.clone()),
            _ => ty,
        }
    }

    fn expr_ty(&self, expr: &Expr) -> Type {
        match expr {
            Expr::Int(_, _) => Type::I64,
            Expr::Float(_, _) => Type::F64,
            Expr::Str(_, _) => Type::String,
            Expr::Bool(_, _) => Type::Bool,
            Expr::None(_) => Type::I64,
            Expr::Void(_) => Type::Void,
            Expr::Ident(n, _) => {
                if let Some((_, t)) = self.find_var(n) {
                    t.clone()
                } else if let Some((enum_name, _)) = self.variant_tags.get(n) {
                    Type::Enum(enum_name.clone())
                } else if let Some((_, ptys, ret)) = self.fns.get(n) {
                    Type::Fn(ptys.clone(), Box::new(ret.clone()))
                } else {
                    Type::I64
                }
            }
            Expr::BinOp(l, op, _, _) => match op {
                BinOp::Eq
                | BinOp::Ne
                | BinOp::Lt
                | BinOp::Gt
                | BinOp::Le
                | BinOp::Ge
                | BinOp::And
                | BinOp::Or => Type::Bool,
                _ => self.expr_ty(l),
            },
            Expr::UnaryOp(UnaryOp::Not, _, _) => Type::Bool,
            Expr::UnaryOp(_, e, _) => self.expr_ty(e),
            Expr::Call(callee, args, _) => {
                if let Expr::Ident(n, _) = callee.as_ref() {
                    if n == "rc" && args.len() == 1 {
                        return Type::Rc(Box::new(self.expr_ty(&args[0])));
                    }
                    if n == "to_string" {
                        return Type::String;
                    }
                    if let Some((_, _, r)) = self.fns.get(n) {
                        return r.clone();
                    }
                    if let Some(gf) = self.generic_fns.get(n) {
                        let gf = gf.clone();
                        let arg_tys: Vec<Type> = args.iter().map(|e| self.expr_ty(e)).collect();
                        let mut type_map = HashMap::new();
                        for (i, p) in gf.params.iter().enumerate() {
                            if let Some(Type::Param(tp)) = &p.ty {
                                if i < arg_tys.len() {
                                    type_map.insert(tp.clone(), arg_tys[i].clone());
                                }
                            }
                        }
                        let mangled = Self::mangle_generic(n, &type_map, &gf.type_params);
                        if let Some((_, _, r)) = self.fns.get(&mangled) {
                            return r.clone();
                        }
                    }
                }
                Type::I64
            }
            Expr::Ternary(_, t, _, _) => self.expr_ty(t),
            Expr::As(_, ty, _) => ty.clone(),
            Expr::Struct(name, _, _) => {
                if let Some((enum_name, _)) = self.variant_tags.get(name) {
                    Type::Enum(enum_name.clone())
                } else {
                    Type::Struct(name.clone())
                }
            }
            Expr::Field(obj, f, _) => {
                if let Type::Struct(n) = self.expr_ty(obj) {
                    if let Some(fields) = self.structs.get(&n) {
                        if let Some((_, ty)) = fields.iter().find(|(n, _)| n == f) {
                            return ty.clone();
                        }
                    }
                }
                Type::I64
            }
            Expr::Array(elems, _) => {
                let et = elems.first().map(|e| self.expr_ty(e)).unwrap_or(Type::I64);
                Type::Array(Box::new(et), elems.len())
            }
            Expr::Tuple(elems, _) => Type::Tuple(elems.iter().map(|e| self.expr_ty(e)).collect()),
            Expr::Index(arr, _, _) => match self.expr_ty(arr) {
                Type::Array(et, _) => *et,
                Type::Tuple(tys) => tys.first().cloned().unwrap_or(Type::I64),
                _ => Type::I64,
            },
            Expr::Lambda(params, ret, body, _) => {
                let ptys: Vec<Type> = params
                    .iter()
                    .map(|p| p.ty.clone().unwrap_or(Type::I64))
                    .collect();
                let ret_ty = ret.clone().unwrap_or_else(|| match body.last() {
                    Some(Stmt::Expr(e)) => self.expr_ty(e),
                    _ => Type::Void,
                });
                Type::Fn(ptys, Box::new(ret_ty))
            }
            Expr::Pipe(_, right, _, _) => match self.expr_ty(right) {
                Type::Fn(_, ret) => *ret,
                _ => Type::I64,
            },
            Expr::Placeholder(_) => Type::I64,
            Expr::Ref(inner, _) => Type::Ptr(Box::new(self.expr_ty(inner))),
            Expr::Deref(inner, _) => match self.expr_ty(inner) {
                Type::Ptr(inner_ty) | Type::Rc(inner_ty) => *inner_ty,
                _ => Type::I64,
            },
            Expr::ListComp(body, _, _, _, _, _) => Type::Ptr(Box::new(self.expr_ty(body))),
            Expr::Syscall(_, _) => Type::I64,
            Expr::Method(obj, m, _, _) => {
                if matches!(self.expr_ty(obj), Type::String) {
                    match m.as_str() {
                        "contains" | "starts_with" | "ends_with" => Type::Bool,
                        "char_at" => Type::I64,
                        "slice" => Type::String,
                        _ => Type::I64,
                    }
                } else {
                    Type::I64
                }
            }
            _ => Type::I64,
        }
    }

    fn infer_ret(&self, f: &Fn) -> Type {
        match f.body.last() {
            Some(Stmt::Expr(e)) => self.expr_ty(e),
            Some(Stmt::Ret(Some(e), _)) => self.expr_ty(e),
            Some(Stmt::Match(_)) => Type::I64,
            _ => Type::Void,
        }
    }

    fn infer_field_ty(&self, f: &Field) -> Type {
        f.default
            .as_ref()
            .map(|e| self.expr_ty(e))
            .unwrap_or(Type::I64)
    }

    fn target_machine(&self, opt: OptimizationLevel) -> Result<TargetMachine, String> {
        Target::initialize_native(&InitializationConfig::default()).map_err(|e| e.to_string())?;
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| e.to_string())?;
        target
            .create_target_machine(
                &triple,
                TargetMachine::get_host_cpu_name().to_str().unwrap(),
                TargetMachine::get_host_cpu_features().to_str().unwrap(),
                opt,
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| "failed to create target machine".into())
    }

    fn compile_asm(&mut self, asm: &AsmBlock) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let i64t = self.ctx.i64_type();
        let mut constraints = Vec::new();
        let mut input_vals: Vec<BasicValueEnum<'ctx>> = Vec::new();
        for (_name, _) in &asm.outputs {
            constraints.push("=r".to_string());
        }
        for (name, expr) in &asm.inputs {
            constraints.push("r".to_string());
            let val = if let Some((ptr, _)) = self.find_var(name).cloned() {
                b!(self.bld.build_load(i64t, ptr, name))
            } else {
                self.compile_expr(expr)?
            };
            input_vals.push(val);
        }
        let constraint_str = constraints.join(",");
        let input_types: Vec<BasicTypeEnum<'ctx>> =
            input_vals.iter().map(|v| v.get_type()).collect();
        let has_output = !asm.outputs.is_empty();
        let asm_fn_ty = if has_output {
            i64t.fn_type(
                &input_types
                    .iter()
                    .map(|t| (*t).into())
                    .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>(),
                false,
            )
        } else {
            self.ctx.void_type().fn_type(
                &input_types
                    .iter()
                    .map(|t| (*t).into())
                    .collect::<Vec<BasicMetadataTypeEnum<'ctx>>>(),
                false,
            )
        };
        let inline_asm = self.ctx.create_inline_asm(
            asm_fn_ty,
            asm.template.clone(),
            constraint_str,
            true,
            false,
            None,
            false,
        );
        let args_meta: Vec<BasicMetadataValueEnum<'ctx>> =
            input_vals.iter().map(|v| (*v).into()).collect();
        let result = b!(self
            .bld
            .build_indirect_call(asm_fn_ty, inline_asm, &args_meta, "asm"));
        if has_output {
            let val = result
                .try_as_basic_value()
                .basic()
                .unwrap_or_else(|| i64t.const_int(0, false).into());
            if let Some((name, _)) = asm.outputs.first() {
                if let Some((ptr, _)) = self.find_var(name).cloned() {
                    b!(self.bld.build_store(ptr, val));
                } else {
                    let a = self.entry_alloca(i64t.into(), name);
                    b!(self.bld.build_store(a, val));
                    self.set_var(name, a, Type::I64);
                }
            }
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    fn compile_ref(&mut self, inner: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match inner {
            Expr::Ident(name, _) => self
                .find_var(name)
                .map(|(ptr, _)| *ptr)
                .ok_or_else(|| format!("cannot take address of '{name}'"))
                .map(|p| p.into()),
            _ => Err("& requires a variable name".into()),
        }
    }

    fn compile_deref(&mut self, inner: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        let inner_ty = self.expr_ty(inner);
        if let Type::Rc(ref elem_ty) = inner_ty {
            let rv = self.compile_expr(inner)?;
            return self.rc_deref(rv, elem_ty);
        }
        let ptr_val = self.compile_expr(inner)?;
        Ok(b!(self.bld.build_load(
            self.ctx.i64_type(),
            ptr_val.into_pointer_value(),
            "deref"
        )))
    }

    fn compile_syscall(&mut self, args: &[Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("syscall requires at least 1 argument (syscall number)".into());
        }
        let i64t = self.ctx.i64_type();
        let mut vals: Vec<BasicValueEnum<'ctx>> = Vec::new();
        for arg in args {
            vals.push(self.compile_expr(arg)?);
        }
        let nargs = vals.len();
        let (template, constraints) = match nargs {
            1 => (
                "syscall".into(),
                "={rax},{rax},~{rcx},~{r11},~{memory}".into(),
            ),
            2 => (
                "syscall".into(),
                "={rax},{rax},{rdi},~{rcx},~{r11},~{memory}".into(),
            ),
            3 => (
                "syscall".into(),
                "={rax},{rax},{rdi},{rsi},~{rcx},~{r11},~{memory}".into(),
            ),
            4 => (
                "syscall".into(),
                "={rax},{rax},{rdi},{rsi},{rdx},~{rcx},~{r11},~{memory}".into(),
            ),
            5 => (
                "syscall".into(),
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},~{rcx},~{r11},~{memory}".into(),
            ),
            6 => (
                "syscall".into(),
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},~{rcx},~{r11},~{memory}".into(),
            ),
            7 => (
                "syscall".into(),
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}".into(),
            ),
            _ => return Err("syscall supports 0-6 arguments".into()),
        };
        let input_types: Vec<BasicMetadataTypeEnum<'ctx>> =
            vals.iter().map(|_| i64t.into()).collect();
        let ft = i64t.fn_type(&input_types, false);
        let inline_asm =
            self.ctx
                .create_inline_asm(ft, template, constraints, true, false, None, false);
        let args_meta: Vec<BasicMetadataValueEnum<'ctx>> =
            vals.iter().map(|v| (*v).into()).collect();
        let result = b!(self
            .bld
            .build_indirect_call(ft, inline_asm, &args_meta, "syscall"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| i64t.const_int(0, false).into()))
    }

    fn compile_list_comp(
        &mut self,
        body: &Expr,
        bind: &str,
        start: &Expr,
        end: Option<&Expr>,
        cond: Option<&Expr>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let end_expr = end.ok_or("list comprehension requires 'to' end bound")?;
        let i64t = self.ctx.i64_type();
        let start_val = self.compile_expr(start)?.into_int_value();
        let end_val = self.compile_expr(end_expr)?.into_int_value();
        let elem_ty = i64t;
        let max_size = 1024u64;
        let arr_ty = elem_ty.array_type(max_size as u32);
        let arr_ptr = self.entry_alloca(arr_ty.into(), "comp_arr");
        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "comp_loop");
        let body_bb = self.ctx.append_basic_block(fv, "comp_body");
        let skip_bb = if cond.is_some() {
            Some(self.ctx.append_basic_block(fv, "comp_skip"))
        } else {
            None
        };
        let done_bb = self.ctx.append_basic_block(fv, "comp_done");
        let idx_ptr = self.entry_alloca(i64t.into(), "comp_idx");
        let cnt_ptr = self.entry_alloca(i64t.into(), "comp_cnt");
        b!(self.bld.build_store(idx_ptr, start_val));
        b!(self.bld.build_store(cnt_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur_idx, end_val, "cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));
        self.bld.position_at_end(body_bb);
        self.vars.push(HashMap::new());
        let bind_alloca = self.entry_alloca(i64t.into(), bind);
        b!(self.bld.build_store(bind_alloca, cur_idx));
        self.set_var(bind, bind_alloca, Type::I64);
        if let Some(cond_expr) = cond {
            let store_bb = self.ctx.append_basic_block(fv, "comp_store");
            let cond_val = self.compile_expr(cond_expr)?;
            let cbool = self.to_bool(cond_val);
            b!(self
                .bld
                .build_conditional_branch(cbool, store_bb, skip_bb.unwrap()));
            self.bld.position_at_end(store_bb);
        }
        let val = self.compile_expr(body)?;
        let cur_cnt = b!(self.bld.build_load(i64t, cnt_ptr, "cnt")).into_int_value();
        let elem_ptr = unsafe { b!(self.bld.build_gep(elem_ty, arr_ptr, &[cur_cnt], "elem")) };
        b!(self.bld.build_store(elem_ptr, val));
        let next_cnt = b!(self
            .bld
            .build_int_add(cur_cnt, i64t.const_int(1, false), "ncnt"));
        b!(self.bld.build_store(cnt_ptr, next_cnt));
        self.vars.pop();
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "nidx"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        if let Some(skip) = skip_bb {
            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(skip);
            let cur_idx2 = b!(self.bld.build_load(i64t, idx_ptr, "idx2")).into_int_value();
            let next_idx2 = b!(self
                .bld
                .build_int_add(cur_idx2, i64t.const_int(1, false), "nidx2"));
            b!(self.bld.build_store(idx_ptr, next_idx2));
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            b!(self.bld.build_unconditional_branch(loop_bb));
        }
        self.bld.position_at_end(done_bb);
        Ok(arr_ptr.into())
    }

    fn declare_err_def(&mut self, ed: &ErrDef) -> Result<(), String> {
        let i32t = self.ctx.i32_type();
        let mut variants = Vec::new();
        let mut max_payload = 0usize;
        for (tag, v) in ed.variants.iter().enumerate() {
            let payload_bytes: usize = v
                .fields
                .iter()
                .map(|t| {
                    if Self::is_recursive_field(t, &ed.name) {
                        8 // pointer size
                    } else {
                        let lty = self.llvm_ty(t);
                        self.type_store_size(lty) as usize
                    }
                })
                .sum();
            max_payload = max_payload.max(payload_bytes);
            self.variant_tags
                .insert(v.name.clone(), (ed.name.clone(), tag as u32));
            variants.push((v.name.clone(), v.fields.clone()));
        }
        let payload_ty = self.ctx.i8_type().array_type(max_payload as u32);
        let st = self.ctx.opaque_struct_type(&ed.name);
        st.set_body(&[i32t.into(), payload_ty.into()], false);
        self.enums.insert(ed.name.clone(), variants);
        Ok(())
    }
}
