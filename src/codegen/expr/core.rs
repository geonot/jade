//! Core HIR expression dispatch, scalar operations, aggregates, and variants.

use super::*;

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
            hir::ExprKind::Var(_, name) => self.load_var(&name.as_str()),
            hir::ExprKind::GlobalLoad(name) => {
                if let Some((gv, ty)) = self.globals.get(name).cloned() {
                    let lt = self.llvm_ty(&ty);
                    Ok(b!(self.bld.build_load(
                        lt,
                        gv.as_pointer_value(),
                        &format!("global.{name}")
                    )))
                } else {
                    Err(format!("undefined global `{name}`"))
                }
            }
            hir::ExprKind::FnRef(_, name) => {
                if let Some(fv) = self.module.get_function(&name.as_str()) {
                    let wrapper = self.fn_ref_wrapper(fv);
                    let null_env = self
                        .ctx
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    self.make_closure(wrapper, null_env)
                } else {
                    Err(format!("undefined function: {name}"))
                }
            }
            hir::ExprKind::VariantRef(enum_name, variant_name, tag) => {
                self.compile_variant(&enum_name.as_str(), *tag, &variant_name.as_str(), &[])
            }
            hir::ExprKind::BinOp(l, op, r) => self.compile_binop(l, *op, r, &expr.ty),
            hir::ExprKind::UnaryOp(op, e) => self.compile_unary(*op, e),
            hir::ExprKind::Call(_, name, args) => self.compile_direct_call(&name.as_str(), args),
            hir::ExprKind::IndirectCall(callee, args) => self.compile_indirect_call(callee, args),
            hir::ExprKind::Builtin(builtin, args) => self.compile_builtin(builtin, args),
            hir::ExprKind::Method(obj, resolved_name, _method_name, args) => {
                self.compile_method(obj, &resolved_name.as_str(), args)
            }
            hir::ExprKind::StringMethod(obj, method, args) => {
                self.compile_string_method(obj, &method.as_str(), args)
            }
            hir::ExprKind::DeferredMethod(_obj, method, _args) => Err(format!(
                "unresolved deferred method call '.{method}()' — type inference could not determine the receiver type"
            )),
            hir::ExprKind::Field(obj, field, idx) => self.compile_field(obj, &field.as_str(), *idx),
            hir::ExprKind::Index(arr, idx) => self.compile_index(arr, idx),
            hir::ExprKind::Ternary(c, t, e) => self.compile_ternary(c, t, e),
            hir::ExprKind::Coerce(inner, coercion) => {
                let val = self.compile_expr(inner)?;
                self.compile_coercion(val, coercion)
            }
            hir::ExprKind::Cast(inner, target_ty) => self.compile_cast(inner, target_ty),
            hir::ExprKind::Array(elems) => self.compile_array(elems),
            hir::ExprKind::Tuple(elems) => self.compile_tuple(elems),
            hir::ExprKind::Struct(name, inits) => self.compile_struct(&name.as_str(), inits),
            hir::ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                self.compile_variant(&enum_name.as_str(), *tag, &variant_name.as_str(), inits)
            }
            hir::ExprKind::IfExpr(i) => match self.compile_if(i)? {
                Some(v) => Ok(v),
                None => Ok(self.ctx.i64_type().const_int(0, false).into()),
            },
            hir::ExprKind::Pipe(left, _def_id, name, extra_args) => {
                self.compile_pipe(left, &name.as_str(), extra_args)
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
            hir::ExprKind::Spawn(actor_name, inits) => {
                self.compile_spawn_with_inits(&actor_name.as_str(), inits)
            }
            hir::ExprKind::Send(target, actor_name, handler_name, tag, args) => self.compile_send(
                target,
                &actor_name.as_str(),
                &handler_name.as_str(),
                *tag,
                args,
            ),
            hir::ExprKind::StoreQuery(store_name, filter) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_query(&store_name.as_str(), filter, &sd)
            }
            hir::ExprKind::StoreCount(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_count(&store_name.as_str(), &sd)
            }
            hir::ExprKind::StoreAll(store_name) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_all(&store_name.as_str(), &sd)
            }
            hir::ExprKind::ViewCount(store_name, filter) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_query(&store_name.as_str(), filter, &sd)
            }
            hir::ExprKind::ViewAll(store_name, _filter) => {
                let sd = self
                    .store_defs
                    .get(store_name)
                    .ok_or_else(|| format!("unknown store '{store_name}'"))?
                    .clone();
                self.compile_store_all(&store_name.as_str(), &sd)
            }
            hir::ExprKind::StoreGet(store_name, _key_expr) => Err(format!(
                "store.get is not yet implemented (store '{store_name}')"
            )),
            hir::ExprKind::StoreFirst(store_name, _filter) => Err(format!(
                "store.first is not yet implemented (store '{store_name}')"
            )),
            hir::ExprKind::StoreExists(store_name, _filter) => Err(format!(
                "store.exists is not yet implemented (store '{store_name}')"
            )),
            hir::ExprKind::StoreDistinct(store_name, _field) => Err(format!(
                "store.distinct is not yet implemented (store '{store_name}')"
            )),
            hir::ExprKind::StoreSum(s, _) => {
                Err(format!("store.sum is not yet implemented (store '{s}')"))
            }
            hir::ExprKind::StoreAvg(s, _) => {
                Err(format!("store.avg is not yet implemented (store '{s}')"))
            }
            hir::ExprKind::StoreMin(s, _) => {
                Err(format!("store.min is not yet implemented (store '{s}')"))
            }
            hir::ExprKind::StoreMax(s, _) => {
                Err(format!("store.max is not yet implemented (store '{s}')"))
            }
            hir::ExprKind::StoreVersionCount(s, _) => Err(format!(
                "store.version_count is not yet implemented (store '{s}')"
            )),
            hir::ExprKind::StoreHistory(s, _) => Err(format!(
                "store.history is not yet implemented (store '{s}')"
            )),
            hir::ExprKind::StoreAtVersion(s, _, _) => Err(format!(
                "store.at_version is not yet implemented (store '{s}')"
            )),
            hir::ExprKind::VecNew(elems) => self.compile_vec_new(elems),
            hir::ExprKind::MapNew => self.compile_map_new(),
            hir::ExprKind::VecMethod(obj, method, args) => {
                self.compile_vec_method(obj, &method.as_str(), args)
            }
            hir::ExprKind::MapMethod(obj, method, args) => {
                self.compile_map_method(obj, &method.as_str(), args)
            }
            hir::ExprKind::CoroutineCreate(name, body) => {
                self.compile_coroutine_create(&name.as_str(), body)
            }
            hir::ExprKind::CoroutineNext(coro) => self.compile_coroutine_next(coro, &expr.ty),
            hir::ExprKind::Yield(_inner) => {
                panic!("yield expression outside of coroutine body")
            }
            hir::ExprKind::IterNext(iter_var, type_name, method_name) => self
                .compile_iter_next_by_name(
                    &iter_var.as_str(),
                    &type_name.as_str(),
                    &method_name.as_str(),
                ),
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
                let fn_val = self.current_fn();
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
            hir::ExprKind::AsFormat(inner, fmt) => self.compile_as_format(inner, &fmt.as_str()),
            hir::ExprKind::AtomicLoad(ptr_expr) => self.compile_atomic_load(ptr_expr),
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
            hir::ExprKind::Slice(obj, start, end) => self.compile_slice(obj, start, end),
            hir::ExprKind::Grad(inner) => self.compile_grad(inner),
            hir::ExprKind::Einsum(notation, args) => self.compile_einsum(&notation.as_str(), args),
            hir::ExprKind::Builder(name, fields) => {
                // Desugar builder to struct init
                let inits: Vec<hir::FieldInit> = fields
                    .iter()
                    .map(|(fname, expr)| hir::FieldInit {
                        name: Some(fname.clone()),
                        value: expr.clone(),
                    })
                    .collect();
                self.compile_struct(&name.as_str(), &inits)
            }
            hir::ExprKind::GeneratorCreate(_, name, body, captures) => {
                self.compile_coroutine_create(&name.as_str(), body, captures)
            }
            hir::ExprKind::GeneratorNext(gen_expr) => {
                self.compile_coroutine_next(gen_expr, &expr.ty)
            }
            // KV / specialized store ops are lowered through MIR magic calls; never reached here.
            hir::ExprKind::KvGet(..)
            | hir::ExprKind::KvHas(..)
            | hir::ExprKind::KvCount(..)
            | hir::ExprKind::KvSet(..)
            | hir::ExprKind::KvDel(..)
            | hir::ExprKind::KvIncr(..)
            | hir::ExprKind::VecNearest(..)
            | hir::ExprKind::VecInsert(..)
            | hir::ExprKind::VecCount(..)
            | hir::ExprKind::BloomTest(..)
            | hir::ExprKind::FtsSearch(..)
            | hir::ExprKind::FtsCount(..)
            | hir::ExprKind::GraphFrom(..)
            | hir::ExprKind::GraphTo(..)
            | hir::ExprKind::TsLatest(..)
            | hir::ExprKind::EnumUnwrap(..)
            | hir::ExprKind::EnumIs(..) => Err(format!(
                "ICE: specialized store expr {:?} should have been lowered in MIR",
                expr.kind
            )),
        }
    }

    pub(crate) fn compile_short_circuit(
        &mut self,
        left: &hir::Expr,
        right: &hir::Expr,
        is_and: bool,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
        let lhs = self.compile_expr(left)?;
        let lbool = self.to_bool(lhs);
        let rhs_bb = self.ctx.append_basic_block(fv, "sc.rhs");
        let merge_bb = self.ctx.append_basic_block(fv, "sc.merge");
        let lhs_bb = self.current_bb();
        if is_and {
            b!(self.bld.build_conditional_branch(lbool, rhs_bb, merge_bb));
        } else {
            b!(self.bld.build_conditional_branch(lbool, merge_bb, rhs_bb));
        }
        self.bld.position_at_end(rhs_bb);
        let rhs = self.compile_expr(right)?;
        let rbool = self.to_bool(rhs);
        let rhs_end = self.current_bb();
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

    pub(in crate::codegen) fn compile_unary(
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

    pub(in crate::codegen) fn compile_ternary(
        &mut self,
        cond: &hir::Expr,
        then_e: &hir::Expr,
        else_e: &hir::Expr,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.current_fn();
        let tv = self.compile_expr(cond)?;
        let cv = self.to_bool(tv);
        let tbb = self.ctx.append_basic_block(fv, "t.then");
        let ebb = self.ctx.append_basic_block(fv, "t.else");
        let mbb = self.ctx.append_basic_block(fv, "t.merge");
        b!(self.bld.build_conditional_branch(cv, tbb, ebb));
        self.bld.position_at_end(tbb);
        let tv = self.compile_expr(then_e)?;
        let tbb_end = self.current_bb();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(ebb);
        let ev = self.compile_expr(else_e)?;
        let ebb_end = self.current_bb();
        b!(self.bld.build_unconditional_branch(mbb));
        self.bld.position_at_end(mbb);
        let phi = b!(self.bld.build_phi(self.llvm_ty(&then_e.ty), "tern"));
        phi.add_incoming(&[(&tv, tbb_end), (&ev, ebb_end)]);
        Ok(phi.as_basic_value())
    }

    pub(in crate::codegen) fn compile_cast(
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
                .find(|fi| fi.name.map_or(false, |n| n == fname.as_str()))
                .or_else(|| {
                    // Only use positional fallback for unnamed (positional) inits
                    inits.get(i).filter(|fi| fi.name.is_none())
                })
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
                    .expect("ICE: call returned void")
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
}
