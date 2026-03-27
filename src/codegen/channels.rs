use inkwell::AddressSpace;
use inkwell::values::BasicValueEnum;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_channel_create(
        &mut self,
        elem_ty: &Type,
        cap_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let elem_size = self.type_store_size(self.llvm_ty(elem_ty));
        let cap_val = self.compile_expr(cap_expr)?;
        let cap_i64 = if cap_val.is_int_value() {
            let iv = cap_val.into_int_value();
            if iv.get_type().get_bit_width() == 64 {
                iv
            } else {
                b!(self.bld.build_int_z_extend(iv, i64t, "cap.zext"))
            }
        } else {
            i64t.const_int(64, false)
        };
        let chan_create = self.module.get_function("jade_chan_create").unwrap();
        let ch = b!(self.bld.build_call(
            chan_create,
            &[i64t.const_int(elem_size, false).into(), cap_i64.into(),],
            "ch"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();
        Ok(ch)
    }

    pub(crate) fn compile_channel_send(
        &mut self,
        ch_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let val = self.compile_expr(val_expr)?;
        let val_ty = val.get_type();
        let val_alloca = self.entry_alloca(val_ty, "chan.send.val");
        b!(self.bld.build_store(val_alloca, val));
        let chan_send = self.module.get_function("jade_chan_send").unwrap();
        b!(self
            .bld
            .build_call(chan_send, &[ch_ptr.into(), val_alloca.into()], ""));
        Ok(self.ctx.i64_type().const_int(0, false).into())
    }

    pub(crate) fn compile_channel_recv(
        &mut self,
        ch_expr: &hir::Expr,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let elem_llvm_ty = self.llvm_ty(result_ty);
        let recv_alloca = self.entry_alloca(elem_llvm_ty, "chan.recv.val");
        let chan_recv = self.module.get_function("jade_chan_recv").unwrap();
        b!(self.bld.build_call(
            chan_recv,
            &[ch_ptr.into(), recv_alloca.into()],
            "chan.recv.ok"
        ));
        let result = b!(self
            .bld
            .build_load(elem_llvm_ty, recv_alloca, "chan.recv.result"));
        Ok(result)
    }

    pub(crate) fn compile_channel_close(&mut self, ch_expr: &hir::Expr) -> Result<(), String> {
        let ch_ptr = self.compile_expr(ch_expr)?.into_pointer_value();
        let chan_close = self.module.get_function("jade_chan_close").unwrap();
        b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
        Ok(())
    }

    pub(crate) fn compile_stop(&mut self, actor_expr: &hir::Expr) -> Result<(), String> {
        let actor_ptr = self.compile_expr(actor_expr)?.into_pointer_value();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ch_ptr = b!(self.bld.build_load(ptr_ty, actor_ptr, "stop_ch_ptr"));
        let chan_close = self.module.get_function("jade_chan_close").unwrap();
        b!(self.bld.build_call(chan_close, &[ch_ptr.into()], ""));
        Ok(())
    }

    pub(crate) fn compile_select(
        &mut self,
        arms: &[hir::SelectArm],
        default_body: Option<&hir::Block>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();
        let n = arms.len();

        let case_struct_ty = self
            .ctx
            .struct_type(&[ptr_ty.into(), ptr_ty.into(), i32t.into()], false);

        let cases_array_ty = case_struct_ty.array_type(n as u32);
        let cases_alloca = self.entry_alloca(cases_array_ty.into(), "select.cases");

        let mut data_allocas: Vec<inkwell::values::PointerValue<'ctx>> = Vec::new();
        for (i, arm) in arms.iter().enumerate() {
            let ch_val = self.compile_expr(&arm.chan)?.into_pointer_value();

            let elem_llvm_ty = self.llvm_ty(&arm.elem_ty);
            let data_alloca = self.entry_alloca(elem_llvm_ty, &format!("select.data.{i}"));
            data_allocas.push(data_alloca);

            if arm.is_send {
                if let Some(ref val_expr) = arm.value {
                    let val = self.compile_expr(val_expr)?;
                    b!(self.bld.build_store(data_alloca, val));
                }
            }

            let idx0 = i32t.const_int(0, false);
            let idx_i = i32t.const_int(i as u64, false);
            let case_ptr = unsafe {
                b!(self.bld.build_gep(
                    cases_array_ty,
                    cases_alloca,
                    &[idx0, idx_i],
                    &format!("select.case.{i}")
                ))
            };

            let chan_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 0, "case.chan"));
            b!(self.bld.build_store(chan_field, ch_val));

            let data_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 1, "case.data"));
            b!(self.bld.build_store(data_field, data_alloca));

            let is_send_field =
                b!(self
                    .bld
                    .build_struct_gep(case_struct_ty, case_ptr, 2, "case.is_send"));
            b!(self.bld.build_store(
                is_send_field,
                i32t.const_int(if arm.is_send { 1 } else { 0 }, false)
            ));
        }

        let select_fn = self.module.get_function("jade_select").unwrap();
        let has_default_val = self
            .ctx
            .bool_type()
            .const_int(if default_body.is_some() { 1 } else { 0 }, false);
        let result = b!(self.bld.build_call(
            select_fn,
            &[
                cases_alloca.into(),
                i32t.const_int(n as u64, false).into(),
                has_default_val.into(),
            ],
            "select.result"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_int_value();

        let default_bb = self.ctx.append_basic_block(fv, "select.default");
        let merge_bb = self.ctx.append_basic_block(fv, "select.merge");
        let arm_bbs: Vec<_> = (0..n)
            .map(|i| self.ctx.append_basic_block(fv, &format!("select.arm.{i}")))
            .collect();

        let cases_vec: Vec<_> = arm_bbs
            .iter()
            .enumerate()
            .map(|(i, bb)| (i32t.const_int(i as u64, false), *bb))
            .collect();
        b!(self.bld.build_switch(result, default_bb, &cases_vec));

        for (i, (arm, bb)) in arms.iter().zip(arm_bbs.iter()).enumerate() {
            self.bld.position_at_end(*bb);
            self.vars.push(std::collections::HashMap::new());

            if !arm.is_send {
                if let Some(ref bind_name) = arm.binding {
                    let elem_llvm_ty = self.llvm_ty(&arm.elem_ty);
                    let val = b!(self
                        .bld
                        .build_load(elem_llvm_ty, data_allocas[i], bind_name));
                    let alloca = self.entry_alloca(elem_llvm_ty, bind_name);
                    b!(self.bld.build_store(alloca, val));
                    self.set_var(bind_name, alloca, arm.elem_ty.clone());
                }
            }

            self.compile_block(&arm.body)?;

            self.vars.pop();
            if self.no_term() {
                b!(self.bld.build_unconditional_branch(merge_bb));
            }
        }

        self.bld.position_at_end(default_bb);
        if let Some(body) = default_body {
            self.compile_block(body)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(merge_bb));
        }

        self.bld.position_at_end(merge_bb);
        Ok(i64t.const_int(0, false).into())
    }
}
