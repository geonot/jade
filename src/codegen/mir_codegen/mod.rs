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

mod helpers;
mod intrinsics;
mod magic;
mod store;
mod store_ext;

use std::collections::HashMap;

use indexmap::IndexMap;
use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock as LLVMBlock;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum, PhiValue, PointerValue};

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
    /// MIR ValueId → original LLVM BasicTypeEnum for self_allocs entries (for lazy reload).
    self_alloc_types: HashMap<mir::ValueId, inkwell::types::BasicTypeEnum<'ctx>>,
    /// MIR BlockId → actual LLVM exit block (may differ from block_map entry
    /// when helpers like string_concat create intermediate LLVM blocks).
    block_exit_map: HashMap<mir::BlockId, LLVMBlock<'ctx>>,
    /// Generated migration function values to call from main wrapper.
    migration_fns: Vec<inkwell::values::FunctionValue<'ctx>>,
    /// Generated global initializer function to call from main wrapper.
    global_init_fn: Option<inkwell::values::FunctionValue<'ctx>>,
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
            self_alloc_types: HashMap::new(),
            block_exit_map: HashMap::new(),
            migration_fns: Vec::new(),
            global_init_fn: None,
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
            let ltys: Vec<BasicTypeEnum<'ctx>> = td
                .fields
                .iter()
                .map(|(_, ty)| self.comp.llvm_ty(ty))
                .collect();
            let st = self.comp.ctx.opaque_struct_type(&td.name);
            st.set_body(&ltys, false);
            let fields: Vec<(String, Type)> = td.fields.clone();
            self.comp.structs.insert(td.name.clone(), fields);
        }

        // Populate struct_defaults from HIR type definitions.
        for td in &hir_prog.types {
            let defaults: indexmap::IndexMap<String, hir::Expr> = td
                .fields
                .iter()
                .filter_map(|f| f.default.as_ref().map(|d| (f.name.clone(), d.clone())))
                .collect();
            if !defaults.is_empty() {
                self.comp.struct_defaults.insert(td.name.clone(), defaults);
            }
            // Also register struct_layouts for alignment info.
            self.comp
                .struct_layouts
                .insert(td.name.clone(), td.layout.clone());
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
            let ptys: Vec<BasicMetadataTypeEnum<'ctx>> = ext
                .params
                .iter()
                .map(|t| {
                    // Extern functions use C ABI: String → ptr (char*)
                    if matches!(t, Type::String) {
                        self.comp
                            .ctx
                            .ptr_type(inkwell::AddressSpace::default())
                            .into()
                    } else {
                        self.comp.llvm_ty(t).into()
                    }
                })
                .collect();
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
                            || name.starts_with("__store_")
                            || name.starts_with("__kv_")
                            || name.starts_with("__graph_")
                            || name.starts_with("__ts_")
                            || name.starts_with("__vec_")
                            || name.starts_with("__bloom_")
                            || name.starts_with("__fts_")
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

        // ── Detect TLS / crypto usage (requires OpenSSL) ──
        let needs_ssl = prog.externs.iter().any(|e| {
            e.name.starts_with("jade_tls_")
                || e.name.starts_with("jade_sha")
                || e.name.starts_with("jade_hmac")
                || e.name.starts_with("jade_aes")
                || e.name == "jade_random_bytes"
                || e.name == "jade_bytes_to_hex"
        });
        self.comp.needs_ssl = needs_ssl;

        // ── Detect SQLite usage ──
        let needs_sqlite = prog.externs.iter().any(|e| e.name.starts_with("jade_sqlite_"));
        self.comp.needs_sqlite = needs_sqlite;

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
            self.comp.declare_gen_runtime(); // jade_gen_resume/suspend/destroy
        }

        // ── Declare HIR actors (just declarations, no body compilation yet) ──
        if !hir_prog.actors.is_empty() {
            self.comp.declare_actor_runtime(); // malloc, memset, free
            for ad in &hir_prog.actors {
                self.comp.declare_actor(ad)?;
                self.actor_defs.insert(ad.name.clone(), ad.clone());
                self.comp.actor_defs.insert(ad.name.clone(), ad.clone());
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

        // ── Generate migration functions ──
        if !hir_prog.migrations.is_empty() {
            if hir_prog.stores.is_empty() {
                self.comp.declare_store_runtime();
            }
            for mig in &hir_prog.migrations {
                let mfn = self.comp.gen_migration(mig)?;
                self.migration_fns.push(mfn);
            }
        }

        // ── Extract coroutine/generator bodies from HIR ──
        Self::extract_coro_bodies_from_program(hir_prog, &mut self.coro_bodies);

        // ── Declare all MIR functions (forward-declare so calls resolve) ──
        // NOTE: This must be AFTER runtime declarations so main wrapper
        // can find jade_sched_init/run/shutdown.

        // ── Declare global mutable variables ──
        for gdef in &prog.globals {
            let llvm_ty = self.comp.llvm_ty(&gdef.ty);
            let gv = self.comp.module.add_global(llvm_ty, None, &format!("__jade_global_{}", gdef.name));
            gv.set_initializer(&self.comp.zero_init(&gdef.ty));
            gv.set_linkage(Linkage::Internal);
            self.comp.globals.insert(gdef.name.clone(), (gv, gdef.ty.clone()));
        }

        // ── Declare global initializer function (called from main wrapper) ──
        if !hir_prog.globals.is_empty() {
            let void_ty = self.comp.ctx.void_type().fn_type(&[], false);
            let init_fn = self.comp.module.add_function("__jade_init_globals", void_ty, None);
            let entry = self.comp.ctx.append_basic_block(init_fn, "entry");
            self.comp.bld.position_at_end(entry);
            for g in &hir_prog.globals {
                let val = self.comp.compile_const_expr(&g.init)?;
                let (gv, _) = self.comp.globals.get(&g.name).unwrap();
                b!(self.comp.bld.build_store(gv.as_pointer_value(), val));
            }
            b!(self.comp.bld.build_return(None));
            self.global_init_fn = Some(init_fn);
        }

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
        if std::env::var("JADE_DUMP_IR").is_ok() {
            self.comp.module.print_to_stderr();
        }
        self.comp.module.verify().map_err(|e| e.to_string())
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
            let user_fv = self.comp.module.add_function("__jade_user_main", ft, None);
            self.comp.tag_fn(user_fv);
            self.apply_fn_attrs(user_fv, &func.attrs);
            user_fv.set_linkage(Linkage::Internal);
            self.comp
                .fns
                .insert(func.name.clone(), (user_fv, ptys, ret));

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
            // Initialize globals before user code
            if let Some(init_fn) = &self.global_init_fn {
                b!(self.comp.bld.build_call(*init_fn, &[], ""));
            }
            // Run migrations before user code
            for mig_fn in &self.migration_fns {
                b!(self.comp.bld.build_call(*mig_fn, &[], ""));
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
                b!(self.comp.bld.build_return(Some(&i32t.const_int(0, false))));
            }
        } else {
            let ft = self.comp.mk_fn_type(&ret, &lp, false);
            let fv = self.comp.module.add_function(&func.name, ft, None);
            self.comp.tag_fn(fv);
            self.apply_fn_attrs(fv, &func.attrs);
            self.comp.fns.insert(func.name.clone(), (fv, ptys, ret));
        }
        Ok(())
    }

    // ── apply @inline / @noinline / @cold / @hot attributes ────────

    fn apply_fn_attrs(
        &self,
        fv: inkwell::values::FunctionValue<'ctx>,
        attrs: &crate::ast::FnAttrs,
    ) {
        use inkwell::attributes::AttributeLoc;
        if attrs.inline {
            fv.add_attribute(AttributeLoc::Function, self.comp.attr("alwaysinline"));
        }
        if attrs.noinline {
            fv.add_attribute(AttributeLoc::Function, self.comp.attr("noinline"));
        }
        if attrs.cold {
            fv.add_attribute(AttributeLoc::Function, self.comp.attr("cold"));
        }
        if attrs.hot {
            fv.add_attribute(AttributeLoc::Function, self.comp.attr("hot"));
        }
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
        self.comp.vars = IndexMap::new();
        self.comp.var_shadows.clear();
        self.comp.var_scope_markers.clear();

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
                let phi_val = b!(self
                    .comp
                    .bld
                    .build_phi(llvm_ty, &format!("v{}", phi.dest.0)));
                self.value_map.insert(phi.dest, phi_val.as_basic_value());
                self.pending_phis.push(PendingPhi {
                    phi: phi_val,
                    incoming: phi.incoming.clone(),
                });
            }

            // 3b. Emit instructions.
            for inst in &bb.insts {
                if std::env::var("JADE_DEBUG_MIR_CODEGEN").is_ok() {
                    eprintln!(
                        "  emit {:?} dest={:?} kind={:?}",
                        inst.dest, inst.dest, inst.kind
                    );
                }
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
                    let llvm_bb = self
                        .block_exit_map
                        .get(block_id)
                        .or_else(|| self.block_map.get(block_id))?;
                    let llvm_val = self.value_map.get(val_id)?;
                    // Coerce void sentinel (i8 0) to the phi's actual type.
                    let v = if llvm_val.get_type() != phi_ty {
                        let is_void_sentinel = llvm_val.get_type().is_int_type()
                            && llvm_val.get_type().into_int_type().get_bit_width() == 8;
                        if !is_void_sentinel {
                            panic!(
                                "ICE: phi node type mismatch: incoming {:?} vs phi {:?} (block {:?}, val {:?})",
                                llvm_val.get_type(),
                                phi_ty,
                                block_id,
                                val_id,
                            );
                        }
                        if phi_ty.is_int_type() {
                            phi_ty.into_int_type().const_int(0, false).into()
                        } else if phi_ty.is_float_type() {
                            phi_ty.into_float_type().const_float(0.0).into()
                        } else if phi_ty.is_pointer_type() {
                            phi_ty.into_pointer_type().const_null().into()
                        } else if phi_ty.is_struct_type() {
                            phi_ty.into_struct_type().const_zero().into()
                        } else {
                            panic!(
                                "ICE: cannot coerce void sentinel to phi type {:?}",
                                phi_ty,
                            );
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
        let void_val =
            || -> BasicValueEnum<'ctx> { self.comp.ctx.i8_type().const_int(0, false).into() };

        match &inst.kind {
            // ── Constants ──
            mir::InstKind::IntConst(n) => {
                let llvm_ty = self.comp.llvm_ty(&inst.ty);
                Ok(match &inst.ty {
                    Type::F32 => self.comp.ctx.f32_type().const_float(*n as f64).into(),
                    Type::F64 => self.comp.ctx.f64_type().const_float(*n as f64).into(),
                    _ => llvm_ty.into_int_type().const_int(*n as u64, true).into(),
                })
            }
            mir::InstKind::FloatConst(f) => Ok(match &inst.ty {
                Type::F32 => self.comp.ctx.f32_type().const_float(*f).into(),
                _ => self.comp.ctx.f64_type().const_float(*f).into(),
            }),
            mir::InstKind::BoolConst(b) => {
                Ok(self.comp.ctx.bool_type().const_int(*b as u64, false).into())
            }
            mir::InstKind::StringConst(s) => self.emit_string_const(s),
            mir::InstKind::Void => Ok(void_val()),

            // ── Arithmetic ──
            mir::InstKind::BinOp(op, lhs, rhs) => self.emit_binop(*op, *lhs, *rhs, &inst.ty),
            mir::InstKind::UnaryOp(op, val) => self.emit_unary(*op, *val, &inst.ty),
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
                let arg_vals: Vec<BasicValueEnum<'ctx>> =
                    args.iter().map(|a| self.val(*a)).collect();
                if let Some((fv, _, _)) = self.comp.fns.get(name).cloned() {
                    let ptypes = fv.get_type().get_param_types();
                    let st = self.comp.string_type();
                    let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = arg_vals
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            if let Some(pt) = ptypes.get(i) {
                                if v.get_type() == st.into() && pt.is_pointer_type() {
                                    self.comp.string_data(*v).unwrap_or(*v).into()
                                } else {
                                    (*v).into()
                                }
                            } else {
                                (*v).into()
                            }
                        })
                        .collect();
                    let csv = b!(self.comp.bld.build_call(fv, &md, "call"));
                    Ok(self.comp.call_result(csv))
                } else {
                    // Try looking up as a module-level function.
                    if let Some(fv) = self.comp.module.get_function(name) {
                        let ptypes = fv.get_type().get_param_types();
                        let st = self.comp.string_type();
                        let md: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = arg_vals
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                if let Some(pt) = ptypes.get(i) {
                                    if v.get_type() == st.into() && pt.is_pointer_type() {
                                        self.comp.string_data(*v).unwrap_or(*v).into()
                                    } else {
                                        (*v).into()
                                    }
                                } else {
                                    (*v).into()
                                }
                            })
                            .collect();
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

                let is_vec_or_array =
                    matches!(&recv_ty, Some(Type::Vec(_)) | Some(Type::Array(_, _)));
                if is_vec_or_array {
                    let recv_val = self.val(*recv);
                    let elem_ty = match &recv_ty {
                        Some(Type::Vec(et)) => *et.clone(),
                        Some(Type::Array(et, _)) => *et.clone(),
                        _ => Type::I64,
                    };
                    // Fixed-size array: len returns constant, contains is inline scan
                    if let Some(Type::Array(_, arr_len)) = recv_ty {
                        match method.as_str() {
                            "len" => {
                                return Ok(self
                                    .comp
                                    .ctx
                                    .i64_type()
                                    .const_int(arr_len as u64, false)
                                    .into());
                            }
                            _ => {}
                        }
                    }
                    let header_ptr = if recv_val.is_pointer_value() {
                        recv_val.into_pointer_value()
                    } else {
                        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                        b!(self.comp.bld.build_int_to_ptr(
                            recv_val.into_int_value(),
                            ptr_ty,
                            "vec.ptr"
                        ))
                    };
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
                let is_map = matches!(&recv_ty, Some(Type::Map(_, _)))
                    || matches!(&recv_ty, Some(Type::Struct(n, _)) if n.starts_with("Map_"));
                if is_map {
                    let recv_val = self.val(*recv);
                    let header_ptr = if recv_val.is_pointer_value() {
                        recv_val.into_pointer_value()
                    } else {
                        let ptr_ty = self.comp.ctx.ptr_type(AddressSpace::default());
                        b!(self.comp.bld.build_int_to_ptr(
                            recv_val.into_int_value(),
                            ptr_ty,
                            "map.ptr"
                        ))
                    };
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
                    let self_arg: BasicValueEnum<'ctx> =
                        if first_param_is_ptr && !recv_val.is_pointer_value() {
                            // Struct value but method expects pointer: alloca + store.
                            // Cache the alloca so mutations from the method persist across calls
                            // (e.g. iterator .next() mutating self.n in a loop).
                            if let Some(cached) = self.self_allocs.get(recv) {
                                (*cached).into()
                            } else {
                                let tmp = self.comp.entry_alloca(recv_val.get_type(), "self.tmp");
                                // Store the initial value into the alloca.  We must place
                                // this store in the entry block so it only runs once —
                                // otherwise a loop would re-init the alloca every iteration,
                                // clobbering mutations made by the method.
                                //
                                // If recv_val was produced in a later block (e.g. via
                                // insertvalue in a branch), it won't dominate the entry
                                // block.  In that case, fall back to storing at the
                                // current position — this is correct for non-loop cases.
                                let cur_fn = self.comp.cur_fn.unwrap();
                                let entry_bb = cur_fn.get_first_basic_block().unwrap();
                                let cur_bb = self.comp.bld.get_insert_block().unwrap();
                                let recv_in_entry =
                                    if let Some(inst) = recv_val.as_instruction_value() {
                                        inst.get_parent().map_or(false, |bb| bb == entry_bb)
                                    } else {
                                        true // constants dominate everything
                                    };
                                if recv_in_entry {
                                    let entry_bld = self.comp.ctx.create_builder();
                                    if let Some(term) = entry_bb.get_terminator() {
                                        entry_bld.position_before(&term);
                                    } else {
                                        entry_bld.position_at_end(entry_bb);
                                    }
                                    entry_bld.build_store(tmp, recv_val).unwrap();
                                } else {
                                    b!(self.comp.bld.build_store(tmp, recv_val));
                                }
                                self.self_allocs.insert(*recv, tmp);
                                self.self_alloc_types.insert(*recv, recv_val.get_type());
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
                    // After a method call that may have mutated self through the alloca pointer,
                    // update the value map to point to the alloca pointer.
                    // FieldGet/FieldSet already handle pointer values via GEP,
                    // and subsequent method calls will pass the pointer directly.
                    // We avoid reloading the struct here because the reload would be
                    // placed in the current block and may not dominate later uses.
                    if let Some(alloca_ptr) = self.self_allocs.get(recv).copied() {
                        self.value_map.insert(*recv, alloca_ptr.into());
                    }
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
                    let null_env = self
                        .comp
                        .ctx
                        .ptr_type(inkwell::AddressSpace::default())
                        .const_null();
                    self.comp.make_closure(wrapper, null_env)
                } else {
                    Err(format!("mir_codegen: undefined function `{name}` in FnRef"))
                }
            }
            mir::InstKind::Load(name) => {
                if let Some((ptr, ty)) = self.var_allocs.get(name).cloned() {
                    let lt = self.comp.llvm_ty(&ty);
                    let val = b!(self.comp.bld.build_load(lt, ptr, name));
                    if let Some(inst) = val.as_instruction_value() {
                        let tbaa_name = Compiler::tbaa_type_name(&ty);
                        self.comp.set_tbaa(inst, tbaa_name);
                    }
                    Ok(val)
                } else {
                    // Fall back to Compiler's var lookup.
                    if let Some((ptr, ty)) = self.comp.find_var(name).cloned() {
                        let lt = self.comp.llvm_ty(&ty);
                        let val = b!(self.comp.bld.build_load(lt, ptr, name));
                        if let Some(inst) = val.as_instruction_value() {
                            let tbaa_name = Compiler::tbaa_type_name(&ty);
                            self.comp.set_tbaa(inst, tbaa_name);
                        }
                        Ok(val)
                    } else {
                        Err(format!("mir_codegen: Load of undefined variable `{name}`"))
                    }
                }
            }
            mir::InstKind::Store(name, val) => {
                let v = self.val(*val);
                if let Some((ptr, ty)) = self.var_allocs.get(name).cloned() {
                    let store_inst = b!(self.comp.bld.build_store(ptr, v));
                    let tbaa_name = Compiler::tbaa_type_name(&ty);
                    self.comp.set_tbaa(store_inst, tbaa_name);
                } else {
                    // First store → create alloca.
                    let lt = v.get_type();
                    let ty = inst.ty.clone();
                    let ptr = self.comp.entry_alloca(lt, name);
                    let store_inst = b!(self.comp.bld.build_store(ptr, v));
                    let tbaa_name = Compiler::tbaa_type_name(&ty);
                    self.comp.set_tbaa(store_inst, tbaa_name);
                    self.var_allocs.insert(name.clone(), (ptr, ty.clone()));
                    self.comp.set_var(name, ptr, ty);
                }
                Ok(void_val())
            }

            // ── Globals ──
            mir::InstKind::GlobalLoad(name) => {
                if let Some((gv, ty)) = self.comp.globals.get(name).cloned() {
                    let lt = self.comp.llvm_ty(&ty);
                    let val = b!(self.comp.bld.build_load(lt, gv.as_pointer_value(), name));
                    if let Some(inst) = val.as_instruction_value() {
                        self.comp.set_tbaa(inst, Compiler::tbaa_type_name(&ty));
                    }
                    Ok(val)
                } else {
                    Err(format!("mir_codegen: GlobalLoad of undefined global `{name}`"))
                }
            }
            mir::InstKind::GlobalStore(name, val_id) => {
                let v = self.val(*val_id);
                if let Some((gv, ty)) = self.comp.globals.get(name).cloned() {
                    let si = b!(self.comp.bld.build_store(gv.as_pointer_value(), v));
                    self.comp.set_tbaa(si, Compiler::tbaa_type_name(&ty));
                    Ok(void_val())
                } else {
                    Err(format!("mir_codegen: GlobalStore to undefined global `{name}`"))
                }
            }

            // ── Struct/Aggregate ──
            mir::InstKind::StructInit(name, fields) => {
                let st = self
                    .comp
                    .module
                    .get_struct_type(name)
                    .ok_or_else(|| format!("mir_codegen: unknown struct `{name}`"))?;
                let field_defs: Vec<(String, Type)> =
                    self.comp.structs.get(name).cloned().unwrap_or_default();
                let defaults = self.comp.struct_defaults.get(name).cloned();
                let mut agg: BasicValueEnum<'ctx> = st.const_zero().into();
                // Track which field indices were explicitly provided.
                let mut provided = std::collections::HashSet::new();
                for (i, (fname, vid)) in fields.iter().enumerate() {
                    let v = self.val(*vid);
                    let idx =
                        if fname.is_empty() {
                            provided.insert(i as u32);
                            i as u32
                        } else {
                            let pos = field_defs.iter().position(|(n, _)| n == fname).ok_or_else(
                                || format!("mir_codegen: struct `{name}` has no field `{fname}`"),
                            )? as u32;
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
                    let payload_gep = b!(self.comp.bld.build_struct_gep(st, alloca, 1, "payload"));
                    // Look up variant field types for recursive-field detection.
                    let variant_field_types: Vec<Type> = self
                        .comp
                        .enums
                        .get(enum_name)
                        .and_then(|vs| vs.iter().find(|(vn, _)| vn == variant))
                        .map(|(_, ftys)| ftys.clone())
                        .unwrap_or_default();
                    // Store payload fields at proper byte offsets based on actual type sizes.
                    let mut byte_offset: u64 = 0;
                    for (i, vid) in payload.iter().enumerate() {
                        let v = self.val(*vid);
                        let is_rec = variant_field_types
                            .get(i)
                            .map(|fty| Compiler::is_recursive_field(fty, enum_name))
                            .unwrap_or(false);
                        let field_ptr = if byte_offset == 0 {
                            payload_gep
                        } else {
                            let offset_val = self.comp.ctx.i64_type().const_int(byte_offset, false);
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
                            let actual_ty = self
                                .comp
                                .llvm_ty(variant_field_types.get(i).unwrap_or(&Type::I64));
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
                            let type_size = v
                                .get_type()
                                .size_of()
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
                        b!(self
                            .comp
                            .bld
                            .build_gep(arr_ty, alloca, &[zero, idx], "arr.elem"))
                    };
                    b!(self.comp.bld.build_store(ptr, *v));
                }
                Ok(b!(self.comp.bld.build_load(arr_ty, alloca, "arr.val")).into())
            }

            // ── Field access ──
            mir::InstKind::FieldGet(obj, field) => self.emit_field_get(*obj, field, &inst.ty),
            mir::InstKind::FieldSet(obj, field, val) => {
                // If the object has a self_allocs entry, use the alloca pointer directly
                // to avoid SSA domination issues with insertvalue across branches.
                let obj_val = if let Some(alloca_ptr) = self.self_allocs.get(obj).copied() {
                    alloca_ptr.into()
                } else {
                    self.val(*obj)
                };
                let v = self.val(*val);
                if obj_val.is_pointer_value() {
                    // obj is a pointer to a struct (alloca).
                    // inst.ty carries the struct type from lowering.
                    let struct_name = self.struct_name_from_type(&inst.ty).or_else(|| {
                        // Also try var_allocs for the struct name.
                        self.var_allocs
                            .values()
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
                                st,
                                obj_val.into_pointer_value(),
                                field_idx,
                                field
                            ));
                            b!(self.comp.bld.build_store(gep, v));
                        }
                    }
                    // Return the pointer so MIR SSA chaining of field assignments
                    // continues to target the same struct (e.g. self.a is X; self.b is Y).
                    return Ok(obj_val);
                } else if obj_val.is_struct_value() {
                    // SSA struct value — use insert_value for immutable update.
                    let sv = obj_val.into_struct_value();
                    let struct_ty_name = sv
                        .get_type()
                        .get_name()
                        .map(|n| n.to_str().unwrap_or("").to_string());
                    if let Some(name) = &struct_ty_name {
                        let field_idx = self.field_index(name, field);
                        let updated = b!(self.comp.bld.build_insert_value(sv, v, field_idx, field));
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
                            let gep =
                                b!(self.comp.bld.build_struct_gep(st, alloca, field_idx, field));
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
                        inkwell::IntPredicate::SLT,
                        idx_int,
                        i64t.const_int(0, false),
                        "neg"
                    ));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(
                        idx_int,
                        i64t.const_int(arr_len, false),
                        "wrap"
                    ));
                    let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                        .into_int_value();
                    let alloca = self.comp.entry_alloca(arr_ty.into(), "idx.tmp");
                    b!(self.comp.bld.build_store(alloca, base_val));
                    let zero = i64t.const_int(0, false);
                    let ptr = unsafe {
                        b!(self
                            .comp
                            .bld
                            .build_gep(arr_ty, alloca, &[zero, final_idx], "idx.ptr"))
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
                    let ptr_gep = b!(self
                        .comp
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 0, "vi.ptrp"));
                    let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vi.data"))
                        .into_pointer_value();
                    let len_gep = b!(self
                        .comp
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 1, "vi.lenp"));
                    let len =
                        b!(self.comp.bld.build_load(i64t, len_gep, "vi.len")).into_int_value();
                    // Wrap negative indices: if idx < 0, idx = len + idx
                    let idx_int = idx_val.into_int_value();
                    let is_neg = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::SLT,
                        idx_int,
                        i64t.const_int(0, false),
                        "neg"
                    ));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(idx_int, len, "wrap"));
                    let final_idx = b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
                        .into_int_value();
                    self.comp.emit_vec_bounds_check(final_idx, len)?;
                    let elem_gep = unsafe {
                        b!(self
                            .comp
                            .bld
                            .build_gep(elem_ty, data_ptr, &[final_idx], "vi.egep"))
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
                                st,
                                alloca,
                                &[zero, idx_val.into_int_value()],
                                "tup.ptr"
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
                        inkwell::IntPredicate::SLT,
                        idx_int,
                        zero,
                        "neg"
                    ));
                    let wrapped = b!(self.comp.bld.build_int_nsw_add(
                        idx_int,
                        i64t.const_int(arr_len, false),
                        "wrap"
                    ));
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
                    let ptr_gep = b!(self
                        .comp
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                    let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                        .into_pointer_value();
                    let len_gep = b!(self
                        .comp
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                    let len =
                        b!(self.comp.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                    self.comp
                        .emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                    let elem_gep = unsafe {
                        b!(self.comp.bld.build_gep(
                            elem_ty,
                            data_ptr,
                            &[idx_val.into_int_value()],
                            "vis.egep"
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
                            inkwell::IntPredicate::SLT,
                            idx_int,
                            zero,
                            "neg"
                        ));
                        let wrapped = b!(self.comp.bld.build_int_nsw_add(
                            idx_int,
                            i64t.const_int(arr_len, false),
                            "wrap"
                        ));
                        let final_idx =
                            b!(self.comp.bld.build_select(is_neg, wrapped, idx_int, "idx"))
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
                        let ptr_gep = b!(self
                            .comp
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 0, "vis.ptrp"));
                        let data_ptr = b!(self.comp.bld.build_load(ptr_ty, ptr_gep, "vis.data"))
                            .into_pointer_value();
                        let len_gep = b!(self
                            .comp
                            .bld
                            .build_struct_gep(header_ty, header_ptr, 1, "vis.lenp"));
                        let len =
                            b!(self.comp.bld.build_load(i64t, len_gep, "vis.len")).into_int_value();
                        let elem_ty = v.get_type();
                        self.comp
                            .emit_vec_bounds_check(idx_val.into_int_value(), len)?;
                        let elem_gep = unsafe {
                            b!(self.comp.bld.build_gep(
                                elem_ty,
                                data_ptr,
                                &[idx_val.into_int_value()],
                                "vis.egep"
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
                    let cur_fn = self
                        .comp
                        .bld
                        .get_insert_block()
                        .unwrap()
                        .get_parent()
                        .unwrap();
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
                let size = v
                    .get_type()
                    .size_of()
                    .unwrap_or(self.comp.ctx.i64_type().const_int(8, false));
                let ptr = b!(self.comp.bld.build_call(malloc, &[size.into()], "alloc"))
                    .try_as_basic_value()
                    .basic()
                    .unwrap();
                b!(self.comp.bld.build_store(ptr.into_pointer_value(), v));
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
            mir::InstKind::Slice(base, lo, hi) => self.emit_slice(*base, *lo, *hi, &inst.ty),

            // ── Collections ──
            mir::InstKind::VecNew(elems) => self.emit_vec_new(elems, &inst.ty),
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
                    let len_gep = b!(self
                        .comp
                        .bld
                        .build_struct_gep(header_ty, header_ptr, 1, "vl.len"));
                    Ok(b!(self.comp.bld.build_load(i64t, len_gep, "vl.v")))
                } else if vec_val.is_array_value() {
                    // Fixed-size array: length is known at compile time.
                    let arr_len = vec_val.into_array_value().get_type().len();
                    Ok(i64t.const_int(arr_len as u64, false).into())
                } else {
                    Err("mir_codegen: VecLen on non-vec/array value".into())
                }
            }
            mir::InstKind::MapInit => self.comp.compile_map_new(),
            mir::InstKind::SetInit => {
                if let Some(fv) = self.comp.module.get_function("jade_set_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "set"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err(
                        "mir_codegen: SetInit used but jade_set_new runtime function not declared"
                            .into(),
                    )
                }
            }
            mir::InstKind::PQInit => {
                if let Some(fv) = self.comp.module.get_function("jade_pq_new") {
                    let csv = b!(self.comp.bld.build_call(fv, &[], "pq"));
                    Ok(self.comp.call_result(csv))
                } else {
                    Err(
                        "mir_codegen: PQInit used but jade_pq_new runtime function not declared"
                            .into(),
                    )
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
                let fn_ptr = b!(self.comp.bld.build_extract_value(closure_st, 0, "fn_ptr"))
                    .into_pointer_value();
                let env_ptr = b!(self.comp.bld.build_extract_value(closure_st, 1, "env_ptr"));
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
                let csv =
                    b!(self
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
            mir::InstKind::ChanCreate(elem_ty, cap) => self.emit_chan_create(elem_ty, cap.as_ref()),
            mir::InstKind::ChanSend(ch, val) => self.emit_chan_send(*ch, *val),
            mir::InstKind::ChanRecv(ch) => self.emit_chan_recv(*ch, &inst.ty),
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
                    b!(self
                        .comp
                        .bld
                        .build_call(printf, &[gv.as_pointer_value().into()], ""));
                }
                // Call abort.
                let abort = self.comp.module.get_function("abort").unwrap_or_else(|| {
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

                let vtable_ptr = self
                    .comp
                    .vtables
                    .get(&(type_name.to_string(), trait_name.to_string()))
                    .map(|gv| gv.as_pointer_value())
                    .unwrap_or_else(|| ptr_ty.const_null());

                let fat_ty = self
                    .comp
                    .ctx
                    .struct_type(&[ptr_ty.into(), ptr_ty.into()], false);
                let fat = fat_ty.const_zero();
                let fat = b!(self
                    .comp
                    .bld
                    .build_insert_value(fat, data_ptr, 0, "dyn.fat.data"))
                .into_struct_value();
                let fat =
                    b!(self
                        .comp
                        .bld
                        .build_insert_value(fat, vtable_ptr, 1, "dyn.fat.vtable"))
                    .into_struct_value();
                Ok(fat.into())
            }

            mir::InstKind::InlineAsm(template, args) => {
                let arg_vals: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> =
                    args.iter().map(|a| self.val(*a).into()).collect();
                let arg_tys: Vec<inkwell::types::BasicMetadataTypeEnum<'ctx>> = arg_vals
                    .iter()
                    .map(|v| match v {
                        inkwell::values::BasicMetadataValueEnum::IntValue(iv) => {
                            iv.get_type().into()
                        }
                        inkwell::values::BasicMetadataValueEnum::FloatValue(fv) => {
                            fv.get_type().into()
                        }
                        inkwell::values::BasicMetadataValueEnum::PointerValue(pv) => {
                            pv.get_type().into()
                        }
                        _ => self.comp.ctx.i64_type().into(),
                    })
                    .collect();
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

    fn emit_terminator(&mut self, term: &mir::Terminator, ret_ty: &Type) -> Result<(), String> {
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
                        let iv = disc_val.get_type().const_int(*val as u64, true);
                        (iv, self.block_map[bid])
                    })
                    .collect();
                let _switch = b!(self.comp.bld.build_switch(disc_val, default_bb, &case_bbs));
            }
            mir::Terminator::Unreachable => {
                b!(self.comp.bld.build_unreachable());
            }
        }
        Ok(())
    }

    // ── helpers ────────────────────────────────────────────────────

    /// Look up an MIR ValueId in the value map.
    /// If the value corresponds to a self_allocs entry (struct stored in an alloca
    /// for method call mutation), reload from the alloca to get the current value.
    fn val(&mut self, id: mir::ValueId) -> BasicValueEnum<'ctx> {
        let v = self.value_map.get(&id).copied().unwrap_or_else(|| {
            eprintln!("MIR codegen: missing value for {:?}", id);
            eprintln!(
                "  available values: {:?}",
                self.value_map.keys().collect::<Vec<_>>()
            );
            panic!(
                "MIR codegen: missing value for {:?} — this is a compiler bug",
                id
            );
        });
        // If this ValueId has a self_allocs entry AND the value in the map is the
        // alloca pointer itself, reload the struct value from the alloca.
        // This avoids LLVM domination issues when a reload placed in one branch
        // is used in another.
        if let Some(alloca_ptr) = self.self_allocs.get(&id).copied() {
            if v.is_pointer_value() && v.into_pointer_value() == alloca_ptr {
                if let Some(orig_ty) = self.self_alloc_types.get(&id).copied() {
                    return self
                        .comp
                        .bld
                        .build_load(orig_ty, alloca_ptr, "self.reload")
                        .unwrap()
                        .into();
                }
            }
        }
        v
    }

}