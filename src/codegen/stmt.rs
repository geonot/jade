use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

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
            // Drop stmts are cleanup side-effects; they must not clobber
            // the block's semantic return value.
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
            hir::Stmt::Loop(l) => l.body.first().map(|s| match s {
                hir::Stmt::Expr(e) => e.span,
                _ => crate::ast::Span::dummy(),
            }).unwrap_or(crate::ast::Span::dummy()),
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
                let sd = self.store_defs.get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_insert(store_name, values, &sd)?;
                Ok(None)
            }
            hir::Stmt::StoreDelete(store_name, filter, _) => {
                let sd = self.store_defs.get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_delete(store_name, filter, &sd)?;
                Ok(None)
            }
            hir::Stmt::StoreSet(store_name, assignments, filter, _) => {
                let sd = self.store_defs.get(store_name)
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

    pub(crate) fn compile_while(
        &mut self,
        w: &hir::While,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
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
        self.loop_stack.push(super::LoopCtx {
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

    pub(crate) fn compile_for(
        &mut self,
        f: &hir::For,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if f.end.is_none() && f.step.is_none() {
            if let Type::Array(ref elem_ty, len) = f.bind_ty {
                return self.compile_for_array(f, elem_ty, len);
            }
            let iter_ty = &f.iter.ty;
            if let Type::Array(elem_ty, len) = iter_ty {
                return self.compile_for_array(f, elem_ty, *len);
            }
            if let Type::Vec(ref elem_ty) = f.bind_ty {
                return self.compile_for_vec(f, elem_ty);
            }
            if let Type::Vec(elem_ty) = iter_ty {
                return self.compile_for_vec(f, elem_ty);
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
        self.loop_stack.push(super::LoopCtx {
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
        f: &hir::For,
        elem_ty: &Type,
        len: usize,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let arr_ptr = match &f.iter.kind {
            hir::ExprKind::Var(_, n) => self
                .find_var(n)
                .map(|(p, _)| *p)
                .ok_or_else(|| format!("undefined: {n}"))?,
            _ => self.compile_expr(&f.iter)?.into_pointer_value(),
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
        self.loop_stack.push(super::LoopCtx {
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

    fn compile_for_vec(
        &mut self,
        f: &hir::For,
        elem_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let i64t = self.ctx.i64_type();
        let vec_val = self.compile_expr(&f.iter)?;
        let vec_ptr = vec_val.into_pointer_value();
        let header_ty = self.vec_header_type();
        let lty = self.llvm_ty(elem_ty);

        // Load len and data ptr
        let len_gep = b!(self.bld.build_struct_gep(header_ty, vec_ptr, 1, "fv.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "fv.len")).into_int_value();
        let ptr_gep = b!(self.bld.build_struct_gep(header_ty, vec_ptr, 0, "fv.ptrp"));
        let data_ptr = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "fv.data"
        )).into_pointer_value();

        let idx_alloca = self.entry_alloca(i64t.into(), "__idx");
        b!(self.bld.build_store(idx_alloca, i64t.const_int(0, false)));
        let elem_alloca = self.entry_alloca(lty, &f.bind);
        self.set_var(&f.bind, elem_alloca, elem_ty.clone());

        let cond_bb = self.ctx.append_basic_block(fv, "fv.cond");
        let body_bb = self.ctx.append_basic_block(fv, "fv.body");
        let inc_bb = self.ctx.append_basic_block(fv, "fv.inc");
        let end_bb = self.ctx.append_basic_block(fv, "fv.end");
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let idx = b!(self.bld.build_load(i64t, idx_alloca, "idx")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, idx, len, "fv.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx], "fv.egep")) };
        let elem = b!(self.bld.build_load(lty, elem_gep, "fv.elem"));
        b!(self.bld.build_store(elem_alloca, elem));

        self.loop_stack.push(super::LoopCtx {
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
        let next = b!(self.bld.build_int_nuw_add(idx, i64t.const_int(1, false), "inc"));
        b!(self.bld.build_store(idx_alloca, next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(end_bb);
        Ok(None)
    }

    pub(crate) fn compile_loop(
        &mut self,
        l: &hir::Loop,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let body_bb = self.ctx.append_basic_block(fv, "loop");
        let end_bb = self.ctx.append_basic_block(fv, "loop.end");
        b!(self.bld.build_unconditional_branch(body_bb));
        self.bld.position_at_end(body_bb);
        self.loop_stack.push(super::LoopCtx {
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

    pub(crate) fn compile_match(
        &mut self,
        m: &hir::Match,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let subject_val = self.compile_expr(&m.subject)?;
        let subject_ty = self.resolve_ty(m.subject.ty.clone());

        let is_enum = matches!(subject_ty, Type::Enum(_))
            || matches!(&subject_ty, Type::Struct(n) if self.enums.contains_key(n));

        if !is_enum {
            return self.compile_value_match(m, subject_val, &subject_ty);
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
        let mut seen_tags = std::collections::HashSet::new();
        for (i, arm) in m.arms.iter().enumerate() {
            match &arm.pat {
                hir::Pat::Ctor(_, tag, _, _) => {
                    if seen_tags.insert(*tag) {
                        cases.push((
                            self.ctx.i32_type().const_int(*tag as u64, false),
                            arm_bbs[i],
                        ));
                    }
                }
                hir::Pat::Or(pats, _) => {
                    for pat in pats {
                        if let hir::Pat::Ctor(_, tag, _, _) = pat {
                            if seen_tags.insert(*tag) {
                                cases.push((
                                    self.ctx.i32_type().const_int(*tag as u64, false),
                                    arm_bbs[i],
                                ));
                            }
                        }
                    }
                }
                hir::Pat::Wild(_) | hir::Pat::Bind(_, _, _, _) => {
                    if default_bb.is_none() {
                        default_bb = Some(arm_bbs[i]);
                    }
                }
                _ => {}
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
            if let hir::Pat::Ctor(vname, _, sub_pats, _) = &arm.pat {
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
                            let heap_ptr = b!(self.bld.build_load(ptr_ty, field_ptr, "box.ptr"))
                                .into_pointer_value();
                            let actual_ty = self.llvm_ty(&fty);
                            let fval = b!(self.bld.build_load(actual_ty, heap_ptr, "field"));
                            if let hir::Pat::Bind(_, bname, _bty, _) = pat {
                                let a = self.entry_alloca(actual_ty, bname);
                                b!(self.bld.build_store(a, fval));
                                self.set_var(bname, a, fty);
                            }
                            offset += 8;
                        } else {
                            let lty = self.llvm_ty(&fty);
                            let fval = b!(self.bld.build_load(lty, field_ptr, "field"));
                            if let hir::Pat::Bind(_, bname, _bty, _) = pat {
                                let a = self.entry_alloca(lty, bname);
                                b!(self.bld.build_store(a, fval));
                                self.set_var(bname, a, fty);
                            }
                            offset += self.type_store_size(lty);
                        }
                    }
                }
            } else if let hir::Pat::Bind(_, ref name, ref _bty, _) = arm.pat {
                let a = self.entry_alloca(st.into(), name);
                b!(self.bld.build_store(a, subject_val));
                self.set_var(name, a, subject_ty.clone());
            }
            if let Some(ref guard) = arm.guard {
                let guard_val = self.compile_expr(guard)?;
                let gv = guard_val.into_int_value();
                let guard_pass = self.ctx.append_basic_block(fv, &format!("match.guard_pass{i}"));
                let guard_fail = if i + 1 < arm_bbs.len() {
                    arm_bbs[i + 1]
                } else {
                    merge_bb
                };
                b!(self.bld.build_conditional_branch(gv, guard_pass, guard_fail));
                self.bld.position_at_end(guard_pass);
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

    fn compile_value_match(
        &mut self,
        m: &hir::Match,
        subject_val: BasicValueEnum<'ctx>,
        subject_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let merge_bb = self.ctx.append_basic_block(fv, "match.end");

        // Check if any arm uses Range or Or patterns — if so, use if-else chain
        let has_complex = m.arms.iter().any(|a| {
            matches!(&a.pat, hir::Pat::Range(..) | hir::Pat::Or(..) | hir::Pat::Tuple(..) | hir::Pat::Array(..))
        });

        if has_complex {
            // If-else chain: separate check blocks + arm blocks
            let check_bbs: Vec<_> = m.arms.iter().enumerate()
                .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.check{i}")))
                .collect();
            let arm_bbs: Vec<_> = m.arms.iter().enumerate()
                .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.arm{i}")))
                .collect();
            // Only convert to int when the subject is actually an int (not tuple/array)
            let iv_opt = subject_val.is_int_value().then(|| subject_val.into_int_value());
            let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
            let mut all_valued = true;

            // Entry → first check block
            b!(self.bld.build_unconditional_branch(check_bbs[0]));

            for (i, arm) in m.arms.iter().enumerate() {
                let next_bb = if i + 1 < check_bbs.len() {
                    check_bbs[i + 1]
                } else {
                    merge_bb
                };

                // Build condition in check block
                self.bld.position_at_end(check_bbs[i]);
                match &arm.pat {
                    hir::Pat::Wild(_) | hir::Pat::Bind(_, _, _, _) => {
                        b!(self.bld.build_unconditional_branch(arm_bbs[i]));
                    }
                    hir::Pat::Lit(expr) => {
                        let lit_val = self.compile_expr(expr)?;
                        let iv = iv_opt.unwrap();
                        let cmp = b!(self.bld.build_int_compare(
                            IntPredicate::EQ, iv, lit_val.into_int_value(), "match.cmp"
                        ));
                        b!(self.bld.build_conditional_branch(cmp, arm_bbs[i], next_bb));
                    }
                    hir::Pat::Range(lo, hi, _) => {
                        let iv = iv_opt.unwrap();
                        let lo_val = self.compile_expr(lo)?.into_int_value();
                        let hi_val = self.compile_expr(hi)?.into_int_value();
                        let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, iv, lo_val, "rng.ge"));
                        let le = b!(self.bld.build_int_compare(IntPredicate::SLE, iv, hi_val, "rng.le"));
                        let in_range = b!(self.bld.build_and(ge, le, "rng.in"));
                        b!(self.bld.build_conditional_branch(in_range, arm_bbs[i], next_bb));
                    }
                    hir::Pat::Or(pats, _) => {
                        let iv = iv_opt.unwrap();
                        let mut any_match = self.ctx.bool_type().const_int(0, false);
                        for pat in pats {
                            let sub_match = match pat {
                                hir::Pat::Lit(e) => {
                                    let lv = self.compile_expr(e)?.into_int_value();
                                    b!(self.bld.build_int_compare(IntPredicate::EQ, iv, lv, "or.cmp"))
                                }
                                hir::Pat::Range(lo, hi, _) => {
                                    let lo_val = self.compile_expr(lo)?.into_int_value();
                                    let hi_val = self.compile_expr(hi)?.into_int_value();
                                    let ge = b!(self.bld.build_int_compare(IntPredicate::SGE, iv, lo_val, "or.ge"));
                                    let le = b!(self.bld.build_int_compare(IntPredicate::SLE, iv, hi_val, "or.le"));
                                    b!(self.bld.build_and(ge, le, "or.rng"))
                                }
                                _ => self.ctx.bool_type().const_int(1, false),
                            };
                            any_match = b!(self.bld.build_or(any_match, sub_match, "or.any"));
                        }
                        b!(self.bld.build_conditional_branch(any_match, arm_bbs[i], next_bb));
                    }
                    _ => {
                        b!(self.bld.build_unconditional_branch(arm_bbs[i]));
                    }
                }

                // Compile arm body in arm block
                self.bld.position_at_end(arm_bbs[i]);
                self.vars.push(HashMap::new());
                if let hir::Pat::Bind(_, ref name, ref _bty, _) = arm.pat {
                    let a = self.entry_alloca(self.llvm_ty(subject_ty), name);
                    b!(self.bld.build_store(a, subject_val));
                    self.set_var(name, a, subject_ty.clone());
                } else if let hir::Pat::Tuple(ref pats, _) = arm.pat {
                    self.bind_tuple_pat(pats, subject_val, subject_ty)?;
                } else if let hir::Pat::Array(ref pats, _) = arm.pat {
                    self.bind_array_pat(pats, subject_val, subject_ty)?;
                }
                if let Some(ref guard) = arm.guard {
                    let guard_val = self.compile_expr(guard)?;
                    let gv = guard_val.into_int_value();
                    let guard_pass = self.ctx.append_basic_block(fv, &format!("match.guard_pass{i}"));
                    b!(self.bld.build_conditional_branch(gv, guard_pass, next_bb));
                    self.bld.position_at_end(guard_pass);
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
            return self.build_match_phi(&phi_in, all_valued);
        }

        // Original switch-based path for simple Lit/Wild/Bind patterns
        let arm_bbs: Vec<_> = m
            .arms
            .iter()
            .enumerate()
            .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.arm{i}")))
            .collect();
        let iv = subject_val.into_int_value();
        let mut cases = Vec::new();
        let mut default_bb = None;
        for (i, arm) in m.arms.iter().enumerate() {
            match &arm.pat {
                hir::Pat::Lit(expr) => {
                    if let hir::ExprKind::Int(n) = expr.kind {
                        cases.push((self.ctx.i64_type().const_int(n as u64, true), arm_bbs[i]));
                    } else if let hir::ExprKind::Bool(b) = expr.kind {
                        cases.push((self.ctx.bool_type().const_int(b as u64, false), arm_bbs[i]));
                    }
                }
                hir::Pat::Wild(_) | hir::Pat::Bind(_, _, _, _) => {
                    if default_bb.is_none() {
                        default_bb = Some(arm_bbs[i]);
                    }
                }
                _ => return Err("unsupported match pattern".into()),
            }
        }
        let def = default_bb.unwrap_or_else(|| self.ctx.append_basic_block(fv, "match.unreach"));
        b!(self.bld.build_switch(iv, def, &cases));
        if default_bb.is_none() {
            self.bld.position_at_end(def);
            b!(self.bld.build_unreachable());
        }
        let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut all_valued = true;
        for (i, arm) in m.arms.iter().enumerate() {
            self.bld.position_at_end(arm_bbs[i]);
            if let hir::Pat::Bind(_, ref name, ref _bty, _) = arm.pat {
                let a = self.entry_alloca(self.llvm_ty(subject_ty), name);
                b!(self.bld.build_store(a, subject_val));
                self.set_var(name, a, subject_ty.clone());
            }
            if let Some(ref guard) = arm.guard {
                let guard_val = self.compile_expr(guard)?;
                let gv = guard_val.into_int_value();
                let guard_pass = self.ctx.append_basic_block(fv, &format!("match.guard_pass{i}"));
                let guard_fail = if i + 1 < arm_bbs.len() {
                    arm_bbs[i + 1]
                } else {
                    merge_bb
                };
                b!(self.bld.build_conditional_branch(gv, guard_pass, guard_fail));
                self.bld.position_at_end(guard_pass);
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
        self.build_match_phi(&phi_in, all_valued)
    }

    pub(crate) fn build_match_phi(
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
                if let Type::Struct(sname) = obj_ty {
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
                        _ => return Err("cannot assign field on rvalue".into()),
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

    fn bind_tuple_pat(
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
            // Wild pattern = skip
        }
        Ok(())
    }

    fn bind_array_pat(
        &mut self,
        pats: &[hir::Pat],
        subject_val: BasicValueEnum<'ctx>,
        subject_ty: &Type,
    ) -> Result<(), String> {
        let elem_ty = match subject_ty {
            Type::Array(inner, _) => inner.as_ref().clone(),
            _ => Type::I64,
        };
        // If the subject is a pointer, use it directly; otherwise store to stack
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
