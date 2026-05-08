//! MIR closure, channel, type-layout, dynamic dispatch, slice, and coroutine extraction helpers.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_closure_create(
        &mut self,
        fn_name: &str,
        captures: &[mir::ValueId],
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let closure_ty = self.closure_type();

        // Look up the inner lambda function (has captures prepended as params).
        let inner_fv = if let Some((fv, _, _)) = self.fns.get(fn_name).cloned() {
            Some(fv)
        } else {
            self.module.get_function(fn_name)
        };

        // Build env struct from capture values.
        let cap_vals: Vec<BasicValueEnum<'ctx>> = captures.iter().map(|v| self.val(*v)).collect();
        let cap_tys: Vec<BasicTypeEnum<'ctx>> = cap_vals.iter().map(|v| v.get_type()).collect();

        let env_ptr = if !captures.is_empty() {
            let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
            let env_size = env_struct_ty.size_of().expect("ICE: type has no size");
            let malloc = self.ensure_malloc();
            let ep = b!(self.bld.build_call(malloc, &[env_size.into()], "env.alloc"))
                .try_as_basic_value()
                .basic()
                .expect("ICE: call returned void")
                .into_pointer_value();
            for (i, v) in cap_vals.iter().enumerate() {
                let gep = b!(self
                    .bld
                    .build_struct_gep(env_struct_ty, ep, i as u32, "env.field"));
                b!(self.bld.build_store(gep, *v));
            }
            ep
        } else {
            ptr_ty.const_null()
        };

        // Build a wrapper function that takes (env_ptr, ...declared_params)
        // and calls the inner function with (captures..., declared_params...).
        let wrapper_ptr = if let Some(ifv) = inner_fv {
            let wrapper_name = format!("{fn_name}.env_wrap");
            if let Some(w) = self.module.get_function(&wrapper_name) {
                w.as_global_value().as_pointer_value()
            } else {
                let inner_type = ifv.get_type();
                let inner_params = inner_type.get_param_types();
                let n_captures = captures.len();
                // Declared params are everything after the captures.
                let declared_param_tys = &inner_params[n_captures..];
                let mut wrapper_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                wrapper_params.extend(
                    declared_param_tys
                        .iter()
                        .map(|t| BasicMetadataTypeEnum::from(*t)),
                );
                let wrapper_ft = match inner_type.get_return_type() {
                    Some(ret) => ret.fn_type(&wrapper_params, false),
                    None => self.ctx.void_type().fn_type(&wrapper_params, false),
                };
                let wrapper_fv = self.module.add_function(
                    &wrapper_name,
                    wrapper_ft,
                    Some(inkwell::module::Linkage::Internal),
                );
                self.tag_fn(wrapper_fv);

                let saved_bb = self.bld.get_insert_block();
                let entry = self.ctx.append_basic_block(wrapper_fv, "entry");
                self.bld.position_at_end(entry);

                // Build call args: unpack captures from env, then forward declared params.
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
                if n_captures > 0 {
                    let env_struct_ty = self.ctx.struct_type(&cap_tys, false);
                    let env_param = wrapper_fv
                        .get_nth_param(0)
                        .expect("ICE: missing param")
                        .into_pointer_value();
                    for i in 0..n_captures {
                        let gep = b!(self.bld.build_struct_gep(
                            env_struct_ty,
                            env_param,
                            i as u32,
                            "cap.gep"
                        ));
                        let load_ty: BasicTypeEnum<'ctx> = inner_params[i].try_into().unwrap();
                        let cap = b!(self.bld.build_load(load_ty, gep, "cap.load"));
                        call_args.push(cap.into());
                    }
                }
                // Forward declared params (skip env_ptr at index 0).
                for i in 0..declared_param_tys.len() {
                    let p = wrapper_fv.get_nth_param((i + 1) as u32).unwrap();
                    call_args.push(p.into());
                }

                let result = self.bld.build_call(ifv, &call_args, "lam.call").unwrap();
                match inner_type.get_return_type() {
                    Some(_) => {
                        let rv = self.call_result(result);
                        self.bld.build_return(Some(&rv)).unwrap();
                    }
                    None => {
                        self.bld.build_return(None).unwrap();
                    }
                }

                if let Some(bb) = saved_bb {
                    self.bld.position_at_end(bb);
                }
                wrapper_fv.as_global_value().as_pointer_value()
            }
        } else {
            // Fallback: no function found, use null.
            ptr_ty.const_null()
        };

        // Build {wrapper_ptr, env_ptr} closure struct.
        let mut agg: BasicValueEnum<'ctx> = closure_ty.const_zero().into();
        agg =
            b!(self
                .bld
                .build_insert_value(agg.into_struct_value(), wrapper_ptr, 0, "closure.fn"))
            .into_struct_value()
            .into();
        agg = b!(self
            .bld
            .build_insert_value(agg.into_struct_value(), env_ptr, 1, "closure.env"))
        .into_struct_value()
        .into();
        Ok(agg)
    }

    pub(in crate::codegen) fn emit_chan_create(
        &mut self,
        elem_ty: &Type,
        cap: Option<&mir::ValueId>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        if let Some(fv) = self.module.get_function("jinn_chan_create") {
            let elem_size = self
                .llvm_ty(elem_ty)
                .size_of()
                .unwrap_or(i64t.const_int(8, false));
            let capacity = if let Some(cap_id) = cap {
                self.val(*cap_id).into_int_value()
            } else {
                i64t.const_int(64, false) // default capacity
            };
            let csv = b!(self
                .bld
                .build_call(fv, &[elem_size.into(), capacity.into()], "chan"));
            Ok(self.call_result(csv))
        } else {
            Ok(ptr_ty.const_null().into())
        }
    }

    pub(in crate::codegen) fn emit_chan_send(
        &mut self,
        ch: mir::ValueId,
        val: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        let v = self.val(val);
        if let Some(fv) = self.module.get_function("jinn_chan_send") {
            let alloca = self.entry_alloca(v.get_type(), "send.tmp");
            b!(self.bld.build_store(alloca, v));
            b!(self.bld.build_call(fv, &[ch_val.into(), alloca.into()], ""));
        }
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_chan_recv(
        &mut self,
        ch: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        if let Some(fv) = self.module.get_function("jinn_chan_recv") {
            let elem_llvm = self.llvm_ty(result_ty);
            let alloca = self.entry_alloca(elem_llvm, "recv.tmp");
            b!(self.bld.build_call(fv, &[ch_val.into(), alloca.into()], ""));
            Ok(b!(self.bld.build_load(elem_llvm, alloca, "recv.val")))
        } else {
            Ok(self.default_val(result_ty))
        }
    }

    pub(in crate::codegen) fn field_index(&self, struct_name: &str, field: &str) -> u32 {
        self.structs
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|(n, _)| n == field))
            .unwrap_or(0) as u32
    }

    pub(in crate::codegen) fn struct_name_from_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(name, _) => Some(name.as_str()),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(name, _) => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Compute the byte offset for enum payload field at `target_idx`,
    /// matching the VariantInit layout (8-byte aligned actual type sizes).
    /// When we don't have type info, defaults to `target_idx * 8`.
    pub(in crate::codegen) fn compute_enum_payload_offset(
        &self,
        enum_name: &str,
        target_idx: usize,
    ) -> u64 {
        if let Some(variants) = self.enums.get(enum_name) {
            for (_, field_types) in variants {
                if field_types.len() > target_idx {
                    let mut offset: u64 = 0;
                    for (i, fty) in field_types.iter().enumerate() {
                        if i == target_idx {
                            return offset;
                        }
                        let type_size = if Compiler::is_recursive_field(fty, enum_name) {
                            8 // pointer
                        } else {
                            self.llvm_ty(fty)
                                .size_of()
                                .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                .unwrap_or(8)
                        };
                        offset += (type_size + 7) & !7;
                    }
                }
            }
        }
        (target_idx * 8) as u64
    }

    /// Emit dynamic dispatch: fat pointer vtable lookup and indirect call.
    pub(in crate::codegen) fn emit_dyn_dispatch(
        &mut self,
        obj: mir::ValueId,
        trait_name: &str,
        method: &str,
        args: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.val(obj);
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

        let tmp = self.entry_alloca(fat_ty.into(), "dyn.tmp");
        b!(self.bld.build_store(tmp, fat));
        let data_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 0, "dyn.data.gep"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dyn.data")).into_pointer_value();
        let vtable_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 1, "dyn.vtable.gep"));
        let vtable_ptr =
            b!(self.bld.build_load(ptr_ty, vtable_gep, "dyn.vtable")).into_pointer_value();

        let method_idx = self
            .trait_method_order
            .get(trait_name)
            .and_then(|methods| methods.iter().position(|m| m == method))
            .unwrap_or(0) as u64;

        let fn_ptr_gep = unsafe {
            b!(self.bld.build_gep(
                ptr_ty,
                vtable_ptr,
                &[self.ctx.i64_type().const_int(method_idx, false)],
                "dyn.fn.gep"
            ))
        };
        let fn_ptr = b!(self.bld.build_load(ptr_ty, fn_ptr_gep, "dyn.fn")).into_pointer_value();

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![data_ptr.into()];
        for a in args {
            call_args.push(self.val(*a).into());
        }

        let ret_ty = self.llvm_ty(result_ty);
        let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for a in args {
            param_tys.push(self.val(*a).get_type().into());
        }
        let fn_ty = ret_ty.fn_type(&param_tys, false);
        let result = b!(self
            .bld
            .build_indirect_call(fn_ty, fn_ptr, &call_args, "dyn.call"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }

    /// Emit slice operation for Vec or String types.
    pub(in crate::codegen) fn emit_slice(
        &mut self,
        base: mir::ValueId,
        lo: mir::ValueId,
        hi: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let base_val = self.val(base);
        let lo_val = self.val(lo);
        let hi_val = self.val(hi);

        match result_ty {
            Type::Vec(_) => {
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jinn_vec_slice")
                    .unwrap_or_else(|| {
                        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                        self.module
                            .add_function("__jinn_vec_slice", ft, Some(Linkage::External))
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "slice"
                ));
                Ok(self.call_result(result))
            }
            Type::String => {
                let st = self.llvm_ty(&Type::String);
                let i64t = self.ctx.i64_type();
                let slice_fn = self
                    .module
                    .get_function("__jinn_str_slice")
                    .unwrap_or_else(|| {
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.module
                            .add_function("__jinn_str_slice", ft, Some(Linkage::External))
                    });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "str.slice"
                ));
                Ok(self.call_result(result))
            }
            _ => Ok(self.ctx.i8_type().const_int(0, false).into()),
        }
    }

    // ── HIR coroutine/generator body extraction ───────────────────

    /// Walk the entire HIR program to extract CoroutineCreate and GeneratorCreate
    /// bodies, keyed by their name for later use in MIR codegen.
    pub(in crate::codegen) fn extract_coro_bodies_from_program(
        prog: &hir::Program,
        out: &mut HashMap<Symbol, Vec<hir::Stmt>>,
    ) {
        for f in &prog.fns {
            for stmt in &f.body {
                Self::extract_coro_bodies_from_stmt(stmt, out);
            }
        }
        for td in &prog.types {
            for m in &td.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
    }

    pub(in crate::codegen) fn extract_coro_bodies_from_stmt(
        stmt: &hir::Stmt,
        out: &mut HashMap<Symbol, Vec<hir::Stmt>>,
    ) {
        match stmt {
            hir::Stmt::Bind(b) => Self::extract_coro_bodies_from_expr(&b.value, out),
            hir::Stmt::Expr(e) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::If(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
                if let Some(ref eb) = i.els {
                    for s in eb {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
                for elif in &i.elifs {
                    Self::extract_coro_bodies_from_expr(&elif.0, out);
                    for s in &elif.1 {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::Stmt::While(w) => {
                Self::extract_coro_bodies_from_expr(&w.cond, out);
                for s in &w.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::For(f) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::Ret(Some(e), _, _) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::Assign(a, b, _) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::Stmt::Match(m) => {
                Self::extract_coro_bodies_from_expr(&m.subject, out);
                for arm in &m.arms {
                    for s in &arm.body {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::Stmt::SimFor(f, _) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::Stmt::SimBlock(b, _) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            _ => {}
        }
    }

    pub(in crate::codegen) fn extract_coro_bodies_from_expr(
        expr: &hir::Expr,
        out: &mut HashMap<Symbol, Vec<hir::Stmt>>,
    ) {
        match &expr.kind {
            hir::ExprKind::CoroutineCreate(name, body) => {
                out.insert(name.clone(), body.clone());
                // Also recurse into the body for nested coroutines
                for s in body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::GeneratorCreate(_, name, body) => {
                out.insert(name.clone(), body.clone());
                for s in body {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::BinOp(a, _, b) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::ExprKind::UnaryOp(_, a) => Self::extract_coro_bodies_from_expr(a, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args {
                    Self::extract_coro_bodies_from_expr(a, out);
                }
            }
            hir::ExprKind::IndirectCall(f, args) => {
                Self::extract_coro_bodies_from_expr(f, out);
                for a in args {
                    Self::extract_coro_bodies_from_expr(a, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
                if let Some(ref eb) = i.els {
                    for s in eb {
                        Self::extract_coro_bodies_from_stmt(s, out);
                    }
                }
            }
            hir::ExprKind::Block(b) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            hir::ExprKind::Lambda(_, b) => {
                for s in b {
                    Self::extract_coro_bodies_from_stmt(s, out);
                }
            }
            _ => {}
        }
    }
}
