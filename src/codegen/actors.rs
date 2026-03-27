use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

const MAILBOX_CAP: u64 = 256;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn declare_actor_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        if self.module.get_function("malloc").is_none() {
            let ft = ptr.fn_type(&[i64t.into()], false);
            self.module
                .add_function("malloc", ft, Some(Linkage::External));
        }
        if self.module.get_function("memset").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External));
        }
        if self.module.get_function("free").is_none() {
            let ft = self.ctx.void_type().fn_type(&[ptr.into()], false);
            self.module
                .add_function("free", ft, Some(Linkage::External));
        }
    }

    pub(crate) fn declare_actor(&mut self, ad: &hir::ActorDef) -> Result<(), String> {
        let name = &ad.name;

        let state_name = format!("{name}_state");
        let state_fields: Vec<(String, Type)> = ad
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        let state_ltys: Vec<BasicTypeEnum<'ctx>> =
            state_fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
        let state_st = self.ctx.opaque_struct_type(&state_name);
        state_st.set_body(&state_ltys, false);
        self.structs.insert(state_name.clone(), state_fields);

        let mut max_payload_bytes: u64 = 8;
        for h in &ad.handlers {
            let mut handler_size: u64 = 0;
            for p in &h.params {
                handler_size += self.type_store_size(self.llvm_ty(&p.ty));
            }
            max_payload_bytes = max_payload_bytes.max(handler_size);
        }

        let msg_name = format!("{name}_msg");
        let msg_st = self.ctx.opaque_struct_type(&msg_name);
        let i32t = self.ctx.i32_type();
        let payload_ty = self.ctx.i8_type().array_type(max_payload_bytes as u32);
        msg_st.set_body(&[i32t.into(), payload_ty.into()], false);

        let mb_name = format!("{name}_mailbox");
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let mb_st = self.ctx.opaque_struct_type(&mb_name);
        mb_st.set_body(&[ptr_ty.into(), i32t.into(), state_st.into()], false);

        Ok(())
    }

    pub(crate) fn compile_actor_loop(&mut self, ad: &hir::ActorDef) -> Result<(), String> {
        let name = &ad.name;
        let mb_name = format!("{name}_mailbox");
        let msg_name = format!("{name}_msg");
        let loop_name = format!("{name}_loop");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();

        let mb_st = self.module.get_struct_type(&mb_name).unwrap();
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let fv = self
            .module
            .add_function(&loop_name, ft, Some(Linkage::Internal));

        let entry = self.ctx.append_basic_block(fv, "entry");
        let loop_bb = self.ctx.append_basic_block(fv, "loop");
        let dispatch_bb = self.ctx.append_basic_block(fv, "dispatch");
        let exit_bb = self.ctx.append_basic_block(fv, "exit");

        let old_fn = self.cur_fn;
        self.cur_fn = Some(fv);

        let mb_ptr = fv.get_nth_param(0).unwrap().into_pointer_value();

        self.bld.position_at_end(entry);
        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr")).into_pointer_value();
        let state_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 2, "state_ptr"));

        let msg_alloca = self.entry_alloca(msg_st.into(), "msg_buf");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let chan_recv = self.module.get_function("jade_chan_recv").unwrap();
        let recv_ok =
            b!(self
                .bld
                .build_call(chan_recv, &[ch_ptr.into(), msg_alloca.into()], "recv_ok"))
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_int_value();

        let ok = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            recv_ok,
            i32t.const_int(0, false),
            "ok"
        ));
        b!(self.bld.build_conditional_branch(ok, dispatch_bb, exit_bb));

        self.bld.position_at_end(dispatch_bb);
        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        let tag_val = b!(self.bld.build_load(i32t, tag_ptr, "tag"));
        let payload_ptr = b!(self
            .bld
            .build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));

        if ad.handlers.is_empty() {
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            let mut handler_bbs = Vec::new();
            for h in &ad.handlers {
                let bb = self
                    .ctx
                    .append_basic_block(fv, &format!("handler_{}", h.name));
                handler_bbs.push((h.tag, bb));
            }

            let default_bb = self.ctx.append_basic_block(fv, "default_handler");
            self.bld.position_at_end(default_bb);
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(dispatch_bb);
            let _switch = b!(self.bld.build_switch(
                tag_val.into_int_value(),
                default_bb,
                &handler_bbs
                    .iter()
                    .map(|(tag, bb)| (i32t.const_int(*tag as u64, false), *bb))
                    .collect::<Vec<_>>()
            ));

            let state_name = format!("{name}_state");
            let state_st = self.module.get_struct_type(&state_name).unwrap();

            for (i, h) in ad.handlers.iter().enumerate() {
                let bb = handler_bbs[i].1;
                self.bld.position_at_end(bb);

                self.vars.push(std::collections::HashMap::new());

                for (fi, field) in ad.fields.iter().enumerate() {
                    let field_ptr = b!(self.bld.build_struct_gep(
                        state_st,
                        state_ptr,
                        fi as u32,
                        &format!("state_{}", field.name)
                    ));
                    self.set_var(&field.name, field_ptr, field.ty.clone());
                }

                let i64t = self.ctx.i64_type();
                let mut param_offset: u64 = 0;
                for p in &h.params {
                    let pty = self.llvm_ty(&p.ty);
                    let psize = self.type_store_size(pty);
                    let offset_val = i64t.const_int(param_offset, false);
                    let param_ptr = unsafe {
                        b!(self.bld.build_gep(
                            self.ctx.i8_type(),
                            payload_ptr,
                            &[offset_val.into()],
                            &format!("param_{}_ptr", p.name)
                        ))
                    };
                    let param_val = b!(self.bld.build_load(pty, param_ptr, &p.name));

                    let alloca = self.entry_alloca(pty, &p.name);
                    b!(self.bld.build_store(alloca, param_val));
                    self.set_var(&p.name, alloca, p.ty.clone());

                    param_offset += psize;
                }

                self.compile_block(&h.body)?;

                if self.no_term() {
                    b!(self.bld.build_unconditional_branch(loop_bb));
                }

                self.vars.pop();
            }
        }

        self.bld.position_at_end(exit_bb);
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(())
    }

    pub(crate) fn compile_spawn(
        &mut self,
        actor_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");
        let loop_name = format!("{actor_name}_loop");

        let _ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("actor '{actor_name}' not declared"))?;
        let msg_st = self.module.get_struct_type(&msg_name).unwrap();

        let mb_size = self.type_store_size(mb_st.into());
        let msg_size = self.type_store_size(msg_st.into());

        let malloc_fn = self.module.get_function("malloc").unwrap();
        let memset_fn = self.module.get_function("memset").unwrap();

        let mb_ptr = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(mb_size, false).into()],
            "mb_raw"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[
                mb_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(mb_size, false).into()
            ],
            ""
        ));

        let mb_ptr_v = mb_ptr.into_pointer_value();

        let chan_create = self.module.get_function("jade_chan_create").unwrap();
        let ch = b!(self.bld.build_call(
            chan_create,
            &[
                i64t.const_int(msg_size, false).into(),
                i64t.const_int(MAILBOX_CAP, false).into(),
            ],
            "actor_ch"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();

        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 0, "ch_ptr_ptr"));
        b!(self.bld.build_store(ch_ptr_ptr, ch));

        let alive_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr_v, 1, "alive_ptr"));
        b!(self.bld.build_store(alive_ptr, i32t.const_int(1, false)));

        let loop_fn = self
            .module
            .get_function(&loop_name)
            .ok_or_else(|| format!("actor loop fn '{loop_name}' not found"))?;
        let coro_create = self.module.get_function("jade_coro_create").unwrap();
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                loop_fn.as_global_value().as_pointer_value().into(),
                mb_ptr_v.into(),
            ],
            "actor_coro"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();

        let set_daemon = self.module.get_function("jade_coro_set_daemon").unwrap();
        b!(self.bld.build_call(set_daemon, &[coro.into()], ""));

        let sched_spawn = self.module.get_function("jade_sched_spawn").unwrap();
        b!(self.bld.build_call(sched_spawn, &[coro.into()], ""));

        Ok(mb_ptr_v.into())
    }

    pub(crate) fn compile_send(
        &mut self,
        target: &hir::Expr,
        actor_name: &str,
        _handler_name: &str,
        tag: u32,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();

        let mb_st = self
            .module
            .get_struct_type(&mb_name)
            .ok_or_else(|| format!("mailbox type '{mb_name}' not found"))?;
        let msg_st = self
            .module
            .get_struct_type(&msg_name)
            .ok_or_else(|| format!("message type '{msg_name}' not found"))?;

        let mb_ptr = self.compile_expr(target)?.into_pointer_value();

        let ch_ptr_ptr = b!(self.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr"));

        let msg_alloca = self.entry_alloca(msg_st.into(), "send_msg");

        let tag_ptr = b!(self.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        b!(self
            .bld
            .build_store(tag_ptr, i32t.const_int(tag as u64, false)));

        let payload_ptr = b!(self
            .bld
            .build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));
        let mut arg_offset: u64 = 0;
        for arg in args {
            let val = self.compile_expr(arg)?;
            let pty = self.llvm_ty(&arg.ty);
            let psize = self.type_store_size(pty);
            let offset_val = i64t.const_int(arg_offset, false);
            let dest = unsafe {
                b!(self.bld.build_gep(
                    self.ctx.i8_type(),
                    payload_ptr,
                    &[offset_val.into()],
                    "arg_ptr"
                ))
            };
            b!(self.bld.build_store(dest, val));
            arg_offset += psize;
        }

        let chan_send = self.module.get_function("jade_chan_send").unwrap();
        b!(self
            .bld
            .build_call(chan_send, &[ch_ptr.into(), msg_alloca.into()], ""));

        Ok(i64t.const_int(0, false).into())
    }

}