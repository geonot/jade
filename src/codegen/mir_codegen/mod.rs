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

mod emit_inst;
mod helpers;
mod intrinsics;
mod magic;
mod store;
mod store_ext;

use crate::intern::Symbol;
use std::collections::HashMap;

use indexmap::IndexMap;
use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock as LLVMBlock;
use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum};
use inkwell::values::{BasicValue, BasicValueEnum};

use crate::hir;
use crate::mir;
use crate::perceus::PerceusHints;
use crate::types::Type;

use super::Compiler;
use super::PendingPhi;
use super::b;

/// Backwards-compatible alias for the merged code-generator. Prior to the
/// C.1 cleanup `MirCodegen` was a distinct struct that borrowed `Compiler`;
/// the two have since been collapsed into one type. New code should use
/// [`Compiler`] directly.
pub type MirCodegen<'ctx> = Compiler<'ctx>;

impl<'ctx> Compiler<'ctx> {
    fn compute_vec_growth_floors(func: &mir::Function) -> HashMap<mir::ValueId, u64> {
        let mut pushes_by_vec: HashMap<mir::ValueId, u32> = HashMap::new();
        for bb in &func.blocks {
            for inst in &bb.insts {
                if let mir::InstKind::VecPush(vec_id, _) = inst.kind {
                    *pushes_by_vec.entry(vec_id).or_insert(0) += 1;
                }
            }
        }

        let mut floors = HashMap::new();
        for bb in &func.blocks {
            for inst in &bb.insts {
                if let (Some(dest), mir::InstKind::VecNew(elems)) = (&inst.dest, &inst.kind) {
                    if !elems.is_empty() {
                        continue;
                    }
                    let pushes = *pushes_by_vec.get(dest).unwrap_or(&0);
                    let floor = if pushes >= 32 {
                        64
                    } else if pushes >= 16 {
                        32
                    } else {
                        16
                    };
                    floors.insert(*dest, floor);
                }
            }
        }
        floors
    }

    // ── public entry point ─────────────────────────────────────────

