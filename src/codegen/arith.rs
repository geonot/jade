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

    pub(crate) fn emit_trap(&mut self, msg: &str) {
        let trap_fn = self
            .module
            .get_function("__jinn_trap")
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
        let i32t = self.ctx.i32_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let ft = self.ctx.void_type().fn_type(&[ptr_ty.into()], false);
        let f = self
            .module
            .add_function("__jinn_trap", ft, Some(Linkage::Internal));
        let entry = self.ctx.append_basic_block(f, "entry");
        let saved_bb = self.bld.get_insert_block();
        self.bld.position_at_end(entry);
        let fprintf_fn = self.module.get_function("fprintf").unwrap_or_else(|| {
            let ft2 = i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], true);
            self.module
                .add_function("fprintf", ft2, Some(Linkage::External))
        });
        let stderr_g = self.module.get_global("stderr").unwrap_or_else(|| {
            let g = self.module.add_global(ptr_ty, None, "stderr");
            g.set_linkage(Linkage::External);
            g
        });
        let stderr_val = self
            .bld
            .build_load(ptr_ty, stderr_g.as_pointer_value(), "se")
            .expect("load stderr");
        let fmt = self
            .bld
            .build_global_string_ptr("runtime error: %s\n", "trap.fmt")
            .expect("fmt string");
        let msg_param = f.get_nth_param(0).expect("ICE: missing param");
        self.bld
            .build_call(
                fprintf_fn,
                &[
                    stderr_val.into(),
                    fmt.as_pointer_value().into(),
                    msg_param.into(),
                ],
                "",
            )
            .expect("call fprintf");
        let abort_fn = self.module.get_function("abort").unwrap_or_else(|| {
            let ft3 = self.ctx.void_type().fn_type(&[], false);
            self.module
                .add_function("abort", ft3, Some(Linkage::External))
        });
        self.bld.build_call(abort_fn, &[], "").expect("call abort");
        self.bld.build_unreachable().expect("unreachable");
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        f
    }
}
