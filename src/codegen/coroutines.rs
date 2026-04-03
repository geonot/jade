use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

/// Generator control block layout (shared between producer coroutine and consumer):
///   offset 0:  coro_ptr        (*jade_coro_t)    — 8 bytes
///   offset 8:  value           (i64)             — 8 bytes
///   offset 16: has_value       (u8)              — 1 byte
///   offset 17: done            (u8)              — 1 byte
///   offset 24: caller_ctx_ptr  (*jade_context_t) — 8 bytes
/// Total: 32 bytes
impl<'ctx> Compiler<'ctx> {
    pub(crate) const GEN_CORO_PTR_OFF: u64 = 0;
    pub(crate) const GEN_VALUE_OFF: u64 = 8;
    pub(crate) const GEN_HAS_VALUE_OFF: u64 = 16;
    pub(crate) const GEN_DONE_OFF: u64 = 17;
    pub(crate) const GEN_CALLER_CTX_PTR_OFF: u64 = 24;
    pub(crate) const GEN_SIZE: u64 = 32;

    pub(crate) fn gen_field_ptr(
        &self,
        gen_ptr: inkwell::values::PointerValue<'ctx>,
        offset: u64,
        name: &str,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();
        if offset == 0 {
            Ok(gen_ptr)
        } else {
            Ok(unsafe {
                b!(self
                    .bld
                    .build_gep(i8t, gen_ptr, &[i64t.const_int(offset, false)], name))
            })
        }
    }

    /// Declare jade_gen_resume, jade_gen_suspend, jade_gen_destroy if not already declared.
    pub(crate) fn declare_gen_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let void = self.ctx.void_type();
        let ft = void.fn_type(&[ptr.into()], false);

