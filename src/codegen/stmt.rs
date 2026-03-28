use inkwell::basic_block::BasicBlock;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_block(
        &mut self,
        block: &hir::Block,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let mut last = None;
        for s in block {
            let v = self.compile_stmt(s)?;
            if !matches!(s, hir::Stmt::Drop(..)) {
                last = v;
            }
        }
        Ok(last)
    }

    pub(crate) fn compile_stmt(
        &mut self,
        stmt: &hir::Stmt,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let span = match stmt {
            hir::Stmt::Bind(b) => b.span,
            hir::Stmt::TupleBind(_, _, s) => *s,
            hir::Stmt::Assign(_, _, s) => *s,
            hir::Stmt::Expr(e) => e.span,
            hir::Stmt::If(i) => i.cond.span,
            hir::Stmt::While(w) => w.cond.span,
            hir::Stmt::For(f) => f.iter.span,
            hir::Stmt::Loop(l) => l
                .body
                .first()
                .map(|s| match s {
                    hir::Stmt::Expr(e) => e.span,
                    _ => crate::ast::Span::dummy(),
                })
                .unwrap_or(crate::ast::Span::dummy()),
            hir::Stmt::Ret(_, _, s) => *s,
            hir::Stmt::Break(_, s) => *s,
            hir::Stmt::Continue(s) => *s,
            hir::Stmt::Match(m) => m.subject.span,
            hir::Stmt::Asm(a) => a.span,
            hir::Stmt::Drop(_, _, _, s) => *s,
            hir::Stmt::ErrReturn(_, _, s) => *s,
            hir::Stmt::StoreInsert(_, _, s) => *s,
            hir::Stmt::StoreDelete(_, _, s) => *s,
            hir::Stmt::StoreSet(_, _, _, s) => *s,
            hir::Stmt::Transaction(_, s) => *s,
            hir::Stmt::ChannelClose(_, s) => *s,
            hir::Stmt::Stop(_, s) => *s,
        };
        self.set_debug_location(span.line, span.col);
        match stmt {
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
                Ok(None)
            }
            hir::Stmt::TupleBind(bindings, value, _) => {
                let val = self.compile_expr(value)?;
                let tys: Vec<Type> = bindings.iter().map(|(_, _, ty)| ty.clone()).collect();
                let st = self.ctx.struct_type(
                    &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                    false,
                );
                let tmp = self.entry_alloca(st.into(), "tup.tmp");
                b!(self.bld.build_store(tmp, val));
                for (i, (_, name, ety)) in bindings.iter().enumerate() {
                    let lty = self.llvm_ty(ety);
                    let gep = b!(self.bld.build_struct_gep(st, tmp, i as u32, "tup.d"));
                    let elem = b!(self.bld.build_load(lty, gep, name));
                    let a = self.entry_alloca(lty, name);
                    b!(self.bld.build_store(a, elem));
                    self.set_var(name, a, ety.clone());
                }
                Ok(None)
            }
            hir::Stmt::Assign(target, value, _) => {
                self.compile_assign(target, value)?;
                Ok(None)
            }
            hir::Stmt::Expr(e) => Ok(Some(self.compile_expr(e)?)),
            hir::Stmt::If(i) => self.compile_if(i),
            hir::Stmt::While(w) => self.compile_while(w),
            hir::Stmt::For(f) => self.compile_for(f),
            hir::Stmt::Loop(l) => self.compile_loop(l),
            hir::Stmt::Ret(v, _ty, _) => {
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
            hir::Stmt::Break(_, _) => {
                if let Some(lctx) = self.loop_stack.last() {
                    let bb = lctx.break_bb;
                    b!(self.bld.build_unconditional_branch(bb));
                }
                Ok(None)
            }
            hir::Stmt::Continue(_) => {
                if let Some(lctx) = self.loop_stack.last() {
                    let bb = lctx.continue_bb;
                    b!(self.bld.build_unconditional_branch(bb));
                }
                Ok(None)
            }
            hir::Stmt::Match(m) => self.compile_match(m),
            hir::Stmt::Asm(a) => self.compile_asm(a),
            hir::Stmt::Drop(def_id, name, ty, _span) => {
                if self.hints.elide_drops.contains(def_id) {
                    return Ok(None);
                }
                if self.hints.reuse_candidates.contains_key(def_id)
                    || self.hints.speculative_reuse.contains_key(def_id)
                {
                    return Ok(None);
                }
                if self.hints.borrow_to_move.contains(def_id) {
                    return Ok(None);
                }
                match ty {
                    Type::String => {
                        if let Some((ptr, _)) = self.find_var(name).cloned() {
                            let st = self.string_type();
                            let val = b!(self.bld.build_load(st, ptr, "drop.str"));
                            self.drop_string(val)?;
                        }
                    }
                    Type::Vec(_) => {
                        if let Some((ptr, _)) = self.find_var(name).cloned() {
                            self.drop_vec(ptr)?;
                        }
                    }
                    Type::Map(_, _) => {
                        if let Some((ptr, _)) = self.find_var(name).cloned() {
                            self.drop_map(ptr)?;
                        }
                    }
                    Type::Rc(inner) => {
                        if let Some((ptr, _)) = self.find_var(name).cloned() {
                            let loaded = b!(self.bld.build_load(
                                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                                ptr,
                                "rc.ptr"
                            ));
                            self.rc_release(loaded, inner)?;
                        }
                    }
                    Type::Weak(inner) => {
                        if let Some((ptr, _)) = self.find_var(name).cloned() {
                            let loaded = b!(self.bld.build_load(
                                self.ctx.ptr_type(inkwell::AddressSpace::default()),
                                ptr,
                                "weak.ptr"
                            ));
                            self.weak_release(loaded, inner)?;
                        }
                    }
                    _ => {}
                }
                Ok(None)
            }
            hir::Stmt::ErrReturn(e, _ty, _) => {
                let val = self.compile_expr(e)?;
                b!(self.bld.build_return(Some(&val)));
                Ok(None)
            }
            hir::Stmt::StoreInsert(store_name, values, _) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_insert(store_name, values, &sd)?;
                Ok(None)
            }
            hir::Stmt::StoreDelete(store_name, filter, _) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_delete(store_name, filter, &sd)?;
                Ok(None)
            }
            hir::Stmt::StoreSet(store_name, assignments, filter, _) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_set(store_name, assignments, filter, &sd)?;
                Ok(None)
            }
            hir::Stmt::Transaction(body, _) => {
                self.compile_block(body)?;
                Ok(None)
            }
            hir::Stmt::ChannelClose(ch_expr, _) => {
                self.compile_channel_close(ch_expr)?;
                Ok(None)
            }
            hir::Stmt::Stop(actor_expr, _) => {
                self.compile_stop(actor_expr)?;
                Ok(None)
            }
        }
    }

    pub(crate) fn compile_if(
        &mut self,
        i: &hir::If,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let merge = self.ctx.append_basic_block(fv, "merge");
        let cv = self.compile_expr(&i.cond)?;
        let cond = self.to_bool(cv);
        let then_bb = self.ctx.append_basic_block(fv, "then");
        let mut else_bb = self.ctx.append_basic_block(fv, "else");
        b!(self.bld.build_conditional_branch(cond, then_bb, else_bb));

        let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut all_valued = i.els.is_some();

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
        } else if self.no_term() {
            b!(self.bld.build_unconditional_branch(merge));
        }

        self.bld.position_at_end(merge);
        self.build_match_phi(&phi_in, all_valued)
    }

    pub(crate) fn compile_assign(
        &mut self,
        target: &hir::Expr,
        value: &hir::Expr,
    ) -> Result<(), String> {
        match &target.kind {
            hir::ExprKind::Index(arr_expr, idx_expr) => {
                let arr_ty = &arr_expr.ty;
                let val = self.compile_expr(value)?;
                let idx_val = self.compile_expr(idx_expr)?.into_int_value();
                match arr_ty {
                    Type::Array(elem_ty, n) => {
                        let lty = self.llvm_ty(elem_ty);
                        let arr_llvm = lty.array_type(*n as u32);
                        let arr_ptr = match &arr_expr.kind {
                            hir::ExprKind::Var(_, name) => self
                                .find_var(name)
                                .map(|(ptr, _)| *ptr)
                                .ok_or_else(|| format!("undefined: {name}"))?,
                            _ => return Err("cannot assign to rvalue index".into()),
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
            hir::ExprKind::Field(obj_expr, field, _idx) => {
                let obj_ty = &obj_expr.ty;
                let val = self.compile_expr(value)?;
                let (sname, is_ptr) = match obj_ty {
                    Type::Struct(n, _) => (n.as_str(), false),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(n, _) => (n.as_str(), true),
                        _ => return Err("field assignment only on structs".into()),
                    },
                    _ => return Err("field assignment only on structs".into()),
                };
                {
                    let fields = self
                        .structs
                        .get(sname)
                        .ok_or_else(|| format!("unknown type: {sname}"))?
                        .clone();
                    let idx = fields
                        .iter()
                        .position(|(n, _)| n == field)
                        .ok_or_else(|| format!("no field {field} on {sname}"))?;
                    let obj_ptr = match &obj_expr.kind {
                        hir::ExprKind::Var(_, name) => self
                            .find_var(name)
                            .map(|(ptr, _)| *ptr)
                            .ok_or_else(|| format!("undefined: {name}"))?,
                        hir::ExprKind::Field(..) => self.compile_lvalue_ptr(obj_expr)?,
                        _ => return Err("cannot assign field on rvalue".into()),
                    };
                    let ftys: Vec<_> = fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
                    let field_lty = ftys[idx];
                    let val = self.coerce_val(val, field_lty);
                    let st = self.ctx.struct_type(&ftys, false);
                    if is_ptr {
                        let struct_ptr = b!(self.bld.build_load(
                            self.ctx.ptr_type(inkwell::AddressSpace::default()),
                            obj_ptr,
                            "self.ptr"
                        ))
                        .into_pointer_value();
                        let gep = b!(self.bld.build_struct_gep(
                            st,
                            struct_ptr,
                            idx as u32,
                            "field.assign"
                        ));
                        b!(self.bld.build_store(gep, val));
                    } else {
                        let gep =
                            b!(self
                                .bld
                                .build_struct_gep(st, obj_ptr, idx as u32, "field.assign"));
                        b!(self.bld.build_store(gep, val));
                    }
                }
            }
            _ => return Err("invalid assignment target".into()),
        }
        Ok(())
    }

    pub(crate) fn compile_asm(
        &mut self,
        asm: &hir::AsmBlock,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
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
                    .collect::<Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>>>(),
                false,
            )
        } else {
            self.ctx.void_type().fn_type(
                &input_types
                    .iter()
                    .map(|t| (*t).into())
                    .collect::<Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>>>(),
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
        let args_meta: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
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

    pub(crate) fn bind_tuple_pat(
        &mut self,
        pats: &[hir::Pat],
        subject_val: BasicValueEnum<'ctx>,
        subject_ty: &Type,
    ) -> Result<(), String> {
        let elem_tys: Vec<Type> = match subject_ty {
            Type::Tuple(ts) => ts.clone(),
            _ => vec![Type::I64; pats.len()],
        };
        let llvm_tys: Vec<BasicTypeEnum<'ctx>> = elem_tys.iter().map(|t| self.llvm_ty(t)).collect();
        let st = self.ctx.struct_type(&llvm_tys, false);
        let tmp = self.entry_alloca(st.into(), "tup.match");
        b!(self.bld.build_store(tmp, subject_val));
        for (i, pat) in pats.iter().enumerate() {
            if let hir::Pat::Bind(_, name, _, _) = pat {
                let ety = elem_tys.get(i).cloned().unwrap_or(Type::I64);
                let lty = self.llvm_ty(&ety);
                let gep = b!(self.bld.build_struct_gep(st, tmp, i as u32, "tup.el"));
                let elem = b!(self.bld.build_load(lty, gep, name));
                let a = self.entry_alloca(lty, name);
                b!(self.bld.build_store(a, elem));
                self.set_var(name, a, ety);
            }
        }
        Ok(())
    }

    pub(crate) fn bind_array_pat(
        &mut self,
        pats: &[hir::Pat],
        subject_val: BasicValueEnum<'ctx>,
        subject_ty: &Type,
    ) -> Result<(), String> {
        let elem_ty = match subject_ty {
            Type::Array(inner, _) => inner.as_ref().clone(),
            _ => Type::I64,
        };
        let arr_ptr = if subject_val.is_pointer_value() {
            subject_val.into_pointer_value()
        } else {
            let alloc = self.entry_alloca(subject_val.get_type(), "arr.tmp");
            b!(self.bld.build_store(alloc, subject_val));
            alloc
        };
        let lty = self.llvm_ty(&elem_ty);
        let i64t = self.ctx.i64_type();
        for (i, pat) in pats.iter().enumerate() {
            if let hir::Pat::Bind(_, name, _, _) = pat {
                let gep = unsafe {
                    b!(self.bld.build_gep(
                        lty,
                        arr_ptr,
                        &[i64t.const_int(i as u64, false)],
                        "arr.el"
                    ))
                };
                let elem = b!(self.bld.build_load(lty, gep, name));
                let a = self.entry_alloca(lty, name);
                b!(self.bld.build_store(a, elem));
                self.set_var(name, a, elem_ty.clone());
            }
        }
        Ok(())
    }
}
