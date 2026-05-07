#![allow(unused_imports, unused_variables)]
use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn emit_store_insert(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();

        // Build fake hir::Expr values from MIR values — we need to call compile_store_insert
        // which expects &[hir::Expr]. Instead, we'll emit the LLVM IR directly.
        let ensure_fn_name = format!("__store_ensure_{store_name}");
        if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
            b!(self.bld.build_call(ensure_fn, &[], ""));
        } else {
            // Generate the ensure_open function
            let ensure_fn = self.gen_store_ensure_open(&sd)?;
            b!(self.bld.build_call(ensure_fn, &[], ""));
        }

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;

        // @before_insert hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeInsert(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;
        let rec_size = self.store_record_size(&sd);

        let rec_ptr = self.entry_alloca(st.into(), "store.rec");
        let memset_fn = crate::codegen::fn_or_die(&self.module, "memset");
        b!(self.bld.build_call(
            memset_fn,
            &[
                rec_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));

        // Read current count for sid assignment
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        let count_for_sid = self.entry_alloca(i64t.into(), "ins.cnt");
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_for_sid.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));
        let old_cnt = b!(self.bld.build_load(i64t, count_for_sid, "old.cnt")).into_int_value();
        let new_sid = b!(self
            .bld
            .build_int_add(old_cnt, i64t.const_int(1, false), "new.sid"));

        // Get current time
        self.ensure_time_fn();
        let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
        let time_fn = crate::codegen::fn_or_die(&self.module, "time");
        let now = self
            .call_result(b!(self.bld.build_call(
                time_fn,
                &[ptr_ty.const_null().into()],
                "now"
            )))
            .into_int_value();

        // Populate fields: auto-fill builtins, map user args to user fields
        let builtin_names = [
            "sid",
            "uuid",
            "hash",
            "created",
            "updated",
            "deleted",
            "__version",
        ];
        let is_simple = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Simple);
        let mut user_val_idx = 0usize;
        for (i, field_def) in sd.fields.iter().enumerate() {
            let gep = b!(self
                .bld
                .build_struct_gep(st, rec_ptr, i as u32, &field_def.name.as_str()));
            if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                match &*field_def.name.as_str() {
                    "sid" => {
                        b!(self.bld.build_store(gep, new_sid));
                    }
                    "uuid" => {
                        let uuid_str = self.gen_store_uuid(new_sid, now)?;
                        self.copy_string_to_fixed_buf(uuid_str, gep)?;
                    }
                    "hash" => {
                        let empty = self.compile_str_literal("")?;
                        self.copy_string_to_fixed_buf(empty, gep)?;
                    }
                    "created" | "updated" => {
                        b!(self.bld.build_store(gep, now));
                    }
                    "deleted" => {
                        b!(self.bld.build_store(gep, i64t.const_int(0, false)));
                    }
                    "__version" => {
                        b!(self.bld.build_store(gep, i64t.const_int(1, false)));
                    }
                    _ => {}
                }
            } else {
                if user_val_idx < args.len() {
                    let val = self.val(args[user_val_idx]);
                    match &field_def.ty {
                        Type::String => {
                            self.copy_string_to_fixed_buf(val, gep)?;
                        }
                        _ => {
                            b!(self.bld.build_store(gep, val));
                        }
                    }
                    user_val_idx += 1;
                }
            }
        }

        // @unique enforcement: check uniqueness before writing
        // Create a skip block for duplicates — branches past the write
        let fn_val = self
            .bld
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let insert_done_bb = self.ctx.append_basic_block(fn_val, "insert.done");
        {
            let contains_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_contains");
            let mut user_idx = 0usize;
            for field_def in &sd.fields {
                let is_unique = Compiler::field_is_unique(field_def);
                if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                    continue;
                }
                if is_unique && user_idx < args.len() {
                    let val = self.val(args[user_idx]);
                    let idx_ptr = self.load_store_idx(store_name, &field_def.name.as_str())?;
                    let hash = self.idx_hash_field(val, &field_def.ty)?;
                    let result = b!(self.bld.build_call(
                        contains_fn,
                        &[idx_ptr.into(), hash.into()],
                        "uniq.chk"
                    ))
                    .try_as_basic_value()
                    .basic()
                    .expect("ICE: call returned void")
                    .into_int_value();
                    let cmp = b!(self.bld.build_int_compare(
                        inkwell::IntPredicate::NE,
                        result,
                        self.ctx.i32_type().const_int(0, false),
                        "uniq.fail"
                    ));
                    let dup_bb = self.ctx.append_basic_block(fn_val, "uniq.dup");
                    let ok_bb = self.ctx.append_basic_block(fn_val, "uniq.ok");
                    b!(self.bld.build_conditional_branch(cmp, dup_bb, ok_bb));

                    // Duplicate: unlock and skip to done
                    self.bld.position_at_end(dup_bb);
                    self.store_unlock(fp)?;
                    b!(self.bld.build_unconditional_branch(insert_done_bb));

                    self.bld.position_at_end(ok_bb);
                }
                if !(!is_simple && builtin_names.contains(&&*field_def.name.as_str())) {
                    user_idx += 1;
                }
            }
        }

        // Seek to logical end (R13: file may be reserved larger; SEEK_END
        // would land in zero-padded reserved region). Compute logical end
        // first, reserve, then seek.
        let fseek_fn = crate::codegen::fn_or_die(&self.module, "fseek");
        let header_size = crate::codegen::stores::HEADER_SIZE;
        let reserve_fn = self
            .module
            .get_function("jade_store_reserve")
            .unwrap_or_else(|| {
                let void_ty = self.ctx.void_type();
                let ptr_ty = self.ctx.ptr_type(inkwell::AddressSpace::default());
                let ft = void_ty.fn_type(&[ptr_ty.into(), i64t.into(), i64t.into()], false);
                self.module.add_function(
                    "jade_store_reserve",
                    ft,
                    Some(inkwell::module::Linkage::External),
                )
            });
        b!(self.bld.build_call(
            reserve_fn,
            &[
                fp.into(),
                old_cnt.into(),
                i64t.const_int(rec_size, false).into(),
            ],
            ""
        ));

        let logical_end_off = b!(self.bld.build_int_nsw_mul(
            old_cnt,
            i64t.const_int(rec_size, false),
            "ins.logoff"
        ));
        let logical_end = b!(self.bld.build_int_nsw_add(
            logical_end_off,
            i64t.const_int(header_size, false),
            "ins.logend"
        ));
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                logical_end.into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));

        // Compute the byte offset of this new record for index insertion
        let rec_byte_offset =
            b!(self
                .bld
                .build_int_mul(old_cnt, i64t.const_int(rec_size, false), "rec.mul"));
        let rec_byte_offset = b!(self.bld.build_int_add(
            rec_byte_offset,
            i64t.const_int(header_size, false),
            "rec.off"
        ));

        let fwrite_fn = crate::codegen::fn_or_die(&self.module, "fwrite");
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));

        // Update count
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "count.buf");
        let fread_fn = crate::codegen::fn_or_die(&self.module, "fread");
        b!(self.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));
        let old_count = b!(self.bld.build_load(i64t, count_buf, "old.count")).into_int_value();
        let new_count =
            b!(self
                .bld
                .build_int_add(old_count, i64t.const_int(1, false), "new.count"));
        b!(self.bld.build_store(count_buf, new_count));
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));

        let fflush_fn = crate::codegen::fn_or_die(&self.module, "fflush");
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        // Index insertion: update all @index/@unique indexes with (hash, offset)
        {
            let insert_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_insert");
            let mut user_idx = 0usize;
            for field_def in &sd.fields {
                let has_idx = Compiler::field_has_index(field_def);
                if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                    // builtin fields — skip index (no @index on builtins)
                    continue;
                }
                if has_idx && user_idx < args.len() {
                    let val = self.val(args[user_idx]);
                    let idx_ptr = self.load_store_idx(store_name, &field_def.name.as_str())?;
                    let hash = self.idx_hash_field(val, &field_def.ty)?;
                    b!(self.bld.build_call(
                        insert_fn,
                        &[idx_ptr.into(), hash.into(), rec_byte_offset.into()],
                        ""
                    ));
                }
                if !(!is_simple && builtin_names.contains(&&*field_def.name.as_str())) {
                    user_idx += 1;
                }
            }
        }

        // @column store: append numeric user fields to column files
        let is_column = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Column);
        if is_column {
            let i64t = self.ctx.i64_type();
            let col_append_fn = crate::codegen::fn_or_die(&self.module, "jade_col_append");
            let mut col_user_idx = 0usize;
            for field_def in &sd.fields {
                if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                    continue;
                }
                if col_user_idx < args.len() {
                    if field_def.ty == Type::I64 || field_def.ty == Type::F64 {
                        let col_handle =
                            self.load_col_handle(store_name, &field_def.name.as_str(), 8)?;
                        let val = self.val(args[col_user_idx]);
                        let tmp = self.entry_alloca(i64t.into(), "col.tmp");
                        b!(self.bld.build_store(tmp, val));
                        b!(self.bld.build_call(
                            col_append_fn,
                            &[col_handle.into(), tmp.into()],
                            ""
                        ));
                    }
                }
                col_user_idx += 1;
            }
        }

        // @bloom fields: add values to bloom filters
        {
            let mut bloom_user_idx = 0usize;
            for field_def in &sd.fields {
                if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                    continue;
                }
                let has_bloom = field_def
                    .decorators
                    .iter()
                    .any(|d| *d == crate::ast::FieldDecorator::Bloom);
                if has_bloom && bloom_user_idx < args.len() {
                    let bloom = self
                        .load_bloom_handle(store_name, &field_def.name.as_str(), 10000)?;
                    if field_def.ty == Type::I64 {
                        let val = self.val(args[bloom_user_idx]);
                        let add_fn = crate::codegen::fn_or_die(&self.module, "jade_bloom_add_i64");
                        b!(self
                            .bld
                            .build_call(add_fn, &[bloom.into(), val.into()], ""));
                    }
                }
                bloom_user_idx += 1;
            }
        }

        // @search fields: add text to FTS index
        {
            let mut fts_user_idx = 0usize;
            for field_def in &sd.fields {
                if !is_simple && builtin_names.contains(&&*field_def.name.as_str()) {
                    continue;
                }
                let has_search = field_def
                    .decorators
                    .iter()
                    .any(|d| *d == crate::ast::FieldDecorator::Search);
                if has_search && fts_user_idx < args.len() && field_def.ty == Type::String {
                    let fts = self.load_fts_handle(store_name, &field_def.name.as_str())?;
                    let val = self.val(args[fts_user_idx]);
                    let data = self.string_data(val)?;
                    let len = self.string_len(val)?;
                    let add_fn = crate::codegen::fn_or_die(&self.module, "jade_fts_add");
                    let doc_id = new_sid; // use the record's sid as document id
                    b!(self.bld.build_call(
                        add_fn,
                        &[fts.into(), doc_id.into(), data.into(), len.into()],
                        ""
                    ));
                }
                fts_user_idx += 1;
            }
        }

        // WAL: log the insert
        self.wal_write_insert(store_name, rec_ptr, rec_size)?;

        // @after_insert hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterInsert(fname) = dec {
                if let Some(hook_fn) = self.module.get_function(&fname.as_str()) {
                    b!(self.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        self.store_unlock(fp)?;

        // Branch to the done block (for uniqueness skip merging)
        b!(self.bld.build_unconditional_branch(insert_done_bb));
        self.bld.position_at_end(insert_done_bb);

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }
}
