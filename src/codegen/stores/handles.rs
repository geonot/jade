//! Store handle loading, open/lock/WAL helpers, UUIDs, and index handles.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Lazily open a column file for a specific field. Returns JinnCol* pointer.
    pub(crate) fn load_col_handle(
        &mut self,
        store_name: &str,
        field_name: &str,
        elem_size: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let global_name = format!("__store_{store_name}_col_{field_name}");

        // Create or get the global for this column handle
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

    /// Lazily open a bloom filter for a specific field. Returns JinnBloom* pointer.
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

    /// Lazily open an FTS index for a specific field. Returns JinnFts* pointer.
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

        // @transient stores always create fresh — skip trying to open existing file
        let is_transient = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Transient);
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

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }

        Ok(fv)
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

    /// Load WAL file pointer for a store, opening it lazily if needed.
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
        // Load again to get the potentially-updated pointer
        let result = b!(self
            .bld
            .build_load(ptr_ty, wal_global.as_pointer_value(), "wal.fp2"))
        .into_pointer_value();
        Ok(result)
    }

    /// Write a WAL entry for an insert operation (op=1).
    pub(crate) fn wal_write_insert(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write");
        let op = self.ctx.i8_type().const_int(1, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    /// Write a WAL entry for a soft-delete operation (op=3).
    pub(crate) fn wal_write_delete(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write");
        let op = self.ctx.i8_type().const_int(3, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    /// Write a WAL entry for an update operation (op=2).
    pub(crate) fn wal_write_update(
        &mut self,
        store_name: &str,
        record_ptr: PointerValue<'ctx>,
        rec_size: u64,
    ) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_write");
        let op = self.ctx.i8_type().const_int(2, false);
        let size = self.ctx.i32_type().const_int(rec_size, false);
        b!(self.bld.build_call(
            wal_write_fn,
            &[wal.into(), op.into(), record_ptr.into(), size.into()],
            ""
        ));
        Ok(())
    }

    /// Checkpoint WAL (truncate to just header).
    pub(crate) fn wal_checkpoint(&mut self, store_name: &str) -> Result<(), String> {
        let wal = self.load_store_wal(store_name)?;
        let wal_cp_fn = crate::codegen::fn_or_die(&self.module, "jinn_wal_checkpoint");
        b!(self.bld.build_call(wal_cp_fn, &[wal.into()], ""));
        Ok(())
    }

    const LOCK_EX: u64 = 2;
    const LOCK_UN: u64 = 8;

    pub(crate) fn store_lock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        self.store_flock(fp, Self::LOCK_EX)
    }

    pub(crate) fn store_unlock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        self.store_flock(fp, Self::LOCK_UN)
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

    /// Generate a simple UUID-like string from sid and timestamp.
    /// Format: "00000sid-0000-0000-0000-00timestamp0" (36 chars).
    pub(crate) fn gen_store_uuid(
        &mut self,
        sid: inkwell::values::IntValue<'ctx>,
        time_val: inkwell::values::IntValue<'ctx>,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, String> {
        let _snprintf_fn_val = self.ensure_snprintf();
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let _ptr = self.ctx.ptr_type(AddressSpace::default());

        // Allocate 40 bytes for the UUID string
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

        // Build a Jinn String from the buffer
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

    /// Check if a field has the @index or @unique decorator
    pub(crate) fn field_has_index(field: &hir::StoreField) -> bool {
        field.decorators.iter().any(|d| {
            matches!(
                d,
                crate::ast::FieldDecorator::Index | crate::ast::FieldDecorator::Unique
            )
        })
    }

    /// Check if a field has the @unique decorator
    pub(crate) fn field_is_unique(field: &hir::StoreField) -> bool {
        field
            .decorators
            .iter()
            .any(|d| matches!(d, crate::ast::FieldDecorator::Unique))
    }

    /// Lazily open an index file, returning the JinnIndex pointer.
    /// Global: __store_{name}_idx_{field}
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

        // open: open the index file
        self.bld.position_at_end(open_bb);
        let idx_path = format!("{store_name}.{field_name}.idx\0");
        let idx_str = b!(self.bld.build_global_string_ptr(&idx_path, "idx.path"));
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[idx_str.as_pointer_value().into()],
                "idx.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "idx.fp2"))
        .into_pointer_value();
        Ok(result)
    }
}
