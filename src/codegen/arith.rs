use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn checked_divmod(
        &mut self,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
        is_div: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
        let prefix = if is_div { "div" } else { "rem" };
        let zero = r.get_type().const_int(0, false);
        let is_zero =
            b!(self
                .bld
                .build_int_compare(IntPredicate::EQ, r, zero, &format!("{prefix}.z")));
        let trap_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.trap"));
        let chk_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.chk"));
        b!(self.bld.build_conditional_branch(is_zero, trap_bb, chk_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap(if is_div {
            "integer division by zero"
        } else {
            "integer remainder by zero"
        });

        self.bld.position_at_end(chk_bb);
        if signed {
            let bits = r.get_type().get_bit_width();
            let int_min = r.get_type().const_int(1u64 << (bits - 1), false);
            let minus_one = r.get_type().const_all_ones();
            let r_is_neg1 = b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                r,
                minus_one,
                &format!("{prefix}.neg1")
            ));
            let l_is_min = b!(self.bld.build_int_compare(
                IntPredicate::EQ,
                l,
                int_min,
                &format!("{prefix}.min")
            ));
            let ov = b!(self
                .bld
                .build_and(r_is_neg1, l_is_min, &format!("{prefix}.ov")));
            let ov_trap_bb = self
                .ctx
                .append_basic_block(fv, &format!("{prefix}.ov.trap"));
            let ok_bb = self.ctx.append_basic_block(fv, &format!("{prefix}.ok"));
            b!(self.bld.build_conditional_branch(ov, ov_trap_bb, ok_bb));
            self.bld.position_at_end(ov_trap_bb);
            self.emit_trap(if is_div {
                "signed integer overflow in division (INT_MIN / -1)"
            } else {
                "signed integer overflow in remainder (INT_MIN % -1)"
            });
            self.bld.position_at_end(ok_bb);
        }
        Ok(match (is_div, signed) {
            (true, true) => b!(self.bld.build_int_signed_div(l, r, "sdiv")).into(),
            (true, false) => b!(self.bld.build_int_unsigned_div(l, r, "udiv")).into(),
            (false, true) => b!(self.bld.build_int_signed_rem(l, r, "srem")).into(),
            (false, false) => b!(self.bld.build_int_unsigned_rem(l, r, "urem")).into(),
        })
    }

    pub(crate) fn proven_arith(
        &mut self,
        op: crate::mir::BinOp,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        use crate::mir::BinOp::{Add, Sub};
        Ok(match (op, signed) {
            (Add, true) => b!(self.bld.build_int_nsw_add(l, r, "add")),
            (Add, false) => b!(self.bld.build_int_nuw_add(l, r, "add")),
            (Sub, true) => b!(self.bld.build_int_nsw_sub(l, r, "sub")),
            (Sub, false) => b!(self.bld.build_int_nuw_sub(l, r, "sub")),
            (_, true) => b!(self.bld.build_int_nsw_mul(l, r, "mul")),
            (_, false) => b!(self.bld.build_int_nuw_mul(l, r, "mul")),
        })
    }

    pub(crate) fn checked_arith(
        &mut self,
        op: crate::mir::BinOp,
        l: inkwell::values::IntValue<'ctx>,
        r: inkwell::values::IntValue<'ctx>,
        signed: bool,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let kind = match op {
            crate::mir::BinOp::Add => "add",
            crate::mir::BinOp::Sub => "sub",
            _ => "mul",
        };
        let it = l.get_type();
        let intr = format!(
            "llvm.{}{kind}.with.overflow.i{}",
            if signed { 's' } else { 'u' },
            it.get_bit_width()
        );
        let f = self.module.get_function(&intr).unwrap_or_else(|| {
            let st = self
                .ctx
                .struct_type(&[it.into(), self.ctx.bool_type().into()], false);
            self.module
                .add_function(&intr, st.fn_type(&[it.into(), it.into()], false), None)
        });
        let pair = b!(self.bld.build_call(f, &[l.into(), r.into()], kind))
            .try_as_basic_value()
            .basic()
            .expect("ICE: overflow intrinsic returned void")
            .into_struct_value();
        let val = b!(self.bld.build_extract_value(pair, 0, "ov.val")).into_int_value();
        let ovf = b!(self.bld.build_extract_value(pair, 1, "ov.flag")).into_int_value();
        let fv = self.current_fn();
        let trap_bb = self.ctx.append_basic_block(fv, "ov.trap");
        let ok_bb = self.ctx.append_basic_block(fv, "ov.ok");
        b!(self.bld.build_conditional_branch(ovf, trap_bb, ok_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("integer overflow");
        self.bld.position_at_end(ok_bb);
        Ok(val)
    }

    pub(crate) fn checked_shift_count(
        &mut self,
        count: inkwell::values::IntValue<'ctx>,
    ) -> Result<(), String> {
        let bits = count.get_type().get_bit_width();
        let width = count.get_type().const_int(bits as u64, false);
        let fv = self.current_fn();
        let ok = b!(self
            .bld
            .build_int_compare(IntPredicate::ULT, count, width, "sh.ok"));
        let ok_bb = self.ctx.append_basic_block(fv, "sh.ok");
        let trap_bb = self.ctx.append_basic_block(fv, "sh.trap");
        b!(self.bld.build_conditional_branch(ok, ok_bb, trap_bb));
        self.bld.position_at_end(trap_bb);
        self.emit_trap("shift count exceeds bit width");
        self.bld.position_at_end(ok_bb);
        Ok(())
    }

    pub(crate) fn emit_trap(&mut self, msg: &str) {
        let trap_fn = self
            .module
            .get_function("jinn_trap")
            .unwrap_or_else(|| self.build_trap_fn());
        let msg_str = self
            .bld
            .build_global_string_ptr(msg, "trap.msg")
            .expect("build_global_string_ptr");
        self.bld
            .build_call(trap_fn, &[msg_str.as_pointer_value().into()], "")
            .expect("build_call trap");
        self.bld.build_unreachable().expect("build_unreachable");
    }

    fn build_trap_fn(&mut self) -> inkwell::values::FunctionValue<'ctx> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let f = self
            .module
            .add_function("jinn_trap", ft, Some(Linkage::External));
        f.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.ctx.create_enum_attribute(
                inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn"),
                0,
            ),
        );
        f.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.ctx.create_enum_attribute(
                inkwell::attributes::Attribute::get_named_enum_kind_id("cold"),
                0,
            ),
        );
        f
    }
}
