//! MIR → LLVM IR code generation.
//!
//! This module walks a `mir::Program` in SSA form and emits LLVM IR via
//! inkwell.  It reuses the existing `Compiler` infrastructure (type mapping,
//! RC helpers, drop dispatcher, etc.) but reads MIR instructions instead of
//! HIR expressions.
//!
//! Architecture:
//!   - `value_map`:  MIR `ValueId` → LLVM `BasicValueEnum`
//!   - `block_map`:  MIR `BlockId` → LLVM `BasicBlock`
//!   - `fn_map`:     function name  → LLVM `FunctionValue`
//!
//! The overall flow per function is:
//!   1. Create the LLVM function and all basic blocks up-front.
//!   2. Wire parameters into `value_map`.
//!   3. Walk blocks in order — emit phi placeholders, instructions, terminator.
//!   4. Back-patch phi incoming edges once all blocks are materialised.

use std::collections::HashMap;

use inkwell::basic_block::BasicBlock as LLVMBlock;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, PhiValue, PointerValue};
use inkwell::AddressSpace;

use crate::hir;
use crate::mir;
use crate::perceus::PerceusHints;
use crate::types::Type;

use super::Compiler;
use super::b;

/// MIR-based code generator.  Borrows the existing `Compiler` for type mapping,
/// RC ops, drop dispatch, debug info, and LLVM module/builder management.
pub struct MirCodegen<'a, 'ctx> {
    /// The underlying HIR codegen compiler — we delegate type mapping, RC, etc.
    pub(crate) comp: &'a mut Compiler<'ctx>,
    /// MIR ValueId → LLVM value.
    value_map: HashMap<mir::ValueId, BasicValueEnum<'ctx>>,
    /// MIR BlockId → LLVM basic block  (per-function, rebuilt each time).
    block_map: HashMap<mir::BlockId, LLVMBlock<'ctx>>,
    /// Phi nodes that need back-patching after all blocks are emitted.
    pending_phis: Vec<PendingPhi<'ctx>>,
    /// MIR ValueId → variable alloca (for Store/Load variable pairs).
    var_allocs: HashMap<String, (PointerValue<'ctx>, Type)>,
    /// Coroutine/generator bodies extracted from HIR, keyed by name.
    coro_bodies: HashMap<String, Vec<hir::Stmt>>,
    /// Actor definitions from HIR, keyed by name.
    actor_defs: HashMap<String, hir::ActorDef>,
    /// Select data buffers: select_val ValueId → Vec<PointerValue> (one per arm).
    select_data_bufs: HashMap<mir::ValueId, Vec<PointerValue<'ctx>>>,
}

struct PendingPhi<'ctx> {
    phi: PhiValue<'ctx>,
    incoming: Vec<(mir::BlockId, mir::ValueId)>,
}

impl<'a, 'ctx> MirCodegen<'a, 'ctx> {
    pub fn new(comp: &'a mut Compiler<'ctx>) -> Self {
        Self {
            comp,
            value_map: HashMap::new(),
            block_map: HashMap::new(),
            pending_phis: Vec::new(),
            var_allocs: HashMap::new(),
            coro_bodies: HashMap::new(),
            actor_defs: HashMap::new(),
            select_data_bufs: HashMap::new(),
        }
    }

    // ── public entry point ─────────────────────────────────────────

    /// Compile a full MIR program into the LLVM module owned by `self.comp`.
    pub fn compile_program(
        &mut self,
        prog: &mir::Program,
        hir_prog: &hir::Program,
        hints: PerceusHints,
    ) -> Result<(), String> {
        self.comp.hints = hints;
        self.comp.setup_target()?;
        self.comp.declare_builtins();

        // Register struct types from MIR type defs.
        for td in &prog.types {
            let ltys: Vec<BasicTypeEnum<'ctx>> =
                td.fields.iter().map(|(_, ty)| self.comp.llvm_ty(ty)).collect();
            let st = self.comp.ctx.opaque_struct_type(&td.name);
            st.set_body(&ltys, false);
            let fields: Vec<(String, Type)> = td.fields.clone();
            self.comp.structs.insert(td.name.clone(), fields);
        }

        // Register HIR enum definitions (MIR doesn't carry enum info yet).
        for ed in &hir_prog.enums {
            let _ = self.comp.declare_enum(ed);
        }

        // Register extern declarations.
        for ext in &prog.externs {
            let ptys: Vec<BasicMetadataTypeEnum<'ctx>> =
                ext.params.iter().map(|t| self.comp.llvm_ty(t).into()).collect();
            let ft = self.comp.mk_fn_type(&ext.ret, &ptys, false);
            let fv = self
                .comp
                .module
                .add_function(&ext.name, ft, Some(Linkage::External));
            fv.add_attribute(
                inkwell::attributes::AttributeLoc::Function,
                self.comp.attr("nounwind"),
            );
            let param_tys: Vec<Type> = ext.params.clone();
            self.comp
                .fns
                .insert(ext.name.clone(), (fv, param_tys, ext.ret.clone()));
        }

        // ── Detect runtime needs from MIR (BEFORE declaring functions so
        //    main wrapper can find scheduler symbols) ──
        let needs_runtime = prog.functions.iter().any(|f| {
            f.blocks.iter().any(|bb| {
                bb.insts.iter().any(|i| match &i.kind {
                    mir::InstKind::SpawnActor(..)
                        | mir::InstKind::ChanCreate(..)
                        | mir::InstKind::ChanSend(..)
                        | mir::InstKind::ChanRecv(..)
                        | mir::InstKind::SelectArm(..) => true,
                    mir::InstKind::Call(name, _) => {
                        name.starts_with("__coro_create_")
                            || name.starts_with("__gen_create_")
                            || name.starts_with("__coro_next")
                            || name.starts_with("__gen_next")
                            || name.starts_with("__yield")
                            || name.starts_with("__send_")
                    }
                    _ => false,
                })
            })
        }) || !hir_prog.actors.is_empty()
           || prog.externs.iter().any(|e| e.name.starts_with("jade_"))
           || super::Compiler::uses_concurrency(hir_prog)
           || super::Compiler::uses_pool(hir_prog);
        self.comp.needs_runtime = needs_runtime;
        if needs_runtime {
            self.comp.declare_jade_runtime();
        }

        // ── Also detect coroutine/generator usage and declare gen runtime ──
        let uses_coro = prog.functions.iter().any(|f| {
            f.blocks.iter().any(|bb| {
                bb.insts.iter().any(|i| match &i.kind {
                    mir::InstKind::Call(name, _) => {
                        name.starts_with("__coro_create_")
                            || name.starts_with("__gen_create_")
                            || name.starts_with("__coro_next")
                            || name.starts_with("__gen_next")
                            || name.starts_with("__yield")
                    }
                    _ => false,
                })
            })
        });
        if uses_coro {
            self.comp.declare_actor_runtime(); // malloc, memset, free
            self.comp.declare_gen_runtime();   // jade_gen_resume/suspend/destroy
        }

        // ── Declare HIR actors (just declarations, no body compilation yet) ──
        if !hir_prog.actors.is_empty() {
            self.comp.declare_actor_runtime(); // malloc, memset, free
            for ad in &hir_prog.actors {
                self.comp.declare_actor(ad)?;
                self.actor_defs.insert(ad.name.clone(), ad.clone());
            }
        }

        // ── Process HIR stores ──
        if !hir_prog.stores.is_empty() {
            self.comp.declare_store_runtime();
            for sd in &hir_prog.stores {
                self.comp.declare_store(sd)?;
                self.comp.store_defs.insert(sd.name.clone(), sd.clone());
            }
        }

        // ── Extract coroutine/generator bodies from HIR ──
        Self::extract_coro_bodies_from_program(hir_prog, &mut self.coro_bodies);

        // ── Declare all MIR functions (forward-declare so calls resolve) ──
        // NOTE: This must be AFTER runtime declarations so main wrapper
        // can find jade_sched_init/run/shutdown.
        for func in &prog.functions {
            self.declare_mir_fn(func)?;
        }

        // ── Compile actor loop bodies (after MIR fn declarations so
        //    functions like fib are available for actor handlers) ──
        if !hir_prog.actors.is_empty() {
            for ad in &hir_prog.actors {
                self.comp.compile_actor_loop(ad)?;
            }
        }

        // ── Compile each MIR function body ──
        for func in &prog.functions {
            self.compile_mir_fn(func)?;
        }

        self.comp.finalize_debug();
        self.comp
            .module
            .verify()
            .map_err(|e| e.to_string())
    }

    // ── function declaration ───────────────────────────────────────

    fn declare_mir_fn(&mut self, func: &mir::Function) -> Result<(), String> {
        let ptys: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
        let ret = func.ret_ty.clone();

        // Build LLVM parameter types.
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.comp.llvm_ty(t).into()).collect();

