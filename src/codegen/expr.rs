use std::collections::HashMap;

use inkwell::IntPredicate;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum};

use crate::ast::UnaryOp;
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_expr(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Int(n) => Ok(self.int_const(*n, &expr.ty)),
            hir::ExprKind::Float(n) => {
                let ft = if expr.ty == Type::F32 {
                    self.ctx.f32_type().const_float(*n).into()
                } else {
                    self.ctx.f64_type().const_float(*n).into()
                };
                Ok(ft)
            }
            hir::ExprKind::Str(s) => self.compile_str_literal(s),
            hir::ExprKind::Bool(v) => Ok(self.ctx.bool_type().const_int(*v as u64, false).into()),
            hir::ExprKind::None | hir::ExprKind::Void => {
                Ok(self.ctx.i64_type().const_int(0, false).into())
            }
            hir::ExprKind::Var(_, name) => self.load_var(name),
            hir::ExprKind::FnRef(_, name) => {
                if let Some(fv) = self.module.get_function(name) {
                    Ok(fv.as_global_value().as_pointer_value().into())
                } else {
                    Err(format!("undefined function: {name}"))
                }
            }
            hir::ExprKind::VariantRef(enum_name, variant_name, tag) => {
                self.compile_variant(enum_name, *tag, variant_name, &[])
            }
            hir::ExprKind::BinOp(l, op, r) => self.compile_binop(l, *op, r, &expr.ty),
            hir::ExprKind::UnaryOp(op, e) => self.compile_unary(*op, e),
            hir::ExprKind::Call(_, name, args) => self.compile_direct_call(name, args),
            hir::ExprKind::IndirectCall(callee, args) => self.compile_indirect_call(callee, args),
            hir::ExprKind::Builtin(builtin, args) => self.compile_builtin(builtin, args),
            hir::ExprKind::Method(obj, resolved_name, _method_name, args) => {
                self.compile_method(obj, resolved_name, args)
            }
            hir::ExprKind::StringMethod(obj, method, args) => {
                self.compile_string_method(obj, method, args)
            }
            hir::ExprKind::Field(obj, field, idx) => self.compile_field(obj, field, *idx),
            hir::ExprKind::Index(arr, idx) => self.compile_index(arr, idx),
            hir::ExprKind::Ternary(c, t, e) => self.compile_ternary(c, t, e),
            hir::ExprKind::Coerce(inner, coercion) => {
                let val = self.compile_expr(inner)?;
                self.compile_coercion(val, coercion)
            }
            hir::ExprKind::Cast(inner, target_ty) => self.compile_cast(inner, target_ty),
            hir::ExprKind::Array(elems) => self.compile_array(elems),
            hir::ExprKind::Tuple(elems) => self.compile_tuple(elems),
            hir::ExprKind::Struct(name, inits) => self.compile_struct(name, inits),
            hir::ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                self.compile_variant(enum_name, *tag, variant_name, inits)
            }
            hir::ExprKind::IfExpr(i) => match self.compile_if(i)? {
                Some(v) => Ok(v),
                None => Ok(self.ctx.i64_type().const_int(0, false).into()),
            },
            hir::ExprKind::Pipe(left, _def_id, name, extra_args) => {
                self.compile_pipe(left, name, extra_args)
            }
            hir::ExprKind::Block(block) => match self.compile_block(block)? {
                Some(v) => Ok(v),
                None => Ok(self.ctx.i64_type().const_int(0, false).into()),
            },
            hir::ExprKind::Lambda(params, body) => self.compile_lambda(params, body, &expr.ty),
            hir::ExprKind::Ref(inner) => self.compile_ref(inner),
            hir::ExprKind::Deref(inner) => self.compile_deref(inner),
            hir::ExprKind::ListComp(body, _def_id, bind, iter, end, cond) => {
                self.compile_list_comp(body, bind, iter, end.as_deref(), cond.as_deref())
            }
            hir::ExprKind::Syscall(args) => self.compile_syscall(args),
            hir::ExprKind::Spawn(actor_name) => self.compile_spawn(actor_name),
            hir::ExprKind::Send(target, actor_name, handler_name, tag, args) => {
                self.compile_send(target, actor_name, handler_name, *tag, args)
            }
            hir::ExprKind::StoreQuery(store_name, filter) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_query(store_name, filter, &sd)
            }
            hir::ExprKind::StoreCount(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_count(store_name, &sd)
            }
            hir::ExprKind::StoreAll(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_all(store_name, &sd)
            }
            hir::ExprKind::VecNew(elems) => self.compile_vec_new(elems),
            hir::ExprKind::MapNew => self.compile_map_new(),
            hir::ExprKind::VecMethod(obj, method, args) => {
                self.compile_vec_method(obj, method, args)
            }
            hir::ExprKind::MapMethod(obj, method, args) => {
                self.compile_map_method(obj, method, args)
            }
            hir::ExprKind::SetNew => self.compile_set_new(),
            hir::ExprKind::SetMethod(obj, method, args) => {
                self.compile_set_method(obj, method, args)
            }
            hir::ExprKind::PQNew => self.compile_pq_new(),
            hir::ExprKind::PQMethod(obj, method, args) => {
                self.compile_pq_method(obj, method, args)
            }
            hir::ExprKind::NDArrayNew(dims) => {
                self.compile_ndarray_new(dims)
            }
            hir::ExprKind::SIMDNew(elems) => {
                self.compile_simd_new(elems, &expr.ty)
            }
            hir::ExprKind::CoroutineCreate(name, body) => self.compile_coroutine_create(name, body),
            hir::ExprKind::CoroutineNext(coro) => self.compile_coroutine_next(coro),
            hir::ExprKind::Yield(_inner) => {
                panic!("yield expression outside of coroutine body")
            }
            hir::ExprKind::DynDispatch(obj, trait_name, method, args) => {
                self.compile_dyn_dispatch(obj, trait_name, method, args, &expr.ty)
            }
            hir::ExprKind::DynCoerce(inner, type_name, trait_name) => {
                self.compile_dyn_coerce(inner, type_name, trait_name)
            }
            hir::ExprKind::IterNext(iter_var, type_name, method_name) => {
                self.compile_iter_next_by_name(iter_var, type_name, method_name)
            }
            hir::ExprKind::ChannelCreate(elem_ty, cap_expr) => {
                self.compile_channel_create(elem_ty, cap_expr)
            }
            hir::ExprKind::ChannelSend(ch_expr, val_expr) => {
                self.compile_channel_send(ch_expr, val_expr)
            }
            hir::ExprKind::ChannelRecv(ch_expr) => self.compile_channel_recv(ch_expr, &expr.ty),
            hir::ExprKind::Select(arms, default_body) => {
                self.compile_select(arms, default_body.as_ref())
            }
            hir::ExprKind::Unreachable => {
                let fn_val = self.cur_fn.unwrap();
                let unreachable_bb = self.ctx.append_basic_block(fn_val, "unreachable");
                b!(self.bld.build_unconditional_branch(unreachable_bb));
                self.bld.position_at_end(unreachable_bb);
                b!(self.bld.build_unreachable());
                // Return a dummy value — this code is never reached
                Ok(self.ctx.i64_type().const_zero().into())
            }
            hir::ExprKind::StrictCast(inner, target_ty) => {
                self.compile_strict_cast(inner, target_ty)
            }
            hir::ExprKind::AsFormat(inner, fmt) => {
                self.compile_as_format(inner, fmt)
            }
            hir::ExprKind::AtomicLoad(ptr_expr) => {
                self.compile_atomic_load(ptr_expr)
            }
            hir::ExprKind::AtomicStore(ptr_expr, val_expr) => {
                self.compile_atomic_store(ptr_expr, val_expr)
            }
            hir::ExprKind::AtomicAdd(ptr_expr, val_expr) => {
                self.compile_atomic_add(ptr_expr, val_expr)
            }
            hir::ExprKind::AtomicSub(ptr_expr, val_expr) => {
                self.compile_atomic_sub(ptr_expr, val_expr)
            }
            hir::ExprKind::AtomicCas(ptr_expr, expected_expr, new_expr) => {
                self.compile_atomic_cas(ptr_expr, expected_expr, new_expr)
            }
            hir::ExprKind::Slice(obj, start, end) => {
                self.compile_slice(obj, start, end)
            }
            hir::ExprKind::DequeNew => {
                Err("DequeNew not yet implemented in codegen".into())
            }
            hir::ExprKind::DequeMethod(_, _, _) => {
                Err("DequeMethod not yet implemented in codegen".into())
            }
            hir::ExprKind::Grad(_) => {
                Err("Grad not yet implemented in codegen".into())
            }
            hir::ExprKind::Einsum(_, _) => {
                Err("Einsum not yet implemented in codegen".into())
            }
            hir::ExprKind::Builder(_, _) => {
                Err("Builder not yet implemented in codegen".into())
            }
            hir::ExprKind::CowWrap(_) => {
                Err("CowWrap not yet implemented in codegen".into())
            }
            hir::ExprKind::CowClone(_) => {
                Err("CowClone not yet implemented in codegen".into())
            }
            hir::ExprKind::GeneratorCreate(_, _, _) => {
                Err("GeneratorCreate not yet implemented in codegen".into())
            }
            hir::ExprKind::GeneratorNext(_) => {
                Err("GeneratorNext not yet implemented in codegen".into())
            }
        }
    }

    pub(crate) fn compile_short_circuit(
        &mut self,
        left: &hir::Expr,
        right: &hir::Expr,
        is_and: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let lhs = self.compile_expr(left)?;
        let lbool = self.to_bool(lhs);
        let rhs_bb = self.ctx.append_basic_block(fv, "sc.rhs");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");
        let lhs_bb = self.bld.get_insert_block().unwrap();
        if is_and {
            b!(self.bld.build_conditional_branch(lbool, rhs_bb, merge_bb));
        } else {
            b!(self.bld.build_conditional_branch(lbool, merge_bb, rhs_bb));
        }
        self.bld.position_at_end(rhs_bb);
        let rhs = self.compile_expr(right)?;
        let rbool = self.to_bool(rhs);
        let rhs_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(merge_bb));
        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "sc"));
        let short_val = self
            .ctx
            .bool_type()
            .const_int(if is_and { 0 } else { 1 }, false);
        phi.add_incoming(&[(&short_val, lhs_bb), (&rbool, rhs_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_unary(
        &mut self,
        op: UnaryOp,
        expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        Ok(match op {
            UnaryOp::Neg => {
                if expr.ty.is_float() {
                    b!(self.bld.build_float_neg(val.into_float_value(), "fneg")).into()
                } else {
                    b!(self.bld.build_int_nsw_neg(val.into_int_value(), "neg")).into()
                }
            }
            UnaryOp::Not | UnaryOp::BitNot => {
                b!(self.bld.build_not(val.into_int_value(), "not")).into()
            }
        })
    }

    fn compile_ternary(
        &mut self,
        cond: &hir::Expr,
        then_e: &hir::Expr,
        else_e: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let tv = self.compile_expr(cond)?;
        let cv = self.to_bool(tv);
        let tbb = self.ctx.append_basic_block(fv, "t.then");
        let ebb = self.ctx.append_basic_block(fv, "t.else");
        let mbb = self.ctx.append_basic_block(fv, "t.merge");
        b!(self.bld.build_conditional_branch(cv, tbb, ebb));
        self.bld.position_at_end(tbb);
        let tv = self.compile_expr(then_e)?;
        let tbb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(ebb);
        let ev = self.compile_expr(else_e)?;
        let ebb_end = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(mbb);
        let phi = b!(self.bld.build_phi(self.llvm_ty(&then_e.ty), "tern"));
        phi.add_incoming(&[(&tv, tbb_end), (&ev, ebb_end)]);
        Ok(phi.as_basic_value())
    }

    fn compile_cast(
        &mut self,
        expr: &hir::Expr,
        target: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(expr)?;
        let src = &expr.ty;
        let dst = self.llvm_ty(target);
        if src.is_int() && target.is_float() {
            return Ok(if src.is_signed() {
                b!(self.bld.build_signed_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "sitofp"
                ))
                .into()
            } else {
                b!(self.bld.build_unsigned_int_to_float(
                    val.into_int_value(),
                    dst.into_float_type(),
                    "uitofp"
                ))
                .into()
            });
        }
        if src.is_float() && target.is_int() {
            return Ok(if target.is_signed() {
                b!(self.bld.build_float_to_signed_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptosi"
                ))
                .into()
            } else {
                b!(self.bld.build_float_to_unsigned_int(
                    val.into_float_value(),
                    dst.into_int_type(),
                    "fptoui"
                ))
                .into()
            });
        }
        if src.is_int() && target.is_int() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "sext"
                    ))
                    .into()
                } else {
                    b!(self.bld.build_int_z_extend(
                        val.into_int_value(),
                        dst.into_int_type(),
                        "zext"
                    ))
                    .into()
                }
            } else if sb > db {
                b!(self
                    .bld
                    .build_int_truncate(val.into_int_value(), dst.into_int_type(), "trunc"))
                .into()
            } else {
                val
            });
        }
        if src.is_float() && target.is_float() {
            let (sb, db) = (src.bits(), target.bits());
            return Ok(if sb < db {
                b!(self
                    .bld
                    .build_float_ext(val.into_float_value(), dst.into_float_type(), "fpext"))
                .into()
            } else if sb > db {
                b!(self.bld.build_float_trunc(
                    val.into_float_value(),
                    dst.into_float_type(),
                    "fptrunc"
                ))
                .into()
            } else {
                val
            });
        }
        if matches!(src, Type::Bool) && target.is_int() {
            return Ok(b!(self.bld.build_int_z_extend(
                val.into_int_value(),
                dst.into_int_type(),
                "boolext"
            ))
            .into());
        }
        if matches!(src, Type::Ptr(_)) && target.is_int() {
            return Ok(b!(self.bld.build_ptr_to_int(
                val.into_pointer_value(),
                dst.into_int_type(),
                "ptrtoint"
            ))
            .into());
        }
        Err(format!("unsupported cast: {src} as {target}"))
    }

    pub(crate) fn compile_array(
        &mut self,
        elems: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if elems.is_empty() {
            return Err("empty array literal".into());
        }
        let elem_ty = &elems[0].ty;
        let lty = self.llvm_ty(elem_ty);
        let arr_ty = lty.array_type(elems.len() as u32);
        let ptr = self.entry_alloca(arr_ty.into(), "arr");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = unsafe {
                b!(self.bld.build_gep(
                    arr_ty,
                    ptr,
                    &[
                        self.ctx.i64_type().const_int(0, false),
                        self.ctx.i64_type().const_int(i as u64, false)
                    ],
                    "arr.gep"
                ))
            };
            b!(self.bld.build_store(gep, val));
        }
        Ok(ptr.into())
    }

    pub(crate) fn compile_tuple(
        &mut self,
        elems: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ltys: Vec<BasicTypeEnum<'ctx>> = elems.iter().map(|e| self.llvm_ty(&e.ty)).collect();
        let st = self.ctx.struct_type(&ltys, false);
        let ptr = self.entry_alloca(st.into(), "tup");
        for (i, e) in elems.iter().enumerate() {
            let val = self.compile_expr(e)?;
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, "tup.gep"));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, "tup")))
    }

    pub(crate) fn compile_struct(
        &mut self,
        name: &str,
        inits: &[hir::FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fields = self
            .structs
            .get(name)
            .ok_or_else(|| format!("undefined type: {name}"))?
            .clone();
        let st = self
            .module
            .get_struct_type(name)
            .ok_or_else(|| format!("no LLVM struct: {name}"))?;
        let align = self.struct_layouts.get(name).and_then(|l| l.align);
        let ptr = if let Some(a) = align {
            self.entry_alloca_aligned(st.into(), name, a)
        } else {
            self.entry_alloca(st.into(), name)
        };
        let defaults = self.struct_defaults.get(name).cloned();
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let val = inits
                .iter()
                .find(|fi| fi.name.as_deref() == Some(fname))
                .or_else(|| inits.get(i))
                .map(|fi| self.compile_expr(&fi.value))
                .or_else(|| {
                    defaults
                        .as_ref()
                        .and_then(|d| d.get(fname))
                        .map(|def| self.compile_expr(def))
                })
                .transpose()?
                .unwrap_or_else(|| self.default_val(fty));
            let expected_ty = self.llvm_ty(fty);
            let val = self.coerce_val(val, expected_ty);
            let gep = b!(self.bld.build_struct_gep(st, ptr, i as u32, fname));
            b!(self.bld.build_store(gep, val));
        }
        Ok(b!(self.bld.build_load(st, ptr, name)))
    }

    pub(crate) fn compile_variant(
        &mut self,
        enum_name: &str,
        tag: u32,
        variant_name: &str,
        inits: &[hir::FieldInit],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let st = self
            .module
            .get_struct_type(enum_name)
            .ok_or_else(|| format!("no LLVM type: {enum_name}"))?;
        let variants = self
            .enums
            .get(enum_name)
            .cloned()
            .ok_or_else(|| format!("undefined enum: {enum_name}"))?;
        let (_, ftys) = variants
            .iter()
            .find(|(n, _)| n == variant_name)
            .ok_or_else(|| format!("no variant {variant_name}"))?;
        let ftys = ftys.clone();
        let ptr = self.entry_alloca(st.into(), variant_name);
        let tag_gep = b!(self.bld.build_struct_gep(st, ptr, 0, "tag"));
        b!(self
            .bld
            .build_store(tag_gep, self.ctx.i32_type().const_int(tag as u64, false)));
        if !ftys.is_empty() {
            let payload_gep = b!(self.bld.build_struct_gep(st, ptr, 1, "payload"));
            let mut offset = 0u64;
            for (i, fty) in ftys.iter().enumerate() {
                let val = inits
                    .get(i)
                    .map(|fi| self.compile_expr(&fi.value))
                    .transpose()?
                    .unwrap_or_else(|| self.default_val(fty));
                let is_rec = Self::is_recursive_field(fty, enum_name);
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
                    let actual_ty = self.llvm_ty(fty);
                    let size = self.type_store_size(actual_ty);
                    let malloc = self.ensure_malloc();
                    let heap = b!(self.bld.build_call(
                        malloc,
                        &[self.ctx.i64_type().const_int(size, false).into()],
                        "box.alloc"
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                    b!(self.bld.build_store(heap, val));
                    b!(self.bld.build_store(field_ptr, heap));
                    offset += 8;
                } else {
                    let lty = self.llvm_ty(fty);
                    let coerced = self.coerce_val(val, lty);
                    b!(self.bld.build_store(field_ptr, coerced));
                    offset += self.type_store_size(lty);
                }
            }
        }
        Ok(b!(self.bld.build_load(st, ptr, variant_name)))
    }

    fn compile_field(
        &mut self,
        obj: &hir::Expr,
        field: &str,
        _hir_idx: usize,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_ty = &obj.ty;
        if matches!(obj_ty, Type::String) && field == "length" {
            let sv = self.compile_expr(obj)?;
            return self.string_len(sv);
        }
        if matches!(obj_ty, Type::Vec(_)) && field == "length" {
            let v = self.compile_expr(obj)?;
            return self.vec_len(v.into_pointer_value());
        }
        let (ty_name, is_ptr) = match obj_ty {
            Type::Struct(n, _) => (n.as_str(), false),
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(n, _) => (n.as_str(), true),
                other => return Err(format!("field access on non-struct: {other}")),
            },
            other => return Err(format!("field access on non-struct: {other}")),
        };
        let fields = self
            .structs
            .get(ty_name)
            .ok_or_else(|| format!("undefined type: {ty_name}"))?
            .clone();
        let idx = fields
            .iter()
            .position(|(n, _)| n == field)
            .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
        let fty = fields[idx].1.clone();
        let st = self
            .module
            .get_struct_type(ty_name)
            .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;

        // Get a pointer to the struct, either from a variable or by spilling a value
        let struct_ptr = if let hir::ExprKind::Var(_, n) = &obj.kind {
            if let Some((ptr, _)) = self.find_var(n).cloned() {
                if is_ptr {
                    b!(self.bld.build_load(
                        self.ctx.ptr_type(inkwell::AddressSpace::default()),
                        ptr,
                        "self.ptr"
                    ))
                    .into_pointer_value()
                } else {
                    ptr
                }
            } else {
                // Variable not found — compile and spill
                let val = self.compile_expr(obj)?;
                let spill = self.entry_alloca(st.into(), "field.spill");
                b!(self.bld.build_store(spill, val));
                spill
            }
        } else {
            // Non-variable object (e.g., chained field access, function return) — compile and spill
            let val = self.compile_expr(obj)?;
            let spill = self.entry_alloca(st.into(), "field.spill");
            b!(self.bld.build_store(spill, val));
            spill
        };

        let gep = b!(self.bld.build_struct_gep(st, struct_ptr, idx as u32, field));
        Ok(b!(self.bld.build_load(self.llvm_ty(&fty), gep, field)))
    }

    /// Return a pointer to the memory location of an lvalue expression.
    /// Used for chained field assignment (e.g. `a.b.x = 42`).
    pub(crate) fn compile_lvalue_ptr(
        &mut self,
        expr: &hir::Expr,
    ) -> Result<inkwell::values::PointerValue<'ctx>, String> {
        match &expr.kind {
            hir::ExprKind::Var(_, name) => {
                self.find_var(name)
                    .map(|(ptr, _)| *ptr)
                    .ok_or_else(|| format!("undefined: {name}"))
            }
            hir::ExprKind::Field(obj, field, _idx) => {
                let obj_ty = &obj.ty;
                let (ty_name, is_ptr) = match obj_ty {
                    Type::Struct(n, _) => (n.as_str(), false),
                    Type::Ptr(inner) => match inner.as_ref() {
                        Type::Struct(n, _) => (n.as_str(), true),
                        _ => return Err("field lvalue on non-struct".into()),
                    },
                    _ => return Err("field lvalue on non-struct".into()),
                };
                let fields = self
                    .structs
                    .get(ty_name)
                    .ok_or_else(|| format!("undefined type: {ty_name}"))?
                    .clone();
                let fi = fields
                    .iter()
                    .position(|(n, _)| n == field)
                    .ok_or_else(|| format!("no field '{field}' on {ty_name}"))?;
                let st = self
                    .module
                    .get_struct_type(ty_name)
                    .ok_or_else(|| format!("no LLVM struct: {ty_name}"))?;
                let obj_ptr = self.compile_lvalue_ptr(obj)?;
                let struct_ptr = if is_ptr {
                    b!(self.bld.build_load(
                        self.ctx.ptr_type(inkwell::AddressSpace::default()),
                        obj_ptr,
                        "self.ptr"
                    ))
                    .into_pointer_value()
                } else {
                    obj_ptr
                };
                let gep = b!(self.bld.build_struct_gep(st, struct_ptr, fi as u32, field));
                Ok(gep)
            }
            _ => Err("expression is not an lvalue".into()),
        }
    }

    fn compile_index(
        &mut self,
        arr: &hir::Expr,
        idx: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let arr_ty = &arr.ty;
        let idx_val = self.compile_expr(idx)?.into_int_value();
        match arr_ty {
            Type::Array(elem_ty, n) => {
                let lty = self.llvm_ty(elem_ty);
                let arr_llvm = lty.array_type(*n as u32);
                let arr_ptr = match &arr.kind {
                    hir::ExprKind::Var(_, name) => self
                        .find_var(name)
                        .map(|(ptr, _)| *ptr)
                        .ok_or_else(|| format!("undefined: {name}"))?,
                    _ => self.compile_expr(arr)?.into_pointer_value(),
                };
                let idx_val = self.wrap_negative_index(idx_val, *n as u64)?;
                self.emit_bounds_check(idx_val, *n as u64)?;
                let gep = unsafe {
                    b!(self.bld.build_gep(
                        arr_llvm,
                        arr_ptr,
                        &[self.ctx.i64_type().const_int(0, false), idx_val],
                        "idx"
                    ))
                };
                Ok(b!(self.bld.build_load(lty, gep, "elem")))
            }
            Type::Tuple(tys) => {
                let i = idx_val
                    .get_zero_extended_constant()
                    .ok_or("tuple index must be a constant")?;
                let fty = tys
                    .get(i as usize)
                    .ok_or_else(|| format!("tuple index {i} out of bounds"))?;
                let lty = self.llvm_ty(fty);
                if let hir::ExprKind::Var(_, name) = &arr.kind {
                    if let Some((ptr, _)) = self.find_var(name).cloned() {
                        let tup_ty = self.ctx.struct_type(
                            &tys.iter().map(|t| self.llvm_ty(t)).collect::<Vec<_>>(),
                            false,
                        );
                        let gep = b!(self.bld.build_struct_gep(tup_ty, ptr, i as u32, "tup.idx"));
                        return Ok(b!(self.bld.build_load(lty, gep, "tup.elem")));
                    }
                }
                Err("tuple indexing on rvalue not supported".into())
            }
            Type::Vec(elem_ty) => {
                let lty = self.llvm_ty(elem_ty);
                let header_ptr = self.compile_expr(arr)?.into_pointer_value();
                let header_ty = self.vec_header_type();
                let ptr_gep = b!(self
                    .bld
                    .build_struct_gep(header_ty, header_ptr, 0, "vi.ptrp"));
                let data_ptr = b!(self.bld.build_load(
                    self.ctx.ptr_type(inkwell::AddressSpace::default()),
                    ptr_gep,
                    "vi.data"
                ))
                .into_pointer_value();
                let len_gep = b!(self
                    .bld
                    .build_struct_gep(header_ty, header_ptr, 1, "vi.lenp"));
                let len = b!(self.bld.build_load(
                    self.ctx.i64_type(),
                    len_gep,
                    "vi.len"
                ))
                .into_int_value();
                self.emit_vec_bounds_check(idx_val, len)?;
                let elem_gep =
                    unsafe { b!(self.bld.build_gep(lty, data_ptr, &[idx_val], "vi.egep")) };
                Ok(b!(self.bld.build_load(lty, elem_gep, "vi.elem")))
            }
            _ => {
                let arr_ptr = self.compile_expr(arr)?.into_pointer_value();
                let i64t = self.ctx.i64_type();
                let gep = unsafe { b!(self.bld.build_gep(i64t, arr_ptr, &[idx_val], "idx")) };
                Ok(b!(self.bld.build_load(i64t, gep, "elem")))
            }
        }
    }

    pub(crate) fn compile_str_literal(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        if s.len() <= 23 {
            let st = self.string_type();
            let i8t = self.ctx.i8_type();
            let i64t = self.ctx.i64_type();
            let out = self.entry_alloca(st.into(), "slit");
            b!(self.bld.build_store(out, st.const_zero()));
            for (i, byte) in s.bytes().enumerate() {
                let bp = unsafe {
                    b!(self
                        .bld
                        .build_gep(i8t, out, &[i64t.const_int(i as u64, false)], "sso.b"))
                };
                b!(self.bld.build_store(bp, i8t.const_int(byte as u64, false)));
            }
            let tag_ptr = unsafe {
                b!(self
                    .bld
                    .build_gep(i8t, out, &[i64t.const_int(23, false)], "sso.tag"))
            };
            b!(self
                .bld
                .build_store(tag_ptr, i8t.const_int(0x80 | s.len() as u64, false)));
            Ok(b!(self.bld.build_load(st, out, "slit")))
        } else {
            let gstr = b!(self.bld.build_global_string_ptr(s, "str"));
            let i64t = self.ctx.i64_type();
            self.build_string(
                gstr.as_pointer_value(),
                i64t.const_int(s.len() as u64, false),
                i64t.const_int(0, false),
                "slit",
            )
        }
    }

    fn compile_ref(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match &inner.kind {
            hir::ExprKind::Var(_, name) => self
                .find_var(name)
                .map(|(ptr, _)| *ptr)
                .ok_or_else(|| format!("cannot take address of '{name}'"))
                .map(|p| p.into()),
            hir::ExprKind::FnRef(_, name) => {
                if let Some(fv) = self.module.get_function(name) {
                    Ok(fv.as_global_value().as_pointer_value().into())
                } else {
                    Err(format!("undefined function: {name}"))
                }
            }
            _ => Err("& requires a variable name".into()),
        }
    }

    fn compile_deref(&mut self, inner: &hir::Expr) -> Result<BasicValueEnum<'ctx>, String> {
        if let Type::Rc(ref elem_ty) = inner.ty {
            let rv = self.compile_expr(inner)?;
            return self.rc_deref(rv, elem_ty);
        }
        if let Type::Weak(_) = inner.ty {
            return Err("cannot deref a weak reference directly — use weak_upgrade() first".into());
        }
        let ptr_val = self.compile_expr(inner)?;
        let load_ty = match &inner.ty {
            Type::Ptr(inner_ty) => self.llvm_ty(inner_ty),
            _ => self.ctx.i64_type().into(),
        };
        Ok(b!(self.bld.build_load(
            load_ty,
            ptr_val.into_pointer_value(),
            "deref"
        )))
    }

    fn compile_syscall(&mut self, args: &[hir::Expr]) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Err("syscall requires at least 1 argument (syscall number)".into());
        }
        let i64t = self.ctx.i64_type();
        let mut vals: Vec<BasicValueEnum<'ctx>> = Vec::new();
        for arg in args {
            vals.push(self.compile_expr(arg)?);
        }
        let nargs = vals.len();
        let (template, constraints) = match nargs {
            1 => ("syscall", "={rax},{rax},~{rcx},~{r11},~{memory}"),
            2 => ("syscall", "={rax},{rax},{rdi},~{rcx},~{r11},~{memory}"),
            3 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},~{rcx},~{r11},~{memory}",
            ),
            4 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},~{rcx},~{r11},~{memory}",
            ),
            5 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},~{rcx},~{r11},~{memory}",
            ),
            6 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},~{rcx},~{r11},~{memory}",
            ),
            7 => (
                "syscall",
                "={rax},{rax},{rdi},{rsi},{rdx},{r10},{r8},{r9},~{rcx},~{r11},~{memory}",
            ),
            _ => return Err("syscall supports 0-6 arguments".into()),
        };
        let input_types: Vec<BasicMetadataTypeEnum<'ctx>> =
            vals.iter().map(|_| i64t.into()).collect();
        let ft = i64t.fn_type(&input_types, false);
        let inline_asm = self.ctx.create_inline_asm(
            ft,
            template.to_string(),
            constraints.to_string(),
            true,
            false,
            None,
            false,
        );
        let args_meta: Vec<BasicMetadataValueEnum<'ctx>> =
            vals.iter().map(|v| (*v).into()).collect();
        let result = b!(self
            .bld
            .build_indirect_call(ft, inline_asm, &args_meta, "syscall"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| i64t.const_int(0, false).into()))
    }

    fn compile_list_comp(
        &mut self,
        body: &hir::Expr,
        bind: &str,
        start: &hir::Expr,
        end: Option<&hir::Expr>,
        cond: Option<&hir::Expr>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let end_expr = end.ok_or("list comprehension requires 'to' end bound")?;
        let i64t = self.ctx.i64_type();
        let start_val = self.compile_expr(start)?.into_int_value();
        let end_val = self.compile_expr(end_expr)?.into_int_value();
        let elem_ty = i64t;
        let range = b!(self.bld.build_int_sub(end_val, start_val, "comp.range"));
        let zero = i64t.const_int(0, false);
        let is_pos = b!(self
            .bld
            .build_int_compare(IntPredicate::SGT, range, zero, "comp.pos"));
        let safe_range = b!(self.bld.build_select(is_pos, range, zero, "comp.sz")).into_int_value();
        let elem_size = i64t.const_int(8, false);
        let alloc_size = b!(self.bld.build_int_mul(safe_range, elem_size, "comp.bytes"));
        let malloc_fn = self.ensure_malloc();
        let arr_ptr = b!(self
            .bld
            .build_call(malloc_fn, &[alloc_size.into()], "comp_arr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();
        let fv = self.cur_fn.unwrap();
        let loop_bb = self.ctx.append_basic_block(fv, "comp_loop");
        let body_bb = self.ctx.append_basic_block(fv, "comp_body");
        let skip_bb = if cond.is_some() {
            Some(self.ctx.append_basic_block(fv, "comp_skip"))
        } else {
            None
        };
        let done_bb = self.ctx.append_basic_block(fv, "comp_done");
        let idx_ptr = self.entry_alloca(i64t.into(), "comp_idx");
        let cnt_ptr = self.entry_alloca(i64t.into(), "comp_cnt");
        b!(self.bld.build_store(idx_ptr, start_val));
        b!(self.bld.build_store(cnt_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur_idx, end_val, "cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));
        self.bld.position_at_end(body_bb);
        self.vars.push(HashMap::new());
        let bind_alloca = self.entry_alloca(i64t.into(), bind);
        b!(self.bld.build_store(bind_alloca, cur_idx));
        self.set_var(bind, bind_alloca, Type::I64);
        if let Some(cond_expr) = cond {
            let store_bb = self.ctx.append_basic_block(fv, "comp_store");
            let cond_val = self.compile_expr(cond_expr)?;
            let cbool = self.to_bool(cond_val);
            b!(self
                .bld
                .build_conditional_branch(cbool, store_bb, skip_bb.unwrap()));
            self.bld.position_at_end(store_bb);
        }
        let val = self.compile_expr(body)?;
        let cur_cnt = b!(self.bld.build_load(i64t, cnt_ptr, "cnt")).into_int_value();
        let elem_ptr = unsafe { b!(self.bld.build_gep(elem_ty, arr_ptr, &[cur_cnt], "elem")) };
        b!(self.bld.build_store(elem_ptr, val));
        let next_cnt = b!(self
            .bld
            .build_int_add(cur_cnt, i64t.const_int(1, false), "ncnt"));
        b!(self.bld.build_store(cnt_ptr, next_cnt));
        self.vars.pop();
        let next_idx = b!(self
            .bld
            .build_int_add(cur_idx, i64t.const_int(1, false), "nidx"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        if let Some(skip) = skip_bb {
            b!(self.bld.build_unconditional_branch(loop_bb));
            self.bld.position_at_end(skip);
            let cur_idx2 = b!(self.bld.build_load(i64t, idx_ptr, "idx2")).into_int_value();
            let next_idx2 = b!(self
                .bld
                .build_int_add(cur_idx2, i64t.const_int(1, false), "nidx2"));
            b!(self.bld.build_store(idx_ptr, next_idx2));
            b!(self.bld.build_unconditional_branch(loop_bb));
        } else {
            b!(self.bld.build_unconditional_branch(loop_bb));
        }
        self.bld.position_at_end(done_bb);
        Ok(arr_ptr.into())
    }

    fn compile_dyn_coerce(
        &mut self,
        inner: &hir::Expr,
        type_name: &str,
        trait_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let val = self.compile_expr(inner)?;
        let ptr = self.ctx.ptr_type(inkwell::AddressSpace::default());

        let data_ptr = if val.is_pointer_value() {
            val.into_pointer_value()
        } else {
            let lty = val.get_type();
            let alloc = self.entry_alloca(lty, "dyn.data");
            b!(self.bld.build_store(alloc, val));
            alloc
        };

        let vtable_ptr = self
            .vtables
            .get(&(type_name.to_string(), trait_name.to_string()))
            .map(|gv| gv.as_pointer_value())
            .unwrap_or_else(|| ptr.const_null());

        let fat_ty = self.ctx.struct_type(&[ptr.into(), ptr.into()], false);
        let fat = fat_ty.const_zero();
        let fat = b!(self
            .bld
            .build_insert_value(fat, data_ptr, 0, "dyn.fat.data"))
        .into_struct_value();
        let fat = b!(self
            .bld
            .build_insert_value(fat, vtable_ptr, 1, "dyn.fat.vtable"))
        .into_struct_value();
        Ok(fat.into())
    }

    fn compile_dyn_dispatch(
        &mut self,
        obj: &hir::Expr,
        trait_name: &str,
        method: &str,
        args: &[hir::Expr],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.compile_expr(obj)?;
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let fat_ty = self.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

        let tmp = self.entry_alloca(fat_ty.into(), "dyn.tmp");
        b!(self.bld.build_store(tmp, fat));
        let data_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 0, "dyn.data.gep"));
        let data_ptr = b!(self.bld.build_load(ptr_ty, data_gep, "dyn.data")).into_pointer_value();
        let vtable_gep = b!(self.bld.build_struct_gep(fat_ty, tmp, 1, "dyn.vtable.gep"));
        let vtable_ptr =
            b!(self.bld.build_load(ptr_ty, vtable_gep, "dyn.vtable")).into_pointer_value();

        let method_idx = self
            .trait_method_order
            .get(trait_name)
            .and_then(|methods| methods.iter().position(|m| m == method))
            .unwrap_or(0) as u64;

        let fn_ptr_gep = unsafe {
            b!(self.bld.build_gep(
                ptr_ty,
                vtable_ptr,
                &[self.ctx.i64_type().const_int(method_idx, false)],
                "dyn.fn.gep"
            ))
        };
        let fn_ptr = b!(self.bld.build_load(ptr_ty, fn_ptr_gep, "dyn.fn")).into_pointer_value();

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![data_ptr.into()];
        for arg in args {
            let av = self.compile_expr(arg)?;
            call_args.push(av.into());
        }

        let ret_ty = self.llvm_ty(result_ty);
        let mut param_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for arg in args {
            param_tys.push(self.llvm_ty(&arg.ty).into());
        }
        let fn_ty = ret_ty.fn_type(&param_tys, false);
        let result = b!(self
            .bld
            .build_indirect_call(fn_ty, fn_ptr, &call_args, "dyn.call"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }

    fn compile_iter_next_by_name(
        &mut self,
        var_name: &str,
        type_name: &str,
        method_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self
            .find_var(var_name)
            .ok_or_else(|| format!("undefined iter variable: {var_name}"))?
            .0;

        let fn_name = format!("{type_name}_{method_name}");
        let fv = self
            .module
            .get_function(&fn_name)
            .ok_or_else(|| format!("no function {fn_name}"))?;
        let result = b!(self.bld.build_call(fv, &[ptr.into()], "iter.next"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.ctx.i64_type().const_int(0, false).into()))
    }

    fn compile_strict_cast(
        &mut self,
        expr: &hir::Expr,
        target: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Strict cast: same as regular cast but with runtime bounds check for narrowing
        let val = self.compile_expr(expr)?;
        let src = &expr.ty;
        if src.is_int() && target.is_int() {
            let dst = self.llvm_ty(target);
            let (sb, db) = (src.bits(), target.bits());
            if sb > db {
                // Narrowing: truncate then sign-extend back and compare
                let truncated = b!(self.bld.build_int_truncate(
                    val.into_int_value(),
                    dst.into_int_type(),
                    "strict.trunc"
                ));
                let extended = if src.is_signed() {
                    b!(self.bld.build_int_s_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                } else {
                    b!(self.bld.build_int_z_extend(
                        truncated,
                        val.into_int_value().get_type(),
                        "strict.ext"
                    ))
                };
                let ok = b!(self.bld.build_int_compare(
                    IntPredicate::EQ,
                    val.into_int_value(),
                    extended,
                    "strict.ok"
                ));
                let fv = self.cur_fn.unwrap();
                let pass_bb = self.ctx.append_basic_block(fv, "strict.pass");
                let fail_bb = self.ctx.append_basic_block(fv, "strict.fail");
                b!(self.bld.build_conditional_branch(ok, pass_bb, fail_bb));
                self.bld.position_at_end(fail_bb);
                self.emit_trap("strict cast: value out of range");
                self.bld.position_at_end(pass_bb);
                return Ok(truncated.into());
            } else if sb < db {
                return Ok(if src.is_signed() {
                    b!(self.bld.build_int_s_extend(val.into_int_value(), dst.into_int_type(), "strict.sext")).into()
                } else {
                    b!(self.bld.build_int_z_extend(val.into_int_value(), dst.into_int_type(), "strict.zext")).into()
                });
            }
            return Ok(val);
        }
        // For non-int-to-int casts, fall back to regular cast behavior
        self.compile_cast(expr, target)
    }

    fn compile_as_format(
        &mut self,
        expr: &hir::Expr,
        _fmt: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // as json / as map — for now, delegate to the Display/toString path
        let val = self.compile_expr(expr)?;
        let _ = val;
        // Return empty string placeholder — full serialization would need runtime support
        self.compile_str_literal("")
    }

    fn compile_atomic_load(
        &mut self,
        ptr_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let i64t = self.ctx.i64_type();
        let load = b!(self.bld.build_load(i64t, ptr.into_pointer_value(), "atomic.load"));
        // Set atomic ordering
        load.as_instruction_value()
            .unwrap()
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(load)
    }

    fn compile_atomic_store(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let store = b!(self.bld.build_store(ptr.into_pointer_value(), val));
        store
            .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
            .map_err(|_| "failed to set atomic ordering".to_string())?;
        Ok(self.ctx.i64_type().const_zero().into())
    }

    fn compile_atomic_add(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Add,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    fn compile_atomic_sub(
        &mut self,
        ptr_expr: &hir::Expr,
        val_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let val = self.compile_expr(val_expr)?;
        let old = b!(self.bld.build_atomicrmw(
            inkwell::AtomicRMWBinOp::Sub,
            ptr.into_pointer_value(),
            val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        Ok(old.into())
    }

    fn compile_atomic_cas(
        &mut self,
        ptr_expr: &hir::Expr,
        expected_expr: &hir::Expr,
        new_expr: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.compile_expr(ptr_expr)?;
        let expected = self.compile_expr(expected_expr)?;
        let new_val = self.compile_expr(new_expr)?;
        let cas = b!(self.bld.build_cmpxchg(
            ptr.into_pointer_value(),
            expected.into_int_value(),
            new_val.into_int_value(),
            inkwell::AtomicOrdering::SequentiallyConsistent,
            inkwell::AtomicOrdering::SequentiallyConsistent,
        ));
        // cmpxchg returns {value, success_bit}; extract the old value
        let old = b!(self.bld.build_extract_value(cas, 0, "cas.old"));
        Ok(old)
    }

    fn compile_slice(
        &mut self,
        obj: &hir::Expr,
        start: &hir::Expr,
        end: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // For now, compile as a runtime call to a slice helper
        // Arrays: create a new vec from the slice range
        // Strings: create a substring
        let obj_val = self.compile_expr(obj)?;
        let start_val = self.compile_expr(start)?;
        let end_val = self.compile_expr(end)?;
        match &obj.ty {
            Type::Vec(_elem_ty) => {
                // Vec slice: call jade_vec_slice(vec_ptr, start, end) → new vec
                let slice_fn = self.module.get_function("__jade_vec_slice").unwrap_or_else(|| {
                    let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                    let i64t = self.ctx.i64_type();
                    let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                    self.module.add_function("__jade_vec_slice", ft, Some(inkwell::module::Linkage::External))
                });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into()],
                    "slice"
                ));
                Ok(result.try_as_basic_value().basic().unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            Type::String => {
                // String slice: call jade_str_slice(str, start, end) → new str
                let slice_fn = self.module.get_function("__jade_str_slice").unwrap_or_else(|| {
                    let st = self.string_type();
                    let i64t = self.ctx.i64_type();
                    let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                    self.module.add_function("__jade_str_slice", ft, Some(inkwell::module::Linkage::External))
                });
                let result = b!(self.bld.build_call(
                    slice_fn,
                    &[obj_val.into(), start_val.into(), end_val.into()],
                    "str.slice"
                ));
                Ok(result.try_as_basic_value().basic().unwrap_or_else(|| self.ctx.i64_type().const_zero().into()))
            }
            _ => {
                // Fallback: treat as pointer arithmetic (array case)
                Ok(obj_val)
            }
        }
    }
}
