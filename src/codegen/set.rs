use inkwell::values::BasicValueEnum;
use inkwell::AddressSpace;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_set_new(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        // Set uses the same {ptr, len, cap} header as Vec/Map
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();

        let header_size = i64t.const_int(24, false);
        let header_ptr = b!(self
            .bld
            .build_call(malloc, &[header_size.into()], "set.hdr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "set.ptr"));
        b!(self.bld.build_store(
            ptr_gep,
            self.ctx
                .ptr_type(AddressSpace::default())
                .const_null()
        ));

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "set.len"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "set.cap"));
        b!(self.bld.build_store(cap_gep, i64t.const_int(0, false)));

        Ok(header_ptr.into())
    }

    pub(crate) fn compile_set_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let header_ptr = obj_val.into_pointer_value();

        match method {
            "len" => self.vec_len(header_ptr),
            "add" | "remove" | "contains" | "clear" | "union" | "difference" | "intersection"
            | "to_vec" => {
                // Runtime stubs — call __jade_set_<method>
                let rt_name = format!("__jade_set_{method}");
                let i64t = self.ctx.i64_type();
                let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

                let mut arg_vals = vec![header_ptr.into()];
                for a in args {
                    arg_vals.push(self.compile_expr(a)?);
                }

                let fn_ty = match method {
                    "add" | "remove" | "clear" => {
                        let param_tys: Vec<_> =
                            arg_vals.iter().map(|v| v.get_type().into()).collect();
                        self.ctx.void_type().fn_type(&param_tys, false)
                    }
                    "contains" => {
                        let param_tys: Vec<_> =
                            arg_vals.iter().map(|v| v.get_type().into()).collect();
                        self.ctx.bool_type().fn_type(&param_tys, false)
                    }
                    "len" => i64t.fn_type(&[ptr_ty.into()], false),
                    _ => {
                        // union, difference, intersection, to_vec all return a pointer
                        let param_tys: Vec<_> =
                            arg_vals.iter().map(|v| v.get_type().into()).collect();
                        ptr_ty.fn_type(&param_tys, false)
                    }
                };

                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_ty, None));

                let arg_metas: Vec<_> = arg_vals.iter().map(|v| (*v).into()).collect();
                let result = b!(self.bld.build_call(func, &arg_metas, &rt_name));

                match method {
                    "add" | "remove" | "clear" => {
                        Ok(self.ctx.i64_type().const_zero().into())
                    }
                    _ => Ok(result.try_as_basic_value().basic().unwrap()),
                }
            }
            _ => Err(format!("no method '{method}' on Set")),
        }
    }

    pub(crate) fn compile_ndarray_new(
        &mut self,
        dims: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let _f64t = self.ctx.f64_type();
        let malloc = self.ensure_malloc();

        // Compute total elements = product of all dims
        let mut total = i64t.const_int(1, false);
        for dim in dims {
            let dv = self.compile_expr(dim)?.into_int_value();
            total = b!(self.bld.build_int_mul(total, dv, "ndarray.mul"));
        }

        // Allocate total * sizeof(f64) bytes
        let elem_size = i64t.const_int(8, false);
        let byte_size = b!(self.bld.build_int_mul(total, elem_size, "ndarray.bytes"));
        let ptr = b!(self
            .bld
            .build_call(malloc, &[byte_size.into()], "ndarray.ptr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        // Zero-initialize
        let memset = self.module.get_function("llvm.memset.p0.i64").unwrap_or_else(|| {
            self.module.add_function(
                "llvm.memset.p0.i64",
                self.ctx.void_type().fn_type(
                    &[
                        self.ctx.ptr_type(AddressSpace::default()).into(),
                        self.ctx.i8_type().into(),
                        i64t.into(),
                        self.ctx.bool_type().into(),
                    ],
                    false,
                ),
                None,
            )
        });
        b!(self.bld.build_call(
            memset,
            &[
                ptr.into(),
                self.ctx.i8_type().const_zero().into(),
                byte_size.into(),
                self.ctx.bool_type().const_zero().into(),
            ],
            "",
        ));

        Ok(ptr.into())
    }

    pub(crate) fn compile_simd_new(
        &mut self,
        elems: &[hir::Expr],
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (inner, lanes) = match ty {
            Type::SIMD(inner, lanes) => (inner.as_ref(), *lanes),
            _ => return Err("compile_simd_new called with non-SIMD type".into()),
        };
        let elem_llvm = self.llvm_ty(inner);
        let vec_ty = if inner.is_float() {
            elem_llvm.into_float_type().vec_type(lanes as u32)
        } else {
            elem_llvm.into_int_type().vec_type(lanes as u32)
        };
        let mut vec_val = vec_ty.get_undef();
        for (i, elem) in elems.iter().enumerate() {
            let val = self.compile_expr(elem)?;
            let idx = self.ctx.i32_type().const_int(i as u64, false);
            vec_val = b!(self
                .bld
                .build_insert_element(vec_val, val, idx, "simd.ins"));
        }
        Ok(vec_val.into())
    }

    /// Priority queue: allocate a {ptr, len, cap} triple, same layout as Vec/Set
    pub(crate) fn compile_pq_new(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        self.compile_set_new() // same 24-byte header allocation
    }

    /// Priority queue methods — dispatch to runtime helpers
    pub(crate) fn compile_pq_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let rt_name = format!("__jade_pq_{method}");
        let i64t = self.ctx.i64_type();
        let ptr_t = self.ctx.ptr_type(AddressSpace::default());

        match method {
            "push" => {
                let val = self.compile_expr(&args[0])?;
                let priority = self.compile_expr(&args[1])?;
                let fn_type = self.ctx.void_type().fn_type(
                    &[ptr_t.into(), i64t.into(), i64t.into()],
                    false,
                );
                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_type, None));
                b!(self.bld.build_call(
                    func,
                    &[obj_val.into(), val.into(), priority.into()],
                    "",
                ));
                Ok(self.ctx.i64_type().const_zero().into())
            }
            "pop" | "peek" => {
                let fn_type = i64t.fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_type, None));
                let result = b!(self.bld.build_call(func, &[obj_val.into()], "pq.result"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            "len" => {
                let fn_type = i64t.fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_type, None));
                let result = b!(self.bld.build_call(func, &[obj_val.into()], "pq.len"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            "is_empty" => {
                let fn_type = self.ctx.bool_type().fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_type, None));
                let result = b!(self.bld.build_call(func, &[obj_val.into()], "pq.empty"));
                Ok(result.try_as_basic_value().basic().unwrap())
            }
            "clear" => {
                let fn_type = self.ctx.void_type().fn_type(&[ptr_t.into()], false);
                let func = self
                    .module
                    .get_function(&rt_name)
                    .unwrap_or_else(|| self.module.add_function(&rt_name, fn_type, None));
                b!(self.bld.build_call(func, &[obj_val.into()], ""));
                Ok(self.ctx.i64_type().const_zero().into())
            }
            _ => Err(format!("no method '{method}' on PriorityQueue")),
        }
    }
}
