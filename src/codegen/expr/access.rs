use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn compile_field(
        &mut self,
        obj: &hir::Expr,
        field: &str,
        _hir_idx: usize,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_ty = &obj.ty;
        if matches!(obj_ty, Type::String) && field == "length" {
            let sv = self.compile_expr(obj)?;
            return self.string_len(sv);
        }
        if matches!(obj_ty, Type::Vec(_)) && field == "length" {
            let v = self.compile_expr(obj)?;
            return self.vec_len(v.into_pointer_value());
        }

        let row_struct = match obj_ty {
            Type::Row(name) => Some(crate::intern::Symbol::intern(&format!(
                "__store_{}",
                name.as_str()
            ))),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Row(name) => Some(crate::intern::Symbol::intern(&format!(
                    "__store_{}",
                    name.as_str()
                ))),
                _ => None,
            },
            _ => None,
        };
        let (ty_name, is_ptr) = match (obj_ty, row_struct) {
            (_, Some(name)) => (name, matches!(obj_ty, Type::Ptr(_))),
            (Type::Struct(n, _), _) => (*n, false),
            (Type::Ptr(inner), _) => match inner.as_ref() {
                Type::Struct(n, _) => (*n, true),
                other => return Err(format!("field access on non-struct: {other}")),
            },
            (other, _) => return Err(format!("field access on non-struct: {other}")),
        };
        let fields = self
            .structs
            .get(&ty_name)
            .ok_or_else(|| format!("undefined type: {ty_name}"))?
            .clone();
        let idx = fields
            .iter()
            .position(|(n, _)| field == n.as_str())
            .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
        let fty = fields[idx].1.clone();
        let st = self
            .module
            .get_struct_type(&ty_name.as_str())
            .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;

        let struct_ptr = if let hir::ExprKind::Var(_, n) = &obj.kind {
            if let Some((ptr, _)) = self.find_var(&n.as_str()).cloned() {
                if is_ptr {
                    b!(self.bld.build_load(
                        self.ctx.ptr_type(inkwell::AddressSpace::default()),
                        ptr,
                        "self.ptr"
                    ))
                    .into_pointer_value()
                } else {
                    ptr
                }
            } else {
                let val = self.compile_expr(obj)?;
                let spill = self.entry_alloca(st.into(), "field.spill");
                b!(self.bld.build_store(spill, val));
                spill
            }
        } else {
            let val = self.compile_expr(obj)?;
            let spill = self.entry_alloca(st.into(), "field.spill");
            b!(self.bld.build_store(spill, val));
            spill
        };

        let gep = b!(self.bld.build_struct_gep(st, struct_ptr, idx as u32, field));
        Ok(b!(self.bld.build_load(self.llvm_ty(&fty), gep, field)))
    }

    pub(crate) fn compile_lvalue_ptr(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Var(_, name) => self
                .find_var(&name.as_str())
                .map(|(ptr, _)| *ptr)
                .ok_or_else(|| format!("undefined: {name}")),
            hir::ExprKind::Field(obj, field, _idx) => {
                let obj_ty = &obj.ty;

                let row_struct = match obj_ty {
                    Type::Row(name) => Some(crate::intern::Symbol::intern(&format!(
                        "__store_{}",
                        name.as_str()
                    ))),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Row(name) => Some(crate::intern::Symbol::intern(&format!(
                            "__store_{}",
                            name.as_str()
                        ))),
                        _ => None,
                    },
                    _ => None,
                };
                let (ty_name, is_ptr) = match (obj_ty, row_struct) {
                    (_, Some(name)) => (name, matches!(obj_ty, Type::Ptr(_))),
                    (Type::Struct(n, _), _) => (*n, false),
                    (Type::Ptr(inner), _) => match inner.as_ref() {
                        Type::Struct(n, _) => (*n, true),
                        _ => return Err("field lvalue on non-struct".into()),
                    },
                    _ => return Err("field lvalue on non-struct".into()),
                };
                let fields = self
                    .structs
                    .get(&ty_name)
                    .ok_or_else(|| format!("undefined type: {ty_name}"))?
                    .clone();
                let fi = fields
                    .iter()
                    .position(|(n, _)| *field == n.as_str())
                    .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
                let st = self
                    .module
                    .get_struct_type(&ty_name.as_str())
                    .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;
                let obj_ptr = self.compile_lvalue_ptr(obj)?;
                let struct_ptr = if is_ptr {
                    b!(self.bld.build_load(
                        self.ctx.ptr_type(inkwell::AddressSpace::default()),
                        obj_ptr,
                        "self.ptr"
                    ))
                    .into_pointer_value()
                } else {
                    obj_ptr
                };
                let gep = b!(self
                    .bld
                    .build_struct_gep(st, struct_ptr, fi as u32, &field.as_str()));
                Ok(gep)
            }
            _ => Err("expression is not an lvalue".into()),
        }
    }

    pub(in crate::codegen) fn compile_index(
        &mut self,
        arr: &hir::Expr,
        idx: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let arr_ty = &arr.ty;
        let idx_val = self.compile_expr(idx)?.into_int_value();
        match arr_ty {
            Type::Array(elem_ty, n) => {
                let lty = self.llvm_ty(elem_ty);
                let arr_llvm = lty.array_type(*n as u32);
                let arr_ptr = match &arr.kind {
                    hir::ExprKind::Var(_, name) => self
                        .find_var(&name.as_str())
                        .map(|(ptr, _)| *ptr)
                        .ok_or_else(|| format!("undefined: {name}"))?,
                    _ => self.compile_expr(arr)?.into_pointer_value(),
                };
                let idx_val = self.wrap_negative_index(idx_val, *n as u64)?;
                self.emit_bounds_check(idx_val, *n as u64)?;
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
                if let hir::ExprKind::Var(_, name) = &arr.kind {
                    if let Some((ptr, _)) = self.find_var(&name.as_str()).cloned() {
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
            Type::Vec(elem_ty) => {
                let lty = self.llvm_ty(elem_ty);
                let header_ptr = self.compile_expr(arr)?.into_pointer_value();
                let header_ty = self.vec_header_type();
                let ptr_gep = b!(self
                    .bld
                    .build_struct_gep(header_ty, header_ptr, 0, "vi.ptrp"));
                let data_ptr = b!(self.bld.build_load(
                    self.ctx.ptr_type(inkwell::AddressSpace::default()),
                    ptr_gep,
                    "vi.data"
                ))
                .into_pointer_value();
                let len_gep = b!(self
                    .bld
                    .build_struct_gep(header_ty, header_ptr, 1, "vi.lenp"));
                let len = b!(self.bld.build_load(self.ctx.i64_type(), len_gep, "vi.len"))
                    .into_int_value();
                self.emit_vec_bounds_check(idx_val, len)?;
                let elem_gep =
                    unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx_val], "vi.egep")) };
                Ok(b!(self.bld.build_load(lty, elem_gep, "vi.elem")))
            }
            _ => {
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
                let i64t = self.ctx.i64_type();
                let gep = unsafe { b!(self.bld.build_gep(i64t, arr_ptr, &[idx_val], "idx")) };
                Ok(b!(self.bld.build_load(i64t, gep, "elem")))
            }
        }
    }

    pub(crate) fn compile_str_literal(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if s.len() <= 23 {
            let st = self.string_type();
            let i8t = self.ctx.i8_type();
            let i64t = self.ctx.i64_type();
            let out = self.entry_alloca(st.into(), "slit");
            b!(self.bld.build_store(out, st.const_zero()));
            for (i, byte) in s.bytes().enumerate() {
                let bp = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, out, &[i64t.const_int(i as u64, false)], "sso.b"))
                };
                b!(self.bld.build_store(bp, i8t.const_int(byte as u64, false)));
            }
            let tag_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(i8t, out, &[i64t.const_int(23, false)], "sso.tag"))
            };
            b!(self
                .bld
                .build_store(tag_ptr, i8t.const_int(0x80 | s.len() as u64, false)));
            Ok(b!(self.bld.build_load(st, out, "slit")))
        } else {
            let gstr = b!(self.bld.build_global_string_ptr(s, "str"));
            let i64t = self.ctx.i64_type();
            self.build_string(
                gstr.as_pointer_value(),
                i64t.const_int(s.len() as u64, false),
                i64t.const_int(0, false),
                "slit",
            )
        }
    }

    pub(in crate::codegen) fn compile_ref(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match &inner.kind {
            hir::ExprKind::Var(_, name) => self
                .find_var(&name.as_str())
                .map(|(ptr, _)| *ptr)
                .ok_or_else(|| format!("cannot take address of '{name}'"))
                .map(|p| p.into()),
            hir::ExprKind::FnRef(_, name) => {
                if let Some(fv) = self.module.get_function(&name.as_str()) {
                    Ok(fv.as_global_value().as_pointer_value().into())
                } else {
                    Err(format!("undefined function: {name}"))
                }
            }
            _ => Err("& requires a variable name".into()),
        }
    }

    pub(in crate::codegen) fn compile_deref(
        &mut self,
        inner: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_val = self.compile_expr(inner)?;
        let load_ty = match &inner.ty {
            Type::Ptr(inner_ty) => self.llvm_ty(inner_ty),
            _ => self.ctx.i64_type().into(),
        };
        Ok(b!(self.bld.build_load(
            load_ty,
            ptr_val.into_pointer_value(),
            "deref"
        )))
    }

    pub(in crate::codegen) fn compile_syscall(
        &mut self,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
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
            1 => ("syscall", "={rax},{rax},~{rcx},~{r11},~{memory}"),
            2 => ("syscall", "={rax},{rax},{rdi},~{rcx},~{r11},~{memory}"),
            3 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},~{rcx},~{r11},~{memory}",
            ),
            4 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},~{rcx},~{r11},~{memory}",
            ),
            5 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},~{rcx},~{r11},~{memory}",
            ),
            6 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},~{rcx},~{r11},~{memory}",
            ),
            7 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}",
            ),
            _ => return Err("syscall supports 0-6 arguments".into()),
        };
        let input_types: Vec<BasicMetadataTypeEnum<'ctx>> =
            vals.iter().map(|_| i64t.into()).collect();
        let ft = i64t.fn_type(&input_types, false);
        let inline_asm = self.ctx.create_inline_asm(
            ft,
            template.to_string(),
            constraints.to_string(),
            true,
            false,
            None,
            false,
        );
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

    pub(in crate::codegen) fn compile_list_comp(
        &mut self,
        body: &hir::Expr,
        bind: &str,
        start: &hir::Expr,
        end: Option<&hir::Expr>,
        cond: Option<&hir::Expr>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let end_expr = end.ok_or("list comprehension requires 'to' end bound")?;
        let i64t = self.ctx.i64_type();
        let start_val = self.compile_expr(start)?.into_int_value();
        let end_val = self.compile_expr(end_expr)?.into_int_value();
        let elem_ty = i64t;
        let range = b!(self.bld.build_int_sub(end_val, start_val, "comp.range"));
        let zero = i64t.const_int(0, false);
        let is_pos = b!(self
            .bld
            .build_int_compare(IntPredicate::SGT, range, zero, "comp.pos"));
        let safe_range = b!(self.bld.build_select(is_pos, range, zero, "comp.sz")).into_int_value();
        let elem_size = i64t.const_int(8, false);
        let alloc_size = b!(self.bld.build_int_mul(safe_range, elem_size, "comp.bytes"));
        let malloc_fn = self.ensure_malloc();
        let arr_ptr = b!(self
            .bld
            .build_call(malloc_fn, &[alloc_size.into()], "comp_arr"))
        .try_as_basic_value()
        .basic()
        .expect("ICE: call returned void")
        .into_pointer_value();
        let fv = self.current_fn();
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
        self.push_var_scope();
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
        self.pop_var_scope();
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

    pub(in crate::codegen) fn compile_dyn_coerce(
        &mut self,
        inner: &hir::Expr,
        type_name: &str,
        trait_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let data_ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            let lty = val.get_type();
            let alloc = self.entry_alloca(lty, "dyn.data");
            b!(self.bld.build_store(alloc, val));
            alloc
        };

        let vtable_ptr = self
            .vtables
            .get(&(type_name.to_string(), trait_name.to_string()))
            .map(|gv| gv.as_pointer_value())
            .unwrap_or_else(|| ptr.const_null());

        let fat_ty = self.ctx.struct_type(&[ptr.into(), ptr.into()], false);
        let fat = fat_ty.const_zero();
        let fat = b!(self
            .bld
            .build_insert_value(fat, data_ptr, 0, "dyn.fat.data"))
        .into_struct_value();
        let fat = b!(self
            .bld
            .build_insert_value(fat, vtable_ptr, 1, "dyn.fat.vtable"))
        .into_struct_value();
        Ok(fat.into())
    }

    pub(in crate::codegen) fn compile_dyn_dispatch(
        &mut self,
        obj: &hir::Expr,
        trait_name: &str,
        method: &str,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.compile_expr(obj)?;
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
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
        for arg in args {
            let av = self.compile_expr(arg)?;
            call_args.push(av.into());
        }

        let ret_ty = self.llvm_ty(result_ty);
        let mut param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for arg in args {
            param_tys.push(self.llvm_ty(&arg.ty).into());
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

    pub(in crate::codegen) fn compile_iter_next_by_name(
        &mut self,
        var_name: &str,
        type_name: &str,
        method_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self
            .find_var(var_name)
            .ok_or_else(|| format!("undefined iter variable: {var_name}"))?
            .0;

        let fn_name = format!("{type_name}_{method_name}");
        let fv = self
            .module
            .get_function(&fn_name)
            .ok_or_else(|| format!("no function {fn_name}"))?;
        let result = b!(self.bld.build_call(fv, &[ptr.into()], "iter.next"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }
}
