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

    pub fn compile_program(
        &mut self,
        prog: &mir::Program,
        hir_prog: &hir::Program,
        hints: PerceusHints,
    ) -> Result<(), String> {
        self.hints = hints;
        self.setup_target()?;
        self.declare_builtins();

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

        for td in &hir_prog.types {
            let defaults: indexmap::IndexMap<Symbol, hir::Expr> = td
                .fields
                .iter()
                .filter_map(|f| f.default.as_ref().map(|d| (f.name.clone(), d.clone())))
                .collect();
            if !defaults.is_empty() {
                self.struct_defaults.insert(td.name.clone(), defaults);
            }

            self.struct_layouts
                .insert(td.name.clone(), td.layout.clone());
        }

        for ed in &hir_prog.enums {
            let _ = self.declare_enum(ed);
        }

        for ed in &hir_prog.err_defs {
            self.declare_err_def(ed)?;
        }

        for ext in &prog.externs {
            let ptys: Vec<BasicMetadataTypeEnum<'ctx>> = ext
                .params
                .iter()
                .map(|t| {
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

        let needs_runtime = prog.functions.iter().any(|f| {
            f.blocks.iter().any(|bb| {
                bb.insts.iter().any(|i| match &i.kind {
                    mir::InstKind::SpawnActor(..)
                    | mir::InstKind::ChanCreate(..)
                    | mir::InstKind::ChanSend(..)
                    | mir::InstKind::ChanRecv(..)
                    | mir::InstKind::SelectArm(..)
                    | mir::InstKind::Slice(..) => true,

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
                    mir::InstKind::RuntimeOp(_, _) => true,
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

        let needs_ssl = prog.externs.iter().any(|e| {
            e.name.starts_with("jinn_tls_")
                || e.name.starts_with("jinn_sha")
                || e.name.starts_with("jinn_hmac")
                || e.name.starts_with("jinn_aes")
                || e.name == "jinn_random_bytes"
                || e.name == "jinn_bytes_to_hex"
        });
        self.needs_ssl = needs_ssl;

        let needs_sqlite = prog
            .externs
            .iter()
            .any(|e| e.name.starts_with("jinn_sqlite_"));
        self.needs_sqlite = needs_sqlite;

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
            self.declare_actor_runtime();
            self.declare_gen_runtime();
        }

        if !hir_prog.actors.is_empty() {
            self.declare_actor_runtime();
            for ad in &hir_prog.actors {
                self.declare_actor(ad)?;
                self.actor_defs.insert(ad.name.clone(), ad.clone());
            }
        }

        if !hir_prog.stores.is_empty() {
            self.declare_store_runtime();
            for sd in &hir_prog.stores {
                self.declare_store(sd)?;
                self.store_defs.insert(sd.name.clone(), sd.clone());
            }
        }

        if !hir_prog.migrations.is_empty() {
            if hir_prog.stores.is_empty() {
                self.declare_store_runtime();
            }
            for mig in &hir_prog.migrations {
                let mfn = self.gen_migration(mig)?;
                self.migration_fns.push(mfn);
            }
        }

        for gdef in &prog.globals {
            let llvm_ty = self.llvm_ty(&gdef.ty);
            let gv = self
                .module
                .add_global(llvm_ty, None, &format!("__jinn_global_{}", gdef.name));
            gv.set_initializer(&self.zero_init(&gdef.ty));
            gv.set_linkage(Linkage::Internal);
            self.globals.insert(gdef.name, (gv, gdef.ty.clone()));
        }

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

        self.generate_vtables(&hir_prog.trait_impls)?;

        if !hir_prog.actors.is_empty() {
            for ad in &hir_prog.actors {
                self.compile_actor_loop(ad)?;
            }
        }

        for sup in &hir_prog.supervisors {
            self.compile_supervisor(sup)?;
        }

        for func in &prog.functions {
            self.compile_mir_fn(func)?;
        }

        self.finalize_debug();
        if std::env::var("JINN_DUMP_IR").is_ok() {
            self.module.print_to_stderr();
        }
        self.module.verify().map_err(|e| e.to_string())
    }

    fn declare_mir_fn(&mut self, func: &mir::Function) -> Result<(), String> {
        let ptys: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
        let ret = func.ret_ty.clone();

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        // Coroutine/generator bodies have a fixed `void(ptr gen_ptr)` ABI: the
        // single LLVM parameter is the generator struct pointer (captures are
        // reloaded from it in the prologue, see `compile_mir_fn`). They are
        // never called via a normal MIR `Call` — only referenced as a function
        // pointer by `jinn_coro_create` — so the captures do not appear in the
        // LLVM signature. Internal linkage: purely module-local.
        if func.is_coroutine {
            let void = self.ctx.void_type();
            let ft = void.fn_type(&[ptr_ty.into()], false);
            let fv = self
                .module
                .add_function(&func.name.as_str(), ft, Some(Linkage::Internal));
            self.tag_fn(fv);
            self.apply_fn_attrs(fv, &func.attrs);
            self.fns.insert(func.name, (fv, ptys, ret));
            return Ok(());
        }

        let lp: Vec<BasicMetadataTypeEnum<'ctx>> = ptys
            .iter()
            .map(|t| match t {
                Type::Struct(_, _) | Type::Tuple(_) | Type::Enum(_) => ptr_ty.into(),
                _ => self.llvm_ty(t).into(),
            })
            .collect();

        let is_main = func.name == "main";
        if is_main && !self.lib_mode {
            let ft = self.mk_fn_type(&ret, &lp, false);
            let user_fv = self.module.add_function("__jinn_user_main", ft, None);
            self.tag_fn(user_fv);
            self.apply_fn_attrs(user_fv, &func.attrs);
            user_fv.set_linkage(Linkage::Internal);
            self.fns.insert(func.name, (user_fv, ptys, ret));

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

            if let Some(init_fn) = &self.global_init_fn {
                b!(self.bld.build_call(*init_fn, &[], ""));
            }

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
            for (i, p) in func.params.iter().enumerate() {
                let loc = inkwell::attributes::AttributeLoc::Param(i as u32);
                fv.add_attribute(loc, self.attr("noundef"));
                self.tag_param_ownership(fv, loc, &p.ownership, &p.ty);
            }
            self.fns.insert(func.name, (fv, ptys, ret));
        }
        Ok(())
    }

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
        self.cur_fn_is_coroutine = func.is_coroutine;

        for bb in &func.blocks {
            let llvm_bb = self.ctx.append_basic_block(fv, &bb.label.as_str());
            self.block_map.insert(bb.id, llvm_bb);
        }

        if func.is_coroutine {
            // Coroutine ABI: the single LLVM parameter is the generator struct
            // pointer. Stash it in `__coro_ctx` (consumed by `__yield`/`Return`
            // epilogue) and reload each capture (= MIR param) from the struct
            // in the entry block, where it dominates the whole body.
            let entry_bb = self.block_map[&func.entry];
            self.bld.position_at_end(entry_bb);

            let ptr = self.ctx.ptr_type(AddressSpace::default());
            let gen_ptr_param = fv
                .get_first_param()
                .expect("ICE: coroutine fn has no parameter")
                .into_pointer_value();
            let gen_ptr_alloca = self.entry_alloca(ptr.into(), "__coro_ctx");
            b!(self.bld.build_store(gen_ptr_alloca, gen_ptr_param));
            self.set_var("__coro_ctx", gen_ptr_alloca, Type::Ptr(Box::new(Type::I64)));

            for (i, param) in func.params.iter().enumerate() {
                let off = Self::GEN_SIZE + (i as u64) * 8;
                let slot_ptr = self.gen_field_ptr(gen_ptr_param, off, "cap.slot")?;
                let llvm_ty = self.llvm_ty(&param.ty);
                let loaded = b!(self.bld.build_load(llvm_ty, slot_ptr, &param.name.as_str()));
                self.value_map.insert(param.value, loaded);
                self.value_types.insert(param.value, param.ty.clone());
            }
        } else {
            for (i, param) in func.params.iter().enumerate() {
                let llvm_val = fv.get_nth_param(i as u32).unwrap();
                self.value_map.insert(param.value, llvm_val);
                self.value_types.insert(param.value, param.ty.clone());

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

                    self.value_types.insert(param.value, effective_ty);
                }
            }
        }

        for bb in &func.blocks {
            let llvm_bb = self.block_map[&bb.id];
            self.bld.position_at_end(llvm_bb);

            for phi in &bb.phis {
                // Phis merge SSA *values*, including aggregate structs/tuples/
                // enums (LLVM represents these as first-class aggregate phis).
                // A struct local may be *produced* as an alloca-backed pointer
                // (StructInit / in-place FieldSet/FieldClear), but a phi over
                // such producers must still be by *value*: a pointer phi would
                // require coercing by-value incomings (e.g. call results) into
                // a shared entry-block alloca, which is a single memory cell
                // reused across loop iterations — collapsing distinct
                // loop-carried values into one another (a use-after-free /
                // double-free hazard). By-value incomings are used directly;
                // pointer incomings are loaded across the predecessor edge in
                // the coercion pass below. Downstream consumers
                // (FieldGet/FieldSet/FieldClear/drop/coerce_call_args) all
                // already handle the struct-value form, so the phi is left out
                // of `self_allocs`.
                let llvm_ty = self.llvm_ty(&phi.ty);
                let phi_val = b!(self.bld.build_phi(llvm_ty, &format!("v{}", phi.dest.0)));
                self.value_map.insert(phi.dest, phi_val.as_basic_value());
                // Record the phi's source-level type so type-dependent consumers
                // (e.g. FieldGet on a String/struct phi result) resolve it
                // correctly. Without this, a `length` field-get on a phi value
                // misresolves to a raw struct extract.
                self.value_types.insert(phi.dest, phi.ty.clone());
                self.pending_phis.push(PendingPhi {
                    phi: phi_val,
                    incoming: phi.incoming.clone(),
                });
            }

            for inst in &bb.insts {
                tracing::trace!(
                    target: "jinnc::codegen::mir",
                    "  emit {:?} dest={:?} kind={:?}",
                    inst.dest, inst.dest, inst.kind
                );
                let val = self.emit_inst(inst)?;
                if let Some(dest) = inst.dest {
                    self.value_map.insert(dest, val);
                    self.value_types.insert(dest, inst.ty.clone());
                }
            }

            let exit_bb = self
                .bld
                .get_insert_block()
                .expect("ICE: builder has no insert block");
            self.block_exit_map.insert(bb.id, exit_bb);

            self.emit_terminator(&bb.terminator, &func.ret_ty)?;
        }

        for pp in &self.pending_phis {
            let phi_ty = pp.phi.as_basic_value().get_type();
            let incoming: Vec<(BasicValueEnum<'ctx>, LLVMBlock<'ctx>)> = pp
                .incoming
                .iter()
                .filter_map(|(block_id, val_id)| {


                    let llvm_bb = self
                        .block_exit_map
                        .get(block_id)
                        .or_else(|| self.block_map.get(block_id))?;
                    let llvm_val = self.value_map.get(val_id)?;

                    let v = if llvm_val.get_type() != phi_ty {
                        if phi_ty.is_struct_type() && llvm_val.is_pointer_value() {
                            // The phi merges struct *values* (see construction
                            // above), but this incoming was produced as an
                            // alloca-backed pointer (StructInit / in-place
                            // FieldSet/FieldClear). Load the struct value across
                            // the predecessor edge — positioned just before the
                            // predecessor's terminator so the load dominates the
                            // edge — and feed the value to the phi.
                            let ptr = (*llvm_val).into_pointer_value();
                            match llvm_bb.get_terminator() {
                                Some(t) => self.bld.position_before(&t),
                                None => self.bld.position_at_end(*llvm_bb),
                            }
                            self.bld
                                .build_load(phi_ty, ptr, "phi.load")
                                .expect("ICE: failed to load phi coercion")
                        } else {
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

        self.pop_debug_scope();

        Ok(())
    }

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

    fn emit_terminator(&mut self, term: &mir::Terminator, ret_ty: &Type) -> Result<(), String> {
        match term {
            mir::Terminator::Goto(target) => {
                let bb = self.block_map[target];
                b!(self.bld.build_unconditional_branch(bb));
            }
            mir::Terminator::Branch(cond, then_bb, else_bb) => {
                let cond_val = self.val(*cond).into_int_value();

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
                if self.cur_fn_is_coroutine {
                    // A coroutine body never returns to a caller: every exit
                    // point marks the generator done and suspends back to the
                    // resumer. `__coro_ctx` holds the generator struct pointer
                    // (stored in the entry-block prologue). The return value,
                    // if any, is discarded — coroutines communicate via `yield`.
                    let _ = val;
                    let ptr = self.ctx.ptr_type(AddressSpace::default());
                    let i8t = self.ctx.i8_type();
                    let (gen_alloca, _) = self
                        .find_var("__coro_ctx")
                        .cloned()
                        .ok_or("internal: no __coro_ctx in coroutine body")?;
                    let gen_ptr =
                        b!(self.bld.build_load(ptr, gen_alloca, "gen.ctx")).into_pointer_value();
                    let done_ptr = self.gen_field_ptr(gen_ptr, Self::GEN_DONE_OFF, "gen.done")?;
                    b!(self.bld.build_store(done_ptr, i8t.const_int(1, false)));
                    let gen_suspend = self
                        .module
                        .get_function("jinn_gen_suspend")
                        .ok_or("jinn_gen_suspend not declared")?;
                    b!(self.bld.build_call(gen_suspend, &[gen_ptr.into()], ""));
                    b!(self.bld.build_unreachable());
                    return Ok(());
                }
                if let Some(vid) = val {
                    let v = self.val(*vid);
                    let expected = self.llvm_ty(ret_ty);
                    if v.get_type() == expected {
                        b!(self.bld.build_return(Some(&v)));
                    } else if matches!(ret_ty, Type::Tuple(_)) && v.is_array_value() {
                        let alloca = self.entry_alloca(v.get_type(), "tup.coerce");
                        b!(self.bld.build_store(alloca, v));
                        let coerced = b!(self.bld.build_load(expected, alloca, "tup.ret"));
                        b!(self.bld.build_return(Some(&coerced)));
                    } else {
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

    fn val(&mut self, id: mir::ValueId) -> BasicValueEnum<'ctx> {
        let v = self.value_map.get(&id).copied().unwrap_or_else(|| {
            tracing::error!(target: "jinnc::codegen::mir", "missing value for {:?}", id);
            tracing::error!(
                target: "jinnc::codegen::mir",
                "  available values: {:?}",
                self.value_map.keys().collect::<Vec<_>>()
            );
            panic!(
                "MIR codegen: missing value for {:?} \u{2014} this is a compiler bug",
                id
            );
        });

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
