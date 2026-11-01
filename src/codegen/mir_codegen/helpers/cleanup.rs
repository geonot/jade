use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_cleanup_call(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let void = self.ctx.void_type();
        let i64t = self.ctx.i64_type();
        let (runtime_name, signature) = match name {
            "__cleanup_new" => ("jinn_cleanup_new", ptr.fn_type(&[], false)),
            "__cleanup_cell_owned" | "__cleanup_cell_borrowed" => (
                "jinn_cleanup_cell",
                ptr.fn_type(&[ptr.into(), i64t.into(), ptr.into()], false),
            ),
            "__cleanup_push" => (
                "jinn_cleanup_push",
                void.fn_type(&[ptr.into(), ptr.into(), ptr.into()], false),
            ),
            "__cleanup_run" => ("jinn_cleanup_run", void.fn_type(&[ptr.into()], false)),
            "__cleanup_cell_set" => {
                let cell = self.val(args[0]).into_pointer_value();
                let value = self.val(args[1]);
                b!(self.bld.build_store(cell, value));
                return Ok(self.ctx.i8_type().const_zero().into());
            }
            _ => return Err(format!("unknown cleanup operation {name}")),
        };
        let function = self.module.get_function(runtime_name).unwrap_or_else(|| {
            self.module
                .add_function(runtime_name, signature, Some(Linkage::External))
        });
        match name {
            "__cleanup_new" => {
                let call = b!(self.bld.build_call(function, &[], "cleanup.frame"));
                Ok(self.call_result(call))
            }
            "__cleanup_cell_owned" | "__cleanup_cell_borrowed" => {
                let value = self.val(args[1]);
                let ty = self
                    .value_types
                    .get(&args[1])
                    .cloned()
                    .ok_or("missing cleanup cell type")?;
                let drop = if name == "__cleanup_cell_owned" {
                    self.value_drop_fn(&ty)?
                        .as_global_value()
                        .as_pointer_value()
                } else {
                    ptr.const_null()
                };
                let size = value.get_type().size_of().ok_or("unsized cleanup cell")?;
                let frame = self.val(args[0]);
                let call = b!(self.bld.build_call(
                    function,
                    &[frame.into(), size.into(), drop.into()],
                    "cleanup.cell"
                ));
                let cell = self.call_result(call).into_pointer_value();
                b!(self.bld.build_store(cell, value));
                Ok(cell.into())
            }
            "__cleanup_push" => {
                let frame = self.val(args[0]);
                let closure = self.val(args[1]).into_struct_value();
                let run = b!(self.bld.build_extract_value(closure, 0, "cleanup.run"));
                let env = b!(self.bld.build_extract_value(closure, 1, "cleanup.env"));
                b!(self
                    .bld
                    .build_call(function, &[frame.into(), run.into(), env.into()], ""));
                Ok(self.ctx.i8_type().const_zero().into())
            }
            "__cleanup_run" => {
                let frame = self.val(args[0]);
                b!(self.bld.build_call(function, &[frame.into()], ""));
                Ok(self.ctx.i8_type().const_zero().into())
            }
            _ => unreachable!(),
        }
    }

    pub(in crate::codegen) fn value_drop_fn(
        &mut self,
        ty: &Type,
    ) -> Result<inkwell::values::FunctionValue<'ctx>, String> {
        let name = format!("__value_drop.{}", self.module.get_functions().count());
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let signature = self.ctx.void_type().fn_type(&[ptr.into()], false);
        let function = self
            .module
            .add_function(&name, signature, Some(Linkage::Internal));
        self.tag_fn(function);
        let saved_block = self.bld.get_insert_block();
        let saved_fn = self.cur_fn;
        let block = self.ctx.append_basic_block(function, "entry");
        self.bld.position_at_end(block);
        self.cur_fn = Some(function);
        let llvm_ty = self.llvm_ty(ty);
        let value = b!(self.bld.build_load(
            llvm_ty,
            function.get_nth_param(0).unwrap().into_pointer_value(),
            "cleanup.value"
        ));
        self.drop_value(value, ty)?;
        b!(self.bld.build_return(None));
        self.cur_fn = saved_fn;
        if let Some(block) = saved_block {
            self.bld.position_at_end(block);
        }
        Ok(function)
    }
}
