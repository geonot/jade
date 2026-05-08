//! MIR closure, actor, channel, builtin, dynamic dispatch, and inline-asm emission.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_runtime_inst(
        &mut self,
        inst: &mir::Instruction,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        Ok(Some(
            (match &inst.kind {
                mir::InstKind::ClosureCreate(fn_name, captures) => {
                    self.emit_closure_create(&fn_name.as_str(), captures, &inst.ty)
                }
                mir::InstKind::ClosureCall(callee, args) => {
                    // Same as IndirectCall for closures.
                    let callee_val = self.val(*callee);
                    let closure_st = callee_val.into_struct_value();
                    let fn_ptr = b!(self.bld.build_extract_value(closure_st, 0, "fn_ptr"))
                        .into_pointer_value();
                    let env_ptr = b!(self.bld.build_extract_value(closure_st, 1, "env_ptr"));
                    let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
                    let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        vec![env_ptr.into()];
                    let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                    for a in args {
                        let v = self.val(*a);
                        call_args.push(v.into());
                        param_tys.push(v.get_type().into());
                    }
                    let ret_llvm = self.llvm_ty(&inst.ty);
                    let ft = ret_llvm.fn_type(&param_tys, false);
                    let csv =
                        b!(self
                            .bld
                            .build_indirect_call(ft, fn_ptr, &call_args, "closure.call"));
                    Ok(self.call_result(csv))
                }

                // ── Actors / Channels ──
                mir::InstKind::SpawnActor(name, args) => {
                    if !args.is_empty() {
                        return Err(format!(
                            "mir_codegen: SpawnActor '{name}' has {} constructor args but actor spawn does not yet support arguments",
                            args.len()
                        ));
                    }
                    self.emit_spawn_actor(&name.as_str())
                }
                mir::InstKind::ChanCreate(elem_ty, cap) => {
                    self.emit_chan_create(elem_ty, cap.as_ref())
                }
                mir::InstKind::ChanSend(ch, val) => self.emit_chan_send(*ch, *val),
                mir::InstKind::ChanRecv(ch) => self.emit_chan_recv(*ch, &inst.ty),
                mir::InstKind::SelectArm(channels, has_default) => {
                    // Select: build case array, call jinn_select, return index.
                    let ch_vids: Vec<mir::ValueId> = channels.clone();
                    let dest = inst.dest.unwrap();
                    self.emit_select(&ch_vids, dest, *has_default)
                }

                // ── Builtins ──
                mir::InstKind::Log(val) => {
                    let v = self.val(*val);
                    self.emit_log(v, &inst.ty)?;
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                mir::InstKind::Assert(val, msg) => {
                    let v = self.val(*val);
                    let fv = self.cur_fn.expect("ICE: cur_fn not set");
                    let cond = v.into_int_value();
                    let pass_bb = self.ctx.append_basic_block(fv, "assert.pass");
                    let fail_bb = self.ctx.append_basic_block(fv, "assert.fail");
                    b!(self.bld.build_conditional_branch(cond, pass_bb, fail_bb));
                    self.bld.position_at_end(fail_bb);
                    // Print assertion message and abort.
                    if let Some(printf) = self.module.get_function("printf") {
                        let fmt_str = format!("assertion failed: {msg}\n\0");
                        let gv = self
                            .bld
                            .build_global_string_ptr(&fmt_str, "assert.msg")
                            .map_err(|e| e.to_string())?;
                        b!(self
                            .bld
                            .build_call(printf, &[gv.as_pointer_value().into()], ""));
                    }
                    // Call abort.
                    let abort = self.module.get_function("abort").unwrap_or_else(|| {
                        let ft = self.ctx.void_type().fn_type(&[], false);
                        self.module
                            .add_function("abort", ft, Some(Linkage::External))
                    });
                    b!(self.bld.build_call(abort, &[], ""));
                    b!(self.bld.build_unreachable());
                    self.bld.position_at_end(pass_bb);
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }

                // ── Dynamic dispatch ──
                mir::InstKind::DynDispatch(obj, trait_name, method, args) => self
                    .emit_dyn_dispatch(
                        *obj,
                        &trait_name.as_str(),
                        &method.as_str(),
                        args,
                        &inst.ty,
                    ),

                mir::InstKind::DynCoerce(inner, type_name, trait_name) => {
                    let val = self.val(*inner);
                    let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());

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
                        .unwrap_or_else(|| ptr_ty.const_null());

                    let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
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

                mir::InstKind::InlineAsm(template, args) => {
                    let arg_vals: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        args.iter().map(|a| self.val(*a).into()).collect();
                    let arg_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = arg_vals
                        .iter()
                        .map(|v| match v {
                            inkwell::values::BasicMetadataValueEnum::IntValue(iv) => {
                                iv.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::FloatValue(fv) => {
                                fv.get_type().into()
                            }
                            inkwell::values::BasicMetadataValueEnum::PointerValue(pv) => {
                                pv.get_type().into()
                            }
                            _ => self.ctx.i64_type().into(),
                        })
                        .collect();
                    let ft = self.ctx.void_type().fn_type(&arg_tys, false);
                    let asm = self.ctx.create_inline_asm(
                        ft,
                        template.clone(),
                        String::new(), // constraints
                        true,          // has side effects
                        false,         // needs aligned stack
                        None,          // dialect
                        false,         // can throw
                    );
                    b!(self.bld.build_indirect_call(ft, asm, &arg_vals, ""));
                    Ok(self.ctx.i8_type().const_int(0, false).into())
                }
                _ => return Ok(None),
            })?,
        ))
    }
}
