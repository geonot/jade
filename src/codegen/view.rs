use inkwell::IntPredicate;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn view_pack(
        &mut self,
        ptr: PointerValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let vt = self.view_type();
        let mut agg = vt.get_undef();
        agg = b!(self.bld.build_insert_value(agg, ptr, 0, "vw.ptr")).into_struct_value();
        agg = b!(self.bld.build_insert_value(agg, len, 1, "vw.len")).into_struct_value();
        Ok(agg.into())
    }

    pub(crate) fn view_ptr(
        &mut self,
        view: BasicValueEnum<'ctx>,
    ) -> Result<PointerValue<'ctx>, String> {
        let sv = view.into_struct_value();
        Ok(b!(self.bld.build_extract_value(sv, 0, "vw.p")).into_pointer_value())
    }

    pub(crate) fn view_len_val(
        &mut self,
        view: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        let sv = view.into_struct_value();
        Ok(b!(self.bld.build_extract_value(sv, 1, "vw.l")).into_int_value())
    }

    fn view_range_check(
        &mut self,
        lo: IntValue<'ctx>,
        hi: IntValue<'ctx>,
        len: IntValue<'ctx>,
    ) -> Result<(), String> {
        let fv = self.current_fn();
        let i64t = self.ctx.i64_type();
        let lo_ok = b!(self.bld.build_int_compare(
            IntPredicate::SGE,
            lo,
            i64t.const_int(0, false),
            "vw.lo0"
        ));
        let lo_le_hi = b!(self
            .bld
            .build_int_compare(IntPredicate::SLE, lo, hi, "vw.lohi"));
        let hi_le_len = b!(self
            .bld
            .build_int_compare(IntPredicate::SLE, hi, len, "vw.hilen"));
        let ok1 = b!(self.bld.build_and(lo_ok, lo_le_hi, "vw.ok1"));
        let valid = b!(self.bld.build_and(ok1, hi_le_len, "vw.valid"));
        let ok_bb = self.ctx.append_basic_block(fv, "vw.ok");
        let fail_bb = self.ctx.append_basic_block(fv, "vw.fail");
        b!(self.bld.build_conditional_branch(valid, ok_bb, fail_bb));
        self.bld.position_at_end(fail_bb);
        self.emit_trap("view range out of bounds");
        self.bld.position_at_end(ok_bb);
        Ok(())
    }

    pub(crate) fn view_from_vec_range(
        &mut self,
        header_ptr: PointerValue<'ctx>,
        elem_ty: &Type,
        lo: BasicValueEnum<'ctx>,
        hi: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (data, len) = self.vec_data_and_len(header_ptr)?;
        let lo = lo.into_int_value();
        let hi = hi.into_int_value();
        self.view_range_check(lo, hi, len)?;
        let lty = self.llvm_ty(elem_ty);
        let start = unsafe { b!(self.bld.build_gep(lty, data, &[lo], "vw.start")) };
        let vlen = b!(self.bld.build_int_nsw_sub(hi, lo, "vw.vlen"));
        self.view_pack(start, vlen)
    }

    pub(crate) fn view_from_vec_full(
        &mut self,
        header_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (data, len) = self.vec_data_and_len(header_ptr)?;
        self.view_pack(data, len)
    }

    pub(crate) fn view_elem_from_vec(
        &mut self,
        header_ptr: PointerValue<'ctx>,
        elem_ty: &Type,
        idx: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (data, len) = self.vec_data_and_len(header_ptr)?;
        let idx = idx.into_int_value();
        self.emit_vec_bounds_check(idx, len)?;
        let lty = self.llvm_ty(elem_ty);
        let start = unsafe { b!(self.bld.build_gep(lty, data, &[idx], "vw.estart")) };
        let one = self.ctx.i64_type().const_int(1, false);
        self.view_pack(start, one)
    }

    pub(crate) fn view_from_string(
        &mut self,
        s: BasicValueEnum<'ctx>,
        lo: BasicValueEnum<'ctx>,
        hi: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let len = self.string_len(s)?.into_int_value();
        let data = self.string_data(s)?.into_pointer_value();
        let lo = lo.into_int_value();
        let hi = hi.into_int_value();
        self.view_range_check(lo, hi, len)?;
        let i8t = self.ctx.i8_type();
        let start = unsafe { b!(self.bld.build_gep(i8t, data, &[lo], "vw.sstart")) };
        let vlen = b!(self.bld.build_int_nsw_sub(hi, lo, "vw.svlen"));
        self.view_pack(start, vlen)
    }

    pub(crate) fn view_get(
        &mut self,
        view: BasicValueEnum<'ctx>,
        elem_ty: &Type,
        idx: BasicValueEnum<'ctx>,
        checked: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let data = self.view_ptr(view)?;
        let idx = idx.into_int_value();
        if checked {
            let len = self.view_len_val(view)?;
            self.emit_vec_bounds_check(idx, len)?;
        }
        let lty = self.llvm_ty(elem_ty);
        let elem_gep = unsafe { b!(self.bld.build_gep(lty, data, &[idx], "vw.egep")) };
        let raw = b!(self.bld.build_load(lty, elem_gep, "vw.ev"));
        if Self::is_value_clonable(elem_ty) && !elem_ty.is_trivially_droppable() {
            return self.clone_value(raw, elem_ty);
        }
        Ok(raw)
    }
}
