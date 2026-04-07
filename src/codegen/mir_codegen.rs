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
    /// MIR ValueId → Jade type (for FieldGet struct type resolution on parameters).
    value_types: HashMap<mir::ValueId, Type>,
    /// Coroutine/generator bodies extracted from HIR, keyed by name.
    coro_bodies: HashMap<String, Vec<hir::Stmt>>,
    /// Actor definitions from HIR, keyed by name.
    actor_defs: HashMap<String, hir::ActorDef>,
    /// Select data buffers: select_val ValueId → Vec<PointerValue> (one per arm).
    select_data_bufs: HashMap<mir::ValueId, Vec<PointerValue<'ctx>>>,
    /// MIR ValueId → alloca for structs that were passed by pointer to methods (cached so mutations persist).
    self_allocs: HashMap<mir::ValueId, PointerValue<'ctx>>,
    /// MIR BlockId → actual LLVM exit block (may differ from block_map entry
    /// when helpers like string_concat create intermediate LLVM blocks).
    block_exit_map: HashMap<mir::BlockId, LLVMBlock<'ctx>>,
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
            value_types: HashMap::new(),
            coro_bodies: HashMap::new(),
            actor_defs: HashMap::new(),
            select_data_bufs: HashMap::new(),
            self_allocs: HashMap::new(),
            block_exit_map: HashMap::new(),
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

        // Populate struct_defaults from HIR type definitions.
        for td in &hir_prog.types {
            let defaults: std::collections::HashMap<String, hir::Expr> = td
                .fields
                .iter()
                .filter_map(|f| f.default.as_ref().map(|d| (f.name.clone(), d.clone())))
                .collect();
            if !defaults.is_empty() {
                self.comp.struct_defaults.insert(td.name.clone(), defaults);
            }
            // Also register struct_layouts for alignment info.
            self.comp.struct_layouts.insert(td.name.clone(), td.layout.clone());
        }

        // Register HIR enum definitions (MIR doesn't carry enum info yet).
        for ed in &hir_prog.enums {
            let _ = self.comp.declare_enum(ed);
        }

        // Register error definitions (tagged unions like enums).
        for ed in &hir_prog.err_defs {
            self.comp.declare_err_def(ed)?;
        }

        // Register extern declarations.
        for ext in &prog.externs {
            let ptys: Vec<BasicMetadataTypeEnum<'ctx>> =
                ext.params.iter().map(|t| {
                    // Extern functions use C ABI: String → ptr (char*)
                    if matches!(t, Type::String) {
                        self.comp.ctx.ptr_type(inkwell::AddressSpace::default()).into()
                    } else {
                        self.comp.llvm_ty(t).into()
                    }
                }).collect();
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

        // ── Declare trait impl methods not already declared via MIR fns ──
        for ti in &hir_prog.trait_impls {
            for m in &ti.methods {
                if !self.comp.fns.contains_key(&m.name) {
                    self.comp.declare_method(&ti.type_name, m)?;
                }
            }
        }

        // ── Generate vtables for dynamic dispatch ──
        self.comp.generate_vtables(&hir_prog.trait_impls)?;

        // ── Compile actor loop bodies (after MIR fn declarations so
        //    functions like fib are available for actor handlers) ──
        if !hir_prog.actors.is_empty() {
            for ad in &hir_prog.actors {
                self.comp.compile_actor_loop(ad)?;
            }
        }

        // ── Supervisor trees ──
        for sup in &hir_prog.supervisors {
            self.comp.compile_supervisor(sup)?;
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
        self.value_types.clear();
        self.self_allocs.clear();
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
            self.value_types.insert(param.value, param.ty.clone());
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
                    self.value_types.insert(dest, inst.ty.clone());
                }
            }

            // Record actual exit block (helpers like string_concat may have
            // repositioned the builder to intermediate LLVM blocks).
            let exit_bb = self.comp.bld.get_insert_block().unwrap();
            self.block_exit_map.insert(bb.id, exit_bb);

            // 3c. Emit terminator.
            self.emit_terminator(&bb.terminator, &func.ret_ty)?;
        }

        // 4. Back-patch phi incoming edges.
        for pp in &self.pending_phis {
            let phi_ty = pp.phi.as_basic_value().get_type();
            let incoming: Vec<(BasicValueEnum<'ctx>, LLVMBlock<'ctx>)> = pp
                .incoming
                .iter()
                .filter_map(|(block_id, val_id)| {
                    // Use exit block (may differ from entry block if helpers
                    // like string_concat created intermediate LLVM blocks).
                    let llvm_bb = self.block_exit_map.get(block_id)
                        .or_else(|| self.block_map.get(block_id))?;
                    let llvm_val = self.value_map.get(val_id)?;
                    // Coerce void sentinel (i8 0) to the phi's actual type.
                    let v = if llvm_val.get_type() != phi_ty {
                        if phi_ty.is_int_type() {
                            phi_ty.into_int_type().const_int(0, false).into()
                        } else if phi_ty.is_float_type() {
                            phi_ty.into_float_type().const_float(0.0).into()
                        } else if phi_ty.is_pointer_type() {
                            phi_ty.into_pointer_type().const_null().into()
                        } else {
                            *llvm_val
                        }
                    } else {
                        *llvm_val
                    };
                    Some((v, *llvm_bb))
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
            mir::InstKind::Cmp(op, lhs, rhs, operand_ty) => {
                self.emit_cmp(*op, *lhs, *rhs, operand_ty)
            }

            // ── Calls ──
            mir::InstKind::Call(name, args) => {
                // Check for magic call names first (coroutines, actors, stores)
                if let Some(result) = self.try_handle_magic_call(name, args, &inst.ty)? {
                    return Ok(result);
                }
                // Handle overflow builtins that MIR lowered as __builtin_* calls
                if let Some(result) = self.try_handle_overflow_builtin(name, args)? {
                    return Ok(result);
                }
                let arg_vals: Vec<BasicValueEnum<'ctx>> = args
                    .iter()
                    .map(|a| self.val(*a))
                    .collect();
                if let Some((fv, _, _)) = self.comp.fns.get(name).cloned() {
                    let ptypes = fv.get_type().get_param_types();
                    let st = self.comp.string_type();
                    let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        arg_vals.iter().enumerate().map(|(i, v)| {
                            if let Some(pt) = ptypes.get(i) {
                                if v.get_type() == st.into() && pt.is_pointer_type() {
                                    self.comp.string_data(*v).unwrap_or(*v).into()
                                } else { (*v).into() }
                            } else { (*v).into() }
                        }).collect();
                    let csv = b!(self.comp.bld.build_call(fv, &md, "call"));
                    Ok(self.comp.call_result(csv))
                } else {
                    // Try looking up as a module-level function.
                    if let Some(fv) = self.comp.module.get_function(name) {
                        let ptypes = fv.get_type().get_param_types();
                        let st = self.comp.string_type();
                        let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                            arg_vals.iter().enumerate().map(|(i, v)| {
                                if let Some(pt) = ptypes.get(i) {
                                    if v.get_type() == st.into() && pt.is_pointer_type() {
                                        self.comp.string_data(*v).unwrap_or(*v).into()
                                    } else { (*v).into() }
                                } else { (*v).into() }
                            }).collect();
                        let csv = b!(self.comp.bld.build_call(fv, &md, "call"));
                        Ok(self.comp.call_result(csv))
                    } else {
                        Err(format!("mir_codegen: unknown function `{name}`"))
                    }
                }
            }
            mir::InstKind::MethodCall(recv, method, args) => {
                // Try vec/array methods first (these are inline, not compiled functions)
                let recv_ty = self.value_types.get(recv).cloned();

                // String methods
                if matches!(&recv_ty, Some(Type::String)) {
                    let recv_val = self.val(*recv);
                    match method.as_str() {
                        "length" | "len" => return self.comp.string_len(recv_val),
                        "contains" => {
                            if !args.is_empty() {
                                let a = self.val(args[0]);
                                return self.comp.string_contains(recv_val, a);
                            }
                        }
                        "starts_with" => {
                            if !args.is_empty() {
                                let a = self.val(args[0]);
                                return self.comp.string_starts_with(recv_val, a);
                            }
                        }
                        "ends_with" => {
                            if !args.is_empty() {
                                let a = self.val(args[0]);
                                return self.comp.string_ends_with(recv_val, a);
                            }
                        }
                        "char_at" => {
                            if !args.is_empty() {
                                let a = self.val(args[0]);
                                return self.comp.string_char_at(recv_val, a);
                            }
                        }
                        "slice" => {
                            if args.len() >= 2 {
                                let start = self.val(args[0]);
                                let end = self.val(args[1]);
                                return self.comp.string_slice(recv_val, start, end);
                            }
                        }
                        "find" => {
                            if !args.is_empty() {
                                let a = self.val(args[0]);
                                return self.comp.string_find(recv_val, a);
                            }
                        }
                        "trim" => return self.comp.string_trim(recv_val, true, true),
                        "trim_left" => return self.comp.string_trim(recv_val, true, false),
                        "trim_right" => return self.comp.string_trim(recv_val, false, true),
                        "to_upper" => return self.comp.string_case(recv_val, true),
                        "to_lower" => return self.comp.string_case(recv_val, false),
                        "replace" => {
                            if args.len() >= 2 {
                                let old = self.val(args[0]);
                                let new = self.val(args[1]);
                                return self.comp.string_replace(recv_val, old, new);
                            }
                        }
                        "split" => {
                            if !args.is_empty() {
                                let delim = self.val(args[0]);
                                return self.comp.string_split(recv_val, delim);
                            }
                        }
                        "lines" => {
                            let newline = self.comp.compile_str_literal("\n")?;
                            return self.comp.string_split(recv_val, newline);
                        }
                        "repeat" => {
                            if !args.is_empty() {
                                let count = self.val(args[0]);
                                return self.comp.string_repeat(recv_val, count);
                            }
                        }
                        "is_empty" => {
                            let len = self.comp.string_len(recv_val)?.into_int_value();
                            let i64t = self.comp.ctx.i64_type();
                            let cmp = b!(self.comp.bld.build_int_compare(
                                inkwell::IntPredicate::EQ,
                                len,
                                i64t.const_int(0, false),
                                "isempty"
                            ));
                            return Ok(cmp.into());
                        }
                        _ => {} // fall through to function lookup
                    }
                }

                let is_vec_or_array = matches!(&recv_ty, Some(Type::Vec(_)) | Some(Type::Array(_, _)));
                if is_vec_or_array {
                    let recv_val = self.val(*recv);
                    let elem_ty = match &recv_ty {
                        Some(Type::Vec(et)) => *et.clone(),
                        Some(Type::Array(et, _)) => *et.clone(),
                        _ => Type::I64,
                    };
                    // Fixed-size array: len returns constant, contains is inline scan
                    if let Some(Type::Array(_, arr_len)) = &recv_ty {
                        match method.as_str() {
                            "len" => {
                                return Ok(self.comp.ctx.i64_type().const_int(*arr_len as u64, false).into());
                            }
                            _ => {}
                        }
                    }
                    let header_ptr = recv_val.into_pointer_value();
                    let lty = self.comp.llvm_ty(&elem_ty);
                    match method.as_str() {
                        "len" | "count" => return self.comp.vec_len(header_ptr),
                        "push" => {
                            if !args.is_empty() {
                                let val = self.val(args[0]);
                                let elem_size = self.comp.type_store_size(lty);
                                self.comp.vec_push_raw(header_ptr, val, lty, elem_size)?;
                                return Ok(self.comp.ctx.i8_type().const_int(0, false).into());
                            }
                            return Err("push() requires an argument".into());
                        }
                        "pop" => return self.comp.vec_pop(header_ptr, &elem_ty),
                        "get" => {
                            if !args.is_empty() {
                                let idx = self.val(args[0]).into_int_value();
                                return self.comp.vec_get_idx(header_ptr, &elem_ty, idx);
                            }
                            return Err("get() requires an index".into());
                        }
                        "collect" => return Ok(recv_val),
                        "set" => {
                            if args.len() >= 2 {
                                let idx = self.val(args[0]).into_int_value();
                                let val = self.val(args[1]);
                                return self.comp.vec_set_val(header_ptr, &elem_ty, idx, val);
                            }
                            return Err("set() requires index and value".into());
                        }
                        "remove" => {
                            if !args.is_empty() {
                                let idx = self.val(args[0]).into_int_value();
                                return self.comp.vec_remove_val(header_ptr, &elem_ty, idx);
                            }
                            return Err("remove() requires an index".into());
                        }
                        "clear" => return self.comp.vec_clear(header_ptr),
                        _ => {} // fall through to function lookup
                    }
                }
                let is_map = matches!(&recv_ty, Some(Type::Map(_, _)));
                if is_map {
                    let recv_val = self.val(*recv);
                    let header_ptr = recv_val.into_pointer_value();
                    match method.as_str() {
                        "len" | "count" => return self.comp.vec_len(header_ptr),
                        "set" => {
                            if args.len() >= 2 {
                                let k = self.val(args[0]);
                                let v = self.val(args[1]);
                                return self.comp.map_set_val(header_ptr, k, v);
                            }
                            return Err("map.set() requires key and value".into());
                        }
                        "get" => {
                            if !args.is_empty() {
                                let k = self.val(args[0]);
                                return self.comp.map_get_val(header_ptr, k);
                            }
                            return Err("map.get() requires a key".into());
                        }
                        "has" | "contains" => {
                            if !args.is_empty() {
                                let k = self.val(args[0]);
                                return self.comp.map_has_val(header_ptr, k);
                            }
                            return Err("map.has() requires a key".into());
                        }
                        "remove" => {
                            if !args.is_empty() {
                                let k = self.val(args[0]);
                                return self.comp.map_remove_val(header_ptr, k);
                            }
                            return Err("map.remove() requires a key".into());
                        }
                        "clear" => return self.comp.map_clear(header_ptr),
                        _ => {} // fall through
                    }
                }

                let recv_val = self.val(*recv);
                if let Some((fv, _, _)) = self.comp.fns.get(method).cloned() {
                    // Check if the method expects self by pointer (first param is ptr type)
                    let first_param_is_ptr = fv
                        .get_type()
                        .get_param_types()
                        .first()
                        .map(|t| t.is_pointer_type())
                        .unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> = if first_param_is_ptr && !recv_val.is_pointer_value() {
                        // Struct value but method expects pointer: alloca + store.
                        // Cache the alloca so mutations from the method persist across calls
                        // (e.g. iterator .next() mutating self.n in a loop).
                        if let Some(cached) = self.self_allocs.get(recv) {
                            (*cached).into()
                        } else {
                            let tmp = self.comp.entry_alloca(recv_val.get_type(), "self.tmp");
                            // Store must happen in the entry block (not in a loop body)
                            // so it only executes once.
                            let cur_fn = self.comp.cur_fn.unwrap();
                            let entry_bb = cur_fn.get_first_basic_block().unwrap();
                            let entry_bld = self.comp.ctx.create_builder();
                            if let Some(term) = entry_bb.get_terminator() {
                                entry_bld.position_before(&term);
                            } else {
                                entry_bld.position_at_end(entry_bb);
                            }
                            entry_bld.build_store(tmp, recv_val).unwrap();
                            self.self_allocs.insert(*recv, tmp);
                            tmp.into()
                        }
                    } else {
                        recv_val
                    };
                    let mut all_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                        vec![self_arg.into()];
                    for a in args {
                        all_args.push(self.val(*a).into());
                    }
                    let csv = b!(self.comp.bld.build_call(fv, &all_args, "mcall"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err(format!("mir_codegen: unknown method `{method}`"))
                }
            }
            mir::InstKind::IndirectCall(callee, args) => {
                let callee_val = self.val(*callee);
                // Closure call: callee is a {fn_ptr, env_ptr} struct.
                let _closure_ty = self.comp.closure_type();
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
                        Err(format!("mir_codegen: Load of undefined variable `{name}`"))
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
                let field_defs: Vec<(String, Type)> = self
                    .comp
                    .structs
                    .get(name)
                    .cloned()
                    .unwrap_or_default();
                let defaults = self.comp.struct_defaults.get(name).cloned();
                let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                // Track which field indices were explicitly provided.
                let mut provided = std::collections::HashSet::new();
                for (i, (fname, vid)) in fields.iter().enumerate() {
                    let v = self.val(*vid);
                    let idx = if fname.is_empty() {
                        provided.insert(i as u32);
                        i as u32
                    } else {
                        let pos = field_defs
                            .iter()
                            .position(|(n, _)| n == fname)
                            .ok_or_else(|| format!("mir_codegen: struct `{name}` has no field `{fname}`"))? as u32;
                        provided.insert(pos);
                        pos
                    };
                    let label = if fname.is_empty() {
                        field_defs.get(i).map(|s| s.0.as_str()).unwrap_or("field")
                    } else {
                        fname
                    };
                    agg = b!(self.comp.bld.build_insert_value(
                        agg.into_struct_value(),
                        v,
                        idx,
                        label
                    ))
                    .into_struct_value()
                    .into();
                }
                // Fill in defaults for missing fields.
                for (i, (fname, fty)) in field_defs.iter().enumerate() {
                    let idx = i as u32;
                    if provided.contains(&idx) {
                        continue;
                    }
                    let val = if let Some(def_expr) = defaults.as_ref().and_then(|d| d.get(fname)) {
                        self.comp.compile_expr(def_expr)?
                    } else {
                        self.comp.default_val(fty)
                    };
                    agg = b!(self.comp.bld.build_insert_value(
                        agg.into_struct_value(),
                        val,
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
                    // Look up variant field types for recursive-field detection.
                    let variant_field_types: Vec<Type> = self.comp.enums.get(enum_name)
                        .and_then(|vs| vs.iter().find(|(vn, _)| vn == variant))
                        .map(|(_, ftys)| ftys.clone())
                        .unwrap_or_default();
                    // Store payload fields at proper byte offsets based on actual type sizes.
                    let mut byte_offset: u64 = 0;
                    for (i, vid) in payload.iter().enumerate() {
                        let v = self.val(*vid);
                        let is_rec = variant_field_types.get(i)
                            .map(|fty| Compiler::is_recursive_field(fty, enum_name))
                            .unwrap_or(false);
                        let field_ptr = if byte_offset == 0 {
                            payload_gep
                        } else {
                            let offset_val = self.comp.ctx.i64_type().const_int(
                                byte_offset, false
                            );
                            unsafe {
                                b!(self.comp.bld.build_gep(
                                    self.comp.ctx.i8_type(),
                                    payload_gep,
                                    &[offset_val],
                                    "payload.elem"
                                ))
                            }
                        };
                        if is_rec {
                            // Box the recursive field: malloc, store value, store pointer.
                            let actual_ty = self.comp.llvm_ty(
                                variant_field_types.get(i).unwrap_or(&Type::I64)
                            );
                            let size = self.comp.type_store_size(actual_ty);
                            let malloc_fn = self.comp.ensure_malloc();
                            let heap = b!(self.comp.bld.build_call(
                                malloc_fn,
                                &[self.comp.ctx.i64_type().const_int(size, false).into()],
                                "box.alloc"
                            ))
                            .try_as_basic_value()
                            .basic()
                            .unwrap()
                            .into_pointer_value();
                            b!(self.comp.bld.build_store(heap, v));
                            b!(self.comp.bld.build_store(field_ptr, heap));
                            byte_offset += 8;
                        } else {
                            b!(self.comp.bld.build_store(field_ptr, v));
                            let type_size = v.get_type().size_of()
                                .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                .unwrap_or(8);
                            byte_offset += (type_size + 7) & !7;
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
                } else if obj_val.is_struct_value() {
                    // SSA struct value — use insert_value for immutable update.
                    let sv = obj_val.into_struct_value();
                    let struct_ty_name = sv.get_type().get_name()
                        .map(|n| n.to_str().unwrap_or("").to_string());
                    if let Some(name) = &struct_ty_name {
                        let field_idx = self.field_index(name, field);
                        let updated = b!(self.comp.bld.build_insert_value(
                            sv, v, field_idx, field
                        ));
                        return Ok(updated.into_struct_value().into());
                    }
                }
                Ok(void_val())
            }
            mir::InstKind::FieldStore(var_name, field, val) => {
                // Direct field store into a named variable's alloca.
                let v = self.val(*val);
                if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                    let struct_name = self.struct_name_from_type(&ty);
                    if let Some(name) = &struct_name {
                        if let Some(st) = self.comp.module.get_struct_type(name) {
                            let field_idx = self.field_index(name, field);
                            let gep = b!(self.comp.bld.build_struct_gep(
                                st, alloca, field_idx, field
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
                let base_ty = self.value_types.get(base);

                // String indexing: get char at index (returns byte as i64)
                if matches!(base_ty, Some(Type::String)) {
                    return self.comp.string_char_at(base_val, idx_val);
                }

                // For arrays: GEP into the array.
                if base_val.get_type().is_array_type() {
                    let arr_ty = base_val.get_type().into_array_type();
                    let arr_len = arr_ty.len() as u64;
                    let i64t = self.comp.ctx.i64_type();
                    // Wrap negative indices: if idx < 0, idx = len + idx
                    let idx_int = idx_val.into_int_value();
                    let is_neg = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::SLT, idx_int, i64t.const_int(0, false), "neg"));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(
                        idx_int, i64t.const_int(arr_len, false), "wrap"));
                    let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                        .into_int_value();
                    let alloca = self.comp.entry_alloca(arr_ty.into(), "idx.tmp");
                    b!(self.comp.bld.build_store(alloca, base_val));
                    let zero = i64t.const_int(0, false);
                    let ptr = unsafe {
                        b!(self.comp.bld.build_gep(
                            arr_ty, alloca, &[zero, final_idx], "idx.ptr"
                        ))
                    };
                    let elem_ty = self.comp.llvm_ty(&inst.ty);
                    Ok(b!(self.comp.bld.build_load(elem_ty, ptr, "idx.val")))
                } else if base_val.get_type().is_pointer_type() {
                    // Vec indexing: header is { ptr, len, cap }.
                    let header_ptr = base_val.into_pointer_value();
                    let header_ty = self.comp.vec_header_type();
                    let elem_ty = self.comp.llvm_ty(&inst.ty);
                    let i64t = self.comp.ctx.i64_type();
                    let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                    let ptr_gep = b!(self.comp.bld.build_struct_gep(
                        header_ty, header_ptr, 0, "vi.ptrp"
                    ));
                    let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vi.data"))
                        .into_pointer_value();
                    let len_gep = b!(self.comp.bld.build_struct_gep(
                        header_ty, header_ptr, 1, "vi.lenp"
                    ));
                    let len = b!(self.comp.bld.build_load(i64t, len_gep, "vi.len"))
                        .into_int_value();
                    // Wrap negative indices: if idx < 0, idx = len + idx
                    let idx_int = idx_val.into_int_value();
                    let is_neg = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::SLT, idx_int, i64t.const_int(0, false), "neg"));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(idx_int, len, "wrap"));
                    let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                        .into_int_value();
                    self.comp.emit_vec_bounds_check(final_idx, len)?;
                    let elem_gep = unsafe {
                        b!(self.comp.bld.build_gep(
                            elem_ty, data_ptr, &[final_idx], "vi.egep"
                        ))
                    };
                    Ok(b!(self.comp.bld.build_load(elem_ty, elem_gep, "vi.elem")))
                } else if base_val.is_struct_value() {
                    // Tuple indexing: extract element from struct value.
                    if let Some(idx_const) = idx_val.into_int_value().get_zero_extended_constant() {
                        let elem = b!(self.comp.bld.build_extract_value(
                            base_val.into_struct_value(),
                            idx_const as u32,
                            "tup.elem"
                        ));
                        Ok(elem)
                    } else {
                        // Dynamic index: store to alloca and GEP.
                        let st = base_val.get_type();
                        let alloca = self.comp.entry_alloca(st, "tup.idx");
                        b!(self.comp.bld.build_store(alloca, base_val));
                        let elem_ty = self.comp.llvm_ty(&inst.ty);
                        let zero = self.comp.ctx.i64_type().const_int(0, false);
                        let ptr = unsafe {
                            b!(self.comp.bld.build_gep(
                                st, alloca, &[zero, idx_val.into_int_value()], "tup.ptr"
                            ))
                        };
                        Ok(b!(self.comp.bld.build_load(elem_ty, ptr, "tup.val")))
                    }
                } else {
                    Ok(void_val())
                }
            }
            mir::InstKind::IndexSet(base, idx, val) => {
                let base_val = self.val(*base);
                let idx_val = self.val(*idx);
                let v = self.val(*val);
                if base_val.get_type().is_array_type() {
                    let arr_ty = base_val.get_type().into_array_type();
                    let arr_len = arr_ty.len() as u64;
                    let alloca = self.comp.entry_alloca(arr_ty.into(), "idxset.tmp");
                    b!(self.comp.bld.build_store(alloca, base_val));
                    let i64t = self.comp.ctx.i64_type();
                    let zero = i64t.const_int(0, false);
                    // Wrap negative indices
                    let idx_int = idx_val.into_int_value();
                    let is_neg = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::SLT, idx_int, zero, "neg"));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(
                        idx_int, i64t.const_int(arr_len, false), "wrap"));
                    let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                        .into_int_value();
                    let ptr = unsafe {
                        b!(self.comp.bld.build_gep(
                            arr_ty,
                            alloca,
                            &[zero, final_idx],
                            "idxset.ptr"
                        ))
                    };
                    b!(self.comp.bld.build_store(ptr, v));
                    // Load the modified array back so the mutation is visible.
                    let updated = b!(self.comp.bld.build_load(arr_ty, alloca, "idxset.updated"));
                    return Ok(updated);
                } else if base_val.get_type().is_pointer_type() {
                    // Vec: header is { ptr, len, cap }.
                    let header_ptr = base_val.into_pointer_value();
                    let header_ty = self.comp.vec_header_type();
                    let elem_ty = v.get_type();
                    let i64t = self.comp.ctx.i64_type();
                    let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                    let ptr_gep = b!(self.comp.bld.build_struct_gep(
                        header_ty, header_ptr, 0, "vis.ptrp"
                    ));
                    let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                        .into_pointer_value();
                    let len_gep = b!(self.comp.bld.build_struct_gep(
                        header_ty, header_ptr, 1, "vis.lenp"
                    ));
                    let len = b!(self.comp.bld.build_load(i64t, len_gep, "vis.len"))
                        .into_int_value();
                    self.comp.emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                    let elem_gep = unsafe {
                        b!(self.comp.bld.build_gep(
                            elem_ty, data_ptr, &[idx_val.into_int_value()], "vis.egep"
                        ))
                    };
                    b!(self.comp.bld.build_store(elem_gep, v));
                }
                Ok(void_val())
            }
            mir::InstKind::IndexStore(var_name, idx, val) => {
                // Direct index store into a named variable's alloca.
                let idx_val = self.val(*idx);
                let v = self.val(*val);
                if let Some((alloca, ty)) = self.var_allocs.get(var_name).cloned() {
                    let llvm_ty = self.comp.llvm_ty(&ty);
                    if llvm_ty.is_array_type() {
                        let arr_ty = llvm_ty.into_array_type();
                        let arr_len = arr_ty.len() as u64;
                        let i64t = self.comp.ctx.i64_type();
                        let zero = i64t.const_int(0, false);
                        // Wrap negative indices
                        let idx_int = idx_val.into_int_value();
                        let is_neg = b!(self.comp.bld.build_int_compare(
                            inkwell::IntPredicate::SLT, idx_int, zero, "neg"));
                        let wrapped = b!(self.comp.bld.build_int_nsw_add(
                            idx_int, i64t.const_int(arr_len, false), "wrap"));
                        let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                            .into_int_value();
                        let ptr = unsafe {
                            b!(self.comp.bld.build_gep(
                                arr_ty,
                                alloca,
                                &[zero, final_idx],
                                "idxstore.ptr"
                            ))
                        };
                        b!(self.comp.bld.build_store(ptr, v));
                    } else {
                        // Vec or other pointer-based type: load the header and index into data.
                        let header_ty = self.comp.vec_header_type();
                        let i64t = self.comp.ctx.i64_type();
                        let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                        let header_ptr = b!(self.comp.bld.build_load(ptr_ty, alloca, "vis.hdr"))
                            .into_pointer_value();
                        let ptr_gep = b!(self.comp.bld.build_struct_gep(
                            header_ty, header_ptr, 0, "vis.ptrp"
                        ));
                        let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                            .into_pointer_value();
                        let len_gep = b!(self.comp.bld.build_struct_gep(
                            header_ty, header_ptr, 1, "vis.lenp"
                        ));
                        let len = b!(self.comp.bld.build_load(i64t, len_gep, "vis.len"))
                            .into_int_value();
                        let elem_ty = v.get_type();
                        self.comp.emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                        let elem_gep = unsafe {
                            b!(self.comp.bld.build_gep(
                                elem_ty, data_ptr, &[idx_val.into_int_value()], "vis.egep"
                            ))
                        };
                        b!(self.comp.bld.build_store(elem_gep, v));
                    }
                }
                Ok(void_val())
            }

            // ── Cast / Ref / Deref ──
            mir::InstKind::Cast(val, target_ty) => {
                let v = self.val(*val);
                let target_llvm = self.comp.llvm_ty(target_ty);
                self.emit_cast(v, &inst.ty, target_ty, target_llvm)
            }
            mir::InstKind::StrictCast(val, target_ty) => {
                let v = self.val(*val);
                let target_llvm = self.comp.llvm_ty(target_ty);
                let casted = self.emit_cast(v, &inst.ty, target_ty, target_llvm)?;
                // Validate: cast back and compare to original to detect overflow.
                let source_llvm = v.get_type();
                if v.is_int_value() && casted.is_int_value() {
                    let back = self.emit_cast(casted, target_ty, &inst.ty, source_llvm)?;
                    let eq = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::EQ,
                        v.into_int_value(),
                        back.into_int_value(),
                        "strict.eq"
                    ));
                    // If not equal, trap
                    let cur_fn = self.comp.bld.get_insert_block().unwrap().get_parent().unwrap();
                    let ok_bb = self.comp.ctx.append_basic_block(cur_fn, "strict.ok");
                    let trap_bb = self.comp.ctx.append_basic_block(cur_fn, "strict.trap");
                    b!(self.comp.bld.build_conditional_branch(eq, ok_bb, trap_bb));
                    self.comp.bld.position_at_end(trap_bb);
                    if let Some(trap) = self.comp.module.get_function("llvm.trap") {
                        b!(self.comp.bld.build_call(trap, &[], ""));
                    }
                    b!(self.comp.bld.build_unreachable());
                    self.comp.bld.position_at_end(ok_bb);
                }
                Ok(casted)
            }
            mir::InstKind::Ref(val) => {
                let v = self.val(*val);
                let alloca = self.comp.entry_alloca(v.get_type(), "ref");
                b!(self.comp.bld.build_store(alloca, v));
                Ok(alloca.into())
            }
            mir::InstKind::Deref(val) => {
                let v = self.val(*val);
                if !v.is_pointer_value() {
                    return Err(format!("mir_codegen: Deref on non-pointer value {:?}", val));
                }
                // RC deref: skip refcount field, load from field 1
                let val_ty = self.value_types.get(val).cloned();
                if let Some(Type::Rc(ref inner)) = val_ty {
                    return self.comp.rc_deref(v, inner);
                }
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
                self.emit_slice(*base, *lo, *hi, &inst.ty)
            }

            // ── Collections ──
            mir::InstKind::VecNew(elems) => {
                self.emit_vec_new(elems, &inst.ty)
            }
            mir::InstKind::VecPush(vec, elem) => {
                let vec_val = self.val(*vec).into_pointer_value();
                let elem_val = self.val(*elem);
                let lty = elem_val.get_type();
                let elem_size = self.comp.type_store_size(lty);
                self.comp.vec_push_raw(vec_val, elem_val, lty, elem_size)?;
                Ok(void_val())
            }
            mir::InstKind::VecLen(vec) => {
                let vec_val = self.val(*vec);
                let i64t = self.comp.ctx.i64_type();
                let vec_ty = self.value_types.get(vec);
                if matches!(vec_ty, Some(Type::String)) || vec_val.is_struct_value() {
                    // String: struct { ptr, len, cap }; len is field 1.
                    self.comp.string_len(vec_val)
                } else if vec_val.is_pointer_value() {
                    // Vec (heap-allocated): ptr to {ptr, i64, i64}; len is field 1.
                    let header_ty = self.comp.vec_header_type();
                    let header_ptr = vec_val.into_pointer_value();
                    let len_gep = b!(self.comp.bld.build_struct_gep(header_ty, header_ptr, 1, "vl.len"));
                    Ok(b!(self.comp.bld.build_load(i64t, len_gep, "vl.v")))
                } else if vec_val.is_array_value() {
                    // Fixed-size array: length is known at compile time.
                    let arr_len = vec_val.into_array_value().get_type().len();
                    Ok(i64t.const_int(arr_len as u64, false).into())
                } else {
                    Err("mir_codegen: VecLen on non-vec/array value".into())
                }
            }
            mir::InstKind::MapInit => {
                self.comp.compile_map_new()
            }
            mir::InstKind::SetInit => {
                if let Some(fv) = self.comp.module.get_function("jade_set_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "set"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err("mir_codegen: SetInit used but jade_set_new runtime function not declared".into())
                }
            }
            mir::InstKind::PQInit => {
                if let Some(fv) = self.comp.module.get_function("jade_pq_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "pq"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err("mir_codegen: PQInit used but jade_pq_new runtime function not declared".into())
                }
            }
            mir::InstKind::DequeInit => {
                if let Some(fv) = self.comp.module.get_function("jade_deque_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "deque"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err("mir_codegen: DequeInit used but jade_deque_new runtime function not declared".into())
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
                if !args.is_empty() {
                    return Err(format!(
                        "mir_codegen: SpawnActor '{name}' has {} constructor args but actor spawn does not yet support arguments",
                        args.len()
                    ));
                }
                self.emit_spawn_actor(name)
            }
            mir::InstKind::ChanCreate(elem_ty, cap) => {
                self.emit_chan_create(elem_ty, cap.as_ref())
            }
            mir::InstKind::ChanSend(ch, val) => {
                self.emit_chan_send(*ch, *val)
            }
            mir::InstKind::ChanRecv(ch) => {
                self.emit_chan_recv(*ch, &inst.ty)
            }
            mir::InstKind::SelectArm(channels, has_default) => {
                // Select: build case array, call jade_select, return index.
                let ch_vids: Vec<mir::ValueId> = channels.clone();
                let dest = inst.dest.unwrap();
                self.emit_select(&ch_vids, dest, *has_default)
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
                self.emit_dyn_dispatch(*obj, trait_name, method, args, &inst.ty)
            }

            mir::InstKind::DynCoerce(inner, type_name, trait_name) => {
                let val = self.val(*inner);
                let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());

                let data_ptr = if val.is_pointer_value() {
                    val.into_pointer_value()
                } else {
                    let lty = val.get_type();
                    let alloc = self.comp.entry_alloca(lty, "dyn.data");
                    b!(self.comp.bld.build_store(alloc, val));
                    alloc
                };

                let vtable_ptr = self.comp
                    .vtables
                    .get(&(type_name.to_string(), trait_name.to_string()))
                    .map(|gv| gv.as_pointer_value())
                    .unwrap_or_else(|| ptr_ty.const_null());

                let fat_ty = self.comp.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
                let fat = fat_ty.const_zero();
                let fat = b!(self.comp.bld.build_insert_value(fat, data_ptr, 0, "dyn.fat.data"))
                    .into_struct_value();
                let fat = b!(self.comp.bld.build_insert_value(fat, vtable_ptr, 1, "dyn.fat.vtable"))
                    .into_struct_value();
                Ok(fat.into())
            }

            mir::InstKind::InlineAsm(template, args) => {
                let arg_vals: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = args.iter().map(|a| self.val(*a).into()).collect();
                let arg_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = arg_vals.iter().map(|v| {
                    match v {
                        inkwell::values::BasicMetadataValueEnum::IntValue(iv) => iv.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::FloatValue(fv) => fv.get_type().into(),
                        inkwell::values::BasicMetadataValueEnum::PointerValue(pv) => pv.get_type().into(),
                        _ => self.comp.ctx.i64_type().into(),
                    }
                }).collect();
                let ft = self.comp.ctx.void_type().fn_type(&arg_tys, false);
                let asm = self.comp.ctx.create_inline_asm(
                    ft,
                    template.clone(),
                    String::new(), // constraints
                    true,          // has side effects
                    false,         // needs aligned stack
                    None,          // dialect
                    false,         // can throw
                );
                b!(self.comp.bld.build_indirect_call(ft, asm, &arg_vals, ""));
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
                    } else if matches!(ret_ty, Type::Tuple(_)) && v.is_array_value() {
                        // Tuple return: coerce array → struct via alloca bitcast.
                        let alloca = self.comp.entry_alloca(v.get_type(), "tup.coerce");
                        b!(self.comp.bld.build_store(alloca, v));
                        let coerced = b!(self.comp.bld.build_load(expected, alloca, "tup.ret"));
                        b!(self.comp.bld.build_return(Some(&coerced)));
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
                let _switch = b!(self.comp.bld.build_switch(
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
            .unwrap_or_else(|| {
                panic!("MIR codegen: missing value for {:?} — this is a compiler bug", id);
            })
    }

    fn emit_string_const(&mut self, s: &str) -> Result<BasicValueEnum<'ctx>, String> {
        self.comp.compile_str_literal(s)
    }

    fn emit_binop(
        &mut self,
        op: mir::BinOp,
        lhs: mir::ValueId,
        rhs: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Check for struct operator overload dispatch (e.g. Vec2 + Vec2 → Vec2.add(other)).
        if let Some(Type::Struct(name, _)) = self.value_types.get(&lhs) {
            let method = match op {
                mir::BinOp::Add => Some("add"),
                mir::BinOp::Sub => Some("sub"),
                mir::BinOp::Mul => Some("mul"),
                mir::BinOp::Div => Some("div"),
                _ => None,
            };
            if let Some(method_name) = method {
                let fn_name = format!("{name}_{method_name}");
                if let Some((fv, _, _)) = self.comp.fns.get(&fn_name).cloned() {
                    let l = self.val(lhs);
                    let r = self.val(rhs);
                    let first_param_is_ptr = fv.get_type().get_param_types().first()
                        .map(|t| t.is_pointer_type()).unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> = if first_param_is_ptr && !l.is_pointer_value() {
                        let tmp = self.comp.entry_alloca(l.get_type(), "op.self");
                        b!(self.comp.bld.build_store(tmp, l));
                        tmp.into()
                    } else { l };
                    let csv = b!(self.comp.bld.build_call(fv, &[self_arg.into(), r.into()], &format!("{method_name}.call")));
                    return Ok(self.comp.call_result(csv));
                }
            }
        }

        // String concatenation: String + String.
        if matches!(op, mir::BinOp::Add) && matches!(result_ty, Type::String) {
            let l = self.val(lhs);
            let r = self.val(rhs);
            return self.comp.string_concat(l, r);
        }

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
                mir::BinOp::Exp => {
                    let f64t = self.comp.ctx.f64_type();
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
                    return Ok(self.comp.call_result(result));
                }
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
        // Check for struct operator overload dispatch for comparisons.
        if let Some(Type::Struct(name, _)) = self.value_types.get(&lhs) {
            let method = match op {
                mir::CmpOp::Lt => Some("less"),
                mir::CmpOp::Gt => Some("greater"),
                mir::CmpOp::Le => Some("less_eq"),
                mir::CmpOp::Ge => Some("greater_eq"),
                mir::CmpOp::Eq => Some("equal"),
                mir::CmpOp::Ne => Some("equal"), // call equal then negate
            };
            if let Some(method_name) = method {
                let fn_name = format!("{name}_{method_name}");
                if let Some((fv, _, _)) = self.comp.fns.get(&fn_name).cloned() {
                    let l = self.val(lhs);
                    let r = self.val(rhs);
                    let first_param_is_ptr = fv.get_type().get_param_types().first()
                        .map(|t| t.is_pointer_type()).unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> = if first_param_is_ptr && !l.is_pointer_value() {
                        let tmp = self.comp.entry_alloca(l.get_type(), "cmp.self");
                        b!(self.comp.bld.build_store(tmp, l));
                        tmp.into()
                    } else { l };
                    let csv = b!(self.comp.bld.build_call(fv, &[self_arg.into(), r.into()], "cmp.call"));
                    let result = self.comp.call_result(csv);
                    return if matches!(op, mir::CmpOp::Ne) {
                        Ok(b!(self.comp.bld.build_not(result.into_int_value(), "neq")).into())
                    } else {
                        Ok(result)
                    };
                }
            }
        }

        let l = self.val(lhs);
        let r = self.val(rhs);

        // String comparison: delegate to Compiler::string_eq which uses memcmp.
        let is_string_type = matches!(operand_ty, Type::String) ||
            matches!(operand_ty, Type::Struct(n, _) if n == "String");
        if l.is_struct_value() && is_string_type {
            let negate = matches!(op, mir::CmpOp::Ne);
            return self.comp.string_eq(l, r, negate);
        }

        // Determine comparison mode from the actual LLVM value type, not
        // from inst.ty (which is Bool — the result type, not operand type).
        if l.get_type().is_float_type() {
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
            // Use unsigned predicates for unsigned operand types, signed otherwise.
            let is_unsigned = matches!(operand_ty, Type::U8 | Type::U16 | Type::U32 | Type::U64);
            let pred = match (op, is_unsigned) {
                (mir::CmpOp::Eq, _) => inkwell::IntPredicate::EQ,
                (mir::CmpOp::Ne, _) => inkwell::IntPredicate::NE,
                (mir::CmpOp::Lt, false) => inkwell::IntPredicate::SLT,
                (mir::CmpOp::Lt, true)  => inkwell::IntPredicate::ULT,
                (mir::CmpOp::Gt, false) => inkwell::IntPredicate::SGT,
                (mir::CmpOp::Gt, true)  => inkwell::IntPredicate::UGT,
                (mir::CmpOp::Le, false) => inkwell::IntPredicate::SLE,
                (mir::CmpOp::Le, true)  => inkwell::IntPredicate::ULE,
                (mir::CmpOp::Ge, false) => inkwell::IntPredicate::SGE,
                (mir::CmpOp::Ge, true)  => inkwell::IntPredicate::UGE,
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

        // String field access — handle "length"/"len" via SSO-aware string_len
        let obj_ty = self.value_types.get(&obj).cloned();
        if matches!(&obj_ty, Some(Type::String)) {
            match field {
                "length" | "len" => return self.comp.string_len(obj_val),
                "data" => return self.comp.string_data(obj_val),
                _ => {}
            }
        }

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
                        // Tag is at struct index 0 (i32). Extend to i64
                        // since the MIR lowering declares __tag as Type::I64.
                        let tag_i32 = b!(self.comp.bld.build_extract_value(sv, 0, "tag"));
                        let i64t = self.comp.ctx.i64_type();
                        let val = b!(self.comp.bld.build_int_z_extend(
                            tag_i32.into_int_value(), i64t, "tag.ext"
                        ));
                        return Ok(val.into());
                    }
                    // Payload fields: _0, _1, ... — extract from the payload byte array.
                    // Offsets must match VariantInit which uses actual type sizes
                    // with 8-byte alignment.
                    if let Some(idx_str) = field.strip_prefix('_') {
                        if let Ok(idx) = idx_str.parse::<usize>() {
                            let st = sv.get_type();
                            let alloca = self.comp.entry_alloca(st.into(), "enum.tmp");
                            b!(self.comp.bld.build_store(alloca, sv));
                            let payload_gep = b!(self.comp.bld.build_struct_gep(
                                st, alloca, 1, "payload"
                            ));
                            let res_llvm = self.comp.llvm_ty(result_ty);
                            let byte_offset = self.compute_enum_payload_offset(name, idx);
                            let field_ptr = if byte_offset == 0 {
                                payload_gep
                            } else {
                                let offset_val = self.comp.ctx.i64_type().const_int(
                                    byte_offset, false
                                );
                                unsafe {
                                    b!(self.comp.bld.build_gep(
                                        self.comp.ctx.i8_type(),
                                        payload_gep,
                                        &[offset_val],
                                        "payload.field"
                                    ))
                                }
                            };
                            // Check if this field is a recursive reference (boxed as ptr).
                            let is_rec = Compiler::is_recursive_field(result_ty, name);
                            if is_rec {
                                let ptr_ty = self.comp.ctx.ptr_type(
                                    inkwell::AddressSpace::default()
                                );
                                let heap_ptr = b!(self.comp.bld.build_load(
                                    ptr_ty, field_ptr, "box.ptr"
                                )).into_pointer_value();
                                let val = b!(self.comp.bld.build_load(
                                    res_llvm, heap_ptr, field
                                ));
                                return Ok(val);
                            }
                            let val = b!(self.comp.bld.build_load(res_llvm, field_ptr, field));
                            return Ok(val);
                        }
                    }
                }
                let idx = self.field_index(name, field);
                let val = b!(self.comp.bld.build_extract_value(sv, idx, field));
                return Ok(val);
            }
            // Unknown struct type — cannot determine correct field index.
            Err(format!("mir_codegen: FieldGet on unknown struct type for field `{field}`"))
        } else if obj_val.is_pointer_value() {
            // Vec .length/.len: read len field from vec header.
            if matches!(field, "length" | "len") {
                if matches!(&obj_ty, Some(Type::Vec(_))) {
                    let header_ptr = obj_val.into_pointer_value();
                    let header_ty = self.comp.vec_header_type();
                    let i64t = self.comp.ctx.i64_type();
                    let len_gep = b!(self.comp.bld.build_struct_gep(
                        header_ty, header_ptr, 1, "vl.lenp"
                    ));
                    let len = b!(self.comp.bld.build_load(i64t, len_gep, "vl.len"));
                    return Ok(len);
                }
            }
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
                })
                .or_else(|| {
                    // Search value_types for the MIR ValueId's type (covers function parameters).
                    self.value_types.get(&obj).and_then(|ty| match ty {
                        Type::Ptr(inner) => match inner.as_ref() {
                            Type::Struct(name, _) => Some(name.clone()),
                            _ => None,
                        },
                        Type::Struct(name, _) => Some(name.clone()),
                        _ => None,
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
            // No fallback — loading from an unknown struct pointer at offset 0
            // silently produces wrong values for any field other than the first.
            Err(format!("mir_codegen: FieldGet on pointer to unknown struct type for field `{field}`"))
        } else if obj_val.is_array_value() {
            // Tuple — represented as an LLVM array [N x T].
            // Fields are named _0, _1, ...
            if let Some(idx_str) = field.strip_prefix('_') {
                if let Ok(idx) = idx_str.parse::<u32>() {
                    let val = b!(self.comp.bld.build_extract_value(
                        obj_val.into_array_value(), idx, field
                    ));
                    return Ok(val);
                }
            }
            Ok(obj_val)
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
        // Inline vec construction matching HIR codegen: allocate {ptr, i64, i64} header.
        let i64t = self.comp.ctx.i64_type();
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let header_ty = self.comp.vec_header_type();
        let malloc = self.comp.ensure_malloc();

        let header_size = i64t.const_int(24, false);
        let header_ptr = b!(self.comp.bld.build_call(malloc, &[header_size.into()], "vec.hdr"))
            .try_as_basic_value().basic().unwrap().into_pointer_value();

        let n = elems.len();
        let cap = if n == 0 { 0u64 } else { n.next_power_of_two() as u64 };

        if n > 0 {
            let lty = self.comp.llvm_ty(elem_ty);
            let elem_size = self.comp.type_store_size(lty);
            let buf_size = i64t.const_int(cap * elem_size, false);
            let buf = b!(self.comp.bld.build_call(malloc, &[buf_size.into()], "vec.buf"))
                .try_as_basic_value().basic().unwrap().into_pointer_value();

            for (i, vid) in elems.iter().enumerate() {
                let val = self.val(*vid);
                let gep = unsafe {
                    b!(self.comp.bld.build_gep(lty, buf, &[i64t.const_int(i as u64, false)], "vec.elem"))
                };
                b!(self.comp.bld.build_store(gep, val));
            }

            let ptr_gep = b!(self.comp.bld.build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.comp.bld.build_store(ptr_gep, buf));
        } else {
            let ptr_gep = b!(self.comp.bld.build_struct_gep(header_ty, header_ptr, 0, "vec.ptr"));
            b!(self.comp.bld.build_store(ptr_gep, ptr_ty.const_null()));
        }

        let len_gep = b!(self.comp.bld.build_struct_gep(header_ty, header_ptr, 1, "vec.len"));
        b!(self.comp.bld.build_store(len_gep, i64t.const_int(n as u64, false)));

        let cap_gep = b!(self.comp.bld.build_struct_gep(header_ty, header_ptr, 2, "vec.cap"));
        b!(self.comp.bld.build_store(cap_gep, i64t.const_int(cap, false)));

        Ok(header_ptr.into())
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

    fn emit_chan_create(&mut self, elem_ty: &Type, cap: Option<&mir::ValueId>) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.comp.ctx.i64_type();
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        if let Some(fv) = self.comp.module.get_function("jade_chan_create") {
            let elem_size = self.comp.llvm_ty(elem_ty).size_of().unwrap_or(
                i64t.const_int(8, false),
            );
            let capacity = if let Some(cap_id) = cap {
                self.val(*cap_id).into_int_value()
            } else {
                i64t.const_int(64, false) // default capacity
            };
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
            Type::Ptr(inner) => match inner.as_ref() {
                Type::Struct(name, _) => Some(name.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Compute the byte offset for enum payload field at `target_idx`,
    /// matching the VariantInit layout (8-byte aligned actual type sizes).
    /// When we don't have type info, defaults to `target_idx * 8`.
    fn compute_enum_payload_offset(
        &self,
        enum_name: &str,
        target_idx: usize,
    ) -> u64 {
        if let Some(variants) = self.comp.enums.get(enum_name) {
            for (_, field_types) in variants {
                if field_types.len() > target_idx {
                    let mut offset: u64 = 0;
                    for (i, fty) in field_types.iter().enumerate() {
                        if i == target_idx {
                            return offset;
                        }
                        let type_size = if Compiler::is_recursive_field(fty, enum_name) {
                            8 // pointer
                        } else {
                            self.comp.llvm_ty(fty).size_of()
                                .map(|s| s.get_zero_extended_constant().unwrap_or(8))
                                .unwrap_or(8)
                        };
                        offset += (type_size + 7) & !7;
                    }
                }
            }
        }
        (target_idx * 8) as u64
    }

    /// Emit dynamic dispatch: fat pointer vtable lookup and indirect call.
    fn emit_dyn_dispatch(
        &mut self,
        obj: mir::ValueId,
        trait_name: &str,
        method: &str,
        args: &[mir::ValueId],
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fat = self.val(obj);
        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
        let fat_ty = self.comp.ctx.struct_type(&[ptr_ty.into(), ptr_ty.into()], false);

        let tmp = self.comp.entry_alloca(fat_ty.into(), "dyn.tmp");
        b!(self.comp.bld.build_store(tmp, fat));
        let data_gep = b!(self.comp.bld.build_struct_gep(fat_ty, tmp, 0, "dyn.data.gep"));
        let data_ptr = b!(self.comp.bld.build_load(ptr_ty, data_gep, "dyn.data"))
            .into_pointer_value();
        let vtable_gep = b!(self.comp.bld.build_struct_gep(fat_ty, tmp, 1, "dyn.vtable.gep"));
        let vtable_ptr = b!(self.comp.bld.build_load(ptr_ty, vtable_gep, "dyn.vtable"))
            .into_pointer_value();

        let method_idx = self.comp.trait_method_order
            .get(trait_name)
            .and_then(|methods| methods.iter().position(|m| m == method))
            .unwrap_or(0) as u64;

        let fn_ptr_gep = unsafe {
            b!(self.comp.bld.build_gep(
                ptr_ty,
                vtable_ptr,
                &[self.comp.ctx.i64_type().const_int(method_idx, false)],
                "dyn.fn.gep"
            ))
        };
        let fn_ptr = b!(self.comp.bld.build_load(ptr_ty, fn_ptr_gep, "dyn.fn"))
            .into_pointer_value();

        let mut call_args: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
            vec![data_ptr.into()];
        for a in args {
            call_args.push(self.val(*a).into());
        }

        let ret_ty = self.comp.llvm_ty(result_ty);
        let mut param_tys: Vec<BasicMetadataTypeEnum<'ctx>> = vec![ptr_ty.into()];
        for a in args {
            param_tys.push(self.val(*a).get_type().into());
        }
        let fn_ty = ret_ty.fn_type(&param_tys, false);
        let result = b!(self.comp.bld.build_indirect_call(fn_ty, fn_ptr, &call_args, "dyn.call"));
        Ok(result
            .try_as_basic_value()
            .basic()
            .unwrap_or_else(|| self.comp.ctx.i64_type().const_int(0, false).into()))
    }

    /// Emit slice operation for Vec or String types.
    fn emit_slice(
        &mut self,
        base: mir::ValueId,
        lo: mir::ValueId,
        hi: mir::ValueId,
        result_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let base_val = self.val(base);
        let lo_val = self.val(lo);
        let hi_val = self.val(hi);

        match result_ty {
            Type::Vec(_) => {
                let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                let i64t = self.comp.ctx.i64_type();
                let slice_fn = self.comp.module.get_function("__jade_vec_slice")
                    .unwrap_or_else(|| {
                        let ft = ptr_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                        self.comp.module.add_function(
                            "__jade_vec_slice", ft, Some(Linkage::External),
                        )
                    });
                let result = b!(self.comp.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "slice"
                ));
                Ok(self.comp.call_result(result))
            }
            Type::String => {
                let st = self.comp.llvm_ty(&Type::String);
                let i64t = self.comp.ctx.i64_type();
                let slice_fn = self.comp.module.get_function("__jade_str_slice")
                    .unwrap_or_else(|| {
                        let ft = st.fn_type(&[st.into(), i64t.into(), i64t.into()], false);
                        self.comp.module.add_function(
                            "__jade_str_slice", ft, Some(Linkage::External),
                        )
                    });
                let result = b!(self.comp.bld.build_call(
                    slice_fn,
                    &[base_val.into(), lo_val.into(), hi_val.into()],
                    "str.slice"
                ));
                Ok(self.comp.call_result(result))
            }
            _ => Ok(self.comp.ctx.i8_type().const_int(0, false).into()),
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
        _result_ty: &Type,
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
        if let Some(rest) = name.strip_prefix("__store_query_") {
            return self.emit_store_query(rest, args).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_count_") {
            return self.emit_store_count(store_name).map(Some);
        }
        if let Some(store_name) = name.strip_prefix("__store_all_") {
            return self.emit_store_all(store_name).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_delete_") {
            return self.emit_store_delete(rest, args).map(Some);
        }
        if let Some(rest) = name.strip_prefix("__store_set_") {
            return self.emit_store_set(rest, args).map(Some);
        }
        // ── Transaction begin/commit (no-op at LLVM level) ──
        if name == "__txn_begin" || name == "__txn_commit" {
            return Ok(Some(self.comp.ctx.i8_type().const_int(0, false).into()));
        }
        // ── Channel close ──
        if name == "__chan_close" {
            if let Some(&ch_val) = args.first() {
                let ch_ptr = self.val(ch_val).into_pointer_value();
                let chan_close = self.comp.module.get_function("jade_chan_close")
                    .ok_or("jade_chan_close not declared")?;
                b!(self.comp.bld.build_call(chan_close, &[ch_ptr.into()], ""));
                return Ok(Some(self.comp.ctx.i8_type().const_int(0, false).into()));
            }
        }
        // ── Actor stop (close the actor's internal channel) ──
        if name == "__stop" {
            if let Some(&actor_val) = args.first() {
                let actor_ptr = self.val(actor_val).into_pointer_value();
                let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                let ch_ptr = b!(self.comp.bld.build_load(ptr_ty, actor_ptr, "stop.ch"))
                    .into_pointer_value();
                let chan_close = self.comp.module.get_function("jade_chan_close")
                    .ok_or("jade_chan_close not declared")?;
                b!(self.comp.bld.build_call(chan_close, &[ch_ptr.into()], ""));
                return Ok(Some(self.comp.ctx.i8_type().const_int(0, false).into()));
            }
        }
        // ── Atomic operations ──
        if name == "__atomic_load" {
            if let Some(&ptr_val) = args.first() {
                let ptr = self.val(ptr_val).into_pointer_value();
                let i64t = self.comp.ctx.i64_type();
                let load = b!(self.comp.bld.build_load(i64t, ptr, "atomic.load"));
                load.as_instruction_value()
                    .unwrap()
                    .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
                    .map_err(|_| "failed to set atomic ordering")?;
                return Ok(Some(load));
            }
        }
        if name == "__atomic_store" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]);
                let store = b!(self.comp.bld.build_store(ptr, val));
                store
                    .set_atomic_ordering(inkwell::AtomicOrdering::SequentiallyConsistent)
                    .map_err(|_| "failed to set atomic ordering")?;
                return Ok(Some(self.comp.ctx.i64_type().const_zero().into()));
            }
        }
        if name == "__atomic_add" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let old = b!(self.comp.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Add,
                    ptr, val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                return Ok(Some(old.into()));
            }
        }
        if name == "__atomic_sub" {
            if args.len() >= 2 {
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let old = b!(self.comp.bld.build_atomicrmw(
                    inkwell::AtomicRMWBinOp::Sub,
                    ptr, val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                return Ok(Some(old.into()));
            }
        }
        if name == "__atomic_cas" {
            if args.len() >= 3 {
                let ptr = self.val(args[0]).into_pointer_value();
                let expected = self.val(args[1]).into_int_value();
                let new_val = self.val(args[2]).into_int_value();
                let cas = b!(self.comp.bld.build_cmpxchg(
                    ptr, expected, new_val,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                    inkwell::AtomicOrdering::SequentiallyConsistent,
                ));
                let old = b!(self.comp.bld.build_extract_value(cas, 0, "cas.old"));
                return Ok(Some(old));
            }
        }
        // ── COW operations ──
        if name == "__cow_wrap" {
            if let Some(&inner_val_id) = args.first() {
                let val = self.val(inner_val_id);
                let inner_ty = self.value_types.get(&inner_val_id).cloned().unwrap_or(Type::I64);
                let data_ty = self.comp.llvm_ty(&inner_ty);
                let i64t = self.comp.ctx.i64_type();
                let cow_st = self.comp.ctx.struct_type(&[i64t.into(), data_ty], false);
                let malloc = self.comp.ensure_malloc();
                let size = cow_st.size_of().unwrap();
                let ptr = b!(self.comp.bld.build_call(malloc, &[size.into()], "cow.alloc"))
                    .try_as_basic_value().basic().unwrap().into_pointer_value();
                let rc_gep = b!(self.comp.bld.build_struct_gep(cow_st, ptr, 0, "cow.rc"));
                b!(self.comp.bld.build_store(rc_gep, i64t.const_int(1, false)));
                let data_gep = b!(self.comp.bld.build_struct_gep(cow_st, ptr, 1, "cow.data"));
                b!(self.comp.bld.build_store(data_gep, val));
                return Ok(Some(ptr.into()));
            }
        }
        if name == "__cow_clone" {
            if let Some(&inner_val_id) = args.first() {
                let cow_ptr = self.val(inner_val_id).into_pointer_value();
                let cow_ty = self.value_types.get(&inner_val_id).cloned().unwrap_or(Type::I64);
                let inner_ty = match &cow_ty {
                    Type::Cow(t) => t.as_ref().clone(),
                    other => other.clone(),
                };
                let data_ty = self.comp.llvm_ty(&inner_ty);
                let i64t = self.comp.ctx.i64_type();
                let cow_st = self.comp.ctx.struct_type(&[i64t.into(), data_ty], false);

                let rc_gep = b!(self.comp.bld.build_struct_gep(cow_st, cow_ptr, 0, "cow.rcp"));
                let rc = b!(self.comp.bld.build_load(i64t, rc_gep, "cow.rc")).into_int_value();
                let needs_clone = b!(self.comp.bld.build_int_compare(
                    inkwell::IntPredicate::UGT, rc, i64t.const_int(1, false), "cow.shared"
                ));

                let fn_val = self.comp.cur_fn.unwrap();
                let clone_bb = self.comp.ctx.append_basic_block(fn_val, "cow.clone");
                let done_bb = self.comp.ctx.append_basic_block(fn_val, "cow.done");
                let cur_bb = self.comp.bld.get_insert_block().unwrap();
                b!(self.comp.bld.build_conditional_branch(needs_clone, clone_bb, done_bb));

                self.comp.bld.position_at_end(clone_bb);
                let malloc = self.comp.ensure_malloc();
                let size = cow_st.size_of().unwrap();
                let new_ptr = b!(self.comp.bld.build_call(malloc, &[size.into()], "cow.new"))
                    .try_as_basic_value().basic().unwrap().into_pointer_value();
                let new_rc = b!(self.comp.bld.build_struct_gep(cow_st, new_ptr, 0, "cow.nrc"));
                b!(self.comp.bld.build_store(new_rc, i64t.const_int(1, false)));
                let new_data = b!(self.comp.bld.build_struct_gep(cow_st, new_ptr, 1, "cow.ndata"));
                let old_data = b!(self.comp.bld.build_struct_gep(cow_st, cow_ptr, 1, "cow.odata"));
                let old_val = b!(self.comp.bld.build_load(data_ty, old_data, "cow.oval"));
                b!(self.comp.bld.build_store(new_data, old_val));
                let dec = b!(self.comp.bld.build_int_sub(rc, i64t.const_int(1, false), "cow.dec"));
                b!(self.comp.bld.build_store(rc_gep, dec));
                b!(self.comp.bld.build_unconditional_branch(done_bb));

                self.comp.bld.position_at_end(done_bb);
                let ptr_t = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                let phi = b!(self.comp.bld.build_phi(ptr_t, "cow.result"));
                phi.add_incoming(&[(&cow_ptr, cur_bb), (&new_ptr, clone_bb)]);
                return Ok(Some(phi.as_basic_value()));
            }
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
        let _i64t = self.comp.ctx.i64_type();

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
        has_default: bool,
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
        let has_default = self.comp.ctx.bool_type().const_int(has_default as u64, false);
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
        // Name format: {store_name}__{field}__{op}[__and__{field2}__{op2}]*
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let store_name = parts[0];
        let field_name = parts[1];

        // Parse primary op and any extra conditions from parts[2]
        // parts[2] could be "eq" or "eq__and__val__gt" etc.
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_store_op(segments[0]);

        // Parse extra compound conditions: __and/or__field__op
        let mut extra_specs: Vec<(crate::ast::LogicalOp, &str, crate::ast::BinOp)> = Vec::new();
        let mut i = 1;
        while i + 2 < segments.len() {
            let lop = match segments[i] {
                "and" => crate::ast::LogicalOp::And,
                "or" => crate::ast::LogicalOp::Or,
                _ => { i += 1; continue; }
            };
            let efield = segments[i + 1];
            let eop = Self::parse_store_op(segments[i + 2]);
            extra_specs.push((lop, efield, eop));
            i += 3;
        }

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

        let cond = {
            // Build extras for compound filters
            let mut extras: Vec<(crate::ast::LogicalOp, usize, Type, crate::ast::BinOp, BasicValueEnum<'ctx>)> = Vec::new();
            for (ei, (lop, efield, eop)) in extra_specs.iter().enumerate() {
                let (eidx, ety) = sd.fields.iter().enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .ok_or_else(|| format!("unknown field '{efield}' in store '{store_name}'"))?;
                let eval = self.value_map[&args[ei + 1]];
                extras.push((*lop, eidx, ety, *eop, eval));
            }
            self.comp.eval_store_filter(
                rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?
        };
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

    fn parse_store_op(s: &str) -> crate::ast::BinOp {
        match s {
            "eq" => crate::ast::BinOp::Eq,
            "ne" => crate::ast::BinOp::Ne,
            "lt" => crate::ast::BinOp::Lt,
            "le" => crate::ast::BinOp::Le,
            "gt" => crate::ast::BinOp::Gt,
            "ge" => crate::ast::BinOp::Ge,
            _ => crate::ast::BinOp::Eq,
        }
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

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);

        let jade_name = format!("__store_{store_name}");
        let jade_st = self.comp.module.get_struct_type(&jade_name).unwrap();
        let jade_size = self.comp.type_store_size(jade_st.into());

        let count = self.comp.store_read_count(fp)?;
        let raw_buf = self.comp.store_load_records(fp, count, rec_size)?;

        let jade_total =
            b!(self.comp.bld.build_int_mul(count, i64t.const_int(jade_size, false), "all.jade_total"));
        let one = i64t.const_int(1, false);
        let jade_alloc = b!(self.comp.bld.build_select(
            b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::EQ, jade_total, i64t.const_int(0, false), "all.jade_isz"
            )),
            one, jade_total, "all.jade_alloc"
        )).into_int_value();
        let malloc_fn = self.comp.ensure_malloc();
        let jade_buf = self.comp.call_result(
            b!(self.comp.bld.build_call(malloc_fn, &[jade_alloc.into()], "all.jade"))
        ).into_pointer_value();

        let has_strings = sd.fields.iter().any(|f| matches!(f.ty, Type::String));

        if has_strings {
            let fv = self.comp.cur_fn.unwrap();
            let idx_ptr = self.comp.entry_alloca(i64t.into(), "all.idx");
            b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

            let loop_bb = self.comp.ctx.append_basic_block(fv, "all.loop");
            let body_bb = self.comp.ctx.append_basic_block(fv, "all.body");
            let done_bb = self.comp.ctx.append_basic_block(fv, "all.done");

            b!(self.comp.bld.build_unconditional_branch(loop_bb));
            self.comp.bld.position_at_end(loop_bb);
            let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "all.i")).into_int_value();
            let cmp = b!(self.comp.bld.build_int_compare(inkwell::IntPredicate::ULT, idx, count, "all.cmp"));
            b!(self.comp.bld.build_conditional_branch(cmp, body_bb, done_bb));

            self.comp.bld.position_at_end(body_bb);
            let raw_off = b!(self.comp.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "all.roff"));
            let raw_ptr = unsafe {
                b!(self.comp.bld.build_gep(self.comp.ctx.i8_type(), raw_buf, &[raw_off], "all.rptr"))
            };
            let jade_val = self.comp.load_store_record_as_jade(rec_st, raw_ptr, &sd)?;
            let jade_off = b!(self.comp.bld.build_int_mul(idx, i64t.const_int(jade_size, false), "all.joff"));
            let jade_ptr = unsafe {
                b!(self.comp.bld.build_gep(self.comp.ctx.i8_type(), jade_buf, &[jade_off], "all.jptr"))
            };
            b!(self.comp.bld.build_store(jade_ptr, jade_val));

            let next_idx = b!(self.comp.bld.build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.comp.bld.build_store(idx_ptr, next_idx));
            b!(self.comp.bld.build_unconditional_branch(loop_bb));

            self.comp.bld.position_at_end(done_bb);
        } else {
            let total = b!(self.comp.bld.build_int_mul(count, i64t.const_int(rec_size, false), "all.total"));
            let memcpy_fn = self.comp.ensure_memcpy();
            b!(self.comp.bld.build_call(memcpy_fn, &[jade_buf.into(), raw_buf.into(), total.into()], ""));
        }

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[raw_buf.into()], ""));

        Ok(jade_buf.into())
    }

    fn emit_store_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Name format: {store_name}__{field}__{op}[__and/or_{field2}__{op2}]*
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let store_name = parts[0];
        let field_name = parts[1];
        // rest = "eq__and_stock__lt" → split on "__" → ["eq", "and_stock", "lt"]
        let rest_parts: Vec<&str> = parts[2].split("__").collect();
        let primary_op = Self::parse_filter_op(rest_parts[0]);

        let sd = self.comp.store_defs.get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        // Parse extra filter conditions from the name.
        let mut extra_conds: Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)> = Vec::new();
        let mut ri = 1;
        while ri + 1 < rest_parts.len() {
            let logic_field = rest_parts[ri]; // "and_stock" or "or_value"
            let cop_str = rest_parts[ri + 1]; // "lt", "eq", etc.
            let (lop, fname) = if let Some(f) = logic_field.strip_prefix("and_") {
                (crate::ast::LogicalOp::And, f.to_string())
            } else if let Some(f) = logic_field.strip_prefix("or_") {
                (crate::ast::LogicalOp::Or, f.to_string())
            } else {
                ri += 1; continue;
            };
            let cop = Self::parse_filter_op(cop_str);
            extra_conds.push((lop, fname, cop));
            ri += 2;
        }

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.comp.module.get_function(&ensure_fn_name) {
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.comp.gen_store_ensure_open(&sd)?;
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.comp.load_store_fp(store_name)?;
        self.comp.store_lock(fp)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);

        let (field_idx, field_ty) = sd.fields.iter().enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.val(args[0]);

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        // Rewrite file: close and reopen in w+b mode
        let fclose_fn = self.comp.module.get_function("fclose").unwrap();
        b!(self.comp.bld.build_call(fclose_fn, &[fp.into()], ""));

        let filename = format!("{store_name}.store\0");
        let file_str = b!(self.comp.bld.build_global_string_ptr(&filename, "del.path"));
        let mode_wb = b!(self.comp.bld.build_global_string_ptr("w+b\0", "del.mode"));
        let fopen_fn = self.comp.module.get_function("fopen").unwrap();
        let new_fp = self.comp.call_result(
            b!(self.comp.bld.build_call(fopen_fn,
                &[file_str.as_pointer_value().into(), mode_wb.as_pointer_value().into()],
                "del.fp"))
        ).into_pointer_value();

        let global_name = format!("__store_{store_name}_fp");
        let global = self.comp.module.get_global(&global_name).unwrap();
        b!(self.comp.bld.build_store(global.as_pointer_value(), new_fp));

        // Write header: magic + count placeholder + rec_size
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        let magic = b!(self.comp.bld.build_global_string_ptr("JADESTR\0", "del.magic"));
        b!(self.comp.bld.build_call(fwrite_fn,
            &[magic.as_pointer_value().into(), i64t.const_int(1, false).into(), i64t.const_int(8, false).into(), new_fp.into()],
            ""));

        let new_count_ptr = self.comp.entry_alloca(i64t.into(), "del.newcount");
        b!(self.comp.bld.build_store(new_count_ptr, i64t.const_int(0, false)));
        b!(self.comp.bld.build_call(fwrite_fn,
            &[new_count_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""));

        let rec_size_ptr = self.comp.entry_alloca(i64t.into(), "del.recsz");
        b!(self.comp.bld.build_store(rec_size_ptr, i64t.const_int(rec_size, false)));
        b!(self.comp.bld.build_call(fwrite_fn,
            &[rec_size_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""));

        // Loop: keep records that DON'T match the filter
        let fv_fn = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "del.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv_fn, "del.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv_fn, "del.body");
        let keep_bb = self.comp.ctx.append_basic_block(fv_fn, "del.keep");
        let skip_bb = self.comp.ctx.append_basic_block(fv_fn, "del.skip");
        let done_bb = self.comp.ctx.append_basic_block(fv_fn, "del.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "del.i")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(inkwell::IntPredicate::ULT, idx, count, "del.cmp"));
        b!(self.comp.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset = b!(self.comp.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "del.off"));
        let rec_ptr = unsafe {
            b!(self.comp.bld.build_gep(self.comp.ctx.i8_type(), buf, &[offset], "del.rec"))
        };

        let matches = {
            let extras: Vec<(crate::ast::LogicalOp, usize, Type, crate::ast::BinOp, BasicValueEnum<'ctx>)> =
                extra_conds.iter().enumerate().map(|(ei, (lop, fname, cop))| {
                    let (fi, ft) = sd.fields.iter().enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = self.val(args[1 + ei]);
                    (*lop, fi, ft, *cop, ev)
                }).collect();
            self.comp.eval_store_filter(rec_ptr, st, field_idx, &field_ty, primary_op, filter_val, &extras)?
        };
        b!(self.comp.bld.build_conditional_branch(matches, skip_bb, keep_bb));

        self.comp.bld.position_at_end(keep_bb);
        b!(self.comp.bld.build_call(fwrite_fn,
            &[rec_ptr.into(), i64t.const_int(rec_size, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""));
        let kept = b!(self.comp.bld.build_load(i64t, new_count_ptr, "kept")).into_int_value();
        let kept_inc = b!(self.comp.bld.build_int_add(kept, i64t.const_int(1, false), "kept.inc"));
        b!(self.comp.bld.build_store(new_count_ptr, kept_inc));
        b!(self.comp.bld.build_unconditional_branch(skip_bb));

        self.comp.bld.position_at_end(skip_bb);
        let next_idx = b!(self.comp.bld.build_int_add(idx, i64t.const_int(1, false), "del.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        // Update count in header
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(fseek_fn,
            &[new_fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""));
        b!(self.comp.bld.build_call(fwrite_fn,
            &[new_count_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        self.comp.store_unlock(fp)?;
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    fn emit_store_set(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Name format: {store_name}__{field}__{op}[__and/or_{f2}__{op2}]*__fields_{f1}_{f2}_...
        // Split out the __fields_ suffix first.
        let (filter_part, fields_part) = if let Some(pos) = encoded_name.find("__fields_") {
            (&encoded_name[..pos], &encoded_name[pos + 9..]) // skip "__fields_"
        } else {
            return Err(format!("mir_codegen: malformed store.set name '{encoded_name}'"));
        };

        let field_names: Vec<&str> = fields_part.split('_').collect();

        let parts: Vec<&str> = filter_part.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let store_name = parts[0];
        let filter_field = parts[1];
        let rest_parts: Vec<&str> = parts[2].split("__").collect();
        let primary_op = Self::parse_filter_op(rest_parts[0]);

        let sd = self.comp.store_defs.get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        // Parse extra filter conditions.
        let mut extra_conds: Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)> = Vec::new();
        let mut ri = 1;
        while ri + 1 < rest_parts.len() {
            let logic_field = rest_parts[ri];
            let cop_str = rest_parts[ri + 1];
            let (lop, fname) = if let Some(f) = logic_field.strip_prefix("and_") {
                (crate::ast::LogicalOp::And, f.to_string())
            } else if let Some(f) = logic_field.strip_prefix("or_") {
                (crate::ast::LogicalOp::Or, f.to_string())
            } else {
                ri += 1; continue;
            };
            let cop = Self::parse_filter_op(cop_str);
            extra_conds.push((lop, fname, cop));
            ri += 2;
        }
        let extra_count = extra_conds.len();

        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.comp.module.get_function(&ensure_fn_name) {
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        } else {
            let ensure_fn = self.comp.gen_store_ensure_open(&sd)?;
            b!(self.comp.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.comp.load_store_fp(store_name)?;
        self.comp.store_lock(fp)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);

        let (field_idx, field_ty) = sd.fields.iter().enumerate()
            .find(|(_, f)| f.name == filter_field)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{filter_field}' in store '{store_name}'"))?;

        let filter_val = self.val(args[0]);
        // args[1..1+extra_count] are extra filter vals
        // args[1+extra_count..] are the field assignment values
        let assign_start = 1 + extra_count;

        // Pre-gather field assignment values
        let mut assign_vals: Vec<(usize, &str, BasicValueEnum<'ctx>)> = Vec::new();
        for (i, fname) in field_names.iter().enumerate() {
            let arg_idx = assign_start + i;
            if arg_idx >= args.len() { break; }
            let val = self.val(args[arg_idx]);
            let field_pos = sd.fields.iter().position(|f| f.name == *fname)
                .ok_or_else(|| format!("unknown field '{fname}' in store '{store_name}'"))?;
            assign_vals.push((field_pos, fname, val));
        }

        let fseek_fn = self.comp.module.get_function("fseek").unwrap();

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        let fv = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "set.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "set.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "set.body");
        let update_bb = self.comp.ctx.append_basic_block(fv, "set.update");
        let next_bb = self.comp.ctx.append_basic_block(fv, "set.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "set.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "set.i")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::ULT, idx, count, "set.cmp"));
        b!(self.comp.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset = b!(self.comp.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "set.off"));
        let rec_ptr = unsafe {
            b!(self.comp.bld.build_gep(self.comp.ctx.i8_type(), buf, &[offset], "set.rec"))
        };
        let matches = {
            let extras: Vec<(crate::ast::LogicalOp, usize, Type, crate::ast::BinOp, BasicValueEnum<'ctx>)> =
                extra_conds.iter().enumerate().map(|(ei, (lop, fname, cop))| {
                    let (fi, ft) = sd.fields.iter().enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = self.val(args[1 + ei]);
                    (*lop, fi, ft, *cop, ev)
                }).collect();
            self.comp.eval_store_filter(rec_ptr, st, field_idx, &field_ty, primary_op, filter_val, &extras)?
        };
        b!(self.comp.bld.build_conditional_branch(matches, update_bb, next_bb));

        self.comp.bld.position_at_end(update_bb);
        for (fpos, _fname, val) in &assign_vals {
            let fty = &sd.fields[*fpos].ty;
            let gep = b!(self.comp.bld.build_struct_gep(st, rec_ptr, *fpos as u32, "set.assign"));
            match fty {
                Type::String => {
                    self.comp.copy_string_to_fixed_buf(*val, gep)?;
                }
                _ => {
                    b!(self.comp.bld.build_store(gep, *val));
                }
            }
        }
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self.comp.bld.build_int_add(idx, i64t.const_int(1, false), "set.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        b!(self.comp.bld.build_call(fseek_fn,
            &[fp.into(), i64t.const_int(super::stores::HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""));
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(fwrite_fn,
            &[buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.comp.store_unlock(fp)?;
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    /// Parse a filter op string back to a BinOp.
    fn parse_filter_op(s: &str) -> crate::ast::BinOp {
        match s {
            "eq" => crate::ast::BinOp::Eq,
            "ne" => crate::ast::BinOp::Ne,
            "lt" => crate::ast::BinOp::Lt,
            "le" => crate::ast::BinOp::Le,
            "gt" => crate::ast::BinOp::Gt,
            "ge" => crate::ast::BinOp::Ge,
            _ => crate::ast::BinOp::Eq,
        }
    }

    /// Handle overflow builtins that MIR lowered as `__builtin_WrappingAdd` etc.
    fn try_handle_overflow_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        let builtin_name = match name.strip_prefix("__builtin_") {
            Some(n) => n,
            None => return Ok(None),
        };
        // ── Bit intrinsics (1 arg) ──
        match builtin_name {
            "Bswap" | "Popcount" | "Clz" | "Ctz" | "RotateLeft" | "RotateRight" => {
                return self.try_handle_bit_builtin(builtin_name, args);
            }
            "Likely" | "Unlikely" => {
                if args.is_empty() { return Ok(None); }
                let cond = self.val(args[0]);
                let i1ty = self.comp.ctx.bool_type();
                let ft = i1ty.fn_type(&[i1ty.into(), i1ty.into()], false);
                let expect_fn = self.comp.module.get_function("llvm.expect.i1")
                    .unwrap_or_else(|| self.comp.module.add_function("llvm.expect.i1", ft, None));
                let expected = if builtin_name == "Likely" {
                    i1ty.const_int(1, false)
                } else {
                    i1ty.const_int(0, false)
                };
                let r = b!(self.comp.bld.build_call(expect_fn, &[cond.into(), expected.into()], "expect"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "PoolNew" => {
                if args.len() != 2 { return Ok(None); }
                let obj_size = self.val(args[0]).into_int_value();
                let count = self.val(args[1]).into_int_value();
                let ptr_t = self.comp.ctx.ptr_type(AddressSpace::default());
                let i64t = self.comp.ctx.i64_type();
                let ft = ptr_t.fn_type(&[i64t.into(), i64t.into()], false);
                let func = self.comp.module.get_function("jade_pool_create")
                    .unwrap_or_else(|| self.comp.module.add_function("jade_pool_create", ft, Some(Linkage::External)));
                let r = b!(self.comp.bld.build_call(func, &[obj_size.into(), count.into()], "pool.new"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "PoolAlloc" => {
                if args.is_empty() { return Ok(None); }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let ptr_t = self.comp.ctx.ptr_type(AddressSpace::default());
                let ft = ptr_t.fn_type(&[ptr_t.into()], false);
                let func = self.comp.module.get_function("jade_pool_alloc")
                    .unwrap_or_else(|| self.comp.module.add_function("jade_pool_alloc", ft, Some(Linkage::External)));
                let r = b!(self.comp.bld.build_call(func, &[pool_ptr.into()], "pool.alloc"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "PoolFree" => {
                if args.len() != 2 { return Ok(None); }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let obj_ptr = self.val(args[1]).into_pointer_value();
                let ptr_t = self.comp.ctx.ptr_type(AddressSpace::default());
                let void_t = self.comp.ctx.void_type();
                let ft = void_t.fn_type(&[ptr_t.into(), ptr_t.into()], false);
                let func = self.comp.module.get_function("jade_pool_free")
                    .unwrap_or_else(|| self.comp.module.add_function("jade_pool_free", ft, Some(Linkage::External)));
                b!(self.comp.bld.build_call(func, &[pool_ptr.into(), obj_ptr.into()], ""));
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
            "PoolDestroy" => {
                if args.is_empty() { return Ok(None); }
                let pool_ptr = self.val(args[0]).into_pointer_value();
                let ptr_t = self.comp.ctx.ptr_type(AddressSpace::default());
                let void_t = self.comp.ctx.void_type();
                let ft = void_t.fn_type(&[ptr_t.into()], false);
                let func = self.comp.module.get_function("jade_pool_destroy")
                    .unwrap_or_else(|| self.comp.module.add_function("jade_pool_destroy", ft, Some(Linkage::External)));
                b!(self.comp.bld.build_call(func, &[pool_ptr.into()], ""));
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
            "ToString" => {
                if args.is_empty() { return Ok(None); }
                let val = self.val(args[0]);
                let val_ty = self.value_types.get(&args[0]).cloned().unwrap_or(Type::I64);
                return Ok(Some(self.emit_to_string(val, &val_ty)?));
            }
            "FmtHex" => {
                if args.is_empty() { return Ok(None); }
                let val = self.val(args[0]).into_int_value();
                let i64t = self.comp.ctx.i64_type();
                let wide = if val.get_type().get_bit_width() < 64 {
                    b!(self.comp.bld.build_int_s_extend(val, i64t, "fw")).into()
                } else { val.into() };
                return Ok(Some(self.comp.snprintf_to_string("%lx", &[wide], "fh")?));
            }
            "FmtOct" => {
                if args.is_empty() { return Ok(None); }
                let val = self.val(args[0]).into_int_value();
                let i64t = self.comp.ctx.i64_type();
                let wide = if val.get_type().get_bit_width() < 64 {
                    b!(self.comp.bld.build_int_s_extend(val, i64t, "fw")).into()
                } else { val.into() };
                return Ok(Some(self.comp.snprintf_to_string("%lo", &[wide], "fo")?));
            }
            "FmtBin" => {
                if args.is_empty() { return Ok(None); }
                let val = self.val(args[0]).into_int_value();
                return Ok(Some(self.emit_fmt_bin(val)?));
            }
            "FmtFloat" => {
                if args.len() < 2 { return Ok(None); }
                let x = self.val(args[0]).into_float_value();
                let decimals = self.val(args[1]).into_int_value();
                let dec_i32 = b!(self.comp.bld.build_int_truncate(decimals, self.comp.ctx.i32_type(), "dec32"));
                return Ok(Some(self.comp.snprintf_to_string("%.*f", &[dec_i32.into(), x.into()], "ff")?));
            }
            "TimeMonotonic" => {
                return Ok(Some(self.comp.compile_time_monotonic()?));
            }
            "SleepMs" => {
                if args.is_empty() { return Ok(None); }
                let ms = self.val(args[0]).into_int_value();
                return Ok(Some(self.emit_sleep_ms(ms)?));
            }
            "GetArgs" => {
                return Ok(Some(self.comp.compile_get_args()?));
            }
            "StringFromRaw" => {
                if args.len() < 2 { return Ok(None); }
                let ptr = self.val(args[0]);
                let len = self.val(args[1]);
                let cap = if args.len() > 2 { self.val(args[2]) } else { len };
                return Ok(Some(self.comp.build_string(ptr, len, cap, "sfr")?));
            }
            "StringFromPtr" => {
                if args.is_empty() { return Ok(None); }
                let ptr = self.val(args[0]);
                let i64t = self.comp.ctx.i64_type();
                let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                let strlen = self.comp.module.get_function("strlen").unwrap_or_else(|| {
                    self.comp.module.add_function(
                        "strlen",
                        i64t.fn_type(&[ptr_ty.into()], false),
                        Some(inkwell::module::Linkage::External),
                    )
                });
                let len = b!(self.comp.bld.build_call(strlen, &[ptr.into()], "sfp.len"))
                    .try_as_basic_value().basic().unwrap().into_int_value();
                let size = b!(self.comp.bld.build_int_nsw_add(len, i64t.const_int(1, false), "sfp.sz"));
                let malloc = self.comp.ensure_malloc();
                let buf = b!(self.comp.bld.build_call(malloc, &[size.into()], "sfp.buf"))
                    .try_as_basic_value().basic().unwrap();
                let memcpy = self.comp.ensure_memcpy();
                b!(self.comp.bld.build_call(memcpy, &[buf.into(), ptr.into(), size.into()], ""));
                return Ok(Some(self.comp.build_string(buf, len, size, "sfp")?));
            }
            "VolatileLoad" => {
                if args.is_empty() { return Ok(None); }
                let ptr = self.val(args[0]).into_pointer_value();
                let i64t = self.comp.ctx.i64_type();
                let load = b!(self.comp.bld.build_load(i64t, ptr, "vload"));
                load.as_instruction_value().unwrap().set_volatile(true).unwrap();
                return Ok(Some(load));
            }
            "VolatileStore" => {
                if args.len() < 2 { return Ok(None); }
                let ptr = self.val(args[0]).into_pointer_value();
                let val = self.val(args[1]).into_int_value();
                let store_inst = b!(self.comp.bld.build_store(ptr, val));
                store_inst.set_volatile(true).unwrap();
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalHandle" => {
                if args.len() < 2 { return Ok(None); }
                let signum = self.val(args[0]).into_int_value();
                let handler = self.val(args[1]).into_pointer_value();
                let ptr_t = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.comp.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.comp.bld.build_int_truncate(signum, i32t, "sig32"));
                let func = self.comp.module.get_function("signal")
                    .unwrap_or_else(|| self.comp.module.add_function("signal", ft, Some(inkwell::module::Linkage::External)));
                b!(self.comp.bld.build_call(func, &[sig32.into(), handler.into()], ""));
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
            "SignalRaise" => {
                if args.is_empty() { return Ok(None); }
                let signum = self.val(args[0]).into_int_value();
                let i32t = self.comp.ctx.i32_type();
                let ft = i32t.fn_type(&[i32t.into()], false);
                let sig32 = b!(self.comp.bld.build_int_truncate(signum, i32t, "sig32"));
                let func = self.comp.module.get_function("raise")
                    .unwrap_or_else(|| self.comp.module.add_function("raise", ft, Some(inkwell::module::Linkage::External)));
                let r = b!(self.comp.bld.build_call(func, &[sig32.into()], "raise"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "SignalIgnore" => {
                if args.is_empty() { return Ok(None); }
                let signum = self.val(args[0]).into_int_value();
                let ptr_t = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
                let i32t = self.comp.ctx.i32_type();
                let ft = ptr_t.fn_type(&[i32t.into(), ptr_t.into()], false);
                let sig32 = b!(self.comp.bld.build_int_truncate(signum, i32t, "sig32"));
                let sig_ign = b!(self.comp.bld.build_int_to_ptr(
                    self.comp.ctx.i64_type().const_int(1, false), ptr_t, "sig_ign")); // SIG_IGN = 1
                let func = self.comp.module.get_function("signal")
                    .unwrap_or_else(|| self.comp.module.add_function("signal", ft, Some(inkwell::module::Linkage::External)));
                b!(self.comp.bld.build_call(func, &[sig32.into(), sig_ign.into()], ""));
                return Ok(Some(self.comp.ctx.i64_type().const_int(0, false).into()));
            }
            "Ln" | "Log2" | "Log10" | "Exp" | "Exp2" => {
                if args.is_empty() { return Ok(None); }
                let x = self.val(args[0]).into_float_value();
                let f64t = self.comp.ctx.f64_type();
                let intrinsic = match builtin_name {
                    "Ln" => "llvm.log.f64",
                    "Log2" => "llvm.log2.f64",
                    "Log10" => "llvm.log10.f64",
                    "Exp" => "llvm.exp.f64",
                    "Exp2" => "llvm.exp2.f64",
                    _ => unreachable!(),
                };
                let ft = f64t.fn_type(&[f64t.into()], false);
                let func = self.comp.module.get_function(intrinsic)
                    .unwrap_or_else(|| self.comp.module.add_function(intrinsic, ft, None));
                let r = b!(self.comp.bld.build_call(func, &[x.into()], "math"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "PowF" | "Copysign" => {
                if args.len() < 2 { return Ok(None); }
                let x = self.val(args[0]).into_float_value();
                let y = self.val(args[1]).into_float_value();
                let f64t = self.comp.ctx.f64_type();
                let intrinsic = match builtin_name {
                    "PowF" => "llvm.pow.f64",
                    "Copysign" => "llvm.copysign.f64",
                    _ => unreachable!(),
                };
                let ft = f64t.fn_type(&[f64t.into(), f64t.into()], false);
                let func = self.comp.module.get_function(intrinsic)
                    .unwrap_or_else(|| self.comp.module.add_function(intrinsic, ft, None));
                let r = b!(self.comp.bld.build_call(func, &[x.into(), y.into()], "math"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            "Fma" => {
                if args.len() < 3 { return Ok(None); }
                let a = self.val(args[0]).into_float_value();
                let b_val = self.val(args[1]).into_float_value();
                let c = self.val(args[2]).into_float_value();
                let f64t = self.comp.ctx.f64_type();
                let ft = f64t.fn_type(&[f64t.into(), f64t.into(), f64t.into()], false);
                let func = self.comp.module.get_function("llvm.fma.f64")
                    .unwrap_or_else(|| self.comp.module.add_function("llvm.fma.f64", ft, None));
                let r = b!(self.comp.bld.build_call(func, &[a.into(), b_val.into(), c.into()], "fma"))
                    .try_as_basic_value().basic().unwrap();
                return Ok(Some(r));
            }
            _ => {}
        }
        if args.len() != 2 {
            return Ok(None);
        }
        let lhs = self.val(args[0]).into_int_value();
        let rhs = self.val(args[1]).into_int_value();
        let result = match builtin_name {
            // Wrapping ops — just normal LLVM int arithmetic (wraps naturally)
            "WrappingAdd" => b!(self.comp.bld.build_int_add(lhs, rhs, "wrap.add")),
            "WrappingSub" => b!(self.comp.bld.build_int_sub(lhs, rhs, "wrap.sub")),
            "WrappingMul" => b!(self.comp.bld.build_int_mul(lhs, rhs, "wrap.mul")),
            // Saturating ops — use LLVM intrinsics
            "SaturatingAdd" => {
                let bw = lhs.get_type().get_bit_width();
                let name = format!("llvm.sadd.sat.i{bw}");
                let ft = lhs.get_type().fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&name).unwrap_or_else(|| self.comp.module.add_function(&name, ft, None));
                b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "sat.add"))
                    .try_as_basic_value().basic().unwrap().into_int_value()
            }
            "SaturatingSub" => {
                let bw = lhs.get_type().get_bit_width();
                let name = format!("llvm.ssub.sat.i{bw}");
                let ft = lhs.get_type().fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&name).unwrap_or_else(|| self.comp.module.add_function(&name, ft, None));
                b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "sat.sub"))
                    .try_as_basic_value().basic().unwrap().into_int_value()
            }
            "SaturatingMul" => {
                // No LLVM intrinsic for sat mul; use checked mul + select
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self.comp.ctx.struct_type(&[lhs.get_type().into(), self.comp.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&intr).unwrap_or_else(|| self.comp.module.add_function(&intr, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "smul"))
                    .try_as_basic_value().basic().unwrap().into_struct_value();
                let val = b!(self.comp.bld.build_extract_value(r, 0, "smul.val")).into_int_value();
                let ovf = b!(self.comp.bld.build_extract_value(r, 1, "smul.ovf")).into_int_value();
                let max_val = lhs.get_type().const_int(i64::MAX as u64, false);
                b!(self.comp.bld.build_select(ovf, max_val, val, "sat.mul")).into_int_value()
            }
            // Checked ops — return {value, overflow_flag}
            "CheckedAdd" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.sadd.with.overflow.i{bw}");
                let ovf_ty = self.comp.ctx.struct_type(&[lhs.get_type().into(), self.comp.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&intr).unwrap_or_else(|| self.comp.module.add_function(&intr, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "cadd"))
                    .try_as_basic_value().basic().unwrap().into_struct_value();
                // Return just the value; overflow info is in the struct
                b!(self.comp.bld.build_extract_value(r, 0, "cadd.val")).into_int_value()
            }
            "CheckedSub" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.ssub.with.overflow.i{bw}");
                let ovf_ty = self.comp.ctx.struct_type(&[lhs.get_type().into(), self.comp.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&intr).unwrap_or_else(|| self.comp.module.add_function(&intr, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "csub"))
                    .try_as_basic_value().basic().unwrap().into_struct_value();
                b!(self.comp.bld.build_extract_value(r, 0, "csub.val")).into_int_value()
            }
            "CheckedMul" => {
                let bw = lhs.get_type().get_bit_width();
                let intr = format!("llvm.smul.with.overflow.i{bw}");
                let ovf_ty = self.comp.ctx.struct_type(&[lhs.get_type().into(), self.comp.ctx.bool_type().into()], false);
                let ft = ovf_ty.fn_type(&[lhs.get_type().into(), rhs.get_type().into()], false);
                let f = self.comp.module.get_function(&intr).unwrap_or_else(|| self.comp.module.add_function(&intr, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[lhs.into(), rhs.into()], "cmul"))
                    .try_as_basic_value().basic().unwrap().into_struct_value();
                b!(self.comp.bld.build_extract_value(r, 0, "cmul.val")).into_int_value()
            }
            _ => return Ok(None),
        };
        Ok(Some(result.into()))
    }

    /// Handle bit intrinsics: bswap, popcount, clz, ctz, rotate_left, rotate_right.
    fn try_handle_bit_builtin(
        &mut self,
        name: &str,
        args: &[mir::ValueId],
    ) -> Result<Option<BasicValueEnum<'ctx>>, String> {
        if args.is_empty() {
            return Ok(None);
        }
        let val = self.val(args[0]).into_int_value();
        let bw = val.get_type().get_bit_width();
        let it = val.get_type();
        match name {
            "Bswap" => {
                let llvm_name = format!("llvm.bswap.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into()], "bswap"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            "Popcount" => {
                let llvm_name = format!("llvm.ctpop.i{bw}");
                let ft = it.fn_type(&[it.into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into()], "popcount"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            "Clz" => {
                let llvm_name = format!("llvm.ctlz.i{bw}");
                let false_val = self.comp.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.comp.ctx.bool_type().into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into(), false_val.into()], "clz"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            "Ctz" => {
                let llvm_name = format!("llvm.cttz.i{bw}");
                let false_val = self.comp.ctx.bool_type().const_int(0, false);
                let ft = it.fn_type(&[it.into(), self.comp.ctx.bool_type().into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into(), false_val.into()], "ctz"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            "RotateLeft" => {
                if args.len() < 2 { return Ok(None); }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshl.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into(), val.into(), amt.into()], "rotl"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            "RotateRight" => {
                if args.len() < 2 { return Ok(None); }
                let amt = self.val(args[1]).into_int_value();
                let llvm_name = format!("llvm.fshr.i{bw}");
                let ft = it.fn_type(&[it.into(), it.into(), it.into()], false);
                let f = self.comp.module.get_function(&llvm_name)
                    .unwrap_or_else(|| self.comp.module.add_function(&llvm_name, ft, None));
                let r = b!(self.comp.bld.build_call(f, &[val.into(), val.into(), amt.into()], "rotr"))
                    .try_as_basic_value().basic().unwrap();
                Ok(Some(r))
            }
            _ => Ok(None),
        }
    }

    /// Convert a value to a String, matching HIR codegen's compile_to_string.
    fn emit_to_string(
        &mut self,
        val: BasicValueEnum<'ctx>,
        ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        match ty {
            Type::String => Ok(val),
            Type::I64 | Type::I32 | Type::I16 | Type::I8 => self.comp.int_to_string(val, false),
            Type::U64 | Type::U32 | Type::U16 | Type::U8 => self.comp.int_to_string(val, true),
            Type::F64 | Type::F32 => self.comp.float_to_string(val),
            Type::Bool => self.comp.bool_to_string(val),
            Type::Struct(name, _) => {
                let fn_name = format!("{name}_display");
                if let Some((fv, _, _)) = self.comp.fns.get(&fn_name).cloned() {
                    let first_param_is_ptr = fv.get_type().get_param_types().first()
                        .map(|t| t.is_pointer_type()).unwrap_or(false);
                    let self_arg: BasicValueEnum<'ctx> = if first_param_is_ptr && !val.is_pointer_value() {
                        let tmp = self.comp.entry_alloca(val.get_type(), "display.self");
                        b!(self.comp.bld.build_store(tmp, val));
                        tmp.into()
                    } else { val };
                    let result = b!(self.comp.bld.build_call(fv, &[self_arg.into()], "display.call"))
                        .try_as_basic_value().basic().unwrap();
                    Ok(result)
                } else {
                    self.comp.int_to_string(val, false)
                }
            }
            _ => self.comp.int_to_string(val, false),
        }
    }

    fn emit_fmt_bin(
        &mut self,
        val: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.comp.ctx.i64_type();
        let i8t = self.comp.ctx.i8_type();
        let malloc = self.comp.ensure_malloc();
        let buf = b!(self.comp.bld.build_call(malloc, &[i64t.const_int(65, false).into()], "fb.buf"))
            .try_as_basic_value().basic().unwrap();
        let buf_ptr = buf.into_pointer_value();

        let fv = self.comp.cur_fn.unwrap();
        let loop_bb = self.comp.ctx.append_basic_block(fv, "fb.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "fb.body");
        let done_bb = self.comp.ctx.append_basic_block(fv, "fb.done");

        let wide = if val.get_type().get_bit_width() < 64 {
            b!(self.comp.bld.build_int_z_extend(val, i64t, "fb.w"))
        } else { val };

        let clz_name = "llvm.ctlz.i64";
        let clz = self.comp.module.get_function(clz_name).unwrap_or_else(|| {
            let ft = i64t.fn_type(&[i64t.into(), self.comp.ctx.bool_type().into()], false);
            self.comp.module.add_function(clz_name, ft, None)
        });
        let lz = b!(self.comp.bld.build_call(clz, &[wide.into(), self.comp.ctx.bool_type().const_int(1, false).into()], "fb.lz"))
            .try_as_basic_value().basic().unwrap().into_int_value();
        let raw_bits = b!(self.comp.bld.build_int_nsw_sub(i64t.const_int(64, false), lz, "fb.nb"));
        let is_zero = b!(self.comp.bld.build_int_compare(inkwell::IntPredicate::EQ, wide, i64t.const_int(0, false), "fb.z"));
        let nbits = b!(self.comp.bld.build_select(is_zero, i64t.const_int(1, false), raw_bits, "fb.bits")).into_int_value();

        let idx_ptr = self.comp.entry_alloca(i64t.into(), "fb.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let bit_ptr = self.comp.entry_alloca(i64t.into(), "fb.bit");
        b!(self.comp.bld.build_store(bit_ptr, b!(self.comp.bld.build_int_nsw_sub(nbits, i64t.const_int(1, false), "fb.start"))));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "fb.i")).into_int_value();
        let cond = b!(self.comp.bld.build_int_compare(inkwell::IntPredicate::SLT, idx, nbits, "fb.cond"));
        b!(self.comp.bld.build_conditional_branch(cond, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let bit = b!(self.comp.bld.build_load(i64t, bit_ptr, "fb.b")).into_int_value();
        let shifted = b!(self.comp.bld.build_right_shift(wide, bit, false, "fb.sh"));
        let masked = b!(self.comp.bld.build_and(shifted, i64t.const_int(1, false), "fb.m"));
        let ch = b!(self.comp.bld.build_int_nsw_add(
            b!(self.comp.bld.build_int_truncate(masked, i8t, "fb.trunc")),
            i8t.const_int(b'0' as u64, false), "fb.ch"));
        let dest = unsafe { b!(self.comp.bld.build_gep(i8t, buf_ptr, &[idx], "fb.p")) };
        b!(self.comp.bld.build_store(dest, ch));
        let next_idx = b!(self.comp.bld.build_int_nsw_add(idx, i64t.const_int(1, false), "fb.ni"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        let next_bit = b!(self.comp.bld.build_int_nsw_sub(bit, i64t.const_int(1, false), "fb.nb2"));
        b!(self.comp.bld.build_store(bit_ptr, next_bit));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let end = unsafe { b!(self.comp.bld.build_gep(i8t, buf_ptr, &[nbits], "fb.end")) };
        b!(self.comp.bld.build_store(end, i8t.const_int(0, false)));
        self.comp.build_string(buf, nbits, b!(self.comp.bld.build_int_nsw_add(nbits, i64t.const_int(1, false), "fb.cap")), "fb.s")
    }

    fn emit_sleep_ms(
        &mut self,
        ms: inkwell::values::IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i32t = self.comp.ctx.i32_type();
        let i64t = self.comp.ctx.i64_type();
        let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
        let nanosleep = self.comp.module.get_function("nanosleep").unwrap_or_else(|| {
            self.comp.module.add_function("nanosleep",
                i32t.fn_type(&[ptr_ty.into(), ptr_ty.into()], false),
                Some(inkwell::module::Linkage::External))
        });
        let ts_ty = self.comp.ctx.struct_type(&[i64t.into(), i64t.into()], false);
        let ts = self.comp.entry_alloca(ts_ty.into(), "sleep.ts");
        let secs = b!(self.comp.bld.build_int_unsigned_div(ms, i64t.const_int(1000, false), "sleep.s"));
        let ns = b!(self.comp.bld.build_int_unsigned_rem(ms, i64t.const_int(1000, false), "sleep.rem"));
        let ns_full = b!(self.comp.bld.build_int_mul(ns, i64t.const_int(1_000_000, false), "sleep.ns"));
        let s_ptr = b!(self.comp.bld.build_struct_gep(ts_ty, ts, 0, "sleep.sp"));
        b!(self.comp.bld.build_store(s_ptr, secs));
        let n_ptr = b!(self.comp.bld.build_struct_gep(ts_ty, ts, 1, "sleep.np"));
        b!(self.comp.bld.build_store(n_ptr, ns_full));
        let null = ptr_ty.const_null();
        b!(self.comp.bld.build_call(nanosleep, &[ts.into(), null.into()], ""));
        Ok(i64t.const_int(0, false).into())
    }
}