        let is_main = func.name == "main";
        if is_main && !self.comp.lib_mode {
            // Create __jade_user_main + wrapper main that initialises runtime.
            let ft = self.comp.mk_fn_type(&ret, &lp, false);
            let user_fv = self
                .comp
                .module
                .add_function("__jade_user_main", ft, None);
            self.comp.tag_fn(user_fv);
            user_fv.set_linkage(Linkage::Internal);
            self.comp.fns.insert(func.name.clone(), (user_fv, ptys, ret));

            // Build main wrapper (same logic as decl.rs).
            let i32t = self.comp.ctx.i32_type();
            let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
            let main_ft = i32t.fn_type(&[i32t.into(), ptr_ty.into()], false);
            let main_fv = self.comp.module.add_function("main", main_ft, None);

            let argc_global = self.comp.module.add_global(i32t, None, "__jade_argc");
            argc_global.set_initializer(&i32t.const_int(0, false));
            let argv_global = self.comp.module.add_global(ptr_ty, None, "__jade_argv");
            argv_global.set_initializer(&ptr_ty.const_null());

            let entry = self.comp.ctx.append_basic_block(main_fv, "entry");
            self.comp.bld.position_at_end(entry);
            let argc_param = main_fv.get_nth_param(0).unwrap();
            let argv_param = main_fv.get_nth_param(1).unwrap();
            b!(self
                .comp
                .bld
                .build_store(argc_global.as_pointer_value(), argc_param));
            b!(self
                .comp
                .bld
                .build_store(argv_global.as_pointer_value(), argv_param));

            if let Some(sched_init) = self.comp.module.get_function("jade_sched_init") {
                b!(self
                    .comp
                    .bld
                    .build_call(sched_init, &[i32t.const_int(0, false).into()], ""));
            }
            let call_result = b!(self.comp.bld.build_call(user_fv, &[], "user_main"));
            if let Some(sched_run) = self.comp.module.get_function("jade_sched_run") {
                b!(self.comp.bld.build_call(sched_run, &[], ""));
            }
            if let Some(sched_shutdown) = self.comp.module.get_function("jade_sched_shutdown") {
                b!(self.comp.bld.build_call(sched_shutdown, &[], ""));
            }
            if let Some(rv) = call_result.try_as_basic_value().basic() {
                let ret_i32 = if rv.is_int_value() {
                    let iv = rv.into_int_value();
                    if iv.get_type().get_bit_width() != 32 {
                        b!(self.comp.bld.build_int_truncate(iv, i32t, "ret32"))
                    } else {
                        iv
                    }
                } else {
                    i32t.const_int(0, false)
                };
                b!(self.comp.bld.build_return(Some(&ret_i32)));
            } else {
                b!(self
                    .comp
                    .bld
                    .build_return(Some(&i32t.const_int(0, false))));
            }
        } else {
            let ft = self.comp.mk_fn_type(&ret, &lp, false);
            let fv = self.comp.module.add_function(&func.name, ft, None);
            self.comp.tag_fn(fv);
            self.comp.fns.insert(func.name.clone(), (fv, ptys, ret));
        }
        Ok(())
    }

    // ── function body compilation ──────────────────────────────────

    fn compile_mir_fn(&mut self, func: &mir::Function) -> Result<(), String> {
        let (fv, _, _) = self
            .comp
            .fns
            .get(&func.name)
            .ok_or_else(|| format!("mir_codegen: undeclared fn {}", func.name))?
            .clone();

        self.comp.cur_fn = Some(fv);
        self.value_map.clear();
        self.block_map.clear();
        self.pending_phis.clear();
        self.var_allocs.clear();
        self.comp.vars = vec![HashMap::new()];

        // 1. Create all LLVM basic blocks up-front.
        for bb in &func.blocks {
            let llvm_bb = self.comp.ctx.append_basic_block(fv, &bb.label);
            self.block_map.insert(bb.id, llvm_bb);
        }

        // 2. Wire function parameters into value_map.
        for (i, param) in func.params.iter().enumerate() {
            let llvm_val = fv.get_nth_param(i as u32).unwrap();
            self.value_map.insert(param.value, llvm_val);
        }

        // 3. Emit each basic block.
        for bb in &func.blocks {
            let llvm_bb = self.block_map[&bb.id];
            self.comp.bld.position_at_end(llvm_bb);

            // 3a. Emit phi nodes.
            for phi in &bb.phis {
                let llvm_ty = self.comp.llvm_ty(&phi.ty);
                let phi_val = b!(self.comp.bld.build_phi(llvm_ty, &format!("v{}", phi.dest.0)));
                self.value_map.insert(phi.dest, phi_val.as_basic_value());
                self.pending_phis.push(PendingPhi {
                    phi: phi_val,
                    incoming: phi.incoming.clone(),
                });
            }

            // 3b. Emit instructions.
            for inst in &bb.insts {
                let val = self.emit_inst(inst)?;
                if let Some(dest) = inst.dest {
                    self.value_map.insert(dest, val);
                }
            }

            // 3c. Emit terminator.
            self.emit_terminator(&bb.terminator, &func.ret_ty)?;
        }

        // 4. Back-patch phi incoming edges.
        for pp in &self.pending_phis {
            let incoming: Vec<(BasicValueEnum<'ctx>, LLVMBlock<'ctx>)> = pp
                .incoming
                .iter()
                .filter_map(|(block_id, val_id)| {
                    let llvm_bb = self.block_map.get(block_id)?;
                    let llvm_val = self.value_map.get(val_id)?;
                    Some((*llvm_val, *llvm_bb))
                })
                .collect();
            let refs: Vec<(&dyn BasicValue<'ctx>, LLVMBlock<'ctx>)> = incoming
                .iter()
                .map(|(v, bb)| (v as &dyn BasicValue<'ctx>, *bb))
                .collect();
            for (val, bb) in &refs {
                pp.phi.add_incoming(&[(*val, *bb)]);
            }
        }

        Ok(())
    }

    // ── instruction emission ───────────────────────────────────────

    fn emit_inst(&mut self, inst: &mir::Instruction) -> Result<BasicValueEnum<'ctx>, String> {
        let void_val = || -> BasicValueEnum<'ctx> {
            self.comp.ctx.i8_type().const_int(0, false).into()
        };

        match &inst.kind {
            // ── Constants ──
            mir::InstKind::IntConst(n) => {
                let llvm_ty = self.comp.llvm_ty(&inst.ty);
                Ok(match &inst.ty {
                    Type::F32 => self.comp.ctx.f32_type().const_float(*n as f64).into(),
                    Type::F64 => self.comp.ctx.f64_type().const_float(*n as f64).into(),
                    _ => llvm_ty
                        .into_int_type()
                        .const_int(*n as u64, true)
                        .into(),
                })
            }
            mir::InstKind::FloatConst(f) => {
                Ok(match &inst.ty {
                    Type::F32 => self.comp.ctx.f32_type().const_float(*f).into(),
                    _ => self.comp.ctx.f64_type().const_float(*f).into(),
                })
            }
            mir::InstKind::BoolConst(b) => {
                Ok(self
                    .comp
                    .ctx
                    .bool_type()
                    .const_int(*b as u64, false)
                    .into())
            }
            mir::InstKind::StringConst(s) => {
                self.emit_string_const(s)
            }
            mir::InstKind::Void => Ok(void_val()),

            // ── Arithmetic ──
            mir::InstKind::BinOp(op, lhs, rhs) => {
                self.emit_binop(*op, *lhs, *rhs, &inst.ty)
            }
            mir::InstKind::UnaryOp(op, val) => {
                self.emit_unary(*op, *val, &inst.ty)
            }
            mir::InstKind::Cmp(op, lhs, rhs) => {
                self.emit_cmp(*op, *lhs, *rhs, &inst.ty)
            }

            // ── Calls ──
            mir::InstKind::Call(name, args) => {
                // Check for magic call names first (coroutines, actors, stores)
                if let Some(result) = self.try_handle_magic_call(name, args, &inst.ty)? {
                    return Ok(result);
                }
                let arg_vals: Vec<BasicValueEnum<'ctx>> = args
                    .iter()
                    .map(|a| self.val(*a))
                    .collect();
                if let Some((fv, _, _)) = self.comp.fns.get(name).cloned() {
                    let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        arg_vals.iter().map(|v| (*v).into()).collect();
                    let csv = b!(self.comp.bld.build_call(fv, &md, "call"));
                    Ok(self.comp.call_result(csv))
                } else {
                    // Try looking up as a module-level function.
                    if let Some(fv) = self.comp.module.get_function(name) {
                        let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                            arg_vals.iter().map(|v| (*v).into()).collect();
                        let csv = b!(self.comp.bld.build_call(fv, &md, "call"));
                        Ok(self.comp.call_result(csv))
                    } else {
                        Err(format!("mir_codegen: unknown function `{name}`"))
                    }
                }
            }
            mir::InstKind::MethodCall(recv, method, args) => {
                let recv_val = self.val(*recv);
                let mut all_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    vec![recv_val.into()];
                for a in args {
                    all_args.push(self.val(*a).into());
                }
                if let Some((fv, _, _)) = self.comp.fns.get(method).cloned() {
                    let csv = b!(self.comp.bld.build_call(fv, &all_args, "mcall"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err(format!("mir_codegen: unknown method `{method}`"))
                }
            }
            mir::InstKind::IndirectCall(callee, args) => {
                let callee_val = self.val(*callee);
                // Closure call: callee is a {fn_ptr, env_ptr} struct.
                let closure_ty = self.comp.closure_type();
                let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                let fn_ptr = b!(self.comp.bld.build_extract_value(
                    callee_val.into_struct_value(),
                    0,
                    "fn_ptr"
                ))
                .into_pointer_value();
                let env_ptr = b!(self.comp.bld.build_extract_value(
                    callee_val.into_struct_value(),
                    1,
                    "env_ptr"
                ));
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    vec![env_ptr.into()];
                for a in args {
                    call_args.push(self.val(*a).into());
                }
                // Build function type for the indirect call.
                let ret_llvm = self.comp.llvm_ty(&inst.ty);
                let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                for a in args {
                    param_tys.push(self.val(*a).get_type().into());
                }
                let ft = ret_llvm.fn_type(&param_tys, false);
                let csv = b!(self
                    .comp
                    .bld
                    .build_indirect_call(ft, fn_ptr, &call_args, "icall"));
                Ok(self.comp.call_result(csv))
            }

            // ── Variables ──
            mir::InstKind::FnRef(name) => {
                // Create a closure struct {fn_ptr, null_env} wrapping the named function.
                // A wrapper is needed because closures expect (env_ptr, ...params) calling convention,
                // but top-level functions only expect (...params).
                if let Some(fv) = self.comp.module.get_function(name) {
                    let wrapper = self.comp.fn_ref_wrapper(fv);
                    let null_env = self.comp.ctx.ptr_type(inkwell::AddressSpace::default()).const_null();
                    self.comp.make_closure(wrapper, null_env)
                } else {
                    Err(format!("mir_codegen: undefined function `{name}` in FnRef"))
                }
            }
            mir::InstKind::Load(name) => {
                if let Some((ptr, ty)) = self.var_allocs.get(name).cloned() {
                    let lt = self.comp.llvm_ty(&ty);
                    Ok(b!(self.comp.bld.build_load(lt, ptr, name)))
                } else {
                    // Fall back to Compiler's var lookup.
                    if let Some((ptr, ty)) = self.comp.find_var(name).cloned() {
                        let lt = self.comp.llvm_ty(&ty);
                        Ok(b!(self.comp.bld.build_load(lt, ptr, name)))
                    } else {
                        // Try as global constant or function.
                        Ok(void_val())
                    }
                }
            }
            mir::InstKind::Store(name, val) => {
                let v = self.val(*val);
                if let Some((ptr, _)) = self.var_allocs.get(name) {
                    b!(self.comp.bld.build_store(*ptr, v));
                } else {
                    // First store → create alloca.
                    let lt = v.get_type();
                    let ty = inst.ty.clone();
                    let ptr = self.comp.entry_alloca(lt, name);
                    b!(self.comp.bld.build_store(ptr, v));
                    self.var_allocs.insert(name.clone(), (ptr, ty.clone()));
                    self.comp.set_var(name, ptr, ty);
                }
                Ok(void_val())
            }

            // ── Struct/Aggregate ──
            mir::InstKind::StructInit(name, fields) => {
                let st = self.comp.module.get_struct_type(name)
                    .ok_or_else(|| format!("mir_codegen: unknown struct `{name}`"))?;
                let field_order: Vec<String> = self
                    .comp
                    .structs
                    .get(name)
                    .map(|fs| fs.iter().map(|(n, _)| n.clone()).collect())
                    .unwrap_or_default();
                let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                for (fname, vid) in fields {
                    let v = self.val(*vid);
                    let idx = field_order
                        .iter()
                        .position(|n| n == fname)
                        .unwrap_or(0) as u32;
                    agg = b!(self.comp.bld.build_insert_value(
                        agg.into_struct_value(),
                        v,
                        idx,
                        fname
                    ))
                    .into_struct_value()
                    .into();
                }
                Ok(agg)
            }
            mir::InstKind::VariantInit(enum_name, variant, tag, payload) => {
                let enum_ty = self.comp.llvm_ty(&Type::Enum(enum_name.clone()));
                let st = enum_ty.into_struct_type();
                let i32t = self.comp.ctx.i32_type();
                let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                // Field 0 = tag.
                agg = b!(self.comp.bld.build_insert_value(
                    agg.into_struct_value(),
                    i32t.const_int(*tag as u64, false),
                    0,
                    "tag"
                ))
                .into_struct_value()
                .into();
                // Payload into field 1 (stored as a byte array, need to bitcast via alloca).
                if !payload.is_empty() {
                    let alloca = self.comp.entry_alloca(enum_ty, "variant.tmp");
                    b!(self.comp.bld.build_store(alloca, agg));
                    let payload_gep = b!(self.comp.bld.build_struct_gep(
                        st, alloca, 1, "payload"
                    ));
                    // Store payload fields sequentially.
                    for (i, vid) in payload.iter().enumerate() {
                        let v = self.val(*vid);
                        let vty = v.get_type();
                        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                        // Calculate offset within the payload area.
                        if i == 0 {
                            b!(self.comp.bld.build_store(payload_gep, v));
                        } else {
                            let byte_offset = self.comp.ctx.i64_type().const_int(
                                (i * 8) as u64, false
                            );
                            let elem_ptr = unsafe {
                                b!(self.comp.bld.build_gep(
                                    self.comp.ctx.i8_type(),
                                    payload_gep,
                                    &[byte_offset],
                                    "payload.elem"
                                ))
                            };
                            b!(self.comp.bld.build_store(elem_ptr, v));
                        }
                    }
                    agg = b!(self.comp.bld.build_load(enum_ty, alloca, "variant.loaded"));
                }
                Ok(agg)
            }
            mir::InstKind::ArrayInit(elems) => {
                if elems.is_empty() {
                    let arr_ty = self.comp.llvm_ty(&inst.ty);
                    return Ok(arr_ty.const_zero());
                }
                let elem_vals: Vec<BasicValueEnum<'ctx>> =
                    elems.iter().map(|v| self.val(*v)).collect();
                let elem_ty = elem_vals[0].get_type();
                let arr_ty = elem_ty.array_type(elems.len() as u32);
                let alloca = self.comp.entry_alloca(arr_ty.into(), "arr");
                for (i, v) in elem_vals.iter().enumerate() {
                    let idx = self.comp.ctx.i64_type().const_int(i as u64, false);
                    let zero = self.comp.ctx.i64_type().const_int(0, false);
                    let ptr = unsafe {
                        b!(self.comp.bld.build_gep(
                            arr_ty,
                            alloca,
                            &[zero, idx],
                            "arr.elem"
                        ))
                    };
                    b!(self.comp.bld.build_store(ptr, *v));
                }
                Ok(b!(self.comp.bld.build_load(arr_ty, alloca, "arr.val")).into())
            }

            // ── Field access ──
            mir::InstKind::FieldGet(obj, field) => {
                self.emit_field_get(*obj, field, &inst.ty)
            }
            mir::InstKind::FieldSet(obj, field, val) => {
                let obj_val = self.val(*obj);
                let v = self.val(*val);
                if obj_val.is_pointer_value() {
                    // obj is a pointer to a struct (alloca).
                    // inst.ty carries the struct type from lowering.
                    let struct_name = self.struct_name_from_type(&inst.ty)
                        .or_else(|| {
                            // Also try var_allocs for the struct name.
                            self.var_allocs.values()
                                .find(|(ptr, _)| *ptr == obj_val.into_pointer_value())
                                .and_then(|(_, ty)| match ty {
                                    Type::Struct(name, _) => Some(name.clone()),
                                    _ => None,
                                })
                        });
                    if let Some(name) = &struct_name {
                        if let Some(st) = self.comp.module.get_struct_type(name) {
                            let field_idx = self.field_index(name, field);
                            let gep = b!(self.comp.bld.build_struct_gep(
                                st, obj_val.into_pointer_value(), field_idx, field
                            ));
                            b!(self.comp.bld.build_store(gep, v));
                        }
                    }
                }
                Ok(void_val())
            }

            // ── Indexing ──
            mir::InstKind::Index(base, idx) => {
                let base_val = self.val(*base);
                let idx_val = self.val(*idx);
                // For arrays: GEP into the array.
                if base_val.get_type().is_array_type() {
                    let arr_ty = base_val.get_type().into_array_type();
                    let alloca = self.comp.entry_alloca(arr_ty.into(), "idx.tmp");
                    b!(self.comp.bld.build_store(alloca, base_val));
                    let zero = self.comp.ctx.i64_type().const_int(0, false);
                    let ptr = unsafe {
                        b!(self.comp.bld.build_gep(
                            arr_ty,
                            alloca,
                            &[zero, idx_val.into_int_value()],
                            "idx.ptr"
                        ))
                    };
                    let elem_ty = self.comp.llvm_ty(&inst.ty);
                    Ok(b!(self.comp.bld.build_load(elem_ty, ptr, "idx.val")))
                } else {
                    // Vec/pointer: runtime call.
                    Ok(void_val())
                }
            }
            mir::InstKind::IndexSet(base, idx, val) => {
                let base_val = self.val(*base);
                let idx_val = self.val(*idx);
                let v = self.val(*val);
                if base_val.get_type().is_array_type() {
                    let arr_ty = base_val.get_type().into_array_type();
                    let alloca = self.comp.entry_alloca(arr_ty.into(), "idxset.tmp");
                    b!(self.comp.bld.build_store(alloca, base_val));
                    let zero = self.comp.ctx.i64_type().const_int(0, false);
                    let ptr = unsafe {
                        b!(self.comp.bld.build_gep(
                            arr_ty,
                            alloca,
                            &[zero, idx_val.into_int_value()],
                            "idxset.ptr"
                        ))
                    };
                    b!(self.comp.bld.build_store(ptr, v));
                }
                Ok(void_val())
            }

            // ── Cast / Ref / Deref ──
            mir::InstKind::Cast(val, target_ty) => {
                let v = self.val(*val);
                let target_llvm = self.comp.llvm_ty(target_ty);
                self.emit_cast(v, &inst.ty, target_ty, target_llvm)
            }
            mir::InstKind::Ref(val) => {
                let v = self.val(*val);
                let alloca = self.comp.entry_alloca(v.get_type(), "ref");
                b!(self.comp.bld.build_store(alloca, v));
                Ok(alloca.into())
            }
            mir::InstKind::Deref(val) => {
                let v = self.val(*val);
                let inner_ty = self.comp.llvm_ty(&inst.ty);
                Ok(b!(self.comp.bld.build_load(
                    inner_ty,
                    v.into_pointer_value(),
                    "deref"
                )))
            }

            // ── Memory / RC ──
            mir::InstKind::Alloc(val) => {
                let v = self.val(*val);
                let malloc = self.comp.ensure_malloc();
                let size = v.get_type().size_of().unwrap_or(
                    self.comp.ctx.i64_type().const_int(8, false),
                );
                let ptr = b!(self
                    .comp
                    .bld
                    .build_call(malloc, &[size.into()], "alloc"))
                .try_as_basic_value()
                .basic()
                .unwrap();
                b!(self
                    .comp
                    .bld
                    .build_store(ptr.into_pointer_value(), v));
                Ok(ptr)
            }
            mir::InstKind::Drop(val, ty) => {
                let v = self.val(*val);
                self.comp.drop_value(v, ty)?;
                Ok(void_val())
            }
            mir::InstKind::RcInc(val) => {
                let v = self.val(*val);
                if let Type::Rc(inner) = &inst.ty {
                    self.comp.rc_retain(v, inner)?;
                }
                Ok(void_val())
            }
            mir::InstKind::RcDec(val) => {
                let v = self.val(*val);
                if let Type::Rc(inner) = &inst.ty {
                    self.comp.rc_release(v, inner)?;
                }
                Ok(void_val())
            }
            mir::InstKind::RcNew(val, inner_ty) => {
                let v = self.val(*val);
                self.comp.rc_alloc(inner_ty, v)
            }
            mir::InstKind::RcClone(val) => {
                let v = self.val(*val);
                if let Type::Rc(inner) = &inst.ty {
                    self.comp.rc_retain(v, inner)?;
                }
                Ok(v)
            }
            mir::InstKind::WeakUpgrade(val) => {
                let v = self.val(*val);
                if let Type::Weak(inner) | Type::Rc(inner) = &inst.ty {
                    self.comp.weak_upgrade(v, inner)
                } else {
                    Ok(v)
                }
            }

            // ── Copy ──
            mir::InstKind::Copy(val) => Ok(self.val(*val)),

            // ── Slice ──
            mir::InstKind::Slice(base, lo, hi) => {
                // For now, emit as a void — full slice codegen requires
                // runtime support matching the HIR path.
                let _ = (base, lo, hi);
                Ok(void_val())
            }

            // ── Collections ──
            mir::InstKind::VecNew(elems) => {
                self.emit_vec_new(elems, &inst.ty)
            }
            mir::InstKind::VecPush(vec, elem) => {
                let vec_val = self.val(*vec);
                let elem_val = self.val(*elem);
                if let Some(push_fn) = self.comp.module.get_function("jade_vec_push") {
                    let alloca = self.comp.entry_alloca(elem_val.get_type(), "vpush.tmp");
                    b!(self.comp.bld.build_store(alloca, elem_val));
                    b!(self.comp.bld.build_call(
                        push_fn,
                        &[vec_val.into(), alloca.into()],
                        ""
                    ));
                }
                Ok(void_val())
            }
            mir::InstKind::VecLen(vec) => {
                let vec_val = self.val(*vec);
                if let Some(len_fn) = self.comp.module.get_function("jade_vec_len") {
                    let csv = b!(self.comp.bld.build_call(
                        len_fn,
                        &[vec_val.into()],
                        "veclen"
                    ));
                    Ok(self.comp.call_result(csv))
                } else {
                    Ok(self.comp.ctx.i64_type().const_int(0, false).into())
                }
            }
            mir::InstKind::MapInit => {
                // Delegate to runtime jade_map_new (if runtime declared).
                if let Some(fv) = self.comp.module.get_function("jade_map_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "map"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Ok(self
                        .comp
                        .ctx
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into())
                }
            }
            mir::InstKind::SetInit => {
                if let Some(fv) = self.comp.module.get_function("jade_set_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "set"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Ok(self
                        .comp
                        .ctx
                        .ptr_type(AddressSpace::default())
                        .const_null()
                        .into())
                }
            }

            // ── Closures ──
            mir::InstKind::ClosureCreate(fn_name, captures) => {
                self.emit_closure_create(fn_name, captures, &inst.ty)
            }
            mir::InstKind::ClosureCall(callee, args) => {
                // Same as IndirectCall for closures.
                let callee_val = self.val(*callee);
                let closure_st = callee_val.into_struct_value();
                let fn_ptr = b!(self
                    .comp
                    .bld
                    .build_extract_value(closure_st, 0, "fn_ptr"))
                .into_pointer_value();
                let env_ptr = b!(self
                    .comp
                    .bld
                    .build_extract_value(closure_st, 1, "env_ptr"));
                let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    vec![env_ptr.into()];
                let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                for a in args {
                    let v = self.val(*a);
                    call_args.push(v.into());
                    param_tys.push(v.get_type().into());
                }
                let ret_llvm = self.comp.llvm_ty(&inst.ty);
                let ft = ret_llvm.fn_type(&param_tys, false);
                let csv = b!(self
                    .comp
                    .bld
                    .build_indirect_call(ft, fn_ptr, &call_args, "closure.call"));
                Ok(self.comp.call_result(csv))
            }

            // ── Actors / Channels ──
            mir::InstKind::SpawnActor(name, args) => {
                let _ = args;
                self.emit_spawn_actor(name)
            }
            mir::InstKind::ChanCreate(elem_ty) => {
                self.emit_chan_create(elem_ty)
            }
            mir::InstKind::ChanSend(ch, val) => {
                self.emit_chan_send(*ch, *val)
            }
            mir::InstKind::ChanRecv(ch) => {
                self.emit_chan_recv(*ch, &inst.ty)
            }
            mir::InstKind::SelectArm(channels) => {
                // Select: build case array, call jade_select, return index.
                let ch_vids: Vec<mir::ValueId> = channels.clone();
                let dest = inst.dest.unwrap();
                self.emit_select(&ch_vids, dest)
            }

            // ── Builtins ──
            mir::InstKind::Log(val) => {
                let v = self.val(*val);
                self.comp.emit_log(v, &inst.ty)?;
                Ok(void_val())
            }
            mir::InstKind::Assert(val, msg) => {
                let v = self.val(*val);
                let fv = self.comp.cur_fn.unwrap();
                let cond = v.into_int_value();
                let pass_bb = self.comp.ctx.append_basic_block(fv, "assert.pass");
                let fail_bb = self.comp.ctx.append_basic_block(fv, "assert.fail");
                b!(self
                    .comp
                    .bld
                    .build_conditional_branch(cond, pass_bb, fail_bb));
                self.comp.bld.position_at_end(fail_bb);
                // Print assertion message and abort.
                if let Some(printf) = self.comp.module.get_function("printf") {
                    let fmt_str = format!("assertion failed: {msg}\n\0");
                    let gv = self
                        .comp
                        .bld
                        .build_global_string_ptr(&fmt_str, "assert.msg")
                        .map_err(|e| e.to_string())?;
                    b!(self.comp.bld.build_call(
                        printf,
                        &[gv.as_pointer_value().into()],
                        ""
                    ));
                }
                // Call abort.
                let abort = self
                    .comp
                    .module
                    .get_function("abort")
                    .unwrap_or_else(|| {
                        let ft = self.comp.ctx.void_type().fn_type(&[], false);
                        self.comp
                            .module
                            .add_function("abort", ft, Some(Linkage::External))
                    });
                b!(self.comp.bld.build_call(abort, &[], ""));
                b!(self.comp.bld.build_unreachable());
                self.comp.bld.position_at_end(pass_bb);
                Ok(void_val())
            }

            // ── Dynamic dispatch ──
            mir::InstKind::DynDispatch(obj, trait_name, method, args) => {
                let _ = (obj, trait_name, method, args);
                // Requires vtable lookup — stubbed for now.
                Ok(void_val())
            }
        }
    }

    // ── terminator emission ────────────────────────────────────────

    fn emit_terminator(
        &mut self,
        term: &mir::Terminator,
        ret_ty: &Type,
    ) -> Result<(), String> {
        match term {
            mir::Terminator::Goto(target) => {
                let bb = self.block_map[target];
                b!(self.comp.bld.build_unconditional_branch(bb));
            }
            mir::Terminator::Branch(cond, then_bb, else_bb) => {
                let cond_val = self.val(*cond).into_int_value();
                // Ensure condition is i1 — coerce wider integers with != 0.
                let cond_i1 = if cond_val.get_type().get_bit_width() != 1 {
                    b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::NE,
                        cond_val,
                        cond_val.get_type().const_zero(),
                        "tobool",
                    ))
                } else {
                    cond_val
                };
                let t = self.block_map[then_bb];
                let e = self.block_map[else_bb];
                b!(self.comp.bld.build_conditional_branch(cond_i1, t, e));
            }
            mir::Terminator::Return(val) => {
                if let Some(vid) = val {
                    let v = self.val(*vid);
                    let expected = self.comp.llvm_ty(ret_ty);
                    if v.get_type() == expected {
                        b!(self.comp.bld.build_return(Some(&v)));
                    } else {
                        // Type mismatch (e.g. void-valued last expr in non-void fn).
                        let default = self.comp.default_val(ret_ty);
                        b!(self.comp.bld.build_return(Some(&default)));
                    }
                } else if matches!(ret_ty, Type::Void) {
                    b!(self.comp.bld.build_return(None));
                } else {
                    let default = self.comp.default_val(ret_ty);
                    b!(self.comp.bld.build_return(Some(&default)));
                }
            }
            mir::Terminator::Switch(disc, cases, default) => {
                let disc_val = self.val(*disc).into_int_value();
                let default_bb = self.block_map[default];
                let case_bbs: Vec<(inkwell::values::IntValue<'ctx>, LLVMBlock<'ctx>)> = cases
                    .iter()
                    .map(|(val, bid)| {
                        let iv = disc_val
                            .get_type()
                            .const_int(*val as u64, true);
                        (iv, self.block_map[bid])
                    })
                    .collect();
                let switch = b!(self.comp.bld.build_switch(
                    disc_val,
                    default_bb,
                    &case_bbs
                ));
            }
            mir::Terminator::Unreachable => {
                b!(self.comp.bld.build_unreachable());
            }
        }
        Ok(())
    }

    // ── helpers ────────────────────────────────────────────────────

    /// Look up an MIR ValueId in the value map.
    fn val(&self, id: mir::ValueId) -> BasicValueEnum<'ctx> {
        self.value_map
            .get(&id)
            .copied()
            .unwrap_or_else(|| self.comp.ctx.i64_type().const_int(0, false).into())
    }

    fn emit_string_const(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let string_ty = self.comp.string_type();
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let i64t = self.comp.ctx.i64_type();
        let len = s.len() as u64;

        // SSO layout: {ptr, len, cap}.  For constants, allocate a global
        // and point the ptr field at it.
        let gv = self
            .comp
            .bld
            .build_global_string_ptr(s, "str.data")
            .map_err(|e| e.to_string())?;
        let mut agg: BasicValueEnum<'ctx> = string_ty.const_zero().into();
        agg = b!(self.comp.bld.build_insert_value(
            agg.into_struct_value(),
            gv.as_pointer_value(),
            0,
            "str.ptr"
        ))
        .into_struct_value()
        .into();
        agg = b!(self.comp.bld.build_insert_value(
            agg.into_struct_value(),
            i64t.const_int(len, false),
            1,
            "str.len"
        ))
        .into_struct_value()
        .into();
        agg = b!(self.comp.bld.build_insert_value(
            agg.into_struct_value(),
            i64t.const_int(len, false),
            2,
            "str.cap"
        ))
        .into_struct_value()
        .into();
        Ok(agg)
    }

    fn emit_binop(
        &mut self,
        op: mir::BinOp,
        lhs: mir::ValueId,
        rhs: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let l = self.val(lhs);
        let r = self.val(rhs);

        if result_ty.is_float() {
            let lf = l.into_float_value();
            let rf = r.into_float_value();
            let res = match op {
                mir::BinOp::Add => b!(self.comp.bld.build_float_add(lf, rf, "fadd")),
                mir::BinOp::Sub => b!(self.comp.bld.build_float_sub(lf, rf, "fsub")),
                mir::BinOp::Mul => b!(self.comp.bld.build_float_mul(lf, rf, "fmul")),
                mir::BinOp::Div => b!(self.comp.bld.build_float_div(lf, rf, "fdiv")),
                mir::BinOp::Mod => b!(self.comp.bld.build_float_rem(lf, rf, "fmod")),
                _ => return Err(format!("mir_codegen: unsupported float binop {op:?}")),
            };
            Ok(res.into())
        } else {
            let li = l.into_int_value();
            let ri = r.into_int_value();
            let res = match op {
                mir::BinOp::Add => b!(self.comp.bld.build_int_add(li, ri, "add")),
                mir::BinOp::Sub => b!(self.comp.bld.build_int_sub(li, ri, "sub")),
                mir::BinOp::Mul => b!(self.comp.bld.build_int_mul(li, ri, "mul")),
                mir::BinOp::Div => {
                    if result_ty.is_signed() {
                        b!(self.comp.bld.build_int_signed_div(li, ri, "sdiv"))
                    } else {
                        b!(self.comp.bld.build_int_unsigned_div(li, ri, "udiv"))
                    }
                }
                mir::BinOp::Mod => {
                    if result_ty.is_signed() {
                        b!(self.comp.bld.build_int_signed_rem(li, ri, "srem"))
                    } else {
                        b!(self.comp.bld.build_int_unsigned_rem(li, ri, "urem"))
                    }
                }
                mir::BinOp::BitAnd => b!(self.comp.bld.build_and(li, ri, "and")),
                mir::BinOp::BitOr => b!(self.comp.bld.build_or(li, ri, "or")),
                mir::BinOp::BitXor => b!(self.comp.bld.build_xor(li, ri, "xor")),
                mir::BinOp::Shl => b!(self.comp.bld.build_left_shift(li, ri, "shl")),
                mir::BinOp::Shr => b!(self.comp.bld.build_right_shift(li, ri, result_ty.is_signed(), "shr")),
                mir::BinOp::And => b!(self.comp.bld.build_and(li, ri, "land")),
                mir::BinOp::Or => b!(self.comp.bld.build_or(li, ri, "lor")),
                mir::BinOp::Exp => {
                    // Exponentiation: use llvm.powi intrinsic or loop.
                    // For now, cast to float, call pow, cast back.
                    let f64t = self.comp.ctx.f64_type();
                    let lf = b!(self.comp.bld.build_signed_int_to_float(li, f64t, "exp.l"));
                    let rf = b!(self.comp.bld.build_signed_int_to_float(ri, f64t, "exp.r"));
                    let pow = self
                        .comp
                        .module
                        .get_function("pow")
                        .unwrap_or_else(|| {
                            let ft = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                            self.comp
                                .module
                                .add_function("pow", ft, Some(Linkage::External))
                        });
                    let result = b!(self.comp.bld.build_call(pow, &[lf.into(), rf.into()], "pow"));
                    let fv = self.comp.call_result(result).into_float_value();
                    let iv = b!(self
                        .comp
                        .bld
                        .build_float_to_signed_int(fv, li.get_type(), "exp.i"));
                    return Ok(iv.into());
                }
            };
            Ok(res.into())
        }
    }

    fn emit_unary(
        &mut self,
        op: mir::UnaryOp,
        val: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let v = self.val(val);
        match op {
            mir::UnaryOp::Neg => {
                if result_ty.is_float() {
                    Ok(b!(self.comp.bld.build_float_neg(v.into_float_value(), "fneg")).into())
                } else {
                    let zero = v.into_int_value().get_type().const_int(0, false);
                    Ok(b!(self
                        .comp
                        .bld
                        .build_int_sub(zero, v.into_int_value(), "neg"))
                    .into())
                }
            }
            mir::UnaryOp::Not => {
                Ok(b!(self.comp.bld.build_not(v.into_int_value(), "not")).into())
            }
            mir::UnaryOp::BitNot => {
                Ok(b!(self.comp.bld.build_not(v.into_int_value(), "bitnot")).into())
            }
        }
    }

    fn emit_cmp(
        &mut self,
        op: mir::CmpOp,
        lhs: mir::ValueId,
        rhs: mir::ValueId,
        operand_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let l = self.val(lhs);
        let r = self.val(rhs);

        if operand_ty.is_float() {
            let pred = match op {
                mir::CmpOp::Eq => inkwell::FloatPredicate::OEQ,
                mir::CmpOp::Ne => inkwell::FloatPredicate::ONE,
                mir::CmpOp::Lt => inkwell::FloatPredicate::OLT,
                mir::CmpOp::Gt => inkwell::FloatPredicate::OGT,
                mir::CmpOp::Le => inkwell::FloatPredicate::OLE,
                mir::CmpOp::Ge => inkwell::FloatPredicate::OGE,
            };
            Ok(b!(self.comp.bld.build_float_compare(
                pred,
                l.into_float_value(),
                r.into_float_value(),
                "fcmp"
            ))
            .into())
        } else {
            let pred = if operand_ty.is_signed() {
                match op {
                    mir::CmpOp::Eq => inkwell::IntPredicate::EQ,
                    mir::CmpOp::Ne => inkwell::IntPredicate::NE,
                    mir::CmpOp::Lt => inkwell::IntPredicate::SLT,
                    mir::CmpOp::Gt => inkwell::IntPredicate::SGT,
                    mir::CmpOp::Le => inkwell::IntPredicate::SLE,
                    mir::CmpOp::Ge => inkwell::IntPredicate::SGE,
                }
            } else {
                match op {
                    mir::CmpOp::Eq => inkwell::IntPredicate::EQ,
                    mir::CmpOp::Ne => inkwell::IntPredicate::NE,
                    mir::CmpOp::Lt => inkwell::IntPredicate::ULT,
                    mir::CmpOp::Gt => inkwell::IntPredicate::UGT,
                    mir::CmpOp::Le => inkwell::IntPredicate::ULE,
                    mir::CmpOp::Ge => inkwell::IntPredicate::UGE,
                }
            };
            Ok(b!(self.comp.bld.build_int_compare(
                pred,
                l.into_int_value(),
                r.into_int_value(),
                "icmp"
            ))
            .into())
        }
    }

    fn emit_cast(
        &mut self,
        val: BasicValueEnum<'ctx>,
        _src_ty: &Type,
        target_ty: &Type,
        target_llvm: BasicTypeEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if val.get_type() == target_llvm {
            return Ok(val);
        }
        // Int → Float.
        if val.is_int_value() && target_ty.is_float() {
            return if !_src_ty.is_signed() {
                Ok(b!(self.comp.bld.build_unsigned_int_to_float(
                    val.into_int_value(),
                    target_llvm.into_float_type(),
                    "u2f"
                ))
                .into())
            } else {
                Ok(b!(self.comp.bld.build_signed_int_to_float(
                    val.into_int_value(),
                    target_llvm.into_float_type(),
                    "i2f"
                ))
                .into())
            };
        }
        // Float → Int.
        if val.is_float_value() && target_ty.is_int() {
            return if !target_ty.is_signed() {
                Ok(b!(self.comp.bld.build_float_to_unsigned_int(
                    val.into_float_value(),
                    target_llvm.into_int_type(),
                    "f2u"
                ))
                .into())
            } else {
                Ok(b!(self.comp.bld.build_float_to_signed_int(
                    val.into_float_value(),
                    target_llvm.into_int_type(),
                    "f2i"
                ))
                .into())
            };
        }
        // Int → Int (widen/truncate).
        if val.is_int_value() && target_llvm.is_int_type() {
            let src_bits = val.into_int_value().get_type().get_bit_width();
            let dst_bits = target_llvm.into_int_type().get_bit_width();
            return if dst_bits > src_bits {
                if !_src_ty.is_signed() {
                    Ok(b!(self.comp.bld.build_int_z_extend(
                        val.into_int_value(),
                        target_llvm.into_int_type(),
                        "zext"
                    ))
                    .into())
                } else {
                    Ok(b!(self.comp.bld.build_int_s_extend(
                        val.into_int_value(),
                        target_llvm.into_int_type(),
                        "sext"
                    ))
                    .into())
                }
            } else if dst_bits < src_bits {
                Ok(b!(self.comp.bld.build_int_truncate(
                    val.into_int_value(),
                    target_llvm.into_int_type(),
                    "trunc"
                ))
                .into())
            } else {
                Ok(val)
            };
        }
        // Float → Float.
        if val.is_float_value() && target_llvm.is_float_type() {
            return Ok(b!(self.comp.bld.build_float_cast(
                val.into_float_value(),
                target_llvm.into_float_type(),
                "fcast"
            ))
            .into());
        }
        // Pointer cast.
        if val.is_pointer_value() && target_llvm.is_pointer_type() {
            return Ok(val);
        }
        // Fallback: bitcast via alloca.
        let alloca = self.comp.entry_alloca(val.get_type(), "cast.tmp");
        b!(self.comp.bld.build_store(alloca, val));
        Ok(b!(self.comp.bld.build_load(target_llvm, alloca, "cast")))
    }

    fn emit_field_get(
        &mut self,
        obj: mir::ValueId,
        field: &str,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.val(obj);
        if obj_val.is_struct_value() {
            // Inline struct: extract_value.
            let sv = obj_val.into_struct_value();
            // Look up field index via struct name in types metadata.
            let struct_ty_name = sv
                .get_type()
                .get_name()
                .map(|n| n.to_str().unwrap_or("").to_string());
            if let Some(name) = &struct_ty_name {
                // Check if this is an enum type — enum payloads need special extraction.
                if self.comp.enums.contains_key(name) {
                    if field == "__tag" {
                        // Tag is at struct index 0 (i32).
                        let val = b!(self.comp.bld.build_extract_value(sv, 0, "tag"));
                        return Ok(val);
                    }
                    // Payload fields: _0, _1, ... — extract from the payload byte array.
                    if let Some(idx_str) = field.strip_prefix('_') {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            let st = sv.get_type();
                            let alloca = self.comp.entry_alloca(st.into(), "enum.tmp");
                            b!(self.comp.bld.build_store(alloca, sv));
                            let payload_gep = b!(self.comp.bld.build_struct_gep(
                                st, alloca, 1, "payload"
                            ));
                            let res_llvm = self.comp.llvm_ty(result_ty);
                            let field_ptr = if idx == 0 {
                                payload_gep
                            } else {
                                let byte_offset = self.comp.ctx.i64_type().const_int(
                                    (idx * 8) as u64, false
                                );
                                unsafe {
                                    b!(self.comp.bld.build_gep(
                                        self.comp.ctx.i8_type(),
                                        payload_gep,
                                        &[byte_offset],
                                        "payload.field"
                                    ))
                                }
                            };
                            let val = b!(self.comp.bld.build_load(res_llvm, field_ptr, field));
                            return Ok(val);
                        }
                    }
                }
                let idx = self.field_index(name, field);
                let val = b!(self.comp.bld.build_extract_value(sv, idx, field));
                return Ok(val);
            }
            // Unknown struct — try index 0.
            let val = b!(self.comp.bld.build_extract_value(sv, 0, field));
            Ok(val)
        } else if obj_val.is_pointer_value() {
            // Pointer to struct: GEP + load.
            let ptr = obj_val.into_pointer_value();
            let res_llvm = self.comp.llvm_ty(result_ty);
            // Try to find the struct type from var_allocs or compiler vars.
            let struct_name = self.var_allocs.values()
                .find(|(p, _)| *p == ptr)
                .and_then(|(_, ty)| match ty {
                    Type::Struct(name, _) => Some(name.clone()),
                    _ => None,
                })
                .or_else(|| {
                    // Search compiler's var scopes for a matching pointer.
                    self.comp.vars.iter().rev().find_map(|scope| {
                        scope.values().find_map(|(p, ty)| {
                            if *p == ptr {
                                match ty {
                                    Type::Struct(name, _) => Some(name.clone()),
                                    _ => None,
                                }
                            } else {
                                None
                            }
                        })
                    })
                });
            if let Some(name) = &struct_name {
                if let Some(st) = self.comp.module.get_struct_type(name) {
                    let field_idx = self.field_index(name, field);
                    let gep = b!(self.comp.bld.build_struct_gep(
                        st, ptr, field_idx, field
                    ));
                    return Ok(b!(self.comp.bld.build_load(res_llvm, gep, field)));
                }
            }
            // Fallback: load directly (field 0 or opaque pointer).
            Ok(b!(self.comp.bld.build_load(res_llvm, ptr, field)))
        } else {
            Ok(obj_val)
        }
    }

    fn emit_vec_new(
        &mut self,
        elems: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let elem_ty = match result_ty {
            Type::Vec(e) => e.as_ref(),
            _ => &Type::I64,
        };
        // Call jade_vec_new(elem_size, capacity).
        let i64t = self.comp.ctx.i64_type();
        if let Some(fv) = self.comp.module.get_function("jade_vec_new") {
            let elem_size = self.comp.llvm_ty(elem_ty).size_of().unwrap_or(
                i64t.const_int(8, false),
            );
            let cap = i64t.const_int(elems.len().max(4) as u64, false);
            let ptr = b!(self
                .comp
                .bld
                .build_call(fv, &[elem_size.into(), cap.into()], "vec"))
            .try_as_basic_value()
            .basic()
            .unwrap();
            // Push initial elements.
            if let Some(push_fn) = self.comp.module.get_function("jade_vec_push") {
                for vid in elems {
                    let v = self.val(*vid);
                    let alloca = self.comp.entry_alloca(v.get_type(), "vpush.tmp");
                    b!(self.comp.bld.build_store(alloca, v));
                    b!(self
                        .comp
                        .bld
                        .build_call(push_fn, &[ptr.into(), alloca.into()], ""));
                }
            }
            Ok(ptr)
        } else {
            // No runtime — return null.
            Ok(self
                .comp
                .ctx
                .ptr_type(AddressSpace::default())
                .const_null()
                .into())
        }
    }

    fn emit_closure_create(
        &mut self,
        fn_name: &str,
        captures: &[mir::ValueId],
        _result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let closure_ty = self.comp.closure_type();

        // Look up the inner lambda function (has captures prepended as params).
        let inner_fv = if let Some((fv, _, _)) = self.comp.fns.get(fn_name).cloned() {
            Some(fv)
        } else {
            self.comp.module.get_function(fn_name)
        };

        // Build env struct from capture values.
        let cap_vals: Vec<BasicValueEnum<'ctx>> =
            captures.iter().map(|v| self.val(*v)).collect();
        let cap_tys: Vec<BasicTypeEnum<'ctx>> =
            cap_vals.iter().map(|v| v.get_type()).collect();

        let env_ptr = if !captures.is_empty() {
            let env_struct_ty = self.comp.ctx.struct_type(&cap_tys, false);
            let env_size = env_struct_ty.size_of().unwrap();
            let malloc = self.comp.ensure_malloc();
            let ep = b!(self.comp.bld.build_call(malloc, &[env_size.into()], "env.alloc"))
                .try_as_basic_value()
                .basic()
                .unwrap()
                .into_pointer_value();
            for (i, v) in cap_vals.iter().enumerate() {
                let gep = b!(self.comp.bld.build_struct_gep(
                    env_struct_ty,
                    ep,
                    i as u32,
                    "env.field"
                ));
                b!(self.comp.bld.build_store(gep, *v));
            }
            ep
        } else {
            ptr_ty.const_null()
        };

        // Build a wrapper function that takes (env_ptr, ...declared_params)
        // and calls the inner function with (captures..., declared_params...).
        let wrapper_ptr = if let Some(ifv) = inner_fv {
            let wrapper_name = format!("{fn_name}.env_wrap");
            if let Some(w) = self.comp.module.get_function(&wrapper_name) {
                w.as_global_value().as_pointer_value()
            } else {
                let inner_type = ifv.get_type();
                let inner_params = inner_type.get_param_types();
                let n_captures = captures.len();
                // Declared params are everything after the captures.
                let declared_param_tys = &inner_params[n_captures..];
                let mut wrapper_params: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
                wrapper_params.extend(declared_param_tys.iter().map(|t| BasicMetadataTypeEnum::from(*t)));
                let wrapper_ft = match inner_type.get_return_type() {
                    Some(ret) => ret.fn_type(&wrapper_params, false),
                    None => self.comp.ctx.void_type().fn_type(&wrapper_params, false),
                };
                let wrapper_fv = self.comp.module.add_function(
                    &wrapper_name, wrapper_ft, Some(inkwell::module::Linkage::Internal));
                self.comp.tag_fn(wrapper_fv);

                let saved_bb = self.comp.bld.get_insert_block();
                let entry = self.comp.ctx.append_basic_block(wrapper_fv, "entry");
                self.comp.bld.position_at_end(entry);

                // Build call args: unpack captures from env, then forward declared params.
                let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = Vec::new();
                if n_captures > 0 {
                    let env_struct_ty = self.comp.ctx.struct_type(&cap_tys, false);
                    let env_param = wrapper_fv.get_nth_param(0).unwrap().into_pointer_value();
                    for i in 0..n_captures {
                        let gep = b!(self.comp.bld.build_struct_gep(
                            env_struct_ty, env_param, i as u32, "cap.gep"));
                        let load_ty: BasicTypeEnum<'ctx> = inner_params[i].try_into().unwrap();
                        let cap = b!(self.comp.bld.build_load(
                            load_ty, gep, "cap.load"));
                        call_args.push(cap.into());
                    }
                }
                // Forward declared params (skip env_ptr at index 0).
                for i in 0..declared_param_tys.len() {
                    let p = wrapper_fv.get_nth_param((i + 1) as u32).unwrap();
                    call_args.push(p.into());
                }

                let result = self.comp.bld.build_call(ifv, &call_args, "lam.call").unwrap();
                match inner_type.get_return_type() {
                    Some(_) => {
                        let rv = result.try_as_basic_value().basic().unwrap();
                        self.comp.bld.build_return(Some(&rv)).unwrap();
                    }
                    None => { self.comp.bld.build_return(None).unwrap(); }
                }

                if let Some(bb) = saved_bb {
                    self.comp.bld.position_at_end(bb);
                }
                wrapper_fv.as_global_value().as_pointer_value()
            }
        } else {
            // Fallback: no function found, use null.
            ptr_ty.const_null()
        };

        // Build {wrapper_ptr, env_ptr} closure struct.
        let mut agg: BasicValueEnum<'ctx> = closure_ty.const_zero().into();
        agg = b!(self.comp.bld.build_insert_value(
            agg.into_struct_value(),
            wrapper_ptr,
            0,
            "closure.fn"
        ))
        .into_struct_value()
        .into();
        agg = b!(self.comp.bld.build_insert_value(
            agg.into_struct_value(),
            env_ptr,
            1,
            "closure.env"
        ))
        .into_struct_value()
        .into();
        Ok(agg)
    }

    fn emit_chan_create(&mut self, elem_ty: &Type) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.comp.ctx.i64_type();
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        if let Some(fv) = self.comp.module.get_function("jade_chan_create") {
            let elem_size = self.comp.llvm_ty(elem_ty).size_of().unwrap_or(
                i64t.const_int(8, false),
            );
            let capacity = i64t.const_int(64, false); // default capacity
            let csv = b!(self
                .comp
                .bld
                .build_call(fv, &[elem_size.into(), capacity.into()], "chan"));
            Ok(self.comp.call_result(csv))
        } else {
            Ok(ptr_ty.const_null().into())
        }
    }

    fn emit_chan_send(
        &mut self,
        ch: mir::ValueId,
        val: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        let v = self.val(val);
        if let Some(fv) = self.comp.module.get_function("jade_chan_send") {
            let alloca = self.comp.entry_alloca(v.get_type(), "send.tmp");
            b!(self.comp.bld.build_store(alloca, v));
            b!(self.comp.bld.build_call(
                fv,
                &[ch_val.into(), alloca.into()],
                ""
            ));
        }
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    fn emit_chan_recv(
        &mut self,
        ch: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ch_val = self.val(ch);
        if let Some(fv) = self.comp.module.get_function("jade_chan_recv") {
            let elem_llvm = self.comp.llvm_ty(result_ty);
            let alloca = self.comp.entry_alloca(elem_llvm, "recv.tmp");
            b!(self.comp.bld.build_call(
                fv,
                &[ch_val.into(), alloca.into()],
                ""
            ));
            Ok(b!(self.comp.bld.build_load(elem_llvm, alloca, "recv.val")))
        } else {
            Ok(self.comp.default_val(result_ty))
        }
    }

    fn field_index(&self, struct_name: &str, field: &str) -> u32 {
        self.comp
            .structs
            .get(struct_name)
            .and_then(|fields| fields.iter().position(|(n, _)| n == field))
            .unwrap_or(0) as u32
    }

    fn struct_name_from_type(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct(name, _) => Some(name.clone()),
            _ => None,
        }
    }

    // ── HIR coroutine/generator body extraction ───────────────────

    /// Walk the entire HIR program to extract CoroutineCreate and GeneratorCreate
    /// bodies, keyed by their name for later use in MIR codegen.
    fn extract_coro_bodies_from_program(
        prog: &hir::Program,
        out: &mut HashMap<String, Vec<hir::Stmt>>,
    ) {
        for f in &prog.fns {
            for stmt in &f.body {
                Self::extract_coro_bodies_from_stmt(stmt, out);
            }
        }
        for td in &prog.types {
            for m in &td.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                for stmt in &m.body {
                    Self::extract_coro_bodies_from_stmt(stmt, out);
                }
            }
        }
    }

    fn extract_coro_bodies_from_stmt(
        stmt: &hir::Stmt,
        out: &mut HashMap<String, Vec<hir::Stmt>>,
    ) {
        match stmt {
            hir::Stmt::Bind(b) => Self::extract_coro_bodies_from_expr(&b.value, out),
            hir::Stmt::Expr(e) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::If(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then { Self::extract_coro_bodies_from_stmt(s, out); }
                if let Some(ref eb) = i.els {
                    for s in eb { Self::extract_coro_bodies_from_stmt(s, out); }
                }
                for elif in &i.elifs {
                    Self::extract_coro_bodies_from_expr(&elif.0, out);
                    for s in &elif.1 { Self::extract_coro_bodies_from_stmt(s, out); }
                }
            }
            hir::Stmt::While(w) => {
                Self::extract_coro_bodies_from_expr(&w.cond, out);
                for s in &w.body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::Stmt::For(f) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::Stmt::Loop(l) => {
                for s in &l.body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::Stmt::Ret(Some(e), _, _) => Self::extract_coro_bodies_from_expr(e, out),
            hir::Stmt::Assign(a, b, _) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::Stmt::Match(m) => {
                Self::extract_coro_bodies_from_expr(&m.subject, out);
                for arm in &m.arms {
                    for s in &arm.body { Self::extract_coro_bodies_from_stmt(s, out); }
                }
            }
            hir::Stmt::SimFor(f, _) => {
                Self::extract_coro_bodies_from_expr(&f.iter, out);
                for s in &f.body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::Stmt::SimBlock(b, _) => {
                for s in b { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            _ => {}
        }
    }

    fn extract_coro_bodies_from_expr(
        expr: &hir::Expr,
        out: &mut HashMap<String, Vec<hir::Stmt>>,
    ) {
        match &expr.kind {
            hir::ExprKind::CoroutineCreate(name, body) => {
                out.insert(name.clone(), body.clone());
                // Also recurse into the body for nested coroutines
                for s in body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::ExprKind::GeneratorCreate(_, name, body) => {
                out.insert(name.clone(), body.clone());
                for s in body { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::ExprKind::BinOp(a, _, b) => {
                Self::extract_coro_bodies_from_expr(a, out);
                Self::extract_coro_bodies_from_expr(b, out);
            }
            hir::ExprKind::UnaryOp(_, a) => Self::extract_coro_bodies_from_expr(a, out),
            hir::ExprKind::Call(_, _, args) => {
                for a in args { Self::extract_coro_bodies_from_expr(a, out); }
            }
            hir::ExprKind::IndirectCall(f, args) => {
                Self::extract_coro_bodies_from_expr(f, out);
                for a in args { Self::extract_coro_bodies_from_expr(a, out); }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::extract_coro_bodies_from_expr(&i.cond, out);
                for s in &i.then { Self::extract_coro_bodies_from_stmt(s, out); }
                if let Some(ref eb) = i.els {
                    for s in eb { Self::extract_coro_bodies_from_stmt(s, out); }
                }
            }
            hir::ExprKind::Block(b) => {
                for s in b { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            hir::ExprKind::Lambda(_, b) => {
                for s in b { Self::extract_coro_bodies_from_stmt(s, out); }
            }
            _ => {}
        }
    }

    // ── Magic call interception ───────────────────────────────────

    /// Handle "magic" call names emitted by MIR lowering that need special
    /// codegen treatment (coroutines, generators, actors, stores).
    /// Returns Some(value) if handled, None if it's a normal call.
    fn try_handle_magic_call(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        // ── Coroutine create ──
        if let Some(coro_name) = name.strip_prefix("__coro_create_") {
            return self.emit_coro_create(coro_name).map(Some);
        }
        // ── Generator create (same impl as coroutine) ──
        if let Some(gen_name) = name.strip_prefix("__gen_create_") {
            return self.emit_coro_create(gen_name).map(Some);
        }
        // ── Coroutine/generator next ──
        if name == "__coro_next" || name == "__gen_next" {
            if let Some(&gen_val) = args.first() {
                return self.emit_coro_next(gen_val).map(Some);
            }
        }
        // ── Generator resume (for-in loop) ──
        if name == "__gen_resume" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let gen_resume = self.comp.module.get_function("jade_gen_resume")
                    .ok_or("jade_gen_resume not declared")?;
                b!(self.comp.bld.build_call(gen_resume, &[gen_ptr.into()], ""));
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
        }
        // ── Generator done check (for-in loop) ──
        if name == "__gen_done" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let i8t = self.comp.ctx.i8_type();
                let done_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_DONE_OFF, "gen.done.ptr")?;
                let done = b!(self.comp.bld.build_load(i8t, done_ptr, "gen.done"));
                let done_bool = b!(self.comp.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    done.into_int_value(),
                    i8t.const_int(0, false),
                    "gen.done.bool"
                ));
                return Ok(Some(done_bool.into()));
            }
        }
        // ── Generator read yielded value (for-in loop) ──
        if name == "__gen_next_val" {
            if let Some(&gen_val) = args.first() {
                let gen_ptr = self.val(gen_val).into_pointer_value();
                let i8t = self.comp.ctx.i8_type();
                let i64t = self.comp.ctx.i64_type();
                let value_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.val.ptr")?;
                let result = b!(self.comp.bld.build_load(i64t, value_ptr, "gen.val"));
                // Clear has_value
                let has_val_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.hv.ptr")?;
                b!(self.comp.bld.build_store(has_val_ptr, i8t.const_int(0, false)));
                return Ok(Some(result));
            }
        }
        // ── Yield (inside coroutine body) ──
        if name == "__yield" {
            if let Some(&val) = args.first() {
                return self.emit_coro_yield(val).map(Some);
            }
        }
        // ── Select recv (reads from select data buffer, not jade_chan_recv) ──
        if name == "__select_recv" {
            if args.len() >= 2 {
                let select_vid = args[0];
                let idx_val = self.val(args[1]).into_int_value();
                let idx = idx_val.get_zero_extended_constant().unwrap_or(0) as usize;
                if let Some(bufs) = self.select_data_bufs.get(&select_vid) {
                    if let Some(&buf_ptr) = bufs.get(idx) {
                        let i64t = self.comp.ctx.i64_type();
                        let val = b!(self.comp.bld.build_load(i64t, buf_ptr, "recv.val"));
                        return Ok(Some(val));
                    }
                }
                // Fallback: return 0
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
        }
        // ── Actor send ──
        if let Some(handler_name) = name.strip_prefix("__send_") {
            return self.emit_actor_send(handler_name, args).map(Some);
        }
        // ── Store operations ──
        if let Some(store_name) = name.strip_prefix("__store_insert_") {
            return self.emit_store_insert(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_query_") {
            return self.emit_store_query(store_name, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_count_") {
            return self.emit_store_count(store_name).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_all_") {
            return self.emit_store_all(store_name).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_delete_") {
            return self.emit_store_delete(store_name).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_set_") {
            return self.emit_store_set(store_name, args).map(Some);
        }
        Ok(None)
    }

    // ── Coroutine/Generator codegen ──────────────────────────────

    /// Create a coroutine/generator: builds __coro_{name} function,
    /// allocates 32-byte gen control block, creates coro via jade_coro_create.
    fn emit_coro_create(
        &mut self,
        name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr = self.comp.ctx.ptr_type(AddressSpace::default());
        let i64t = self.comp.ctx.i64_type();

        // Try to find the body in extracted HIR coroutine bodies
        if let Some(body) = self.coro_bodies.get(name).cloned() {
            // Delegate to the HIR coroutine codegen which handles everything:
            // creating the __coro_{name} function, building the gen control block, etc.
            return self.comp.compile_coroutine_create(name, &body);
        }

        // Fallback: no body found — return null pointer
        Ok(ptr.const_null().into())
    }

    /// Resume a coroutine/generator and read the yielded value.
    fn emit_coro_next(
        &mut self,
        gen_val_id: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let gen_ptr = self.val(gen_val_id).into_pointer_value();
        let i8t = self.comp.ctx.i8_type();
        let i64t = self.comp.ctx.i64_type();

        // Resume the producer coroutine (direct context swap)
        let gen_resume = self.comp.module.get_function("jade_gen_resume")
            .ok_or("jade_gen_resume not declared")?;
        b!(self.comp.bld.build_call(gen_resume, &[gen_ptr.into()], ""));

        // Read the yielded value
        let value_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.n.val")?;
        let result = b!(self.comp.bld.build_load(i64t, value_ptr, "gen.result"));

        // Clear has_value
        let has_val_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.n.hv")?;
        b!(self.comp.bld.build_store(has_val_ptr, i8t.const_int(0, false)));

        Ok(result)
    }

    /// Yield a value from inside a coroutine body.
    /// When called from the parent function (no __coro_ctx), this is an inlined
    /// artifact from MIR lowering — the real yield is compiled by compile_coroutine_create
    /// from the extracted HIR body. Just return a dummy value.
    fn emit_coro_yield(
        &mut self,
        val_id: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // If __coro_ctx doesn't exist, we're in the parent function —
        // this yield was inlined by MIR lowering and will be handled
        // by compile_coroutine_create from the HIR body.
        if self.comp.find_var("__coro_ctx").is_none() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }

        let val = self.val(val_id);
        let ptr = self.comp.ctx.ptr_type(AddressSpace::default());
        let i8t = self.comp.ctx.i8_type();

        let (gen_alloca, _) = self.comp.find_var("__coro_ctx")
            .cloned()
            .unwrap();
        let gen_ptr = b!(self.comp.bld.build_load(ptr, gen_alloca, "gen.ctx")).into_pointer_value();

        // Write value to gen block
        let value_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_VALUE_OFF, "gen.y.val")?;
        let i64_val = self.comp.coerce_to_i64(val);
        b!(self.comp.bld.build_store(value_ptr, i64_val));

        // Set has_value = 1
        let has_val_ptr = self.comp.gen_field_ptr(gen_ptr, Compiler::GEN_HAS_VALUE_OFF, "gen.y.hv")?;
        b!(self.comp.bld.build_store(has_val_ptr, i8t.const_int(1, false)));

        // Suspend back to caller
        let gen_suspend = self.comp.module.get_function("jade_gen_suspend")
            .ok_or("jade_gen_suspend not declared")?;
        b!(self.comp.bld.build_call(gen_suspend, &[gen_ptr.into()], ""));

        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    // ── Actor codegen ────────────────────────────────────────────

    /// Spawn an actor: malloc mailbox, create channel, create coro, schedule it.
    fn emit_spawn_actor(
        &mut self,
        actor_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Delegate to the existing HIR actor codegen
        self.comp.compile_spawn(actor_name)
    }

    /// Send a message to an actor. The MIR lowering emits:
    ///   Call("__send_{handler}", [target, arg0, arg1, ...])
    /// We need to find the actor name and tag from the handler name.
    fn emit_actor_send(
        &mut self,
        handler_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let i32t = self.comp.ctx.i32_type();
        let i64t = self.comp.ctx.i64_type();

        // Find which actor owns this handler
        let (actor_name, tag, handler_params) = {
            let mut found = None;
            for (aname, ad) in &self.actor_defs {
                for h in &ad.handlers {
                    if h.name == handler_name {
                        let param_tys: Vec<Type> = h.params.iter().map(|p| p.ty.clone()).collect();
                        found = Some((aname.clone(), h.tag, param_tys));
                        break;
                    }
                }
                if found.is_some() { break; }
            }
            found.ok_or_else(|| format!("mir_codegen: unknown actor handler '{handler_name}'"))?
        };

        let mb_name = format!("{actor_name}_mailbox");
        let msg_name = format!("{actor_name}_msg");

        let mb_st = self.comp.module.get_struct_type(&mb_name)
            .ok_or_else(|| format!("mailbox type '{mb_name}' not found"))?;
        let msg_st = self.comp.module.get_struct_type(&msg_name)
            .ok_or_else(|| format!("message type '{msg_name}' not found"))?;

        // First arg is the target (mailbox pointer)
        let mb_ptr = self.val(args[0]).into_pointer_value();

        // Load channel pointer from mailbox
        let ch_ptr_ptr = b!(self.comp.bld.build_struct_gep(mb_st, mb_ptr, 0, "ch_ptr_ptr"));
        let ch_ptr = b!(self.comp.bld.build_load(ptr_ty, ch_ptr_ptr, "ch_ptr"));

        // Build message: {tag, payload}
        let msg_alloca = self.comp.entry_alloca(msg_st.into(), "send_msg");

        let tag_ptr = b!(self.comp.bld.build_struct_gep(msg_st, msg_alloca, 0, "tag_ptr"));
        b!(self.comp.bld.build_store(tag_ptr, i32t.const_int(tag as u64, false)));

        let payload_ptr = b!(self.comp.bld.build_struct_gep(msg_st, msg_alloca, 1, "payload_ptr"));

        // Store arguments into payload
        let mut arg_offset: u64 = 0;
        for (i, param_ty) in handler_params.iter().enumerate() {
            if i + 1 >= args.len() { break; }
            let val = self.val(args[i + 1]);
            let pty = self.comp.llvm_ty(param_ty);
            let psize = self.comp.type_store_size(pty);
            let offset_val = i64t.const_int(arg_offset, false);
            let dest = unsafe {
                b!(self.comp.bld.build_gep(
                    self.comp.ctx.i8_type(),
                    payload_ptr,
                    &[offset_val.into()],
                    "arg_ptr"
                ))
            };
            b!(self.comp.bld.build_store(dest, val));
            arg_offset += psize;
        }

        // Send message
        let chan_send = self.comp.module.get_function("jade_chan_send")
            .ok_or("jade_chan_send not declared")?;
        b!(self.comp.bld.build_call(chan_send, &[ch_ptr.into(), msg_alloca.into()], ""));

        Ok(i64t.const_int(0, false).into())
    }

    // ── Select codegen ──────────────────────────────────────────

    /// Build jade_select call: construct case array, call jade_select(), return index.
    fn emit_select(
        &mut self,
        channels: &[mir::ValueId],
        dest: mir::ValueId,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let i32t = self.comp.ctx.i32_type();
        let i64t = self.comp.ctx.i64_type();
        let n = channels.len();

        // jade_select_case_t = { chan: ptr, data: ptr, is_send: i32 }
        let case_struct_ty = self.comp.ctx.struct_type(
            &[ptr_ty.into(), ptr_ty.into(), i32t.into()], false,
        );
        let cases_array_ty = case_struct_ty.array_type(n as u32);
        let cases_alloca = self.comp.entry_alloca(cases_array_ty.into(), "select.cases");

        let mut data_bufs = Vec::new();
        for (i, ch_vid) in channels.iter().enumerate() {
            let ch_val = self.val(*ch_vid).into_pointer_value();

            // Allocate recv buffer for each channel
            let data_alloca = self.comp.entry_alloca(i64t.into(), &format!("select.data.{i}"));
            data_bufs.push(data_alloca);

            let idx0 = i32t.const_int(0, false);
            let idx_i = i32t.const_int(i as u64, false);
            let case_ptr = unsafe {
                b!(self.comp.bld.build_gep(
                    cases_array_ty,
                    cases_alloca,
                    &[idx0, idx_i],
                    &format!("select.case.{i}")
                ))
            };

            // case.chan = ch_val
            let chan_field = b!(self.comp.bld.build_struct_gep(case_struct_ty, case_ptr, 0, "case.chan"));
            b!(self.comp.bld.build_store(chan_field, ch_val));

            // case.data = data_alloca
            let data_field = b!(self.comp.bld.build_struct_gep(case_struct_ty, case_ptr, 1, "case.data"));
            b!(self.comp.bld.build_store(data_field, data_alloca));

            // case.is_send = 0 (recv)
            let is_send_field = b!(self.comp.bld.build_struct_gep(case_struct_ty, case_ptr, 2, "case.is_send"));
            b!(self.comp.bld.build_store(is_send_field, i32t.const_int(0, false)));
        }

        // Store data buffers for __select_recv to use
        self.select_data_bufs.insert(dest, data_bufs);

        let select_fn = self.comp.module.get_function("jade_select")
            .ok_or("jade_select not declared")?;
        let has_default = self.comp.ctx.bool_type().const_int(0, false);
        let result = b!(self.comp.bld.build_call(
            select_fn,
            &[
                cases_alloca.into(),
                i32t.const_int(n as u64, false).into(),
                has_default.into(),
            ],
            "select.result"
        )).try_as_basic_value().basic().unwrap();

        Ok(result)
    }

    // ── Store ops codegen ───────────────────────────────────────

    fn emit_store_insert(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self.comp.store_defs.get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        // Build fake hir::Expr values from MIR values — we need to call compile_store_insert
        // which expects &[hir::Expr]. Instead, we'll emit the LLVM IR directly.
        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.comp.module.get_function(&ensure_fn_name) {
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        } else {
            // Generate the ensure_open function
            let ensure_fn = self.comp.gen_store_ensure_open(&sd)?;
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.comp.load_store_fp(store_name)?;
        self.comp.store_lock(fp)?;

        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.comp.module.get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;
        let rec_size = self.comp.store_record_size(&sd);

        let rec_ptr = self.comp.entry_alloca(st.into(), "store.rec");
        let memset_fn = self.comp.module.get_function("memset").unwrap();
        b!(self.comp.bld.build_call(
            memset_fn,
            &[rec_ptr.into(), i32t.const_int(0, false).into(), i64t.const_int(rec_size, false).into()],
            ""
        ));

        for (i, _field_def) in sd.fields.iter().enumerate() {
            if i >= args.len() { break; }
            let val = self.val(args[i]);
            let gep = b!(self.comp.bld.build_struct_gep(st, rec_ptr, i as u32, &sd.fields[i].name));
            match &sd.fields[i].ty {
                Type::String => {
                    self.comp.copy_string_to_fixed_buf(val, gep)?;
                }
                _ => {
                    b!(self.comp.bld.build_store(gep, val));
                }
            }
        }

        // Seek to end and write record
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(0, false).into(), i32t.const_int(2, false).into()],
            ""
        ));
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[rec_ptr.into(), i64t.const_int(rec_size, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));

        // Update count
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.comp.entry_alloca(i64t.into(), "count.buf");
        let fread_fn = self.comp.module.get_function("fread").unwrap();
        b!(self.comp.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        let old_count = b!(self.comp.bld.build_load(i64t, count_buf, "old.count")).into_int_value();
        let new_count = b!(self.comp.bld.build_int_add(old_count, i64t.const_int(1, false), "new.count"));
        b!(self.comp.bld.build_store(count_buf, new_count));
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));
        self.comp.store_unlock(fp)?;

        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    fn emit_store_query(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Name format: {store_name}__{field}__{op}
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let op = match parts[2] {
            "eq" => crate::ast::BinOp::Eq,
            "ne" => crate::ast::BinOp::Ne,
            "lt" => crate::ast::BinOp::Lt,
            "le" => crate::ast::BinOp::Le,
            "gt" => crate::ast::BinOp::Gt,
            "ge" => crate::ast::BinOp::Ge,
            _ => crate::ast::BinOp::Eq,
        };

        let sd = self.comp.store_defs.get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.comp.module.get_function(&ensure_fn_name) {
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.comp.gen_store_ensure_open(&sd)?;
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.comp.load_store_fp(store_name)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);

        // Find field index and type
        let (field_idx, field_ty) = sd.fields.iter().enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        let result_ptr = self.comp.entry_alloca(st.into(), "q.result");
        let memset_fn = self.comp.module.get_function("memset").unwrap();
        b!(self.comp.bld.build_call(
            memset_fn,
            &[
                result_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));

        let fv_fn = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "q.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv_fn, "q.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv_fn, "q.body");
        let match_bb = self.comp.ctx.append_basic_block(fv_fn, "q.match");
        let next_bb = self.comp.ctx.append_basic_block(fv_fn, "q.next");
        let done_bb = self.comp.ctx.append_basic_block(fv_fn, "q.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::ULT, idx, count, "q.cmp"));
        b!(self.comp.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset = b!(self.comp.bld.build_int_mul(
            idx, i64t.const_int(rec_size, false), "q.off"));
        let rec_ptr = unsafe {
            b!(self.comp.bld.build_gep(
                self.comp.ctx.i8_type(), buf, &[offset], "q.rec"))
        };

        let cond = self.comp.eval_store_filter(
            rec_ptr, st, field_idx, &field_ty, op, filter_val, &[])?;
        b!(self.comp.bld.build_conditional_branch(cond, match_bb, next_bb));

        self.comp.bld.position_at_end(match_bb);
        let memcpy_fn = self.comp.ensure_memcpy();
        b!(self.comp.bld.build_call(
            memcpy_fn,
            &[
                result_ptr.into(),
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));
        b!(self.comp.bld.build_unconditional_branch(done_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self.comp.bld.build_int_add(
            idx, i64t.const_int(1, false), "q.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.comp.load_store_record_as_jade(st, result_ptr, &sd)?;
        Ok(result)
    }

    fn emit_store_count(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self.comp.store_defs.get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.comp.module.get_function(&ensure_fn_name) {
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.comp.gen_store_ensure_open(&sd)?;
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.comp.load_store_fp(store_name)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.comp.entry_alloca(i64t.into(), "sc.count");
        b!(self.comp.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.comp.module.get_function("fread").unwrap();
        b!(self.comp.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        Ok(b!(self.comp.bld.build_load(i64t, count_buf, "count")))
    }

    fn emit_store_all(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Store all returns a pointer to all records — complex implementation.
        // For now return null.
        Ok(self.comp.ctx.ptr_type(AddressSpace::default()).const_null().into())
    }

    fn emit_store_delete(
        &mut self,
        store_name: &str,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Store delete requires filter — complex implementation.
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    fn emit_store_set(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Store set requires filter — complex implementation.
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }
}
