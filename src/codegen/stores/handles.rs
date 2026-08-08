use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn load_col_handle(
        &mut self,
        store_name: &str,
        field_name: &str,
        elem_size: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let global_name = format!("__store_{store_name}_col_{field_name}");

        let global = if let Some(g) = self.module.get_global(&global_name) {
            g
        } else {
            let g = self
                .module
                .add_global(ptr_ty, Some(AddressSpace::default()), &global_name);
            g.set_initializer(&ptr_ty.const_null());
            g
        };

        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "col.cur"))
        .into_pointer_value();
        let is_null = b!(self.bld.build_is_null(current, "col.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "col.open");
        let cont_bb = self.ctx.append_basic_block(fv, "col.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let col_path = format!("{store_name}_{field_name}.col\0");
        let col_str = b!(self.bld.build_global_string_ptr(&col_path, "col.path"));
        let size_val = i64t.const_int(elem_size, false);
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_col_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[col_str.as_pointer_value().into(), size_val.into()],
                "col.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "col.ptr"))
        .into_pointer_value();
        Ok(result)
    }

    pub(crate) fn store_record_size(&self, sd: &hir::StoreDef) -> u64 {
        let rec_name = format!("__store_{}_rec", sd.name);
        let st = self
            .module
            .get_struct_type(&rec_name)
            .expect("ICE: struct type not declared");
        self.type_store_size(st.into())
    }

    pub(crate) fn load_bloom_handle(
        &mut self,
        store_name: &str,
        field_name: &str,
        expected_items: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let global_name = format!("__store_{store_name}_bloom_{field_name}");

        let global = if let Some(g) = self.module.get_global(&global_name) {
            g
        } else {
            let g = self
                .module
                .add_global(ptr_ty, Some(AddressSpace::default()), &global_name);
            g.set_initializer(&ptr_ty.const_null());
            g
        };

        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "bloom.cur"))
        .into_pointer_value();
        let is_null = b!(self.bld.build_is_null(current, "bloom.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "bloom.open");
        let cont_bb = self.ctx.append_basic_block(fv, "bloom.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let bloom_path = format!("{store_name}_{field_name}.bloom\0");
        let bloom_str = b!(self.bld.build_global_string_ptr(&bloom_path, "bloom.path"));
        let items_val = i64t.const_int(expected_items, false);
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_bloom_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[bloom_str.as_pointer_value().into(), items_val.into()],
                "bloom.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "bloom.ptr"))
        .into_pointer_value();
        Ok(result)
    }

    pub(crate) fn load_fts_handle(
        &mut self,
        store_name: &str,
        field_name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_fts_{field_name}");

        let global = if let Some(g) = self.module.get_global(&global_name) {
            g
        } else {
            let g = self
                .module
                .add_global(ptr_ty, Some(AddressSpace::default()), &global_name);
            g.set_initializer(&ptr_ty.const_null());
            g
        };

        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "fts.cur"))
        .into_pointer_value();
        let is_null = b!(self.bld.build_is_null(current, "fts.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "fts.open");
        let cont_bb = self.ctx.append_basic_block(fv, "fts.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let fts_path = format!("{store_name}_{field_name}.fts\0");
        let fts_str = b!(self.bld.build_global_string_ptr(&fts_path, "fts.path"));
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_fts_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[fts_str.as_pointer_value().into()],
                "fts.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "fts.ptr"))
        .into_pointer_value();
        Ok(result)
    }

    pub(crate) fn store_schema_version(&self, sd: &hir::StoreDef) -> i64 {
        self.store_schema_versions
            .get(&sd.name)
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn gen_store_ensure_open(
        &mut self,
        sd: &hir::StoreDef,
    ) -> Result<FunctionValue<'ctx>, String> {
        let name = &sd.name;
        let fn_name = format!("__store_{name}_ensure_open");

        if let Some(fv) = self.module.get_function(&fn_name) {
            return Ok(fv);
        }

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let _i32t = self.ctx.i32_type();
        let _i8t = self.ctx.i8_type();

        let ft = self.ctx.void_type().fn_type(&[], false);
        let fv = self
            .module
            .add_function(&fn_name, ft, Some(Linkage::Internal));
        self.tag_fn(fv);

        let entry = self.ctx.append_basic_block(fv, "entry");
        let open_bb = self.ctx.append_basic_block(fv, "do_open");
        let init_bb = self.ctx.append_basic_block(fv, "init_file");
        let done_bb = self.ctx.append_basic_block(fv, "done");

        let old_fn = self.cur_fn;
        let old_bb = self.bld.get_insert_block();
        self.cur_fn = Some(fv);

        self.bld.position_at_end(entry);
        let global_name = format!("__store_{name}_fp");
        let global = self.module.get_global(&global_name).unwrap();
        let fp = b!(self.bld.build_load(ptr_ty, global.as_pointer_value(), "fp"));
        let is_null = b!(self.bld.build_is_null(fp.into_pointer_value(), "is_null"));

        let is_transient = sd
            .decorators
            .contains(&crate::ast::StoreDecorator::Transient);
        if is_transient {
            b!(self.bld.build_conditional_branch(is_null, init_bb, done_bb));
        } else {
            b!(self.bld.build_conditional_branch(is_null, open_bb, done_bb));
        }

        self.bld.position_at_end(open_bb);
        let filename = format!("{name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "store.path"));
        let mode_rw = b!(self.bld.build_global_string_ptr("r+b\0", "mode.rw"));
        let fopen_fn = crate::codegen::fn_or_die(&self.module, "fopen");
        let fp_val = self.call_result(b!(self.bld.build_call(
            fopen_fn,
            &[
                file_str.as_pointer_value().into(),
                mode_rw.as_pointer_value().into()
            ],
            "fp"
        )));
        let fp_null = b!(self
            .bld
            .build_is_null(fp_val.into_pointer_value(), "fp.null"));
        b!(self.bld.build_conditional_branch(fp_null, init_bb, done_bb));

        let store_existing_bb = self.ctx.append_basic_block(fv, "store_existing");
        open_bb
            .get_terminator()
            .expect("ICE: block has no terminator")
            .erase_from_basic_block();
        self.bld.position_at_end(open_bb);
        b!(self
            .bld
            .build_conditional_branch(fp_null, init_bb, store_existing_bb));

        self.bld.position_at_end(store_existing_bb);
        b!(self.bld.build_store(global.as_pointer_value(), fp_val));
        let fingerprint = super::store_schema_fingerprint(sd);
        let schema_version = self.store_schema_version(sd);
        let name_str = b!(self
            .bld
            .build_global_string_ptr(&format!("{name}\0"), "store.name"));
        let check_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_check_schema");
        b!(self.bld.build_call(
            check_fn,
            &[
                fp_val.into(),
                i64t.const_int(fingerprint as u64, false).into(),
                i64t.const_int(schema_version as u64, false).into(),
                name_str.as_pointer_value().into(),
            ],
            ""
        ));
        self.emit_store_recover_call(sd, global.as_pointer_value())?;
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(init_bb);
        let mode_wb = b!(self.bld.build_global_string_ptr("w+b\0", "mode.wb"));
        let new_fp = self.call_result(b!(self.bld.build_call(
            fopen_fn,
            &[
                file_str.as_pointer_value().into(),
                mode_wb.as_pointer_value().into()
            ],
            "new_fp"
        )));
        b!(self.bld.build_store(global.as_pointer_value(), new_fp));

        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");

        let magic = b!(self.bld.build_global_string_ptr("JADESTR\0", "magic"));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                magic.as_pointer_value().into(),
                i64t.const_int(1, false).into(),
                i64t.const_int(8, false).into(),
                new_fp.into(),
            ],
            ""
        ));

        let count_alloca = self.entry_alloca(i64t.into(), "hdr.count");
        b!(self.bld.build_store(count_alloca, i64t.const_int(0, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                count_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into(),
            ],
            ""
        ));

        let rec_size = self.store_record_size(sd);
        let rec_size_alloca = self.entry_alloca(i64t.into(), "hdr.recsz");
        b!(self
            .bld
            .build_store(rec_size_alloca, i64t.const_int(rec_size, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_size_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into(),
            ],
            ""
        ));

        let fingerprint = super::store_schema_fingerprint(sd);
        let schema_version = self.store_schema_version(sd);
        let fp_alloca = self.entry_alloca(i64t.into(), "hdr.fp");
        b!(self
            .bld
            .build_store(fp_alloca, i64t.const_int(fingerprint as u64, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                fp_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into(),
            ],
            ""
        ));
        let ver_alloca = self.entry_alloca(i64t.into(), "hdr.ver");
        b!(self
            .bld
            .build_store(ver_alloca, i64t.const_int(schema_version as u64, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                ver_alloca.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into(),
            ],
            ""
        ));

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        if !is_transient {
            self.emit_store_recover_call(sd, global.as_pointer_value())?;
        }

        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }

        Ok(fv)
    }

    fn emit_store_recover_call(
        &mut self,
        sd: &hir::StoreDef,
        fp_global: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let name = &sd.name;
        let rec_size = self.store_record_size(sd);
        let sid_off = self
            .store_field_offset(sd, "sid")
            .map(|o| o as i64)
            .unwrap_or(-1);
        let del_off = self
            .store_field_offset(sd, "deleted")
            .map(|o| o as i64)
            .unwrap_or(-1);
        let store_str = b!(self
            .bld
            .build_global_string_ptr(&format!("{name}.store\0"), "recover.store"));
        let wal_str = b!(self
            .bld
            .build_global_string_ptr(&format!("{name}.wal\0"), "recover.wal"));
        let recover_fn = crate::codegen::fn_or_die(&self.module, "jinn_store_recover");
        b!(self.bld.build_call(
            recover_fn,
            &[
                fp_global.into(),
                store_str.as_pointer_value().into(),
                wal_str.as_pointer_value().into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(sid_off as u64, true).into(),
                i64t.const_int(del_off as u64, true).into(),
            ],
            ""
        ));
        Ok(())
    }

    pub(crate) fn load_store_fp(&mut self, store_name: &str) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_fp");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no store global for '{store_name}'"))?;
        let fp = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "store.fp"));
        Ok(fp.into_pointer_value())
    }

    pub(crate) fn load_store_wal(
        &mut self,
        store_name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let wal_global_name = format!("__store_{store_name}_wal");
        let wal_global = self
            .module
            .get_global(&wal_global_name)
            .ok_or_else(|| format!("no WAL global for '{store_name}'"))?;
        let wal_fp = b!(self
            .bld
            .build_load(ptr_ty, wal_global.as_pointer_value(), "wal.fp"))
        .into_pointer_value();
        let is_null = b!(self.bld.build_is_null(wal_fp, "wal.null"));

        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "wal.open");
        let cont_bb = self.ctx.append_basic_block(fv, "wal.cont");

        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let wal_path = format!("{store_name}.wal\0");
        let wal_str = b!(self.bld.build_global_string_ptr(&wal_path, "wal.path"));
        let wal_open_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_open");
        let new_wal = self
            .call_result(b!(self.bld.build_call(
                wal_open_fn,
                &[wal_str.as_pointer_value().into()],
                "wal.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(wal_global.as_pointer_value(), new_wal));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);

        let result = b!(self
            .bld
            .build_load(ptr_ty, wal_global.as_pointer_value(), "wal.fp2"))
        .into_pointer_value();
        Ok(result)
    }

    pub(crate) fn wal_write_insert(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write_must");
        let op = self.ctx.i8_type().const_int(1, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    pub(crate) fn wal_write_delete(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write_must");
        let op = self.ctx.i8_type().const_int(3, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    pub(crate) fn wal_write_update(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write_must");
        let op = self.ctx.i8_type().const_int(2, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    pub(crate) fn txn_track_store(
        &mut self,
        store_name: &str,
        fp: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let _ = fp;
        let wal = self.load_store_wal(store_name)?;
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let f = self
            .module
            .get_function("jinn_txn_track_store")
            .unwrap_or_else(|| {
                let ft = self
                    .ctx
                    .void_type()
                    .fn_type(&[ptr_ty.into(), ptr_ty.into(), ptr_ty.into()], false);
                self.module
                    .add_function("jinn_txn_track_store", ft, Some(Linkage::External))
            });
        let global = self
            .module
            .get_global(&format!("__store_{store_name}_fp"))
            .expect("store fp global");
        let path_str = b!(self
            .bld
            .build_global_string_ptr(&format!("{store_name}.store\0"), "txn.path"));
        b!(self.bld.build_call(
            f,
            &[
                global.as_pointer_value().into(),
                wal.into(),
                path_str.as_pointer_value().into()
            ],
            ""
        ));
        Ok(())
    }

    pub(crate) fn txn_swap_fp(
        &mut self,
        old_fp: PointerValue<'ctx>,
        new_fp: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let f = self
            .module
            .get_function("jinn_txn_swap_fp")
            .unwrap_or_else(|| {
                let ft = self
                    .ctx
                    .void_type()
                    .fn_type(&[ptr_ty.into(), ptr_ty.into()], false);
                self.module
                    .add_function("jinn_txn_swap_fp", ft, Some(Linkage::External))
            });
        b!(self.bld.build_call(f, &[old_fp.into(), new_fp.into()], ""));
        Ok(())
    }

    pub(crate) fn wal_checkpoint(&mut self, store_name: &str) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_cp_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_checkpoint");
        b!(self.bld.build_call(wal_cp_fn, &[wal.into()], ""));
        Ok(())
    }

    const LOCK_EX: u64 = 2;
    const LOCK_UN: u64 = 8;

    pub(in crate::codegen) fn invalidate_store_indexes(
        &mut self,
        store_name: &str,
    ) -> Result<(), String> {
        let Some(sd) = self.store_defs.get(store_name).cloned() else {
            return Ok(());
        };
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let close_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_close");
        for field in &sd.fields {
            let has_index = field.decorators.iter().any(|d| {
                matches!(
                    d,
                    crate::ast::FieldDecorator::Index | crate::ast::FieldDecorator::Unique
                )
            });
            if !has_index {
                continue;
            }
            let gname = format!("__store_{}_idx_{}", sd.name, field.name);
            let Some(global) = self.module.get_global(&gname) else {
                continue;
            };
            let cur = b!(self
                .bld
                .build_load(ptr_ty, global.as_pointer_value(), "idx.stale"))
            .into_pointer_value();
            b!(self.bld.build_call(close_fn, &[cur.into()], ""));
            b!(self
                .bld
                .build_store(global.as_pointer_value(), ptr_ty.const_null()));
        }
        Ok(())
    }

    pub(crate) fn store_lock(
        &mut self,
        store_name: &str,
        fp: PointerValue<'ctx>,
    ) -> Result<(), String> {
        self.store_wlock_call(store_name, "jinn_store_wlock")?;
        self.store_flock(fp, Self::LOCK_EX)
    }

    pub(crate) fn store_unlock(
        &mut self,
        store_name: &str,
        fp: PointerValue<'ctx>,
    ) -> Result<(), String> {
        self.store_flock(fp, Self::LOCK_UN)?;
        self.store_wlock_call(store_name, "jinn_store_wunlock")
    }

    fn store_wlock_call(&mut self, store_name: &str, fname: &str) -> Result<(), String> {
        let path = format!("{store_name}.store\0");
        let path_str = b!(self.bld.build_global_string_ptr(&path, "wlock.path"));
        let f = crate::codegen::fn_or_die(&self.module, fname);
        b!(self
            .bld
            .build_call(f, &[path_str.as_pointer_value().into()], ""));
        Ok(())
    }

    pub(in crate::codegen) fn store_flock(
        &mut self,
        fp: PointerValue<'ctx>,
        op: u64,
    ) -> Result<(), String> {
        let fileno_fn = crate::codegen::fn_or_die(&self.module, "fileno");
        let flock_fn = crate::codegen::fn_or_die(&self.module, "flock");
        let fd = self.call_result(b!(self.bld.build_call(fileno_fn, &[fp.into()], "fd")));
        let lock_op = self.ctx.i32_type().const_int(op, false);
        b!(self
            .bld
            .build_call(flock_fn, &[fd.into(), lock_op.into()], ""));
        Ok(())
    }

    pub(crate) fn ensure_time_fn(&mut self) {
        if self.module.get_function("time").is_none() {
            let i64t = self.ctx.i64_type();
            let ptr = self.ctx.ptr_type(AddressSpace::default());
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("time", ft, Some(Linkage::External));
        }
    }

    pub(crate) fn gen_store_uuid(
        &mut self,
        sid: inkwell::values::IntValue<'ctx>,
        time_val: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, String> {
        let _snprintf_fn_val = self.ensure_snprintf();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let _ptr = self.ctx.ptr_type(AddressSpace::default());

        let buf = self.entry_alloca(i8t.array_type(40).into(), "uuid.buf");
        let fmt = b!(self
            .bld
            .build_global_string_ptr("%08lx-0000-4000-8000-%012lx\0", "uuid.fmt"));

        let snprintf_fn = crate::codegen::fn_or_die(&self.module, "snprintf");
        b!(self.bld.build_call(
            snprintf_fn,
            &[
                buf.into(),
                i64t.const_int(37, false).into(),
                fmt.as_pointer_value().into(),
                sid.into(),
                time_val.into(),
            ],
            ""
        ));

        let strlen_fn = crate::codegen::fn_or_die(&self.module, "strlen");
        let len = self
            .call_result(b!(self.bld.build_call(
                strlen_fn,
                &[buf.into()],
                "uuid.len"
            )))
            .into_int_value();

        let malloc_fn = self.ensure_malloc();
        let alloc = b!(self
            .bld
            .build_int_add(len, i64t.const_int(1, false), "uuid.alloc"));
        let heap = self
            .call_result(b!(self.bld.build_call(
                malloc_fn,
                &[alloc.into()],
                "uuid.heap"
            )))
            .into_pointer_value();

        let memcpy_fn = self.ensure_memcpy();
        b!(self
            .bld
            .build_call(memcpy_fn, &[heap.into(), buf.into(), alloc.into()], ""));

        self.build_string(heap, len, i64t.const_int(0, false), "uuid.str")
    }

    pub(crate) fn field_has_index(field: &hir::StoreField) -> bool {
        field.decorators.iter().any(|d| {
            matches!(
                d,
                crate::ast::FieldDecorator::Index | crate::ast::FieldDecorator::Unique
            )
        })
    }

    pub(crate) fn field_is_unique(field: &hir::StoreField) -> bool {
        field
            .decorators
            .iter()
            .any(|d| matches!(d, crate::ast::FieldDecorator::Unique))
    }

    pub(crate) fn load_store_idx(
        &mut self,
        store_name: &str,
        field_name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_idx_{field_name}");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no index global for '{store_name}.{field_name}'"))?;
        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "idx.cur"))
        .into_pointer_value();

        let is_null = b!(self.bld.build_is_null(current, "idx.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "idx.open");
        let cont_bb = self.ctx.append_basic_block(fv, "idx.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("no store def for '{store_name}'"))?
            .clone();
        let fingerprint = super::store_schema_fingerprint(&sd);
        let idx_path = format!("{store_name}.{field_name}.idx\0");
        let idx_str = b!(self.bld.build_global_string_ptr(&idx_path, "idx.path"));
        let rebuild_flag = self.entry_alloca(i32t.into(), "idx.rebuild");
        b!(self.bld.build_store(rebuild_flag, i32t.const_int(0, false)));
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_open_checked");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[
                    idx_str.as_pointer_value().into(),
                    i64t.const_int(fingerprint as u64, false).into(),
                    rebuild_flag.into(),
                ],
                "idx.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));

        let need = b!(self.bld.build_load(i32t, rebuild_flag, "idx.need")).into_int_value();
        let need_b = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            need,
            i32t.const_int(0, false),
            "idx.need.b"
        ));
        let rebuild_bb = self.ctx.append_basic_block(fv, "idx.rebuild.go");
        b!(self
            .bld
            .build_conditional_branch(need_b, rebuild_bb, cont_bb));

        self.bld.position_at_end(rebuild_bb);
        self.gen_idx_rebuild(&sd, store_name, field_name, opened)?;
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "idx.fp2"))
        .into_pointer_value();
        Ok(result)
    }

    fn gen_idx_rebuild(
        &mut self,
        sd: &hir::StoreDef,
        store_name: &str,
        field_name: &str,
        idx_ptr: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();
        let fv = self.current_fn();
        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no record struct for '{store_name}'"))?;
        let rec_size = self.store_record_size(sd);
        let header_size = crate::codegen::stores::HEADER_SIZE;

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i as u32, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in '{store_name}'"))?;
        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        let fp = self.load_store_fp(store_name)?;

        let count_buf = self.entry_alloca(i64t.into(), "rb.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));
        let total = b!(self.bld.build_load(i64t, count_buf, "rb.n")).into_int_value();

        let i_ptr = self.entry_alloca(i64t.into(), "rb.i");
        b!(self.bld.build_store(i_ptr, i64t.const_int(0, false)));
        let rec_ptr = self.entry_alloca(st.into(), "rb.rec");

        let cond_bb = self.ctx.append_basic_block(fv, "rb.cond");
        let body_bb = self.ctx.append_basic_block(fv, "rb.body");
        let next_bb = self.ctx.append_basic_block(fv, "rb.next");
        let end_bb = self.ctx.append_basic_block(fv, "rb.end");

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let i_cur = b!(self.bld.build_load(i64t, i_ptr, "rb.icur")).into_int_value();
        let more =
            b!(self
                .bld
                .build_int_compare(inkwell::IntPredicate::SLT, i_cur, total, "rb.more"));
        b!(self.bld.build_conditional_branch(more, body_bb, end_bb));

        self.bld.position_at_end(body_bb);
        let rec_off = b!(self
            .bld
            .build_int_mul(i_cur, i64t.const_int(rec_size, false), "rb.mul"));
        let rec_off =
            b!(self
                .bld
                .build_int_add(rec_off, i64t.const_int(header_size, false), "rb.off"));
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), rec_off.into(), i32t.const_int(0, false).into()],
            ""
        ));
        b!(self.bld.build_call(
            fread_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        let live_bb = self.ctx.append_basic_block(fv, "rb.live");
        if let Some(del_idx) = deleted_idx {
            let del_gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "rb.del"));
            let del_val = b!(self.bld.build_load(i64t, del_gep, "rb.delv")).into_int_value();
            let is_del = b!(self.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "rb.isdel"
            ));
            b!(self.bld.build_conditional_branch(is_del, next_bb, live_bb));
        } else {
            b!(self.bld.build_unconditional_branch(live_bb));
        }

        self.bld.position_at_end(live_bb);
        let field_gep = b!(self
            .bld
            .build_struct_gep(st, rec_ptr, field_idx, "rb.field"));
        let hash = self.hash_store_field_from_gep(field_gep, &field_ty)?;
        let insert_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_insert");
        b!(self.bld.build_call(
            insert_fn,
            &[idx_ptr.into(), hash.into(), rec_off.into()],
            ""
        ));
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let i_next = b!(self
            .bld
            .build_int_add(i_cur, i64t.const_int(1, false), "rb.inc"));
        b!(self.bld.build_store(i_ptr, i_next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(end_bb);
        Ok(())
    }
}