        for name in &["jade_gen_resume", "jade_gen_suspend", "jade_gen_destroy"] {
            if self.module.get_function(name).is_none() {
                self.module
                    .add_function(name, ft, Some(Linkage::External));
            }
        }
    }

    pub(crate) fn compile_coroutine_create(
        &mut self,
        name: &str,
        body: &[hir::Stmt],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.declare_actor_runtime();
        self.declare_gen_runtime();

        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();
        let i32t = self.ctx.i32_type();
        let i64t = self.ctx.i64_type();
        let void = self.ctx.void_type();

        // Build the coroutine body function: void __coro_<name>(void *arg)
        let coro_fn_name = format!("__coro_{name}");
        let fn_ty = void.fn_type(&[ptr.into()], false);
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

        // arg is the generator control block pointer
        let gen_ptr_param = coro_fn.get_first_param().unwrap().into_pointer_value();
        let gen_ptr_alloca = self.entry_alloca(ptr.into(), "__coro_ctx");
        b!(self.bld.build_store(gen_ptr_alloca, gen_ptr_param));

        self.set_var(
            "__coro_ctx",
            gen_ptr_alloca,
            Type::Ptr(Box::new(Type::I64)),
        );

        self.compile_coroutine_body(body)?;

        // Mark done and suspend back to caller (final suspension)
        if self.no_term() {
            let gen_ptr_val =
                b!(self.bld.build_load(ptr, gen_ptr_alloca, "gen.ptr")).into_pointer_value();
            let done_ptr = self.gen_field_ptr(gen_ptr_val, Self::GEN_DONE_OFF, "gen.done")?;
            b!(self.bld.build_store(done_ptr, i8t.const_int(1, false)));
            let gen_suspend = self.module.get_function("jade_gen_suspend").unwrap();
            b!(self
                .bld
                .build_call(gen_suspend, &[gen_ptr_val.into()], ""));
            b!(self.bld.build_unreachable());
        }

        // Restore caller context
        self.cur_fn = saved_fn;
        self.vars = saved_vars;
        self.loop_stack = saved_loop_stack;

        let fv = self.cur_fn.unwrap();
        let bb = saved_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "coro.after"));
        self.bld.position_at_end(bb);

        // Allocate and zero-init the generator control block
        let malloc_fn = self.ensure_malloc();
        let gen_mem = b!(self.bld.build_call(
            malloc_fn,
            &[i64t.const_int(Self::GEN_SIZE, false).into()],
            "gen.mem"
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
                gen_mem.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(Self::GEN_SIZE, false).into()
            ],
            ""
        ));

        // Create coroutine via jade_coro_create and store ptr in gen block
        let coro_create = self.module.get_function("jade_coro_create").unwrap();
        let coro = b!(self.bld.build_call(
            coro_create,
            &[
                coro_fn.as_global_value().as_pointer_value().into(),
                gen_mem.into(),
            ],
            "gen.coro"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap();

        let coro_ptr_field = self.gen_field_ptr(gen_mem, Self::GEN_CORO_PTR_OFF, "gen.coro_ptr")?;
        b!(self.bld.build_store(coro_ptr_field, coro));

        // Generator is NOT scheduled — it runs via direct context swap (jade_gen_resume/suspend)

        if name != "__anon" {
            let name_alloca = self.entry_alloca(ptr.into(), name);
            b!(self.bld.build_store(name_alloca, gen_mem));
            self.set_var(name, name_alloca, Type::Coroutine(Box::new(Type::I64)));
        }

        Ok(gen_mem.into())
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
                if matches!(ty, Type::Array(_, _)) {
                    self.set_var(&bind.name, val.into_pointer_value(), ty.clone());
                } else if let Some((ptr, _)) = self.find_var(&bind.name).cloned() {
                    b!(self.bld.build_store(ptr, val));
                    self.set_var(&bind.name, ptr, ty.clone());
                } else {
                    let a = self.entry_alloca(self.llvm_ty(ty), &bind.name);
                    b!(self.bld.build_store(a, val));
                    self.set_var(&bind.name, a, ty.clone());
                }
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

    /// Yield a value from inside a coroutine body.
    /// Writes value to gen block, sets has_value=1, then suspends
    /// via direct context swap back to the caller.
    fn emit_coroutine_yield(&mut self, val_expr: &hir::Expr) -> Result<(), String> {
        let val = self.compile_expr(val_expr)?;
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i8t = self.ctx.i8_type();

        let (gen_alloca, _) = self
            .find_var("__coro_ctx")
            .cloned()
            .ok_or("internal: no __coro_ctx in coroutine body")?;
        let gen_ptr = b!(self.bld.build_load(ptr, gen_alloca, "gen.ctx")).into_pointer_value();

        // Write value
        let value_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_VALUE_OFF, "gen.y.val")?;
        let i64_val = self.coerce_to_i64(val);
        b!(self.bld.build_store(value_ptr, i64_val));

        // Set has_value = 1
        let has_val_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_HAS_VALUE_OFF, "gen.y.hv")?;
        b!(self.bld.build_store(has_val_ptr, i8t.const_int(1, false)));

        // Suspend back to caller via direct context swap
        let gen_suspend = self.module.get_function("jade_gen_suspend").unwrap();
        b!(self
            .bld
            .build_call(gen_suspend, &[gen_ptr.into()], ""));

        Ok(())
    }

    pub(crate) fn coerce_to_i64(&self, val: BasicValueEnum<'ctx>) -> inkwell::values::IntValue<'ctx> {
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

    /// .next() — read the next yielded value from a generator.
    /// Directly resumes the producer coroutine via context swap.
    /// When the producer yields or finishes, control returns here.
    pub(crate) fn compile_coroutine_next(
        &mut self,
        coro_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let gen_ptr = self.compile_expr(coro_expr)?.into_pointer_value();
        let i8t = self.ctx.i8_type();
        let i64t = self.ctx.i64_type();

        // Resume the producer coroutine (direct context swap)
        let gen_resume = self.module.get_function("jade_gen_resume").unwrap();
        b!(self
            .bld
            .build_call(gen_resume, &[gen_ptr.into()], ""));

        // After resume returns, the producer has either yielded a value or finished.
        // Read the value (0 if done without yielding).
        let value_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_VALUE_OFF, "gen.n.val")?;
        let result = b!(self.bld.build_load(i64t, value_ptr, "gen.result"));

        // Clear has_value
        let has_val_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_HAS_VALUE_OFF, "gen.n.hv")?;
        b!(self.bld.build_store(has_val_ptr, i8t.const_int(0, false)));

        Ok(result)
    }

    fn compile_coroutine_for(&mut self, f: &hir::For) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();

        // Handle `for i in N` (means 0..N) vs `for i from A to B`
        let start_val = if f.end.is_some() {
            self.compile_expr(&f.iter)?
        } else {
            i64t.const_int(0, false).into()
        };
        let end_val = if let Some(ref end) = f.end {
            self.compile_expr(end)?
        } else {
            self.compile_expr(&f.iter)?
        };

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

        let cur =
            b!(self.bld.build_load(self.llvm_ty(&f.bind_ty), lvar, "cur")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur, end_val.into_int_value(), "cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

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
        let next = b!(self.bld.build_int_nsw_add(cur, step, "next"));
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
        let cond_val = self.compile_expr(&w.cond)?;
        let cond = self.to_bool(cond_val);
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
