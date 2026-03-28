use std::collections::HashMap;

use inkwell::basic_block::BasicBlock;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_match(
        &mut self,
        m: &hir::Match,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let fv = self.cur_fn.unwrap();
        let subject_val = self.compile_expr(&m.subject)?;
        let subject_ty = self.resolve_ty(m.subject.ty.clone());

        let is_enum = matches!(subject_ty, Type::Enum(_))
            || matches!(&subject_ty, Type::Struct(n, _) if self.enums.contains_key(n));

        if !is_enum {
            return self.compile_value_match(m, subject_val, &subject_ty);
        }

        let enum_name = match &subject_ty {
            Type::Enum(n) | Type::Struct(n, _) => n.clone(),
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
                let fail_bb = if i + 1 < arm_bbs.len() {
                    arm_bbs[i + 1]
                } else {
                    merge_bb
                };
                self.compile_guard(guard, fail_bb, i)?;
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

        let has_complex = m.arms.iter().any(|a| {
            matches!(
                &a.pat,
                hir::Pat::Range(..) | hir::Pat::Or(..) | hir::Pat::Tuple(..) | hir::Pat::Array(..)
            )
        });

        if has_complex {
            let check_bbs: Vec<_> = m
                .arms
                .iter()
                .enumerate()
                .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.check{i}")))
                .collect();
            let arm_bbs: Vec<_> = m
                .arms
                .iter()
                .enumerate()
                .map(|(i, _)| self.ctx.append_basic_block(fv, &format!("match.arm{i}")))
                .collect();
            let iv_opt = subject_val
                .is_int_value()
                .then(|| subject_val.into_int_value());
            let mut phi_in: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
            let mut all_valued = true;

            b!(self.bld.build_unconditional_branch(check_bbs[0]));

            for (i, arm) in m.arms.iter().enumerate() {
                let next_bb = if i + 1 < check_bbs.len() {
                    check_bbs[i + 1]
                } else {
                    merge_bb
                };

                self.bld.position_at_end(check_bbs[i]);
                match &arm.pat {
                    hir::Pat::Wild(_) | hir::Pat::Bind(_, _, _, _) => {
                        b!(self.bld.build_unconditional_branch(arm_bbs[i]));
                    }
                    hir::Pat::Lit(expr) => {
                        let lit_val = self.compile_expr(expr)?;
                        let iv = iv_opt.unwrap();
                        let cmp = b!(self.bld.build_int_compare(
                            IntPredicate::EQ,
                            iv,
                            lit_val.into_int_value(),
                            "match.cmp"
                        ));
                        b!(self.bld.build_conditional_branch(cmp, arm_bbs[i], next_bb));
                    }
                    hir::Pat::Range(lo, hi, _) => {
                        let iv = iv_opt.unwrap();
                        let lo_val = self.compile_expr(lo)?.into_int_value();
                        let hi_val = self.compile_expr(hi)?.into_int_value();
                        let ge =
                            b!(self
                                .bld
                                .build_int_compare(IntPredicate::SGE, iv, lo_val, "rng.ge"));
                        let le =
                            b!(self
                                .bld
                                .build_int_compare(IntPredicate::SLE, iv, hi_val, "rng.le"));
                        let in_range = b!(self.bld.build_and(ge, le, "rng.in"));
                        b!(self
                            .bld
                            .build_conditional_branch(in_range, arm_bbs[i], next_bb));
                    }
                    hir::Pat::Or(pats, _) => {
                        let iv = iv_opt.unwrap();
                        let mut any_match = self.ctx.bool_type().const_int(0, false);
                        for pat in pats {
                            let sub_match = match pat {
                                hir::Pat::Lit(e) => {
                                    let lv = self.compile_expr(e)?.into_int_value();
                                    b!(self.bld.build_int_compare(
                                        IntPredicate::EQ,
                                        iv,
                                        lv,
                                        "or.cmp"
                                    ))
                                }
                                hir::Pat::Range(lo, hi, _) => {
                                    let lo_val = self.compile_expr(lo)?.into_int_value();
                                    let hi_val = self.compile_expr(hi)?.into_int_value();
                                    let ge = b!(self.bld.build_int_compare(
                                        IntPredicate::SGE,
                                        iv,
                                        lo_val,
                                        "or.ge"
                                    ));
                                    let le = b!(self.bld.build_int_compare(
                                        IntPredicate::SLE,
                                        iv,
                                        hi_val,
                                        "or.le"
                                    ));
                                    b!(self.bld.build_and(ge, le, "or.rng"))
                                }
                                _ => self.ctx.bool_type().const_int(1, false),
                            };
                            any_match = b!(self.bld.build_or(any_match, sub_match, "or.any"));
                        }
                        b!(self
                            .bld
                            .build_conditional_branch(any_match, arm_bbs[i], next_bb));
                    }
                    _ => {
                        b!(self.bld.build_unconditional_branch(arm_bbs[i]));
                    }
                }

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
                    self.compile_guard(guard, next_bb, i)?;
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
                let fail_bb = if i + 1 < arm_bbs.len() {
                    arm_bbs[i + 1]
                } else {
                    merge_bb
                };
                self.compile_guard(guard, fail_bb, i)?;
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

    fn compile_guard(
        &mut self,
        guard: &hir::Expr,
        fail_bb: BasicBlock<'ctx>,
        i: usize,
    ) -> Result<(), String> {
        let fv = self.cur_fn.unwrap();
        let guard_val = self.compile_expr(guard)?;
        let gv = guard_val.into_int_value();
        let guard_pass = self
            .ctx
            .append_basic_block(fv, &format!("match.guard_pass{i}"));
        b!(self.bld.build_conditional_branch(gv, guard_pass, fail_bb));
        self.bld.position_at_end(guard_pass);
        Ok(())
    }
}
