use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::BasicValueEnum;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) const GEN_CORO_PTR_OFF: u64 = 0;
    pub(crate) const GEN_VALUE_OFF: u64 = 8;
    pub(crate) const GEN_HAS_VALUE_OFF: u64 = 16;
    pub(crate) const GEN_DONE_OFF: u64 = 17;
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

    pub(crate) fn declare_gen_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let void = self.ctx.void_type();
        let ft = void.fn_type(&[ptr.into()], false);

        for name in &["jinn_gen_resume", "jinn_gen_suspend", "jinn_gen_destroy"] {
            if self.module.get_function(name).is_none() {
                self.module.add_function(name, ft, Some(Linkage::External));
            }
        }
    }

    pub(crate) fn coerce_to_i64(
        &self,
        val: BasicValueEnum<'ctx>,
    ) -> inkwell::values::IntValue<'ctx> {
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
}
