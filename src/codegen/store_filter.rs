use inkwell::IntPredicate;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use crate::ast::BinOp;
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

use super::stores::STRING_BUF_SIZE;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn copy_string_to_fixed_buf(
        &mut self,
        string_val: BasicValueEnum<'ctx>,
        buf_ptr: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let i32t = self.ctx.i32_type();

        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[
                buf_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(STRING_BUF_SIZE, false).into()
            ],
            ""
        ));

        let len = self.string_len(string_val)?.into_int_value();
        let data = self.string_data(string_val)?.into_pointer_value();

        let max_data = i64t.const_int(STRING_BUF_SIZE - 8, false);
        let clamped = b!(self.bld.build_select(
            b!(self
                .bld
                .build_int_compare(IntPredicate::UGT, len, max_data, "str.clamp")),
            max_data,
            len,
            "str.len"
        ));

        b!(self.bld.build_store(buf_ptr, clamped));

        let data_dst = unsafe {
            b!(self
                .bld
                .build_gep(i8t, buf_ptr, &[i64t.const_int(8, false)], "str.dst"))
        };
        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[data_dst.into(), data.into(), clamped.into()],
            ""
        ));

        Ok(())
    }

    pub(crate) fn read_string_from_fixed_buf(
        &mut self,
        buf_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();

        let len = b!(self.bld.build_load(i64t, buf_ptr, "str.len")).into_int_value();

        let data_src = unsafe {
            b!(self
                .bld
                .build_gep(i8t, buf_ptr, &[i64t.const_int(8, false)], "str.src"))
        };

        let malloc_fn = self.ensure_malloc();
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_int_add(len, one, "str.alloc"));
        let heap = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[alloc_size.into()],
                "str.heap"
            )))
            .into_pointer_value();

        let memcpy_fn = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy_fn, &[heap.into(), data_src.into(), len.into()], ""));

        let end = unsafe { b!(self.bld.build_gep(i8t, heap, &[len], "str.end")) };
        b!(self.bld.build_store(end, i8t.const_int(0, false)));

        self.build_string(heap, len, i64t.const_int(0, false), "str.from_store")
    }

    pub(crate) fn load_store_record_as_jade(
        &mut self,
        st: inkwell::types::StructType<'ctx>,
        raw_ptr: PointerValue<'ctx>,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let jade_struct_name = format!("__store_{}", sd.name);
        let jade_st = self
            .module
            .get_struct_type(&jade_struct_name)
            .ok_or_else(|| format!("no jade store struct '{jade_struct_name}'"))?;
        let jade_ptr = self.entry_alloca(jade_st.into(), "jade.rec");

        for (i, field) in sd.fields.iter().enumerate() {
            let src_gep = b!(self.bld.build_struct_gep(
                st,
                raw_ptr,
                i as u32,
                &format!("raw.{}", field.name)
            ));
            let dst_gep = b!(self.bld.build_struct_gep(
                jade_st,
                jade_ptr,
                i as u32,
                &format!("jade.{}", field.name)
            ));
            match &field.ty {
                Type::String => {
                    let s = self.read_string_from_fixed_buf(src_gep)?;
                    b!(self.bld.build_store(dst_gep, s));
                }
                ty => {
                    let lty = self.llvm_ty(ty);
                    let val = b!(self.bld.build_load(lty, src_gep, &field.name));
                    b!(self.bld.build_store(dst_gep, val));
                }
            }
        }

        Ok(b!(self.bld.build_load(jade_st, jade_ptr, "jade.result")))
    }

    pub(crate) fn precompile_filter_values(
        &mut self,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<
        (
            usize,
            Type,
            BasicValueEnum<'ctx>,
            Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                BinOp,
                BasicValueEnum<'ctx>,
            )>,
        ),
        String,
    > {
        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == filter.field)
            .map(|(i, f)| (i, f.ty.clone()))
            .unwrap();
        let filter_val = self.compile_expr(&filter.value)?;
        let mut extras = Vec::new();
        for (lop, cond) in &filter.extra {
            let (ci, ct) = sd
                .fields
                .iter()
                .enumerate()
                .find(|(_, f)| f.name == cond.field)
                .map(|(i, f)| (i, f.ty.clone()))
                .unwrap();
            let cv = self.compile_expr(&cond.value)?;
            extras.push((*lop, ci, ct, cond.op, cv));
        }
        Ok((field_idx, field_ty, filter_val, extras))
    }

    pub(crate) fn eval_store_filter(
        &mut self,
        rec_ptr: PointerValue<'ctx>,
        rec_st: inkwell::types::StructType<'ctx>,
        primary_idx: usize,
        primary_ty: &Type,
        primary_op: BinOp,
        primary_val: BasicValueEnum<'ctx>,
        extras: &[(
            crate::ast::LogicalOp,
            usize,
            Type,
            BinOp,
            BasicValueEnum<'ctx>,
        )],
    ) -> Result<IntValue<'ctx>, String> {
        let field_gep =
            b!(self
                .bld
                .build_struct_gep(rec_st, rec_ptr, primary_idx as u32, "sf.field"));
        let mut result =
            self.store_compare_field(field_gep, primary_ty, primary_op, primary_val)?;
        for (lop, ci, ct, op, cv) in extras {
            let cg = b!(self
                .bld
                .build_struct_gep(rec_st, rec_ptr, *ci as u32, "sf.efield"));
            let ecmp = self.store_compare_field(cg, ct, *op, *cv)?;
            result = match lop {
                crate::ast::LogicalOp::And => b!(self.bld.build_and(result, ecmp, "sf.and")),
                crate::ast::LogicalOp::Or => b!(self.bld.build_or(result, ecmp, "sf.or")),
            };
        }
        Ok(result)
    }

    fn store_compare_field(
        &mut self,
        field_ptr: PointerValue<'ctx>,
        field_ty: &Type,
        op: BinOp,
        filter_val: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match field_ty {
            Type::String => {
                let i64t = self.ctx.i64_type();
                let i8t = self.ctx.i8_type();
                let i32t = self.ctx.i32_type();

                let stored_len =
                    b!(self.bld.build_load(i64t, field_ptr, "cmp.slen")).into_int_value();
                let stored_data = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, field_ptr, &[i64t.const_int(8, false)], "cmp.sdata"))
                };

                let filter_len = self.string_len(filter_val)?.into_int_value();
                let filter_data = self.string_data(filter_val)?.into_pointer_value();

                let fv = self.cur_fn.unwrap();
                let len_eq_bb = self.ctx.append_basic_block(fv, "cmp.len_eq");
                let result_bb = self.ctx.append_basic_block(fv, "cmp.result");

                let len_match = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    stored_len,
                    filter_len,
                    "cmp.leneq"
                ));

                match op {
                    BinOp::Eq | BinOp::Ne => {
                        let negate = op == BinOp::Ne;
                        let early_val = u64::from(negate);
                        let early_bb = self.ctx.append_basic_block(fv, "cmp.early");
                        b!(self
                            .bld
                            .build_conditional_branch(len_match, len_eq_bb, early_bb));

                        self.bld.position_at_end(early_bb);
                        b!(self.bld.build_unconditional_branch(result_bb));

                        self.bld.position_at_end(len_eq_bb);
                        let memcmp_fn = self.ensure_memcmp();
                        let cmp_result = self
                            .call_result(b!(self.bld.build_call(
                                memcmp_fn,
                                &[stored_data.into(), filter_data.into(), stored_len.into()],
                                "cmp.mc"
                            )))
                            .into_int_value();
                        let pred = if negate {
                            IntPredicate::NE
                        } else {
                            IntPredicate::EQ
                        };
                        let is_match = b!(self.bld.build_int_compare(
                            pred,
                            cmp_result,
                            i32t.const_int(0, false),
                            "cmp.m"
                        ));
                        b!(self.bld.build_unconditional_branch(result_bb));
                        let len_eq_end = self.bld.get_insert_block().unwrap();

                        self.bld.position_at_end(result_bb);
                        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "cmp.str"));
                        phi.add_incoming(&[
                            (&self.ctx.bool_type().const_int(early_val, false), early_bb),
                            (&is_match, len_eq_end),
                        ]);
                        Ok(phi.as_basic_value().into_int_value())
                    }
                    _ => {
                        result_bb.remove_from_function().unwrap();
                        len_eq_bb.remove_from_function().unwrap();

                        let min_len = b!(self.bld.build_select(
                            b!(self.bld.build_int_compare(
                                IntPredicate::ULT,
                                stored_len,
                                filter_len,
                                "min.cmp"
                            )),
                            stored_len,
                            filter_len,
                            "min.len"
                        ))
                        .into_int_value();

                        let memcmp_fn = self.ensure_memcmp();
                        let cmp_result = self
                            .call_result(b!(self.bld.build_call(
                                memcmp_fn,
                                &[stored_data.into(), filter_data.into(), min_len.into()],
                                "cmp.mc"
                            )))
                            .into_int_value();

                        let pred = match op {
                            BinOp::Lt => IntPredicate::SLT,
                            BinOp::Gt => IntPredicate::SGT,
                            BinOp::Le => IntPredicate::SLE,
                            BinOp::Ge => IntPredicate::SGE,
                            _ => unreachable!(),
                        };
                        Ok(b!(self.bld.build_int_compare(
                            pred,
                            cmp_result,
                            i32t.const_int(0, false),
                            "cmp.ord"
                        )))
                    }
                }
            }
            Type::I64
            | Type::U64
            | Type::I32
            | Type::U32
            | Type::I16
            | Type::U16
            | Type::I8
            | Type::U8 => {
                let lty = self.llvm_ty(field_ty);
                let stored = b!(self.bld.build_load(lty, field_ptr, "cmp.ival")).into_int_value();
                let filter_int = filter_val.into_int_value();
                let pred = match op {
                    BinOp::Eq => IntPredicate::EQ,
                    BinOp::Ne => IntPredicate::NE,
                    BinOp::Lt => IntPredicate::SLT,
                    BinOp::Gt => IntPredicate::SGT,
                    BinOp::Le => IntPredicate::SLE,
                    BinOp::Ge => IntPredicate::SGE,
                    _ => return Err(format!("unsupported store filter op: {:?}", op)),
                };
                Ok(b!(self
                    .bld
                    .build_int_compare(pred, stored, filter_int, "cmp.i")))
            }
            Type::F64 | Type::F32 => {
                let lty = self.llvm_ty(field_ty);
                let stored = b!(self.bld.build_load(lty, field_ptr, "cmp.fval")).into_float_value();
                let filter_float = filter_val.into_float_value();
                use inkwell::FloatPredicate;
                let pred = match op {
                    BinOp::Eq => FloatPredicate::OEQ,
                    BinOp::Ne => FloatPredicate::ONE,
                    BinOp::Lt => FloatPredicate::OLT,
                    BinOp::Gt => FloatPredicate::OGT,
                    BinOp::Le => FloatPredicate::OLE,
                    BinOp::Ge => FloatPredicate::OGE,
                    _ => return Err(format!("unsupported store filter op: {:?}", op)),
                };
                Ok(b!(self.bld.build_float_compare(
                    pred,
                    stored,
                    filter_float,
                    "cmp.f"
                )))
            }
            Type::Bool => {
                let stored = b!(self
                    .bld
                    .build_load(self.ctx.bool_type(), field_ptr, "cmp.bval"))
                .into_int_value();
                let filter_bool = filter_val.into_int_value();
                let pred = match op {
                    BinOp::Eq => IntPredicate::EQ,
                    BinOp::Ne => IntPredicate::NE,
                    _ => return Err("bool fields only support equals/isnt comparisons".into()),
                };
                Ok(b!(self.bld.build_int_compare(
                    pred,
                    stored,
                    filter_bool,
                    "cmp.b"
                )))
            }
            _ => Err(format!(
                "unsupported store field type for filtering: {:?}",
                field_ty
            )),
        }
    }
}
