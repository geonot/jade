use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let vals: Vec<BasicValueEnum<'ctx>> = args.iter().map(|a| self.val(*a)).collect();
        self.emit_store_delete_vals(encoded_name, &vals)
    }

    fn cascade_children(&self, store_name: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        if let Some(sd) = self.store_defs.get(store_name) {
            for r in &sd.relations {
                if r.is_has_many
                    && r.cascade
                    && let Some(child_sd) = self.store_defs.get(&*r.target.as_str())
                    && let Some(back) = child_sd
                        .relations
                        .iter()
                        .find(|b| !b.is_has_many && &*b.target.as_str() == store_name)
                {
                    out.push((r.target.to_string(), back.field.to_string()));
                }
            }
        }
        for (cname, csd) in &self.store_defs {
            for r in &csd.relations {
                if !r.is_has_many && r.cascade && &*r.target.as_str() == store_name {
                    out.push((cname.to_string(), r.field.to_string()));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    fn ensure_cascade_delete_fn(
        &mut self,
        child: &str,
        fk: &str,
        hard: bool,
    ) -> Result<inkwell::values::FunctionValue<'ctx>, String> {
        let name = format!(
            "__cascade_{}_{child}__{fk}",
            if hard { "dst" } else { "del" }
        );
        if let Some(f) = self.module.get_function(&name) {
            return Ok(f);
        }
        let i64t = self.ctx.i64_type();
        let fn_ty = self.ctx.void_type().fn_type(&[i64t.into()], false);
        let f = self.module.add_function(&name, fn_ty, None);
        let saved_block = self.bld.get_insert_block();
        let saved_fn = self.cur_fn;
        let entry = self.ctx.append_basic_block(f, "entry");
        self.bld.position_at_end(entry);
        self.cur_fn = Some(f);
        let sid = f
            .get_nth_param(0)
            .ok_or_else(|| "ICE: cascade fn has no param".to_string())?;
        let rest = format!("{child}__{fk}__eq");
        if hard {
            self.emit_store_hard_delete_vals(&rest, &[sid])?;
        } else {
            self.emit_store_delete_vals(&rest, &[sid])?;
        }
        b!(self.bld.build_return(None));
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_block {
            self.bld.position_at_end(bb);
        }
        Ok(f)
    }

    fn emit_cascade_calls(
        &mut self,
        edges: &[(String, String)],
        sid_buf: PointerValue<'ctx>,
        nsid_ptr: PointerValue<'ctx>,
        hard: bool,
    ) -> Result<(), String> {
        let mut fns = Vec::new();
        for (child, fk) in edges {
            fns.push(self.ensure_cascade_delete_fn(child, fk, hard)?);
        }
        let i64t = self.ctx.i64_type();
        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let j_ptr = self.entry_alloca(i64t.into(), "csc.j");
        b!(self.bld.build_store(j_ptr, i64t.const_int(0, false)));
        let loop_bb = self.ctx.append_basic_block(fv, "csc.loop");
        let body_bb = self.ctx.append_basic_block(fv, "csc.body");
        let done_bb = self.ctx.append_basic_block(fv, "csc.done");
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(loop_bb);
        let j = b!(self.bld.build_load(i64t, j_ptr, "csc.jv")).into_int_value();
        let n = b!(self.bld.build_load(i64t, nsid_ptr, "csc.nv")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::SLT, j, n, "csc.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));
        self.bld.position_at_end(body_bb);
        let slot = unsafe { b!(self.bld.build_gep(i64t, sid_buf, &[j], "csc.slot")) };
        let sid = b!(self.bld.build_load(i64t, slot, "csc.sid")).into_int_value();
        for f in &fns {
            b!(self.bld.build_call(*f, &[sid.into()], ""));
        }
        let j1 = b!(self
            .bld
            .build_int_add(j, i64t.const_int(1, false), "csc.j1"));
        b!(self.bld.build_store(j_ptr, j1));
        b!(self.bld.build_unconditional_branch(loop_bb));
        self.bld.position_at_end(done_bb);
        Ok(())
    }

    fn emit_store_delete_vals(
        &mut self,
        encoded_name: &str,
        vals: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, _, _, _, _) = Self::parse_encoded_filter(encoded_name)?;
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let is_simple = sd.decorators.contains(&crate::ast::StoreDecorator::Simple);

        if is_simple || sd.fields.iter().all(|f| f.name != "deleted") {
            return self.emit_store_hard_delete_vals(encoded_name, vals);
        }

        let (store_name, field_name, op, primary_pred, extra_specs) =
            Self::parse_encoded_filter(encoded_name)?;
        if vals.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let fp = self.store_lock(store_name)?;
        self.txn_track_store(store_name, fp)?;

        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec
                && let Some(hook_fn) = self.module.get_function(&fname.as_str())
            {
                b!(self.bld.build_call(hook_fn, &[], ""));
            }
        }

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted").unwrap();

        let filter_val = vals[0];

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let edges = self.cascade_children(store_name);
        let sid_pos = sd.fields.iter().position(|f| f.name == "sid");
        let cascade = if !edges.is_empty()
            && let Some(sidx) = sid_pos
        {
            let i64t = self.ctx.i64_type();
            let malloc_fn = self.ensure_malloc();
            let bufsz = b!(self.bld.build_int_add(
                b!(self
                    .bld
                    .build_int_mul(count, i64t.const_int(8, false), "csc.sz")),
                i64t.const_int(8, false),
                "csc.sz1"
            ));
            let sid_buf = self
                .call_result(b!(self.bld.build_call(
                    malloc_fn,
                    &[bufsz.into()],
                    "csc.buf"
                )))
                .into_pointer_value();
            let nsid_ptr = self.entry_alloca(i64t.into(), "csc.n");
            b!(self.bld.build_store(nsid_ptr, i64t.const_int(0, false)));
            Some((sid_buf, nsid_ptr, sidx))
        } else {
            None
        };

        self.ensure_time_fn();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let time_fn = crate::codegen::fn_or_die(&self.module, "time");
        let now = self
            .call_result(b!(self.bld.build_call(
                time_fn,
                &[ptr_ty.const_null().into()],
                "del.now"
            )))
            .into_int_value();

        let fv = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "sdel.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "sdel.loop");
        let body_bb = self.ctx.append_basic_block(fv, "sdel.body");
        let mark_bb = self.ctx.append_basic_block(fv, "sdel.mark");
        let next_bb = self.ctx.append_basic_block(fv, "sdel.next");
        let done_bb = self.ctx.append_basic_block(fv, "sdel.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "sdel.i")).into_int_value();
        let cmp =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "sdel.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "sdel.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "sdel.rec"))
        };
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            crate::ast::FilterPred,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop, epred))| {
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = vals[1 + ei];
                (*lop, fi, ft, *eop, *epred, ev)
            })
            .collect();
        let cond = self.eval_store_filter_pred(
            rec_ptr,
            st,
            field_idx,
            &field_ty,
            op,
            primary_pred,
            filter_val,
            &extras,
        )?;
        b!(self.bld.build_conditional_branch(cond, mark_bb, next_bb));

        self.bld.position_at_end(mark_bb);
        let del_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, deleted_idx as u32, "sdel.del"));
        b!(self.bld.build_store(del_gep, now));

        self.wal_write_delete(store_name, rec_ptr, rec_size)?;
        if let Some((sid_buf, nsid_ptr, sidx)) = cascade {
            let sid_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, sidx as u32, "csc.sgep"));
            let sid_v = b!(self.bld.build_load(i64t, sid_gep, "csc.sv")).into_int_value();
            let n = b!(self.bld.build_load(i64t, nsid_ptr, "csc.nl")).into_int_value();
            let slot = unsafe { b!(self.bld.build_gep(i64t, sid_buf, &[n], "csc.sl")) };
            b!(self.bld.build_store(slot, sid_v));
            let n1 = b!(self
                .bld
                .build_int_add(n, i64t.const_int(1, false), "csc.n1"));
            b!(self.bld.build_store(nsid_ptr, n1));
        }
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "sdel.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");

        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec
                && let Some(hook_fn) = self.module.get_function(&fname.as_str())
            {
                b!(self.bld.build_call(hook_fn, &[], ""));
            }
        }

        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(crate::codegen::stores::HEADER_SIZE, false)
                    .into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));
        self.store_unlock(store_name, fp)?;

        if let Some(&crate::ast::StoreDecorator::Compact(threshold)) = sd
            .decorators
            .iter()
            .find(|d| matches!(d, crate::ast::StoreDecorator::Compact(_)))
            && let Some(offset) = self.store_deleted_offset(&sd)
            && let Some(fp_g) = self.module.get_global(&format!("__store_{store_name}_fp"))
        {
            let path_lit = format!("{store_name}.store\0");
            let path_str = b!(self.bld.build_global_string_ptr(&path_lit, "cmp.path"));
            let compact_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_compact_if");
            b!(self.bld.build_call(
                compact_fn,
                &[
                    fp_g.as_pointer_value().into(),
                    path_str.as_pointer_value().into(),
                    i64t.const_int(offset, false).into(),
                    i64t.const_int(threshold, false).into(),
                ],
                ""
            ));
        }

        if let Some((sid_buf, nsid_ptr, _)) = cascade {
            self.emit_cascade_calls(&edges, sid_buf, nsid_ptr, false)?;
            let free_fn = self.ensure_free();
            b!(self.bld.build_call(free_fn, &[sid_buf.into()], ""));
        }

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(in crate::codegen) fn emit_store_hard_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let vals: Vec<BasicValueEnum<'ctx>> = args.iter().map(|a| self.val(*a)).collect();
        self.emit_store_hard_delete_vals(encoded_name, &vals)
    }

    fn emit_store_hard_delete_vals(
        &mut self,
        encoded_name: &str,
        vals: &[BasicValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, primary_op, primary_pred, extra_conds) =
            Self::parse_encoded_filter(encoded_name)?;
        if vals.is_empty() {
            return Ok(self.ctx.i64_type().const_int(0, false).into());
        }

        let (sd, st, rec_size, _fp) = self.setup_store_access(store_name)?;
        let fp = self.store_lock(store_name)?;
        self.txn_track_store(store_name, fp)?;

        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec
                && let Some(hook_fn) = self.module.get_function(&fname.as_str())
            {
                b!(self.bld.build_call(hook_fn, &[], ""));
            }
        }

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = vals[0];

        let count = self.store_read_count(fp)?;
        let buf = self.store_load_records(fp, count, rec_size)?;

        let edges = self.cascade_children(store_name);
        let sid_pos = sd.fields.iter().position(|f| f.name == "sid");
        let cascade = if !edges.is_empty()
            && let Some(sidx) = sid_pos
        {
            let malloc_fn = self.ensure_malloc();
            let bufsz = b!(self.bld.build_int_add(
                b!(self
                    .bld
                    .build_int_mul(count, i64t.const_int(8, false), "csc.sz")),
                i64t.const_int(8, false),
                "csc.sz1"
            ));
            let sid_buf = self
                .call_result(b!(self.bld.build_call(
                    malloc_fn,
                    &[bufsz.into()],
                    "csc.buf"
                )))
                .into_pointer_value();
            let nsid_ptr = self.entry_alloca(i64t.into(), "csc.n");
            b!(self.bld.build_store(nsid_ptr, i64t.const_int(0, false)));
            Some((sid_buf, nsid_ptr, sidx))
        } else {
            None
        };

        let filename = format!("{store_name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "del.path"));

        let rewrite_begin_fn = crate::codegen::fn_or_die(&self.module, "jinn_rewrite_begin");
        let rw = self
            .call_result(b!(self.bld.build_call(
                rewrite_begin_fn,
                &[file_str.as_pointer_value().into()],
                "del.rw"
            )))
            .into_pointer_value();
        let rewrite_file_fn = crate::codegen::fn_or_die(&self.module, "jinn_rewrite_file");
        let new_fp = self
            .call_result(b!(self.bld.build_call(
                rewrite_file_fn,
                &[rw.into()],
                "del.tmpfp"
            )))
            .into_pointer_value();

        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
        let magic = b!(self.bld.build_global_string_ptr("JADESTR\0", "del.magic"));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                magic.as_pointer_value().into(),
                i64t.const_int(1, false).into(),
                i64t.const_int(8, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let new_count_ptr = self.entry_alloca(i64t.into(), "del.newcount");
        b!(self
            .bld
            .build_store(new_count_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let rec_size_ptr = self.entry_alloca(i64t.into(), "del.recsz");
        b!(self
            .bld
            .build_store(rec_size_ptr, i64t.const_int(rec_size, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_size_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let (fingerprint, schema_version) = self
            .store_defs
            .get(store_name)
            .map(|sd| {
                (
                    crate::codegen::stores::store_schema_fingerprint(sd),
                    self.store_schema_versions
                        .get(&sd.name)
                        .copied()
                        .unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        let fp_hdr_ptr = self.entry_alloca(i64t.into(), "del.fp");
        b!(self
            .bld
            .build_store(fp_hdr_ptr, i64t.const_int(fingerprint as u64, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                fp_hdr_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));
        let ver_hdr_ptr = self.entry_alloca(i64t.into(), "del.ver");
        b!(self
            .bld
            .build_store(ver_hdr_ptr, i64t.const_int(schema_version as u64, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                ver_hdr_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let fv_fn = self.cur_fn.expect("ICE: cur_fn not set");
        let idx_ptr = self.entry_alloca(i64t.into(), "del.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv_fn, "del.loop");
        let body_bb = self.ctx.append_basic_block(fv_fn, "del.body");
        let keep_bb = self.ctx.append_basic_block(fv_fn, "del.keep");
        let skip_bb = self.ctx.append_basic_block(fv_fn, "del.skip");
        let done_bb = self.ctx.append_basic_block(fv_fn, "del.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "del.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "del.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "del.off"));
        let rec_ptr = unsafe {
            b!(self
                .bld
                .build_gep(self.ctx.i8_type(), buf, &[offset], "del.rec"))
        };

        let matches = {
            let extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                crate::ast::FilterPred,
                BasicValueEnum<'ctx>,
            )> = extra_conds
                .iter()
                .enumerate()
                .map(|(ei, (lop, fname, cop, cpred))| {
                    let (fi, ft) = sd
                        .fields
                        .iter()
                        .enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = vals[1 + ei];
                    (*lop, fi, ft, *cop, *cpred, ev)
                })
                .collect();
            self.eval_store_filter_pred(
                rec_ptr,
                st,
                field_idx,
                &field_ty,
                primary_op,
                primary_pred,
                filter_val,
                &extras,
            )?
        };
        let del_hook_bb = self.ctx.append_basic_block(fv_fn, "del.hook");
        b!(self
            .bld
            .build_conditional_branch(matches, del_hook_bb, keep_bb));

        self.bld.position_at_end(del_hook_bb);

        self.wal_write_delete(store_name, rec_ptr, rec_size)?;
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec
                && let Some(hook_fn) = self.module.get_function(&fname.as_str())
            {
                b!(self.bld.build_call(hook_fn, &[], ""));
            }
        }
        if let Some((sid_buf, nsid_ptr, sidx)) = cascade {
            let sid_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, sidx as u32, "csc.sgep"));
            let sid_v = b!(self.bld.build_load(i64t, sid_gep, "csc.sv")).into_int_value();
            let n = b!(self.bld.build_load(i64t, nsid_ptr, "csc.nl")).into_int_value();
            let slot = unsafe { b!(self.bld.build_gep(i64t, sid_buf, &[n], "csc.sl")) };
            b!(self.bld.build_store(slot, sid_v));
            let n1 = b!(self
                .bld
                .build_int_add(n, i64t.const_int(1, false), "csc.n1"));
            b!(self.bld.build_store(nsid_ptr, n1));
        }
        b!(self.bld.build_unconditional_branch(skip_bb));

        self.bld.position_at_end(keep_bb);
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));
        let kept = b!(self.bld.build_load(i64t, new_count_ptr, "kept")).into_int_value();
        let kept_inc = b!(self
            .bld
            .build_int_add(kept, i64t.const_int(1, false), "kept.inc"));
        b!(self.bld.build_store(new_count_ptr, kept_inc));
        b!(self.bld.build_unconditional_branch(skip_bb));

        self.bld.position_at_end(skip_bb);
        let next_idx = b!(self
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "del.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(done_bb);

        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                new_fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        let rewrite_commit_fn = crate::codegen::fn_or_die(&self.module, "jinn_rewrite_commit");
        let committed_fp = self
            .call_result(b!(self.bld.build_call(
                rewrite_commit_fn,
                &[rw.into(), fp.into()],
                "del.commit"
            )))
            .into_pointer_value();

        let global_name = format!("__store_{store_name}_fp");
        let global = self.module.get_global(&global_name).unwrap();
        b!(self
            .bld
            .build_store(global.as_pointer_value(), committed_fp));
        self.txn_swap_fp(fp, committed_fp)?;

        let drop_idx_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_drop_indexes");
        b!(self
            .bld
            .build_call(drop_idx_fn, &[file_str.as_pointer_value().into()], ""));
        self.invalidate_store_indexes(store_name)?;

        self.store_unlock(store_name, committed_fp)?;

        if let Some((sid_buf, nsid_ptr, _)) = cascade {
            self.emit_cascade_calls(&edges, sid_buf, nsid_ptr, true)?;
            b!(self.bld.build_call(free_fn, &[sid_buf.into()], ""));
        }

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }
}
