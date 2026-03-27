use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) const CORO_MUTEX_OFF: u64 = 0;
    pub(crate) const CORO_COND_PROD_OFF: u64 = 40;
    pub(crate) const CORO_COND_CONS_OFF: u64 = 88;
    pub(crate) const CORO_VALUE_OFF: u64 = 136;
    pub(crate) const CORO_HAS_VALUE_OFF: u64 = 144;
    pub(crate) const CORO_DONE_OFF: u64 = 145;
    pub(crate) const CORO_SIZE: u64 = 160;

    pub(crate) fn coro_field_ptr(
        &self,
        coro_ptr: inkwell::values::PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        if offset == 0 {
            Ok(coro_ptr)
        } else {
            Ok(unsafe {
                b!(self
                    .bld
                    .build_gep(i8t, coro_ptr, &[i64t.const_int(offset, false)], name))
            })
        }
    }

    pub(crate) fn compile_coroutine_create(
        &mut self,
        name: &str,
        body: &[hir::Stmt],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.declare_actor_runtime();

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i32t = self.ctx.i32_type();
        macro_rules! ensure {
            ($name:expr, $ft:expr) => {
                if self.module.get_function($name).is_none() {
                    self.module
                        .add_function($name, $ft, Some(Linkage::External));
                }
            };
        }
        ensure!(
            "pthread_create",
            i32t.fn_type(&[ptr.into(), ptr.into(), ptr.into(), ptr.into()], false)
        );
        ensure!(
            "pthread_join",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        ensure!(
            "pthread_mutex_init",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        ensure!("pthread_mutex_lock", i32t.fn_type(&[ptr.into()], false));
        ensure!("pthread_mutex_unlock", i32t.fn_type(&[ptr.into()], false));
        ensure!(
            "pthread_cond_init",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        ensure!(
            "pthread_cond_wait",
            i32t.fn_type(&[ptr.into(), ptr.into()], false)
        );
        ensure!("pthread_cond_signal", i32t.fn_type(&[ptr.into()], false));

        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        let coro_fn_name = format!("__coro_{name}");
        let fn_ty = ptr.fn_type(&[ptr.into()], false);
        let coro_fn = self
            .module
            .add_function(&coro_fn_name, fn_ty, Some(Linkage::Internal));

        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        let saved_vars = std::mem::replace(&mut self.vars, vec![std::collections::HashMap::new()]);
        let saved_loop_stack = std::mem::replace(&mut self.loop_stack, Vec::new());

        self.cur_fn = Some(coro_fn);
        let entry = self.ctx.append_basic_block(coro_fn, "entry");
        self.bld.position_at_end(entry);

        let coro_ptr_param = coro_fn.get_first_param().unwrap().into_pointer_value();
        let coro_ptr_alloca = self.entry_alloca(ptr.into(), "__coro_ctx");
        b!(self.bld.build_store(coro_ptr_alloca, coro_ptr_param));

        self.set_var(
            "__coro_ctx",
            coro_ptr_alloca,
            Type::Ptr(Box::new(Type::I64)),
        );

        self.compile_coroutine_body(body)?;

        let coro_ptr_val =
            b!(self.bld.build_load(ptr, coro_ptr_alloca, "coro.ptr")).into_pointer_value();

        let mutex_ptr = self.coro_field_ptr(coro_ptr_val, Self::CORO_MUTEX_OFF, "coro.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        let done_ptr = self.coro_field_ptr(coro_ptr_val, Self::CORO_DONE_OFF, "coro.done")?;
        b!(self.bld.build_store(done_ptr, i8t.const_int(1, false)));

        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        let cond_cons_ptr =
            self.coro_field_ptr(coro_ptr_val, Self::CORO_COND_CONS_OFF, "coro.cond_cons")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self
            .bld
            .build_call(cond_signal_fn, &[cond_cons_ptr.into()], ""));

        let null = ptr.const_null();
        b!(self.bld.build_return(Some(&null)));

        self.cur_fn = saved_fn;
        self.vars = saved_vars;
        self.loop_stack = saved_loop_stack;

        let fv = self.cur_fn.unwrap();
        let bb = saved_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "coro.after"));
        self.bld.position_at_end(bb);

        let malloc_fn = self.module.get_function("malloc").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[i64t.into()], false);
            self.module
                .add_function("malloc", ft, Some(Linkage::External))
        });
        let coro_mem = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(Self::CORO_SIZE, false).into()],
            "coro.mem"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let memset_fn = self.module.get_function("memset").unwrap_or_else(|| {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External))
        });
        b!(self.bld.build_call(
            memset_fn,
            &[
                coro_mem.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(Self::CORO_SIZE, false).into()
            ],
            ""
        ));

        let mutex_ptr = self.coro_field_ptr(coro_mem, Self::CORO_MUTEX_OFF, "coro.mutex.init")?;
        let mutex_init_fn = self.module.get_function("pthread_mutex_init").unwrap();
        b!(self.bld.build_call(
            mutex_init_fn,
            &[mutex_ptr.into(), ptr.const_null().into()],
            ""
        ));

        let cond_init_fn = self.module.get_function("pthread_cond_init").unwrap();
        let cond_prod_ptr =
            self.coro_field_ptr(coro_mem, Self::CORO_COND_PROD_OFF, "coro.cond_prod.init")?;
        b!(self.bld.build_call(
            cond_init_fn,
            &[cond_prod_ptr.into(), ptr.const_null().into()],
            ""
        ));
        let cond_cons_ptr =
            self.coro_field_ptr(coro_mem, Self::CORO_COND_CONS_OFF, "coro.cond_cons.init")?;
        b!(self.bld.build_call(
            cond_init_fn,
            &[cond_cons_ptr.into(), ptr.const_null().into()],
            ""
        ));

        let thread_storage = self.entry_alloca(i64t.into(), "coro.tid");
        let create_fn = self.module.get_function("pthread_create").unwrap();
        b!(self.bld.build_call(
            create_fn,
            &[
                thread_storage.into(),
                ptr.const_null().into(),
                coro_fn.as_global_value().as_pointer_value().into(),
                coro_mem.into(),
            ],
            ""
        ));

        if name != "__anon" {
            let name_alloca = self.entry_alloca(ptr.into(), name);
            b!(self.bld.build_store(name_alloca, coro_mem));
            self.set_var(name, name_alloca, Type::Coroutine(Box::new(Type::I64)));
        }

        Ok(coro_mem.into())
    }

    fn compile_coroutine_body(&mut self, body: &[hir::Stmt]) -> Result<(), String> {
        for stmt in body {
            self.compile_coroutine_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_coroutine_stmt(&mut self, stmt: &hir::Stmt) -> Result<(), String> {
        match stmt {
            hir::Stmt::For(f) => {
                self.compile_coroutine_for(f)?;
            }
            hir::Stmt::While(w) => {
                self.compile_coroutine_while(w)?;
            }
            hir::Stmt::Loop(l) => {
                self.compile_coroutine_loop(l)?;
            }
            hir::Stmt::Expr(e) => {
                if let hir::ExprKind::Yield(inner) = &e.kind {
                    self.emit_coroutine_yield(inner)?;
                } else {
                    self.compile_expr(e)?;
                }
            }
            hir::Stmt::Ret(val, _, _) => {
                if let Some(e) = val {
                    self.emit_coroutine_yield(e)?;
                }
            }
            hir::Stmt::Bind(bind) => {
                let val = self.compile_expr(&bind.value)?;
                let ty = &bind.ty;
                let a = self.entry_alloca(self.llvm_ty(ty), &bind.name);
                b!(self.bld.build_store(a, val));
                self.set_var(&bind.name, a, ty.clone());
            }
            hir::Stmt::Assign(target, value, _) => {
                self.compile_assign(target, value)?;
            }
            hir::Stmt::If(i) => {
                self.compile_if(i)?;
            }
            _ => {
                self.compile_stmt(stmt)?;
            }
        }
        Ok(())
    }

    fn emit_coroutine_yield(&mut self, val_expr: &hir::Expr) -> Result<(), String> {
        let val = self.compile_expr(val_expr)?;
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();

        let (coro_alloca, _) = self
            .find_var("__coro_ctx")
            .cloned()
            .ok_or("internal: no __coro_ctx in coroutine body")?;
        let coro_ptr = b!(self.bld.build_load(ptr, coro_alloca, "coro.ctx")).into_pointer_value();

        let mutex_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_MUTEX_OFF, "coro.y.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        let fv = self.cur_fn.unwrap();
        let wait_bb = self.ctx.append_basic_block(fv, "coro.yield.wait");
        let write_bb = self.ctx.append_basic_block(fv, "coro.yield.write");

        b!(self.bld.build_unconditional_branch(wait_bb));
        self.bld.position_at_end(wait_bb);

        let has_val_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.y.hv")?;
        let has_val = b!(self.bld.build_load(i8t, has_val_ptr, "hv")).into_int_value();
        let is_full = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            has_val,
            i8t.const_int(1, false),
            "full"
        ));
        let wait_body_bb = self.ctx.append_basic_block(fv, "coro.yield.waitbody");
        b!(self
            .bld
            .build_conditional_branch(is_full, wait_body_bb, write_bb));

        self.bld.position_at_end(wait_body_bb);
        let cond_prod_ptr =
            self.coro_field_ptr(coro_ptr, Self::CORO_COND_PROD_OFF, "coro.y.cprod")?;
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self
            .bld
            .build_call(cond_wait_fn, &[cond_prod_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        self.bld.position_at_end(write_bb);
        let value_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_VALUE_OFF, "coro.y.val")?;
        let i64_val = self.coerce_to_i64(val);
        b!(self.bld.build_store(value_ptr, i64_val));

        let has_val_ptr2 = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.y.hv2")?;
        b!(self.bld.build_store(has_val_ptr2, i8t.const_int(1, false)));

        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        let cond_cons_ptr =
            self.coro_field_ptr(coro_ptr, Self::CORO_COND_CONS_OFF, "coro.y.ccons")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self
            .bld
            .build_call(cond_signal_fn, &[cond_cons_ptr.into()], ""));

        Ok(())
    }

    fn coerce_to_i64(&self, val: BasicValueEnum<'ctx>) -> inkwell::values::IntValue<'ctx> {
        let i64t = self.ctx.i64_type();
        match val {
            BasicValueEnum::IntValue(iv) => {
                if iv.get_type().get_bit_width() == 64 {
                    iv
                } else if iv.get_type().get_bit_width() < 64 {
                    self.bld.build_int_z_extend(iv, i64t, "zext").unwrap()
                } else {
                    self.bld.build_int_truncate(iv, i64t, "trunc").unwrap()
                }
            }
            BasicValueEnum::FloatValue(fv) => {
                self.bld.build_float_to_signed_int(fv, i64t, "f2i").unwrap()
            }
            BasicValueEnum::PointerValue(pv) => self.bld.build_ptr_to_int(pv, i64t, "p2i").unwrap(),
            _ => i64t.const_int(0, false),
        }
    }

    pub(crate) fn compile_coroutine_next(
        &mut self,
        coro_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let coro_ptr = self.compile_expr(coro_expr)?.into_pointer_value();
        let _ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.unwrap();

        let mutex_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_MUTEX_OFF, "coro.n.mutex")?;
        let lock_fn = self.module.get_function("pthread_mutex_lock").unwrap();
        b!(self.bld.build_call(lock_fn, &[mutex_ptr.into()], ""));

        let wait_bb = self.ctx.append_basic_block(fv, "coro.next.wait");
        let read_bb = self.ctx.append_basic_block(fv, "coro.next.read");

        b!(self.bld.build_unconditional_branch(wait_bb));
        self.bld.position_at_end(wait_bb);

        let has_val_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.n.hv")?;
        let has_val = b!(self.bld.build_load(i8t, has_val_ptr, "hv")).into_int_value();
        let done_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_DONE_OFF, "coro.n.done")?;
        let done_val = b!(self.bld.build_load(i8t, done_ptr, "done")).into_int_value();

        let has_no_val = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            has_val,
            i8t.const_int(0, false),
            "novalue"
        ));
        let not_done = b!(self.bld.build_int_compare(
            IntPredicate::EQ,
            done_val,
            i8t.const_int(0, false),
            "notdone"
        ));
        let should_wait = b!(self.bld.build_and(has_no_val, not_done, "shouldwait"));

        let wait_body_bb = self.ctx.append_basic_block(fv, "coro.next.waitbody");
        b!(self
            .bld
            .build_conditional_branch(should_wait, wait_body_bb, read_bb));

        self.bld.position_at_end(wait_body_bb);
        let cond_cons_ptr =
            self.coro_field_ptr(coro_ptr, Self::CORO_COND_CONS_OFF, "coro.n.ccons")?;
        let cond_wait_fn = self.module.get_function("pthread_cond_wait").unwrap();
        b!(self
            .bld
            .build_call(cond_wait_fn, &[cond_cons_ptr.into(), mutex_ptr.into()], ""));
        b!(self.bld.build_unconditional_branch(wait_bb));

        self.bld.position_at_end(read_bb);
        let value_ptr = self.coro_field_ptr(coro_ptr, Self::CORO_VALUE_OFF, "coro.n.val")?;
        let result = b!(self.bld.build_load(i64t, value_ptr, "coro.result"));

        let has_val_ptr2 = self.coro_field_ptr(coro_ptr, Self::CORO_HAS_VALUE_OFF, "coro.n.hv2")?;
        b!(self.bld.build_store(has_val_ptr2, i8t.const_int(0, false)));

        let unlock_fn = self.module.get_function("pthread_mutex_unlock").unwrap();
        b!(self.bld.build_call(unlock_fn, &[mutex_ptr.into()], ""));

        let cond_prod_ptr =
            self.coro_field_ptr(coro_ptr, Self::CORO_COND_PROD_OFF, "coro.n.cprod")?;
        let cond_signal_fn = self.module.get_function("pthread_cond_signal").unwrap();
        b!(self
            .bld
            .build_call(cond_signal_fn, &[cond_prod_ptr.into()], ""));

        Ok(result)
    }

    fn compile_coroutine_for(&mut self, f: &hir::For) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();

        let start_val = self.compile_expr(&f.iter)?;
        let lvar = self.entry_alloca(self.llvm_ty(&f.bind_ty), &f.bind);
        b!(self.bld.build_store(lvar, start_val));
        self.set_var(&f.bind, lvar, f.bind_ty.clone());

        let cond_bb = self.ctx.append_basic_block(fv, "coro.for.cond");
        let body_bb = self.ctx.append_basic_block(fv, "coro.for.body");
        let inc_bb = self.ctx.append_basic_block(fv, "coro.for.inc");
        let end_bb = self.ctx.append_basic_block(fv, "coro.for.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: inc_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);

        if let Some(ref end) = f.end {
            let cur =
                b!(self.bld.build_load(self.llvm_ty(&f.bind_ty), lvar, "cur")).into_int_value();
            let end_val = self.compile_expr(end)?.into_int_value();
            let cmp = b!(self
                .bld
                .build_int_compare(IntPredicate::SLT, cur, end_val, "cmp"));
            b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));
        } else {
            b!(self.bld.build_unconditional_branch(body_bb));
        }

        self.bld.position_at_end(body_bb);
        for stmt in &f.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(inc_bb));
        }

        self.bld.position_at_end(inc_bb);
        let cur = b!(self.bld.build_load(self.llvm_ty(&f.bind_ty), lvar, "cur")).into_int_value();
        let step = if let Some(ref s) = f.step {
            self.compile_expr(s)?.into_int_value()
        } else {
            i64t.const_int(1, false)
        };
        let next = b!(self.bld.build_int_add(cur, step, "next"));
        b!(self.bld.build_store(lvar, next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }

    fn compile_coroutine_while(&mut self, w: &hir::While) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let cond_bb = self.ctx.append_basic_block(fv, "coro.while.cond");
        let body_bb = self.ctx.append_basic_block(fv, "coro.while.body");
        let end_bb = self.ctx.append_basic_block(fv, "coro.while.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: cond_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let cond = self.compile_expr(&w.cond)?.into_int_value();
        b!(self.bld.build_conditional_branch(cond, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        for stmt in &w.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(cond_bb));
        }

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }

    fn compile_coroutine_loop(&mut self, l: &hir::Loop) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let body_bb = self.ctx.append_basic_block(fv, "coro.loop.body");
        let end_bb = self.ctx.append_basic_block(fv, "coro.loop.end");

        self.loop_stack.push(super::LoopCtx {
            continue_bb: body_bb,
            break_bb: end_bb,
        });

        b!(self.bld.build_unconditional_branch(body_bb));
        self.bld.position_at_end(body_bb);
        for stmt in &l.body {
            self.compile_coroutine_stmt(stmt)?;
        }
        if self.no_term() {
            b!(self.bld.build_unconditional_branch(body_bb));
        }

        self.bld.position_at_end(end_bb);
        self.loop_stack.pop();
        Ok(())
    }
}
