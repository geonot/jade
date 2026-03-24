use std::collections::HashMap;

use inkwell::attributes::AttributeLoc;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum};

use inkwell::AddressSpace;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn declare_builtins(&mut self) {
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

    pub(crate) fn declare_fn(&mut self, f: &hir::Fn) -> Result<(), String> {
        let ptys: Vec<Type> = f.params.iter().map(|p| p.ty.clone()).collect();
        let ret = if f.name == "main" {
            Type::I32
        } else {
            f.ret.clone()
        };
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();

        // For main: create a C-level main(i32, ptr)->i32 wrapper that stores
        // argc/argv into globals, then calls the user-defined main.
        if f.name == "main" && !self.lib_mode {
            let ft = self.mk_fn_type(&ret, &lp, false);
            let user_fv = self.module.add_function("__jade_user_main", ft, None);
            if self.fn_may_recurse(f) {
                self.tag_fn_noreturn_ok(user_fv);
            } else {
                self.tag_fn(user_fv);
            }
            user_fv.set_linkage(Linkage::Internal);
            self.fns.insert(f.name.clone(), (user_fv, ptys, ret));

            // Create C-level main(i32, ptr) -> i32 wrapper
            let i32t = self.ctx.i32_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let main_ft = i32t.fn_type(&[i32t.into(), ptr_ty.into()], false);
            let main_fv = self.module.add_function("main", main_ft, None);

            // Declare globals for argc/argv
            let argc_global = self.module.add_global(i32t, None, "__jade_argc");
            argc_global.set_initializer(&i32t.const_int(0, false));
            let argv_global = self.module.add_global(ptr_ty, None, "__jade_argv");
            argv_global.set_initializer(&ptr_ty.const_null());

            let entry = self.ctx.append_basic_block(main_fv, "entry");
            self.bld.position_at_end(entry);
            let argc_param = main_fv.get_nth_param(0).unwrap();
            let argv_param = main_fv.get_nth_param(1).unwrap();
            b!(self.bld.build_store(argc_global.as_pointer_value(), argc_param));
            b!(self.bld.build_store(argv_global.as_pointer_value(), argv_param));

            // Initialize scheduler if runtime is declared
            if let Some(sched_init) = self.module.get_function("jade_sched_init") {
                b!(self.bld.build_call(sched_init, &[i32t.const_int(0, false).into()], ""));
            }

            // Call user's main
            let call_result = b!(self.bld.build_call(user_fv, &[], "user_main"));

            // Run scheduler to completion (waits for all non-daemon coroutines)
            if let Some(sched_run) = self.module.get_function("jade_sched_run") {
                b!(self.bld.build_call(sched_run, &[], ""));
            }
            // Shutdown scheduler (joins worker threads, cleans up)
            if let Some(sched_shutdown) = self.module.get_function("jade_sched_shutdown") {
                b!(self.bld.build_call(sched_shutdown, &[], ""));
            }
            if let Some(rv) = call_result.try_as_basic_value().basic() {
                let ret_i32 = if rv.is_int_value() {
                    let iv = rv.into_int_value();
                    if iv.get_type().get_bit_width() != 32 {
                        b!(self.bld.build_int_truncate(iv, i32t, "ret32"))
                    } else {
                        iv
                    }
                } else {
                    i32t.const_int(0, false)
                };
                b!(self.bld.build_return(Some(&ret_i32)));
            } else {
                b!(self.bld.build_return(Some(&i32t.const_int(0, false))));
            }

            return Ok(());
        }

        let ft = self.mk_fn_type(&ret, &lp, false);
        let fv = self.module.add_function(&f.name, ft, None);
        if self.fn_may_recurse(f) {
            self.tag_fn_noreturn_ok(fv);
        } else {
            self.tag_fn(fv);
        }
        if !self.lib_mode {
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

    fn fn_may_recurse(&self, f: &hir::Fn) -> bool {
        let mut refs = std::collections::HashSet::new();
        Self::collect_var_refs_block(&f.body, &mut refs);
        refs.contains(&f.name)
    }

    pub(crate) fn declare_method(&mut self, _type_name: &str, m: &hir::Fn) -> Result<(), String> {
        let method_name = m.name.clone();
        let ptys: Vec<Type> = m.params.iter().map(|p| p.ty.clone()).collect();
        let ret = m.ret.clone();
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

    pub(crate) fn emit_body(
        &mut self,
        fv: inkwell::values::FunctionValue<'ctx>,
        params: &[(String, Type)],
        body: &hir::Block,
        ret: &Type,
        name: &str,
        line: u32,
    ) -> Result<(), String> {
        self.create_debug_function(fv, name, line);
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
        self.pop_debug_scope();
        Ok(())
    }

    pub(crate) fn compile_fn(&mut self, f: &hir::Fn) -> Result<(), String> {
        self.compile_fn_body(&f.name, f)
    }

    pub(crate) fn compile_method_body(
        &mut self,
        _type_name: &str,
        mangled: &str,
        m: &hir::Fn,
    ) -> Result<(), String> {
        self.compile_fn_body(mangled, m)
    }

    fn compile_fn_body(&mut self, key: &str, f: &hir::Fn) -> Result<(), String> {
        let (fv, ptys, ret) = self
            .fns
            .get(key)
            .ok_or_else(|| format!("undeclared: {key}"))?
            .clone();
        let params: Vec<_> = f
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| (p.name.clone(), ptys[i].clone()))
            .collect();
        self.emit_body(fv, &params, &f.body, &ret, &f.name, f.span.line)
    }

    pub(crate) fn declare_type(&mut self, td: &hir::TypeDef) -> Result<(), String> {
        let fields: Vec<(String, Type)> = td
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        let ltys: Vec<BasicTypeEnum<'ctx>> = fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
        let st = self.ctx.opaque_struct_type(&td.name);
        st.set_body(&ltys, td.layout.packed);
        self.structs.insert(td.name.clone(), fields);
        self.struct_layouts.insert(td.name.clone(), td.layout.clone());
        Ok(())
    }

    pub(crate) fn declare_enum(&mut self, ed: &hir::EnumDef) -> Result<(), String> {
        let variants: Vec<(String, Vec<Type>, u32)> = ed
            .variants
            .iter()
            .map(|v| {
                let ftys: Vec<Type> = v.fields.iter().map(|f| f.ty.clone()).collect();
                (v.name.clone(), ftys, v.tag)
            })
            .collect();
        self.declare_tagged_union(&ed.name, &variants)
    }

    pub(crate) fn declare_extern(&mut self, ef: &hir::ExternFn) -> Result<(), String> {
        let ptys: Vec<BasicMetadataTypeEnum<'ctx>> = ef
            .params
            .iter()
            .map(|(_, t)| {
                if matches!(t, Type::String) {
                    self.ctx
                        .ptr_type(inkwell::AddressSpace::default())
                        .into()
                } else {
                    self.llvm_ty(t).into()
                }
            })
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

    pub(crate) fn declare_err_def(&mut self, ed: &hir::ErrDef) -> Result<(), String> {
        let variants: Vec<(String, Vec<Type>, u32)> = ed
            .variants
            .iter()
            .map(|v| (v.name.clone(), v.fields.clone(), v.tag))
            .collect();
        self.declare_tagged_union(&ed.name, &variants)
    }

    fn declare_tagged_union(
        &mut self,
        name: &str,
        variants: &[(String, Vec<Type>, u32)],
    ) -> Result<(), String> {
        let i32t = self.ctx.i32_type();
        let mut resolved = Vec::new();
        let mut max_payload = 0usize;
        for (vname, ftys, tag) in variants {
            let payload_bytes: usize = ftys
                .iter()
                .map(|t| {
                    if Self::is_recursive_field(t, name) {
                        8
                    } else {
                        self.type_store_size(self.llvm_ty(t)) as usize
                    }
                })
                .sum();
            max_payload = max_payload.max(payload_bytes);
            self.variant_tags
                .insert(vname.clone(), (name.to_string(), *tag));
            resolved.push((vname.clone(), ftys.clone()));
        }

        // Zero-cost optimization: fieldless enums → tag-only
        if max_payload == 0 {
            let st = self.ctx.opaque_struct_type(name);
            st.set_body(&[i32t.into()], false);
            self.enums.insert(name.to_string(), resolved);
            return Ok(());
        }

        // Zero-cost optimization: Option-like (1 empty variant + 1 single-pointer variant) → nullable ptr
        if variants.len() == 2 {
            let empty_idx = variants.iter().position(|(_, fs, _)| fs.is_empty());
            let payload_idx = variants.iter().position(|(_, fs, _)| fs.len() == 1);
            if let (Some(_ei), Some(pi)) = (empty_idx, payload_idx) {
                let field_ty = &variants[pi].1[0];
                let is_ptr_like = matches!(
                    field_ty,
                    Type::String | Type::Rc(_) | Type::Weak(_) | Type::Fn(_, _)
                ) || matches!(field_ty, Type::Struct(_) | Type::Enum(_))
                    && !Self::is_recursive_field(field_ty, name);
                if is_ptr_like {
                    // Nullable pointer: the whole enum is just a pointer.
                    // null = empty variant, non-null = payload variant.
                    let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());
                    let st = self.ctx.opaque_struct_type(name);
                    st.set_body(&[ptr.into()], false);
                    self.enums.insert(name.to_string(), resolved);
                    return Ok(());
                }
            }
        }

        let payload_ty = self.ctx.i8_type().array_type(max_payload as u32);
        let st = self.ctx.opaque_struct_type(name);
        st.set_body(&[i32t.into(), payload_ty.into()], false);
        self.enums.insert(name.to_string(), resolved);
        Ok(())
    }
}