    /// Compile a full MIR program into the LLVM module owned by `self`.
    pub fn compile_program(
        &mut self,
        prog: &mir::Program,
        hir_prog: &hir::Program,
        hints: PerceusHints,
    ) -> Result<(), String> {
        self.hints = hints;
        self.setup_target()?;
        self.declare_builtins();

        // Register struct types from MIR type defs.
        // Two-pass: create all opaque struct types first so that mutually-
        // referencing or out-of-order field types resolve correctly. If we
        // computed bodies in a single pass, `llvm_ty(Struct(other))` would
        // fall back to `i64` when `other` had not yet been registered,
        // producing invalid `insertvalue`/`extractvalue` IR.
        for td in &prog.types {
            self.ctx.opaque_struct_type(&td.name.as_str());
        }
        for td in &prog.types {
            let ltys: Vec<BasicTypeEnum<'ctx>> =
                td.fields.iter().map(|(_, ty)| self.llvm_ty(ty)).collect();
            let st = self
                .module
                .get_struct_type(&td.name.as_str())
                .expect("opaque struct just created");
            st.set_body(&ltys, false);
            let fields: Vec<(String, Type)> = td
                .fields
                .iter()
                .map(|(n, t)| (n.as_str(), t.clone()))
                .collect();
            self.structs.insert(td.name, fields);
        }

        // Populate struct_defaults from HIR type definitions.
        for td in &hir_prog.types {
            let defaults: indexmap::IndexMap<Symbol, hir::Expr> = td
                .fields
                .iter()
                .filter_map(|f| f.default.as_ref().map(|d| (f.name.clone(), d.clone())))
                .collect();
            if !defaults.is_empty() {
                self.struct_defaults.insert(td.name.clone(), defaults);
            }
            // Also register struct_layouts for alignment info.
            self.struct_layouts
                .insert(td.name.clone(), td.layout.clone());
        }

        // Register HIR enum definitions (MIR doesn't carry enum info yet).
        for ed in &hir_prog.enums {
            let _ = self.declare_enum(ed);
        }

        // Register error definitions (tagged unions like enums).
        for ed in &hir_prog.err_defs {
            self.declare_err_def(ed)?;
        }

        // Register extern declarations.
        for ext in &prog.externs {
            let ptys: Vec<BasicMetadataTypeEnum<'ctx>> = ext
                .params
                .iter()
                .map(|t| {
                    // Extern functions use C ABI: String → ptr (char*)
                    if matches!(t, Type::String) {
                        self.ctx.ptr_type(inkwell::AddressSpace::default()).into()
                    } else {
                        self.llvm_ty(t).into()
                    }
                })
                .collect();
            let ft = self.mk_fn_type(&ext.ret, &ptys, false);
            let fv = self
                .module
                .add_function(&ext.name.as_str(), ft, Some(Linkage::External));
            fv.add_attribute(
                inkwell::attributes::AttributeLoc::Function,
                self.attr("nounwind"),
            );
            let param_tys: Vec<Type> = ext.params.clone();
            self.fns.insert(ext.name, (fv, param_tys, ext.ret.clone()));
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
                    | mir::InstKind::SelectArm(..)
                    | mir::InstKind::Slice(..) => true,
                    // Vec/array methods may emit calls to runtime helpers
                    // (jinn_sort_i64, __jinn_vec_slice, etc.).
                    mir::InstKind::MethodCall(_, method, _, _) => matches!(
                        &*method.as_str(),
                        "sort"
                            | "reverse"
                            | "sum"
                            | "contains"
                            | "join"
                            | "fold"
                            | "reduce"
                            | "find"
                            | "any"
                            | "all"
                            | "take"
                            | "skip"
                            | "drop"
                            | "slice"
                            | "zip"
                            | "map"
                            | "filter"
                            | "push"
                            | "pop"
                            | "remove"
                            | "clear"
                            | "next"
                    ),
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
                            || name.starts_with("__jinn_")
                            || name.starts_with("jinn_")
                            || name.starts_with("__bloom_")
                            || name.starts_with("__fts_")
                    }
                    _ => false,
                })
            })
        }) || !hir_prog.actors.is_empty()
            || prog.externs.iter().any(|e| e.name.starts_with("jinn_"))
            || super::Compiler::uses_concurrency(hir_prog)
            || super::Compiler::uses_pool(hir_prog);
        self.needs_runtime = needs_runtime;
        if needs_runtime {
            self.declare_jinn_runtime();
        }

        // ── Detect TLS / crypto usage (requires OpenSSL) ──
        let needs_ssl = prog.externs.iter().any(|e| {
            e.name.starts_with("jinn_tls_")
                || e.name.starts_with("jinn_sha")
                || e.name.starts_with("jinn_hmac")
                || e.name.starts_with("jinn_aes")
                || e.name == "jinn_random_bytes"
                || e.name == "jinn_bytes_to_hex"
        });
        self.needs_ssl = needs_ssl;

        // ── Detect SQLite usage ──
        let needs_sqlite = prog
            .externs
            .iter()
            .any(|e| e.name.starts_with("jinn_sqlite_"));
        self.needs_sqlite = needs_sqlite;

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
            self.declare_actor_runtime(); // malloc, memset, free
            self.declare_gen_runtime(); // jinn_gen_resume/suspend/destroy
        }

        // ── Declare HIR actors (just declarations, no body compilation yet) ──
        if !hir_prog.actors.is_empty() {
            self.declare_actor_runtime(); // malloc, memset, free
            for ad in &hir_prog.actors {
                self.declare_actor(ad)?;
                self.actor_defs.insert(ad.name.clone(), ad.clone());
            }
        }

        // ── Process HIR stores ──
        if !hir_prog.stores.is_empty() {
            self.declare_store_runtime();
            for sd in &hir_prog.stores {
                self.declare_store(sd)?;
                self.store_defs.insert(sd.name.clone(), sd.clone());
            }
        }

        // ── Generate migration functions ──
        if !hir_prog.migrations.is_empty() {
            if hir_prog.stores.is_empty() {
                self.declare_store_runtime();
            }
            for mig in &hir_prog.migrations {
                let mfn = self.gen_migration(mig)?;
                self.migration_fns.push(mfn);
            }
        }

        // ── Extract coroutine/generator bodies from HIR ──
        Self::extract_coro_bodies_from_program(hir_prog, &mut self.coro_bodies);

        // ── Declare all MIR functions (forward-declare so calls resolve) ──
        // NOTE: This must be AFTER runtime declarations so main wrapper
        // can find jinn_sched_init/run/shutdown.

        // ── Declare global mutable variables ──
        for gdef in &prog.globals {
            let llvm_ty = self.llvm_ty(&gdef.ty);
            let gv = self
                .module
                .add_global(llvm_ty, None, &format!("__jinn_global_{}", gdef.name));
            gv.set_initializer(&self.zero_init(&gdef.ty));
            gv.set_linkage(Linkage::Internal);
            self.globals.insert(gdef.name, (gv, gdef.ty.clone()));
        }

        // ── Declare global initializer function (called from main wrapper) ──
        if !hir_prog.globals.is_empty() {
            let void_ty = self.ctx.void_type().fn_type(&[], false);
            let init_fn = self
                .module
                .add_function("__jinn_init_globals", void_ty, None);
            let entry = self.ctx.append_basic_block(init_fn, "entry");
            self.bld.position_at_end(entry);
            for g in &hir_prog.globals {
                let val = self.compile_const_expr(&g.init)?;
                let (gv, _) = self.globals.get(&g.name).unwrap();
                b!(self.bld.build_store(gv.as_pointer_value(), val));
            }
            b!(self.bld.build_return(None));
            self.global_init_fn = Some(init_fn);
        }

        for func in &prog.functions {
            self.declare_mir_fn(func)?;
        }

        // ── Declare trait impl methods not already declared via MIR fns ──
        for ti in &hir_prog.trait_impls {
            for m in &ti.methods {
                if !self.fns.contains_key(&m.name) {
                    self.declare_method(&ti.type_name.as_str(), m)?;
                }
            }
        }

        // ── Generate vtables for dynamic dispatch ──
        self.generate_vtables(&hir_prog.trait_impls)?;

        // ── Compile actor loop bodies (after MIR fn declarations so
        //    functions like fib are available for actor handlers) ──
        if !hir_prog.actors.is_empty() {
            for ad in &hir_prog.actors {
                self.compile_actor_loop(ad)?;
            }
        }

        // ── Supervisor trees ──
        for sup in &hir_prog.supervisors {
            self.compile_supervisor(sup)?;
        }

        // ── Compile each MIR function body ──
        for func in &prog.functions {
            self.compile_mir_fn(func)?;
        }

        self.finalize_debug();
        if std::env::var("JINN_DUMP_IR").is_ok() {
            self.module.print_to_stderr();
        }
        self.module.verify().map_err(|e| e.to_string())
    }

    // ── function declaration ───────────────────────────────────────

    fn declare_mir_fn(&mut self, func: &mir::Function) -> Result<(), String> {
        let ptys: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
        let ret = func.ret_ty.clone();

        // Build LLVM parameter types. Struct/Tuple parameters are passed as
        // pointers (reference semantics) so that callee mutations to fields
        // are visible to the caller. This matches user expectations for
        // non-trivial aggregate types (consistent with Vec/Map/etc., which
        // are already pointer-typed).
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> = ptys
            .iter()
            .map(|t| match t {
                Type::Struct(_, _) | Type::Tuple(_) | Type::Enum(_) => ptr_ty.into(),
                _ => self.llvm_ty(t).into(),
            })
            .collect();

        let is_main = func.name == "main";
        if is_main && !self.lib_mode {
            // Create __jinn_user_main + wrapper main that initialises runtime.
            let ft = self.mk_fn_type(&ret, &lp, false);
            let user_fv = self.module.add_function("__jinn_user_main", ft, None);
            self.tag_fn(user_fv);
            self.apply_fn_attrs(user_fv, &func.attrs);
            user_fv.set_linkage(Linkage::Internal);
            self.fns.insert(func.name, (user_fv, ptys, ret));

            // Build main wrapper (same logic as decl.rs).
            let i32t = self.ctx.i32_type();
            let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
            let main_ft = i32t.fn_type(&[i32t.into(), ptr_ty.into()], false);
            let main_fv = self.module.add_function("main", main_ft, None);

            let argc_global = self.module.add_global(i32t, None, "__jinn_argc");
            argc_global.set_initializer(&i32t.const_int(0, false));
            let argv_global = self.module.add_global(ptr_ty, None, "__jinn_argv");
            argv_global.set_initializer(&ptr_ty.const_null());

            let entry = self.ctx.append_basic_block(main_fv, "entry");
            self.bld.position_at_end(entry);
            let argc_param = main_fv.get_nth_param(0).expect("ICE: missing param");
            let argv_param = main_fv.get_nth_param(1).expect("ICE: missing param");
            b!(self
                .bld
                .build_store(argc_global.as_pointer_value(), argc_param));
            b!(self
                .bld
                .build_store(argv_global.as_pointer_value(), argv_param));

            if let Some(sched_init) = self.module.get_function("jinn_sched_init") {
                b!(self
                    .bld
                    .build_call(sched_init, &[i32t.const_int(0, false).into()], ""));
            }
            // Initialize globals before user code
            if let Some(init_fn) = &self.global_init_fn {
                b!(self.bld.build_call(*init_fn, &[], ""));
            }
            // Run migrations before user code
            for mig_fn in &self.migration_fns {
                b!(self.bld.build_call(*mig_fn, &[], ""));
            }
            let call_result = b!(self.bld.build_call(user_fv, &[], "user_main"));
            if let Some(sched_run) = self.module.get_function("jinn_sched_run") {
                b!(self.bld.build_call(sched_run, &[], ""));
            }
            if let Some(sched_shutdown) = self.module.get_function("jinn_sched_shutdown") {
                b!(self.bld.build_call(sched_shutdown, &[], ""));
            }
            if let Some(rv) = call_result.try_as_basic_value().basic() {
                let ret_i32 = if rv.is_int_value() {
                    let iv = rv.into_int_value();
                    if iv.get_type().get_bit_width() != 32 {
                        b!(self.bld.build_int_truncate(iv, i32t, "ret32"))
                    } else {
                        iv
                    }
                } else {
                    i32t.const_int(0, false)
                };
                b!(self.bld.build_return(Some(&ret_i32)));
            } else {
                b!(self.bld.build_return(Some(&i32t.const_int(0, false))));
            }
        } else {
            let ft = self.mk_fn_type(&ret, &lp, false);
            let fv = self.module.add_function(&func.name.as_str(), ft, None);
            self.tag_fn(fv);
            self.apply_fn_attrs(fv, &func.attrs);
            self.fns.insert(func.name, (fv, ptys, ret));
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
            fv.add_attribute(AttributeLoc::Function, self.attr("alwaysinline"));
        }
        if attrs.noinline {
            fv.add_attribute(AttributeLoc::Function, self.attr("noinline"));
        }
        if attrs.cold {
            fv.add_attribute(AttributeLoc::Function, self.attr("cold"));
        }
        if attrs.hot {
            fv.add_attribute(AttributeLoc::Function, self.attr("hot"));
        }
    }

    // ── function body compilation ──────────────────────────────────

    fn compile_mir_fn(&mut self, func: &mir::Function) -> Result<(), String> {
        let (fv, _, _) = self
            .fns
            .get(&func.name)
            .ok_or_else(|| format!("undeclared fn {}", func.name))?
            .clone();

        self.cur_fn = Some(fv);
        self.value_map.clear();
        self.block_map.clear();
        self.pending_phis.clear();
        self.var_allocs.clear();
        self.value_types.clear();
        self.self_allocs.clear();
        self.vec_growth_floor_by_value = Self::compute_vec_growth_floors(func);
        self.current_perceus_meta = func.perceus.clone();
        self.current_reuse_slots.clear();
        self.current_reuse_alloca_slots.clear();
        self.current_alloc_dest = None;
        self.vars = IndexMap::new();
        self.var_shadows.clear();
        self.var_scope_markers.clear();

        // R15: DI subprogram pushing intentionally deferred. Without
        // per-MIR-instruction `set_current_debug_location` calls, the
        // verifier rejects any call instruction inside a DI-bearing fn
        // ("inlinable function call must have a !dbg location"). The
        // `attach_dbg_declare` helper and DICompileUnit are in place;
        // wiring location emission across every MIR opcode handler is
        // tracked as follow-up work after the M1 release.

        // 1. Create all LLVM basic blocks up-front.
        for bb in &func.blocks {
            let llvm_bb = self.ctx.append_basic_block(fv, &bb.label.as_str());
            self.block_map.insert(bb.id, llvm_bb);
        }

        // 2. Wire function parameters into value_map.
        for (i, param) in func.params.iter().enumerate() {
            let llvm_val = fv.get_nth_param(i as u32).unwrap();
            self.value_map.insert(param.value, llvm_val);
            self.value_types.insert(param.value, param.ty.clone());
            // Struct/Tuple params are pointers (see declare_mir_fn). Register
            // them in self_allocs so FieldGet/FieldSet take the GEP path
            // (mutating in place, visible to caller). val() will reload the
            // struct value lazily for non-field uses.
            // Peel a single Type::Ptr wrapper: trait/by-ptr methods declare
            // self as Type::Ptr(Struct(_)) at the typer level, but the LLVM
            // ABI is identical to Type::Struct(_) (always passed as ptr).
            // Treat both forms uniformly so FieldGet/FieldSet take the
            // in-place GEP path.
            let effective_ty = match &param.ty {
                Type::Ptr(inner)
                    if matches!(
                        inner.as_ref(),
                        Type::Struct(_, _) | Type::Tuple(_) | Type::Enum(_)
                    ) =>
                {
                    (**inner).clone()
                }
                _ => param.ty.clone(),
            };
            if matches!(
                effective_ty,
                Type::Struct(_, _) | Type::Tuple(_) | Type::Enum(_)
            ) && llvm_val.is_pointer_value()
            {
                let ptr = llvm_val.into_pointer_value();
                let lt = self.llvm_ty(&effective_ty);
                self.self_allocs.insert(param.value, ptr);
                self.self_alloc_types.insert(param.value, lt);
                // Override value_types so downstream lookups see the inner
                // struct/tuple/enum type (needed by val()'s reload logic
                // and by emit_field_get's struct_name detection).
                self.value_types.insert(param.value, effective_ty);
            }
        }

        // 3. Emit each basic block.
        for bb in &func.blocks {
            let llvm_bb = self.block_map[&bb.id];
            self.bld.position_at_end(llvm_bb);

            // 3a. Emit phi nodes.
            for phi in &bb.phis {
                let llvm_ty = self.llvm_ty(&phi.ty);
                let phi_val = b!(self.bld.build_phi(llvm_ty, &format!("v{}", phi.dest.0)));
                self.value_map.insert(phi.dest, phi_val.as_basic_value());
                self.pending_phis.push(PendingPhi {
                    phi: phi_val,
                    incoming: phi.incoming.clone(),
                });
            }

            // 3b. Emit instructions.
            for inst in &bb.insts {
                if std::env::var("JINN_DEBUG_MIR_CODEGEN").is_ok() {
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
            let exit_bb = self
                .bld
                .get_insert_block()
                .expect("ICE: builder has no insert block");
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

        // R15: pop the DI scope pushed at the top of compile_mir_fn.
        self.pop_debug_scope();

        Ok(())
    }

    // ── instruction emission ───────────────────────────────────────

    fn emit_inst(&mut self, inst: &mir::Instruction) -> Result<BasicValueEnum<'ctx>, String> {
        if let Some(v) = self.emit_core_inst(inst)? {
            return Ok(v);
        }
        if let Some(v) = self.emit_aggregate_memory_inst(inst)? {
            return Ok(v);
        }
        if let Some(v) = self.emit_ownership_collection_inst(inst)? {
            return Ok(v);
        }
        if let Some(v) = self.emit_runtime_inst(inst)? {
            return Ok(v);
        }
        Err(format!("unsupported MIR instruction: {:?}", inst.kind))
    }

    // ── terminator emission ────────────────────────────────────────

    fn emit_terminator(&mut self, term: &mir::Terminator, ret_ty: &Type) -> Result<(), String> {
        match term {
            mir::Terminator::Goto(target) => {
                let bb = self.block_map[target];
                b!(self.bld.build_unconditional_branch(bb));
            }
            mir::Terminator::Branch(cond, then_bb, else_bb) => {
                let cond_val = self.val(*cond).into_int_value();
                // Ensure condition is i1 — coerce wider integers with != 0.
                let cond_i1 = if cond_val.get_type().get_bit_width() != 1 {
                    b!(self.bld.build_int_compare(
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
                b!(self.bld.build_conditional_branch(cond_i1, t, e));
            }
            mir::Terminator::Return(val) => {
                self.drain_reuse_slots();
                if let Some(vid) = val {
                    let v = self.val(*vid);
                    let expected = self.llvm_ty(ret_ty);
                    if v.get_type() == expected {
                        b!(self.bld.build_return(Some(&v)));
                    } else if matches!(ret_ty, Type::Tuple(_)) && v.is_array_value() {
                        // Tuple return: coerce array → struct via alloca bitcast.
                        let alloca = self.entry_alloca(v.get_type(), "tup.coerce");
                        b!(self.bld.build_store(alloca, v));
                        let coerced = b!(self.bld.build_load(expected, alloca, "tup.ret"));
                        b!(self.bld.build_return(Some(&coerced)));
                    } else {
                        // Type mismatch (e.g. void-valued last expr in non-void fn).
                        let default = self.default_val(ret_ty);
                        b!(self.bld.build_return(Some(&default)));
                    }
                } else if matches!(ret_ty, Type::Void) {
                    b!(self.bld.build_return(None));
                } else {
                    let default = self.default_val(ret_ty);
                    b!(self.bld.build_return(Some(&default)));
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
                let _switch = b!(self.bld.build_switch(disc_val, default_bb, &case_bbs));
            }
            mir::Terminator::Unreachable => {
                b!(self.bld.build_unreachable());
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
