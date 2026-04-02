use inkwell::types::{BasicMetadataTypeEnum, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_direct_call(
        &mut self,
        name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(fv) = self.module.get_function(name) {
            let a = self.coerced_args(args, fv)?;
            let csv = b!(self.bld.build_call(fv, &a, name));
            return Ok(self.call_result(csv));
        }
        let fn_ptr = self.load_var(name)?;
        if let Some((_, ptys, ret)) = self.fns.get(name).cloned() {
            let fn_ty = Type::Fn(ptys, Box::new(ret));
            return self.indirect_call(fn_ptr, &fn_ty, args);
        }
        Err(format!("undefined function: {name}"))
    }

    pub(crate) fn compile_indirect_call(
        &mut self,
        callee: &hir::Expr,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fn_ptr = self.compile_expr(callee)?;
        let fn_ty = &callee.ty;
        self.indirect_call(fn_ptr, fn_ty, args)
    }

    fn indirect_call(
        &mut self,
        fn_ptr: BasicValueEnum<'ctx>,
        fn_ty: &Type,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let vals: Vec<BasicValueEnum<'ctx>> = args
            .iter()
            .map(|e| self.compile_expr(e))
            .collect::<Result<_, _>>()?;
        self.indirect_call_vals(fn_ptr, fn_ty, &vals)
    }

    pub(crate) fn indirect_call_vals(
        &mut self,
        closure_val: BasicValueEnum<'ctx>,
        fn_ty: &Type,
        vals: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Fn(ptys, ret) = fn_ty {
            // Extract fn_ptr and env_ptr from closure fat pointer {fn_ptr, env_ptr}
            let sv = closure_val.into_struct_value();
            let fn_ptr = b!(self.bld.build_extract_value(sv, 0, "cl.fn"))
                .into_pointer_value();
            let env_ptr = b!(self.bld.build_extract_value(sv, 1, "cl.env"));

            // Build function type with env_ptr as first parameter
            let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
            let mut lp: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
            lp.extend(ptys.iter().map(|t| BasicMetadataTypeEnum::from(self.llvm_ty(t))));
            let ft = self.mk_fn_type(ret.as_ref(), &lp, false);

            // Prepend env_ptr to argument list
            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![env_ptr.into()];
            a.extend(vals.iter().map(|v| BasicMetadataValueEnum::from(*v)));

            let csv =
                b!(self
                    .bld
                    .build_indirect_call(ft, fn_ptr, &a, "icall"));
            Ok(self.call_result(csv))
        } else {
            Err(format!("cannot call non-function type: {fn_ty}"))
        }
    }

    pub(crate) fn compile_method(
        &mut self,
        obj: &hir::Expr,
        resolved_name: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(fv) = self.module.get_function(resolved_name) {
            // Check if the method expects self by pointer (first param is ptr type)
            let first_param_is_ptr = fv
                .get_type()
                .get_param_types()
                .first()
                .map(|t| t.is_pointer_type())
                .unwrap_or(false);
            let self_val = if first_param_is_ptr {
                // By-pointer method: pass the alloca pointer directly
                if let hir::ExprKind::Var(_, name) = &obj.kind {
                    self.find_var(name)
                        .ok_or_else(|| format!("undefined variable: {name}"))?
                        .0
                        .into()
                } else if let hir::ExprKind::Field(inner, _, _) = &obj.kind {
                    // Nested field access — compile and store to a temp alloca
                    let val = self.compile_expr(obj)?;
                    let ty = val.get_type();
                    let tmp = self.entry_alloca(ty, "method.tmp");
                    b!(self.bld.build_store(tmp, val));
                    tmp.into()
                } else {
                    let val = self.compile_expr(obj)?;
                    let ty = val.get_type();
                    let tmp = self.entry_alloca(ty, "method.tmp");
                    b!(self.bld.build_store(tmp, val));
                    tmp.into()
                }
            } else {
                self.compile_expr(obj)?
            };
            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![self_val.into()];
            let param_types = fv.get_type().get_param_types();
            for (i, arg) in args.iter().enumerate() {
                let mut val = self.compile_expr(arg)?;
                // If the function expects a struct value but we have a pointer (e.g. self passed as arg),
                // load the struct from the pointer.
                if let Some(expected_ty) = param_types.get(i + 1) {
                    if let Ok(basic_ty) = BasicTypeEnum::try_from(*expected_ty) {
                        if basic_ty.is_struct_type() && val.is_pointer_value() {
                            val = b!(self.bld.build_load(basic_ty, val.into_pointer_value(), "deref"));
                        }
                    }
                }
                a.push(val.into());
            }
            let csv = b!(self.bld.build_call(fv, &a, resolved_name));
            return Ok(self.call_result(csv));
        }
        Err(format!("no method '{resolved_name}'"))
    }

    pub(crate) fn compile_pipe(
        &mut self,
        left: &hir::Expr,
        name: &str,
        extra_args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let left_val = self.compile_expr(left)?;
        if name == "log" {
            return self.pipe_log(left_val, &left.ty);
        }
        if let Some(fv) = self.module.get_function(name) {
            let mut a: Vec<BasicMetadataValueEnum<'ctx>> = vec![left_val.into()];
            for arg in extra_args {
                a.push(self.compile_expr(arg)?.into());
            }
            let csv = b!(self.bld.build_call(fv, &a, "pipe"));
            return Ok(self.call_result(csv));
        }
        let fn_ptr = self.load_var(name)?;
        if let Some((_, ptys, ret)) = self.fns.get(name).cloned() {
            let fn_ty = Type::Fn(ptys, Box::new(ret));
            return self.indirect_call_vals(fn_ptr, &fn_ty, &[left_val]);
        }
        Err(format!("pipeline: unresolved function '{name}'"))
    }

    fn pipe_log(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.emit_log(val, ty)
    }

    pub(crate) fn coerced_args(
        &mut self,
        args: &[hir::Expr],
        fv: FunctionValue<'ctx>,
    ) -> Result<Vec<BasicMetadataValueEnum<'ctx>>, String> {
        let ptypes = fv.get_type().get_param_types();
        let st = self.string_type();
        args.iter()
            .enumerate()
            .map(|(i, e)| {
                let v = self.compile_expr(e)?;
                let v = if let Some(pt) = ptypes.get(i) {
                    let target: inkwell::types::BasicTypeEnum =
                        (*pt).try_into().unwrap_or(v.get_type());
                    if v.get_type() == st.into() && target.is_pointer_type() {
                        self.string_data(v)?
                    } else {
                        self.coerce_val(v, target)
                    }
                } else {
                    v
                };
                Ok(v.into())
            })
            .collect()
    }
}
