//! Store operation codegen: insert, query, count, all, delete, set, get, first, exists, destroy, restore, save.

use inkwell::values::{BasicValueEnum, PointerValue};
use crate::hir;
use crate::mir;
use crate::types::Type;
use super::super::Compiler;
use super::super::b;
use super::MirCodegen;

impl<'a, 'ctx> MirCodegen<'a, 'ctx> {
    pub(super) fn emit_store_insert(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .comp
            .store_defs
            .get(store_name)
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

        // @before_insert hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeInsert(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .comp
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;
        let rec_size = self.comp.store_record_size(&sd);

        let rec_ptr = self.comp.entry_alloca(st.into(), "store.rec");
        let memset_fn = self.comp.module.get_function("memset").unwrap();
        b!(self.comp.bld.build_call(
            memset_fn,
            &[
                rec_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into()
            ],
            ""
        ));

        // Read current count for sid assignment
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        let fread_fn = self.comp.module.get_function("fread").unwrap();
        let count_for_sid = self.comp.entry_alloca(i64t.into(), "ins.cnt");
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.comp.bld.build_call(
            fread_fn,
            &[
                count_for_sid.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));
        let old_cnt = b!(self.comp.bld.build_load(i64t, count_for_sid, "old.cnt")).into_int_value();
        let new_sid = b!(self
            .comp
            .bld
            .build_int_add(old_cnt, i64t.const_int(1, false), "new.sid"));

        // Get current time
        self.comp.ensure_time_fn();
        let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
        let time_fn = self.comp.module.get_function("time").unwrap();
        let now = self
            .comp
            .call_result(b!(self.comp.bld.build_call(
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
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, i as u32, &field_def.name));
            if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                match field_def.name.as_str() {
                    "sid" => {
                        b!(self.comp.bld.build_store(gep, new_sid));
                    }
                    "uuid" => {
                        let uuid_str = self.comp.gen_store_uuid(new_sid, now)?;
                        self.comp.copy_string_to_fixed_buf(uuid_str, gep)?;
                    }
                    "hash" => {
                        let empty = self.comp.compile_str_literal("")?;
                        self.comp.copy_string_to_fixed_buf(empty, gep)?;
                    }
                    "created" | "updated" => {
                        b!(self.comp.bld.build_store(gep, now));
                    }
                    "deleted" => {
                        b!(self.comp.bld.build_store(gep, i64t.const_int(0, false)));
                    }
                    "__version" => {
                        b!(self.comp.bld.build_store(gep, i64t.const_int(1, false)));
                    }
                    _ => {}
                }
            } else {
                if user_val_idx < args.len() {
                    let val = self.val(args[user_val_idx]);
                    match &field_def.ty {
                        Type::String => {
                            self.comp.copy_string_to_fixed_buf(val, gep)?;
                        }
                        _ => {
                            b!(self.comp.bld.build_store(gep, val));
                        }
                    }
                    user_val_idx += 1;
                }
            }
        }

        // @unique enforcement: check uniqueness before writing
        // Create a skip block for duplicates — branches past the write
        let fn_val = self
            .comp
            .bld
            .get_insert_block()
            .unwrap()
            .get_parent()
            .unwrap();
        let insert_done_bb = self.comp.ctx.append_basic_block(fn_val, "insert.done");
        let mut has_unique = false;
        {
            let contains_fn = self.comp.module.get_function("jade_idx_contains").unwrap();
            let mut user_idx = 0usize;
            for field_def in &sd.fields {
                let is_unique = Compiler::field_is_unique(field_def);
                if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                    continue;
                }
                if is_unique && user_idx < args.len() {
                    has_unique = true;
                    let val = self.val(args[user_idx]);
                    let idx_ptr = self.comp.load_store_idx(store_name, &field_def.name)?;
                    let hash = self.comp.idx_hash_field(val, &field_def.ty)?;
                    let result = b!(self.comp.bld.build_call(
                        contains_fn,
                        &[idx_ptr.into(), hash.into()],
                        "uniq.chk"
                    ))
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_int_value();
                    let cmp = b!(self.comp.bld.build_int_compare(
                        inkwell::IntPredicate::NE,
                        result,
                        self.comp.ctx.i32_type().const_int(0, false),
                        "uniq.fail"
                    ));
                    let dup_bb = self.comp.ctx.append_basic_block(fn_val, "uniq.dup");
                    let ok_bb = self.comp.ctx.append_basic_block(fn_val, "uniq.ok");
                    b!(self.comp.bld.build_conditional_branch(cmp, dup_bb, ok_bb));

                    // Duplicate: unlock and skip to done
                    self.comp.bld.position_at_end(dup_bb);
                    self.comp.store_unlock(fp)?;
                    b!(self.comp.bld.build_unconditional_branch(insert_done_bb));

                    self.comp.bld.position_at_end(ok_bb);
                }
                if !(!is_simple && builtin_names.contains(&field_def.name.as_str())) {
                    user_idx += 1;
                }
            }
        }

        // Seek to end and write record
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(0, false).into(),
                i32t.const_int(2, false).into()
            ],
            ""
        ));

        // Compute the byte offset of this new record for index insertion
        let header_size = crate::codegen::stores::HEADER_SIZE;
        let rec_byte_offset =
            b!(self
                .comp
                .bld
                .build_int_mul(old_cnt, i64t.const_int(rec_size, false), "rec.mul"));
        let rec_byte_offset = b!(self.comp.bld.build_int_add(
            rec_byte_offset,
            i64t.const_int(header_size, false),
            "rec.off"
        ));

        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(
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
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let count_buf = self.comp.entry_alloca(i64t.into(), "count.buf");
        let fread_fn = self.comp.module.get_function("fread").unwrap();
        b!(self.comp.bld.build_call(
            fread_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));
        let old_count = b!(self.comp.bld.build_load(i64t, count_buf, "old.count")).into_int_value();
        let new_count =
            b!(self
                .comp
                .bld
                .build_int_add(old_count, i64t.const_int(1, false), "new.count"));
        b!(self.comp.bld.build_store(count_buf, new_count));
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into()
            ],
            ""
        ));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));

        // Index insertion: update all @index/@unique indexes with (hash, offset)
        {
            let insert_fn = self.comp.module.get_function("jade_idx_insert").unwrap();
            let mut user_idx = 0usize;
            for field_def in &sd.fields {
                let has_idx = Compiler::field_has_index(field_def);
                if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                    // builtin fields — skip index (no @index on builtins)
                    continue;
                }
                if has_idx && user_idx < args.len() {
                    let val = self.val(args[user_idx]);
                    let idx_ptr = self.comp.load_store_idx(store_name, &field_def.name)?;
                    let hash = self.comp.idx_hash_field(val, &field_def.ty)?;
                    b!(self.comp.bld.build_call(
                        insert_fn,
                        &[idx_ptr.into(), hash.into(), rec_byte_offset.into()],
                        ""
                    ));
                }
                if !(!is_simple && builtin_names.contains(&field_def.name.as_str())) {
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
            let i64t = self.comp.ctx.i64_type();
            let col_append_fn = self.comp.module.get_function("jade_col_append").unwrap();
            let mut col_user_idx = 0usize;
            for field_def in &sd.fields {
                if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                    continue;
                }
                if col_user_idx < args.len() {
                    if field_def.ty == Type::I64 || field_def.ty == Type::F64 {
                        let col_handle =
                            self.comp.load_col_handle(store_name, &field_def.name, 8)?;
                        let val = self.val(args[col_user_idx]);
                        let tmp = self.comp.entry_alloca(i64t.into(), "col.tmp");
                        b!(self.comp.bld.build_store(tmp, val));
                        b!(self.comp.bld.build_call(
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
                if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                    continue;
                }
                let has_bloom = field_def
                    .decorators
                    .iter()
                    .any(|d| *d == crate::ast::FieldDecorator::Bloom);
                if has_bloom && bloom_user_idx < args.len() {
                    let bloom = self
                        .comp
                        .load_bloom_handle(store_name, &field_def.name, 10000)?;
                    if field_def.ty == Type::I64 {
                        let val = self.val(args[bloom_user_idx]);
                        let add_fn = self.comp.module.get_function("jade_bloom_add_i64").unwrap();
                        b!(self
                            .comp
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
                if !is_simple && builtin_names.contains(&field_def.name.as_str()) {
                    continue;
                }
                let has_search = field_def
                    .decorators
                    .iter()
                    .any(|d| *d == crate::ast::FieldDecorator::Search);
                if has_search && fts_user_idx < args.len() && field_def.ty == Type::String {
                    let fts = self.comp.load_fts_handle(store_name, &field_def.name)?;
                    let val = self.val(args[fts_user_idx]);
                    let data = self.comp.string_data(val)?;
                    let len = self.comp.string_len(val)?;
                    let add_fn = self.comp.module.get_function("jade_fts_add").unwrap();
                    let doc_id = new_sid; // use the record's sid as document id
                    b!(self.comp.bld.build_call(
                        add_fn,
                        &[fts.into(), doc_id.into(), data.into(), len.into()],
                        ""
                    ));
                }
                fts_user_idx += 1;
            }
        }

        // WAL: log the insert
        self.comp.wal_write_insert(store_name, rec_ptr, rec_size)?;

        // @after_insert hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterInsert(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        self.comp.store_unlock(fp)?;

        // Branch to the done block (for uniqueness skip merging)
        b!(self.comp.bld.build_unconditional_branch(insert_done_bb));
        self.comp.bld.position_at_end(insert_done_bb);

        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    pub(super) fn emit_store_query(
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
                _ => {
                    i += 1;
                    continue;
                }
            };
            let efield = segments[i + 1];
            let eop = Self::parse_store_op(segments[i + 2]);
            extra_specs.push((lop, efield, eop));
            i += 3;
        }

        let sd = self
            .comp
            .store_defs
            .get(store_name)
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
        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        // ── Index-accelerated path: O(1) lookup for equality on indexed fields ──
        let primary_field = &sd.fields[field_idx];
        let use_index = matches!(op, crate::ast::BinOp::Eq)
            && Compiler::field_has_index(primary_field)
            && extra_specs.is_empty();

        if use_index {
            // Allocate zero'd result
            let result_ptr = self.comp.entry_alloca(st.into(), "qi.result");
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

            // Hash the filter value and look up in index
            let idx_ptr = self.comp.load_store_idx(store_name, field_name)?;
            let hash = self.comp.idx_hash_field(filter_val, &field_ty)?;
            let lookup_fn = self.comp.module.get_function("jade_idx_lookup").unwrap();
            let file_offset = self
                .comp
                .call_result(b!(self.comp.bld.build_call(
                    lookup_fn,
                    &[idx_ptr.into(), hash.into()],
                    "qi.off"
                )))
                .into_int_value();

            let fv_fn = self.comp.cur_fn.unwrap();
            let found_bb = self.comp.ctx.append_basic_block(fv_fn, "qi.found");
            let done_bb = self.comp.ctx.append_basic_block(fv_fn, "qi.done");

            // If offset == -1, not found → return empty result
            let not_found = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                file_offset,
                i64t.const_int(u64::MAX, false), // -1 as unsigned
                "qi.miss"
            ));
            b!(self
                .comp
                .bld
                .build_conditional_branch(not_found, done_bb, found_bb));

            // Found path: seek to the record and read it
            self.comp.bld.position_at_end(found_bb);
            let fseek_fn = self.comp.module.get_function("fseek").unwrap();
            b!(self.comp.bld.build_call(
                fseek_fn,
                &[
                    fp.into(),
                    file_offset.into(),
                    i32t.const_int(0, false).into()
                ],
                ""
            ));
            let fread_fn = self.comp.module.get_function("fread").unwrap();
            b!(self.comp.bld.build_call(
                fread_fn,
                &[
                    result_ptr.into(),
                    i64t.const_int(rec_size, false).into(),
                    i64t.const_int(1, false).into(),
                    fp.into(),
                ],
                ""
            ));

            // Check soft-deleted
            if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
                let del_gep =
                    b!(self
                        .comp
                        .bld
                        .build_struct_gep(st, result_ptr, del_idx as u32, "qi.del"));
                let del_val =
                    b!(self.comp.bld.build_load(i64t, del_gep, "qi.del.val")).into_int_value();
                let is_deleted = b!(self.comp.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    del_val,
                    i64t.const_int(0, false),
                    "qi.is_del"
                ));
                let copy_bb = self.comp.ctx.append_basic_block(fv_fn, "qi.copy");
                // If deleted, zero out result and skip
                let zero_bb = self.comp.ctx.append_basic_block(fv_fn, "qi.zero");
                b!(self
                    .comp
                    .bld
                    .build_conditional_branch(is_deleted, zero_bb, copy_bb));

                self.comp.bld.position_at_end(zero_bb);
                b!(self.comp.bld.build_call(
                    memset_fn,
                    &[
                        result_ptr.into(),
                        i32t.const_int(0, false).into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));
                b!(self.comp.bld.build_unconditional_branch(done_bb));

                self.comp.bld.position_at_end(copy_bb);
            }
            // Record is valid, result_ptr already has the data from fread
            b!(self.comp.bld.build_unconditional_branch(done_bb));

            self.comp.bld.position_at_end(done_bb);
            let result = self.comp.load_store_record_as_jade(st, result_ptr, &sd)?;
            return Ok(result);
        }

        // ── Full scan path: linear search through all records ──
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
        let loop_idx_ptr = self.comp.entry_alloca(i64t.into(), "q.idx");
        b!(self
            .comp
            .bld
            .build_store(loop_idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv_fn, "q.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv_fn, "q.body");
        let match_bb = self.comp.ctx.append_basic_block(fv_fn, "q.match");
        let next_bb = self.comp.ctx.append_basic_block(fv_fn, "q.next");
        let done_bb = self.comp.ctx.append_basic_block(fv_fn, "q.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, loop_idx_ptr, "idx")).into_int_value();
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "q.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset = b!(self
            .comp
            .bld
            .build_int_mul(idx, i64t.const_int(rec_size, false), "q.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "q.rec"))
        };

        // Skip soft-deleted records (deleted != 0)
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "q.del"));
            let del_val = b!(self.comp.bld.build_load(i64t, del_gep, "q.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "q.is_del"
            ));
            let filter_bb = self.comp.ctx.append_basic_block(fv_fn, "q.filter");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.comp.bld.position_at_end(filter_bb);
        }

        let cond = {
            // Build extras for compound filters
            let mut extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                BasicValueEnum<'ctx>,
            )> = Vec::new();
            for (ei, (lop, efield, eop)) in extra_specs.iter().enumerate() {
                let (eidx, ety) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .ok_or_else(|| format!("unknown field '{efield}' in store '{store_name}'"))?;
                let eval = self.value_map[&args[ei + 1]];
                extras.push((*lop, eidx, ety, *eop, eval));
            }
            self.comp
                .eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?
        };
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, match_bb, next_bb));

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
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "q.next"));
        b!(self.comp.bld.build_store(loop_idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.comp.load_store_record_as_jade(st, result_ptr, &sd)?;
        Ok(result)
    }

    pub(super) fn parse_store_op(s: &str) -> crate::ast::BinOp {
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

    pub(super) fn emit_store_count(&mut self, store_name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .comp
            .store_defs
            .get(store_name)
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

        // Check if store has a `deleted` field (non-@simple)
        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        if deleted_idx.is_none() {
            // Simple store: just read header count
            let fseek_fn = self.comp.module.get_function("fseek").unwrap();
            b!(self.comp.bld.build_call(
                fseek_fn,
                &[
                    fp.into(),
                    i64t.const_int(8, false).into(),
                    i32t.const_int(0, false).into()
                ],
                ""
            ));
            let count_buf = self.comp.entry_alloca(i64t.into(), "sc.count");
            b!(self
                .comp
                .bld
                .build_store(count_buf, i64t.const_int(0, false)));
            let fread_fn = self.comp.module.get_function("fread").unwrap();
            b!(self.comp.bld.build_call(
                fread_fn,
                &[
                    count_buf.into(),
                    i64t.const_int(8, false).into(),
                    i64t.const_int(1, false).into(),
                    fp.into()
                ],
                ""
            ));
            return Ok(b!(self.comp.bld.build_load(i64t, count_buf, "count")));
        }

        // Non-simple store: scan records, count where deleted == 0
        let rec_name = format!("__store_{store_name}_rec");
        let st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);
        let del_idx = deleted_idx.unwrap();

        let total_count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, total_count, rec_size)?;

        let fv = self.comp.cur_fn.unwrap();
        let live_ptr = self.comp.entry_alloca(i64t.into(), "sc.live");
        b!(self
            .comp
            .bld
            .build_store(live_ptr, i64t.const_int(0, false)));
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "sc.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "sc.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "sc.body");
        let inc_bb = self.comp.ctx.append_basic_block(fv, "sc.inc");
        let next_bb = self.comp.ctx.append_basic_block(fv, "sc.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "sc.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "sc.i")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            total_count,
            "sc.cmp"
        ));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "sc.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "sc.rec"))
        };
        let del_gep = b!(self
            .comp
            .bld
            .build_struct_gep(st, rec_ptr, del_idx as u32, "sc.del"));
        let del_val = b!(self.comp.bld.build_load(i64t, del_gep, "sc.del.val")).into_int_value();
        let is_live = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            del_val,
            i64t.const_int(0, false),
            "sc.live_cmp"
        ));
        b!(self
            .comp
            .bld
            .build_conditional_branch(is_live, inc_bb, next_bb));

        self.comp.bld.position_at_end(inc_bb);
        let cur = b!(self.comp.bld.build_load(i64t, live_ptr, "sc.cur")).into_int_value();
        let inc = b!(self
            .comp
            .bld
            .build_int_add(cur, i64t.const_int(1, false), "sc.inc"));
        b!(self.comp.bld.build_store(live_ptr, inc));
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "sc.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        Ok(b!(self.comp.bld.build_load(i64t, live_ptr, "count")))
    }

    /// Emit a view count: iterate source store, count records matching filter.
    pub(super) fn emit_view_count(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_store_op(segments[0]);

        let mut extra_specs: Vec<(crate::ast::LogicalOp, &str, crate::ast::BinOp)> = Vec::new();
        let mut i = 1;
        while i + 2 < segments.len() {
            let lop = match segments[i] {
                "and" => crate::ast::LogicalOp::And,
                "or" => crate::ast::LogicalOp::Or,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let efield = segments[i + 1];
            let eop = Self::parse_store_op(segments[i + 2]);
            extra_specs.push((lop, efield, eop));
            i += 3;
        }

        let sd = self
            .comp
            .store_defs
            .get(store_name)
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
        let st = self.comp.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.comp.store_record_size(&sd);

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        let fv = self.comp.cur_fn.unwrap();
        let match_count_ptr = self.comp.entry_alloca(i64t.into(), "vc.cnt");
        b!(self
            .comp
            .bld
            .build_store(match_count_ptr, i64t.const_int(0, false)));
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "vc.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "vc.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "vc.body");
        let match_bb = self.comp.ctx.append_basic_block(fv, "vc.match");
        let next_bb = self.comp.ctx.append_basic_block(fv, "vc.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "vc.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "vc.i")).into_int_value();
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "vc.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "vc.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "vc.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "vc.del"));
            let del_val =
                b!(self.comp.bld.build_load(i64t, del_gep, "vc.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "vc.is_del"
            ));
            let filter_bb = self.comp.ctx.append_basic_block(fv, "vc.filter");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.comp.bld.position_at_end(filter_bb);
        }

        // Apply filter
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (eidx, ety) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap();
                let eval = self.value_map[&args[ei + 1]];
                (*lop, eidx, ety, *eop, eval)
            })
            .collect();
        let cond = self
            .comp
            .eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, match_bb, next_bb));

        // Match: increment counter
        self.comp.bld.position_at_end(match_bb);
        let cur = b!(self.comp.bld.build_load(i64t, match_count_ptr, "vc.cur")).into_int_value();
        let inc = b!(self
            .comp
            .bld
            .build_int_add(cur, i64t.const_int(1, false), "vc.inc"));
        b!(self.comp.bld.build_store(match_count_ptr, inc));
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "vc.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        Ok(b!(self.comp.bld.build_load(
            i64t,
            match_count_ptr,
            "vc.result"
        )))
    }

    /// Emit a view all: iterate source store, collect all records matching filter.
    pub(super) fn emit_view_all(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let parts: Vec<&str> = encoded_name.splitn(3, "__").collect();
        if parts.len() < 3 || args.is_empty() {
            return Ok(self
                .comp
                .ctx
                .ptr_type(inkwell::AddressSpace::default())
                .const_null()
                .into());
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_store_op(segments[0]);

        let mut extra_specs: Vec<(crate::ast::LogicalOp, &str, crate::ast::BinOp)> = Vec::new();
        let mut i = 1;
        while i + 2 < segments.len() {
            let lop = match segments[i] {
                "and" => crate::ast::LogicalOp::And,
                "or" => crate::ast::LogicalOp::Or,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let efield = segments[i + 1];
            let eop = Self::parse_store_op(segments[i + 2]);
            extra_specs.push((lop, efield, eop));
            i += 3;
        }

        let sd = self
            .comp
            .store_defs
            .get(store_name)
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

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.comp.store_read_count(fp)?;
        let raw_buf = self.comp.store_load_records(fp, count, rec_size)?;

        // Allocate max-capacity output buffer (worst case all records match)
        let one = i64t.const_int(1, false);
        let jade_total =
            b!(self
                .comp
                .bld
                .build_int_mul(count, i64t.const_int(jade_size, false), "va.total"));
        let jade_alloc = b!(self.comp.bld.build_select(
            b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jade_total,
                i64t.const_int(0, false),
                "va.isz"
            )),
            one,
            jade_total,
            "va.alloc"
        ))
        .into_int_value();
        let malloc_fn = self.comp.ensure_malloc();
        let jade_buf = self
            .comp
            .call_result(b!(self.comp.bld.build_call(
                malloc_fn,
                &[jade_alloc.into()],
                "va.buf"
            )))
            .into_pointer_value();

        let fv = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "va.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));
        let out_ptr = self.comp.entry_alloca(i64t.into(), "va.out");
        b!(self.comp.bld.build_store(out_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "va.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "va.body");
        let copy_bb = self.comp.ctx.append_basic_block(fv, "va.copy");
        let next_bb = self.comp.ctx.append_basic_block(fv, "va.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "va.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "va.i")).into_int_value();
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "va.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let raw_off =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "va.roff"));
        let raw_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), raw_buf, &[raw_off], "va.rptr"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep =
                b!(self
                    .comp
                    .bld
                    .build_struct_gep(rec_st, raw_ptr, del_idx as u32, "va.del"));
            let del_val =
                b!(self.comp.bld.build_load(i64t, del_gep, "va.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "va.is_del"
            ));
            let filter_bb = self.comp.ctx.append_basic_block(fv, "va.filter");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.comp.bld.position_at_end(filter_bb);
        }

        // Apply filter
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (eidx, ety) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap();
                let eval = self.value_map[&args[ei + 1]];
                (*lop, eidx, ety, *eop, eval)
            })
            .collect();
        let cond = self.comp.eval_store_filter(
            raw_ptr, rec_st, field_idx, &field_ty, op, filter_val, &extras,
        )?;
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, copy_bb, next_bb));

        // Copy matching record
        self.comp.bld.position_at_end(copy_bb);
        let out_idx = b!(self.comp.bld.build_load(i64t, out_ptr, "va.oi")).into_int_value();
        let jade_val = self.comp.load_store_record_as_jade(rec_st, raw_ptr, &sd)?;
        let jade_off =
            b!(self
                .comp
                .bld
                .build_int_mul(out_idx, i64t.const_int(jade_size, false), "va.joff"));
        let jade_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), jade_buf, &[jade_off], "va.jptr"))
        };
        b!(self.comp.bld.build_store(jade_ptr, jade_val));
        let next_out =
            b!(self
                .comp
                .bld
                .build_int_add(out_idx, i64t.const_int(1, false), "va.oinc"));
        b!(self.comp.bld.build_store(out_ptr, next_out));
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_i = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "va.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_i));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[raw_buf.into()], ""));
        let final_count = b!(self.comp.bld.build_load(i64t, out_ptr, "va.count")).into_int_value();

        // Build vec header: {ptr, len, cap}
        let vec_ty = self.comp.ctx.struct_type(
            &[
                self.comp
                    .ctx
                    .ptr_type(inkwell::AddressSpace::default())
                    .into(),
                i64t.into(),
                i64t.into(),
            ],
            false,
        );
        let vec_ptr = self.comp.entry_alloca(vec_ty.into(), "va.vec");
        let ptr_gep = b!(self
            .comp
            .bld
            .build_struct_gep(vec_ty, vec_ptr, 0, "va.vec.ptr"));
        b!(self.comp.bld.build_store(ptr_gep, jade_buf));
        let len_gep = b!(self
            .comp
            .bld
            .build_struct_gep(vec_ty, vec_ptr, 1, "va.vec.len"));
        b!(self.comp.bld.build_store(len_gep, final_count));
        let cap_gep = b!(self
            .comp
            .bld
            .build_struct_gep(vec_ty, vec_ptr, 2, "va.vec.cap"));
        b!(self.comp.bld.build_store(cap_gep, count));

        Ok(b!(self.comp.bld.build_load(vec_ty, vec_ptr, "va.result")))
    }

    pub(super) fn emit_store_all(&mut self, store_name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let sd = self
            .comp
            .store_defs
            .get(store_name)
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

        let jade_total = b!(self.comp.bld.build_int_mul(
            count,
            i64t.const_int(jade_size, false),
            "all.jade_total"
        ));
        let one = i64t.const_int(1, false);
        let jade_alloc = b!(self.comp.bld.build_select(
            b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::EQ,
                jade_total,
                i64t.const_int(0, false),
                "all.jade_isz"
            )),
            one,
            jade_total,
            "all.jade_alloc"
        ))
        .into_int_value();
        let malloc_fn = self.comp.ensure_malloc();
        let jade_buf = self
            .comp
            .call_result(b!(self.comp.bld.build_call(
                malloc_fn,
                &[jade_alloc.into()],
                "all.jade"
            )))
            .into_pointer_value();

        let has_strings = sd.fields.iter().any(|f| matches!(f.ty, Type::String));
        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted");

        if has_strings || deleted_idx.is_some() {
            // Need a loop: either for string conversion or soft-delete filtering (or both)
            let fv = self.comp.cur_fn.unwrap();
            let idx_ptr = self.comp.entry_alloca(i64t.into(), "all.idx");
            b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

            // Separate output counter for soft-delete filtering
            let out_ptr = self.comp.entry_alloca(i64t.into(), "all.out");
            b!(self.comp.bld.build_store(out_ptr, i64t.const_int(0, false)));

            let loop_bb = self.comp.ctx.append_basic_block(fv, "all.loop");
            let body_bb = self.comp.ctx.append_basic_block(fv, "all.body");
            let copy_bb = self.comp.ctx.append_basic_block(fv, "all.copy");
            let next_bb = self.comp.ctx.append_basic_block(fv, "all.next");
            let done_bb = self.comp.ctx.append_basic_block(fv, "all.done");

            b!(self.comp.bld.build_unconditional_branch(loop_bb));
            self.comp.bld.position_at_end(loop_bb);
            let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "all.i")).into_int_value();
            let cmp = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::ULT,
                idx,
                count,
                "all.cmp"
            ));
            b!(self
                .comp
                .bld
                .build_conditional_branch(cmp, body_bb, done_bb));

            self.comp.bld.position_at_end(body_bb);
            let raw_off =
                b!(self
                    .comp
                    .bld
                    .build_int_mul(idx, i64t.const_int(rec_size, false), "all.roff"));
            let raw_ptr = unsafe {
                b!(self.comp.bld.build_gep(
                    self.comp.ctx.i8_type(),
                    raw_buf,
                    &[raw_off],
                    "all.rptr"
                ))
            };

            // Skip soft-deleted records
            if let Some(del_idx) = deleted_idx {
                let del_gep =
                    b!(self
                        .comp
                        .bld
                        .build_struct_gep(rec_st, raw_ptr, del_idx as u32, "all.del"));
                let del_val =
                    b!(self.comp.bld.build_load(i64t, del_gep, "all.del.val")).into_int_value();
                let is_deleted = b!(self.comp.bld.build_int_compare(
                    inkwell::IntPredicate::NE,
                    del_val,
                    i64t.const_int(0, false),
                    "all.is_del"
                ));
                b!(self
                    .comp
                    .bld
                    .build_conditional_branch(is_deleted, next_bb, copy_bb));
            } else {
                b!(self.comp.bld.build_unconditional_branch(copy_bb));
            }

            self.comp.bld.position_at_end(copy_bb);
            let out_idx = b!(self.comp.bld.build_load(i64t, out_ptr, "all.oi")).into_int_value();

            if has_strings {
                let jade_val = self.comp.load_store_record_as_jade(rec_st, raw_ptr, &sd)?;
                let jade_off = b!(self.comp.bld.build_int_mul(
                    out_idx,
                    i64t.const_int(jade_size, false),
                    "all.joff"
                ));
                let jade_ptr = unsafe {
                    b!(self.comp.bld.build_gep(
                        self.comp.ctx.i8_type(),
                        jade_buf,
                        &[jade_off],
                        "all.jptr"
                    ))
                };
                b!(self.comp.bld.build_store(jade_ptr, jade_val));
            } else {
                let src_off = b!(self.comp.bld.build_int_mul(
                    idx,
                    i64t.const_int(rec_size, false),
                    "all.soff"
                ));
                let src_ptr = unsafe {
                    b!(self.comp.bld.build_gep(
                        self.comp.ctx.i8_type(),
                        raw_buf,
                        &[src_off],
                        "all.src"
                    ))
                };
                let dst_off = b!(self.comp.bld.build_int_mul(
                    out_idx,
                    i64t.const_int(rec_size, false),
                    "all.doff"
                ));
                let dst_ptr = unsafe {
                    b!(self.comp.bld.build_gep(
                        self.comp.ctx.i8_type(),
                        jade_buf,
                        &[dst_off],
                        "all.dst"
                    ))
                };
                let memcpy_fn = self.comp.ensure_memcpy();
                b!(self.comp.bld.build_call(
                    memcpy_fn,
                    &[
                        dst_ptr.into(),
                        src_ptr.into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));
            }

            let next_out =
                b!(self
                    .comp
                    .bld
                    .build_int_add(out_idx, i64t.const_int(1, false), "all.onext"));
            b!(self.comp.bld.build_store(out_ptr, next_out));
            b!(self.comp.bld.build_unconditional_branch(next_bb));

            self.comp.bld.position_at_end(next_bb);
            let next_idx =
                b!(self
                    .comp
                    .bld
                    .build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.comp.bld.build_store(idx_ptr, next_idx));
            b!(self.comp.bld.build_unconditional_branch(loop_bb));

            self.comp.bld.position_at_end(done_bb);
        } else {
            // Simple store, no strings, no deleted field: memcpy
            let total = b!(self.comp.bld.build_int_mul(
                count,
                i64t.const_int(rec_size, false),
                "all.total"
            ));
            let memcpy_fn = self.comp.ensure_memcpy();
            b!(self.comp.bld.build_call(
                memcpy_fn,
                &[jade_buf.into(), raw_buf.into(), total.into()],
                ""
            ));
        }

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[raw_buf.into()], ""));

        Ok(jade_buf.into())
    }

    pub(super) fn emit_store_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // For non-@simple stores: soft-delete by setting deleted timestamp
        // For @simple stores: fall back to hard delete
        let (store_name, _, _, _) = Self::parse_encoded_filter(encoded_name)?;
        let sd = self
            .comp
            .store_defs
            .get(store_name)
            .ok_or_else(|| format!("unknown store '{store_name}'"))?
            .clone();
        let is_simple = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Simple);

        if is_simple || sd.fields.iter().all(|f| f.name != "deleted") {
            return self.emit_store_hard_delete(encoded_name, args);
        }

        // Soft delete: set deleted = time() on matching records
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.comp.store_lock(fp)?;

        // @before_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let deleted_idx = sd.fields.iter().position(|f| f.name == "deleted").unwrap();

        let filter_val = self.val(args[0]);

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        // Get current time for the deleted timestamp
        self.comp.ensure_time_fn();
        let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
        let time_fn = self.comp.module.get_function("time").unwrap();
        let now = self
            .comp
            .call_result(b!(self.comp.bld.build_call(
                time_fn,
                &[ptr_ty.const_null().into()],
                "del.now"
            )))
            .into_int_value();

        let fv = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "sdel.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "sdel.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "sdel.body");
        let mark_bb = self.comp.ctx.append_basic_block(fv, "sdel.mark");
        let next_bb = self.comp.ctx.append_basic_block(fv, "sdel.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "sdel.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "sdel.i")).into_int_value();
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "sdel.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "sdel.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "sdel.rec"))
        };
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = self.val(args[1 + ei]);
                (*lop, fi, ft, *eop, ev)
            })
            .collect();
        let cond = self
            .comp
            .eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, mark_bb, next_bb));

        self.comp.bld.position_at_end(mark_bb);
        let del_gep =
            b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, deleted_idx as u32, "sdel.del"));
        b!(self.comp.bld.build_store(del_gep, now));
        // WAL: log the soft-delete
        self.comp.wal_write_delete(store_name, rec_ptr, rec_size)?;
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "sdel.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        // Write updated records back
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();

        // @after_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(super::super::stores::HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));
        self.comp.store_unlock(fp)?;

        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    pub(super) fn emit_store_hard_delete(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, primary_op, extra_conds) =
            Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }

        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.comp.store_lock(fp)?;

        // @before_delete hook
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::BeforeDelete(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }

        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
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
        let new_fp = self
            .comp
            .call_result(b!(self.comp.bld.build_call(
                fopen_fn,
                &[
                    file_str.as_pointer_value().into(),
                    mode_wb.as_pointer_value().into()
                ],
                "del.fp"
            )))
            .into_pointer_value();

        let global_name = format!("__store_{store_name}_fp");
        let global = self.comp.module.get_global(&global_name).unwrap();
        b!(self.comp.bld.build_store(global.as_pointer_value(), new_fp));

        // Write header: magic + count placeholder + rec_size
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        let magic = b!(self
            .comp
            .bld
            .build_global_string_ptr("JADESTR\0", "del.magic"));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                magic.as_pointer_value().into(),
                i64t.const_int(1, false).into(),
                i64t.const_int(8, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let new_count_ptr = self.comp.entry_alloca(i64t.into(), "del.newcount");
        b!(self
            .comp
            .bld
            .build_store(new_count_ptr, i64t.const_int(0, false)));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let rec_size_ptr = self.comp.entry_alloca(i64t.into(), "del.recsz");
        b!(self
            .comp
            .bld
            .build_store(rec_size_ptr, i64t.const_int(rec_size, false)));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                rec_size_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

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
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "del.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "del.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "del.rec"))
        };

        let matches = {
            let extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                BasicValueEnum<'ctx>,
            )> = extra_conds
                .iter()
                .enumerate()
                .map(|(ei, (lop, fname, cop))| {
                    let (fi, ft) = sd
                        .fields
                        .iter()
                        .enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = self.val(args[1 + ei]);
                    (*lop, fi, ft, *cop, ev)
                })
                .collect();
            self.comp.eval_store_filter(
                rec_ptr, st, field_idx, &field_ty, primary_op, filter_val, &extras,
            )?
        };
        let del_hook_bb = self.comp.ctx.append_basic_block(fv_fn, "del.hook");
        b!(self
            .comp
            .bld
            .build_conditional_branch(matches, del_hook_bb, keep_bb));

        // @after_delete hook — only fires for records being deleted
        self.comp.bld.position_at_end(del_hook_bb);
        // WAL: log the hard-delete
        self.comp.wal_write_delete(store_name, rec_ptr, rec_size)?;
        for dec in &sd.decorators {
            if let crate::ast::StoreDecorator::AfterDelete(fname) = dec {
                if let Some(hook_fn) = self.comp.module.get_function(fname) {
                    b!(self.comp.bld.build_call(hook_fn, &[], ""));
                }
            }
        }
        b!(self.comp.bld.build_unconditional_branch(skip_bb));

        self.comp.bld.position_at_end(keep_bb);
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));
        let kept = b!(self.comp.bld.build_load(i64t, new_count_ptr, "kept")).into_int_value();
        let kept_inc = b!(self
            .comp
            .bld
            .build_int_add(kept, i64t.const_int(1, false), "kept.inc"));
        b!(self.comp.bld.build_store(new_count_ptr, kept_inc));
        b!(self.comp.bld.build_unconditional_branch(skip_bb));

        self.comp.bld.position_at_end(skip_bb);
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "del.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        // Update count in header
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                new_fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                new_count_ptr.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                new_fp.into()
            ],
            ""
        ));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        self.comp.store_unlock(fp)?;
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    pub(super) fn emit_store_set(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Name format: {store_name}__{field}__{op}[__and/or__{f2}__{op2}]*__fields_{f1}_{f2}_...
        // Split out the __fields_ suffix first.
        let (filter_part, fields_part) = if let Some(pos) = encoded_name.find("__fields_") {
            (&encoded_name[..pos], &encoded_name[pos + 9..]) // skip "__fields_"
        } else {
            return Err(format!(
                "mir_codegen: malformed store.set name '{encoded_name}'"
            ));
        };

        let field_names: Vec<&str> = fields_part.split('_').collect();

        let (store_name, filter_field, primary_op, extra_conds) =
            Self::parse_encoded_filter(filter_part)?;
        if args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let extra_count = extra_conds.len();

        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.comp.store_lock(fp)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
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
            if arg_idx >= args.len() {
                break;
            }
            let val = self.val(args[arg_idx]);
            let field_pos = sd
                .fields
                .iter()
                .position(|f| f.name == *fname)
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
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "set.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "set.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "set.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep =
                b!(self
                    .comp
                    .bld
                    .build_struct_gep(st, rec_ptr, del_idx as u32, "set.del"));
            let del_val =
                b!(self.comp.bld.build_load(i64t, del_gep, "set.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "set.is_del"
            ));
            let filter_bb = self.comp.ctx.append_basic_block(fv, "set.filter");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.comp.bld.position_at_end(filter_bb);
        }

        let matches = {
            let extras: Vec<(
                crate::ast::LogicalOp,
                usize,
                Type,
                crate::ast::BinOp,
                BasicValueEnum<'ctx>,
            )> = extra_conds
                .iter()
                .enumerate()
                .map(|(ei, (lop, fname, cop))| {
                    let (fi, ft) = sd
                        .fields
                        .iter()
                        .enumerate()
                        .find(|(_, f)| f.name == *fname)
                        .map(|(i, f)| (i, f.ty.clone()))
                        .unwrap_or((0, Type::I64));
                    let ev = self.val(args[1 + ei]);
                    (*lop, fi, ft, *cop, ev)
                })
                .collect();
            self.comp.eval_store_filter(
                rec_ptr, st, field_idx, &field_ty, primary_op, filter_val, &extras,
            )?
        };
        b!(self
            .comp
            .bld
            .build_conditional_branch(matches, update_bb, next_bb));

        self.comp.bld.position_at_end(update_bb);

        // Phase 7: If @versioned, save old record to versions file and increment __version
        let is_versioned = Compiler::store_is_versioned(&sd);
        if is_versioned {
            // Read the record's current sid and __version
            let sid_idx = sd.fields.iter().position(|f| f.name == "sid");
            let ver_idx = sd.fields.iter().position(|f| f.name == "__version");
            if let (Some(si), Some(vi)) = (sid_idx, ver_idx) {
                let sid_gep =
                    b!(self
                        .comp
                        .bld
                        .build_struct_gep(st, rec_ptr, si as u32, "ver.sid.gep"));
                let sid_val =
                    b!(self.comp.bld.build_load(i64t, sid_gep, "ver.sid")).into_int_value();
                let ver_gep =
                    b!(self
                        .comp
                        .bld
                        .build_struct_gep(st, rec_ptr, vi as u32, "ver.ver.gep"));
                let old_ver =
                    b!(self.comp.bld.build_load(i64t, ver_gep, "ver.old")).into_int_value();

                // Save old record to versions file
                let ver_fp = self.comp.load_store_ver(store_name)?;
                let ver_append_fn = self.comp.module.get_function("jade_ver_append").unwrap();
                b!(self.comp.bld.build_call(
                    ver_append_fn,
                    &[
                        ver_fp.into(),
                        sid_val.into(),
                        old_ver.into(),
                        rec_ptr.into(),
                        i64t.const_int(rec_size, false).into()
                    ],
                    ""
                ));

                // Increment __version
                let new_ver =
                    b!(self
                        .comp
                        .bld
                        .build_int_add(old_ver, i64t.const_int(1, false), "ver.new"));
                b!(self.comp.bld.build_store(ver_gep, new_ver));
            }
        }

        for (fpos, _fname, val) in &assign_vals {
            let fty = &sd.fields[*fpos].ty;
            let gep = b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, *fpos as u32, "set.assign"));
            match fty {
                Type::String => {
                    self.comp.copy_string_to_fixed_buf(*val, gep)?;
                }
                _ => {
                    b!(self.comp.bld.build_store(gep, *val));
                }
            }
        }

        // Update the `updated` timestamp on modified records
        if let Some(upd_idx) = sd.fields.iter().position(|f| f.name == "updated") {
            self.comp.ensure_time_fn();
            let time_fn = self.comp.module.get_function("time").unwrap();
            let ptr_ty = self.comp.ctx.ptr_type(inkwell::AddressSpace::default());
            let now = self
                .comp
                .call_result(b!(self.comp.bld.build_call(
                    time_fn,
                    &[ptr_ty.const_null().into()],
                    "set.now"
                )))
                .into_int_value();
            let upd_gep =
                b!(self
                    .comp
                    .bld
                    .build_struct_gep(st, rec_ptr, upd_idx as u32, "set.upd"));
            b!(self.comp.bld.build_store(upd_gep, now));
        }

        // WAL: log the update
        self.comp.wal_write_update(store_name, rec_ptr, rec_size)?;

        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "set.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(super::super::stores::HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.comp.store_unlock(fp)?;
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    /// Parse a filter op string back to a BinOp.
    pub(super) fn parse_filter_op(s: &str) -> crate::ast::BinOp {
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

    /// Parse the encoded filter name into (store_name, field, op, extra_conds).
    pub(super) fn parse_encoded_filter(
        encoded: &str,
    ) -> Result<
        (
            &str,
            &str,
            crate::ast::BinOp,
            Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)>,
        ),
        String,
    > {
        let parts: Vec<&str> = encoded.splitn(3, "__").collect();
        if parts.len() < 3 {
            return Err(format!("malformed encoded filter: '{encoded}'"));
        }
        let store_name = parts[0];
        let field_name = parts[1];
        let remainder = parts[2];
        let segments: Vec<&str> = remainder.split("__").collect();
        let op = Self::parse_filter_op(segments[0]);

        let mut extra: Vec<(crate::ast::LogicalOp, String, crate::ast::BinOp)> = Vec::new();
        let mut i = 1;
        while i + 2 < segments.len() {
            let lop = match segments[i] {
                "and" => crate::ast::LogicalOp::And,
                "or" => crate::ast::LogicalOp::Or,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let efield = segments[i + 1].to_string();
            let eop = Self::parse_filter_op(segments[i + 2]);
            extra.push((lop, efield, eop));
            i += 3;
        }
        Ok((store_name, field_name, op, extra))
    }

    /// Common helper to set up store access: ensure open, load_fp, get sd + rec type + rec size.
    pub(super) fn setup_store_access(
        &mut self,
        store_name: &str,
    ) -> Result<
        (
            hir::StoreDef,
            inkwell::types::StructType<'ctx>,
            u64,
            PointerValue<'ctx>,
        ),
        String,
    > {
        let sd = self
            .comp
            .store_defs
            .get(store_name)
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
        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .comp
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;
        let rec_size = self.comp.store_record_size(&sd);
        Ok((sd, st, rec_size, fp))
    }

    // ── StoreGet: lookup by sid (i64) ──
    pub(super) fn emit_store_get(
        &mut self,
        store_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        if args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let sid_val = self.val(args[0]).into_int_value();

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        let result_ptr = self.comp.entry_alloca(st.into(), "get.result");
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

        // sid is always the first field (index 0) for non-@simple stores
        // Find sid index
        let sid_idx = sd.fields.iter().position(|f| f.name == "sid").unwrap_or(0);

        let fv = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "get.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "get.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "get.body");
        let match_bb = self.comp.ctx.append_basic_block(fv, "get.match");
        let next_bb = self.comp.ctx.append_basic_block(fv, "get.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "get.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "get.i")).into_int_value();
        let cmp =
            b!(self
                .comp
                .bld
                .build_int_compare(inkwell::IntPredicate::ULT, idx, count, "get.cmp"));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "get.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "get.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep =
                b!(self
                    .comp
                    .bld
                    .build_struct_gep(st, rec_ptr, del_idx as u32, "get.del"));
            let del_val =
                b!(self.comp.bld.build_load(i64t, del_gep, "get.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "get.is_del"
            ));
            let check_bb = self.comp.ctx.append_basic_block(fv, "get.check");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, check_bb));
            self.comp.bld.position_at_end(check_bb);
        }

        let rec_sid_gep =
            b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, sid_idx as u32, "get.sid"));
        let rec_sid =
            b!(self.comp.bld.build_load(i64t, rec_sid_gep, "get.sid.val")).into_int_value();
        let match_cond = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::EQ,
            rec_sid,
            sid_val,
            "get.eq"
        ));
        b!(self
            .comp
            .bld
            .build_conditional_branch(match_cond, match_bb, next_bb));

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
        let next_idx = b!(self
            .comp
            .bld
            .build_int_add(idx, i64t.const_int(1, false), "get.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.comp.load_store_record_as_jade(st, result_ptr, &sd)?;
        Ok(result)
    }

    // ── StoreFirst: like query but returns first match ──
    pub(super) fn emit_store_first(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Same as emit_store_query — the query already returns first match
        self.emit_store_query(encoded_name, args)
    }

    // ── StoreExists: returns bool (1 if match found, 0 otherwise) ──
    pub(super) fn emit_store_exists(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.comp.ctx.bool_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        let i64t = self.comp.ctx.i64_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.value_map[&args[0]];

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        let fv = self.comp.cur_fn.unwrap();
        let found_ptr = self
            .comp
            .entry_alloca(self.comp.ctx.bool_type().into(), "exists.found");
        b!(self
            .comp
            .bld
            .build_store(found_ptr, self.comp.ctx.bool_type().const_int(0, false)));
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "exists.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "exists.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "exists.body");
        let match_bb = self.comp.ctx.append_basic_block(fv, "exists.match");
        let next_bb = self.comp.ctx.append_basic_block(fv, "exists.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "exists.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "exists.i")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            count,
            "exists.cmp"
        ));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "exists.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "exists.rec"))
        };

        // Skip soft-deleted records
        if let Some(del_idx) = sd.fields.iter().position(|f| f.name == "deleted") {
            let del_gep = b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, del_idx as u32, "ex.del"));
            let del_val =
                b!(self.comp.bld.build_load(i64t, del_gep, "ex.del.val")).into_int_value();
            let is_deleted = b!(self.comp.bld.build_int_compare(
                inkwell::IntPredicate::NE,
                del_val,
                i64t.const_int(0, false),
                "ex.is_del"
            ));
            let filter_bb = self.comp.ctx.append_basic_block(fv, "exists.filter");
            b!(self
                .comp
                .bld
                .build_conditional_branch(is_deleted, next_bb, filter_bb));
            self.comp.bld.position_at_end(filter_bb);
        }

        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = self.value_map[&args[ei + 1]];
                (*lop, fi, ft, *eop, ev)
            })
            .collect();
        let cond = self
            .comp
            .eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, match_bb, next_bb));

        self.comp.bld.position_at_end(match_bb);
        b!(self
            .comp
            .bld
            .build_store(found_ptr, self.comp.ctx.bool_type().const_int(1, false)));
        b!(self.comp.bld.build_unconditional_branch(done_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx =
            b!(self
                .comp
                .bld
                .build_int_add(idx, i64t.const_int(1, false), "exists.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let result =
            b!(self
                .comp
                .bld
                .build_load(self.comp.ctx.bool_type(), found_ptr, "exists.result"));
        Ok(result)
    }

    // ── StoreDestroy: hard delete (physically remove matching records) ──
    pub(super) fn emit_store_destroy(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        // Physical removal — same as what delete used to do
        self.emit_store_hard_delete(encoded_name, args)
    }

    // ── StoreRestore: clear the deleted timestamp on soft-deleted records ──
    pub(super) fn emit_store_restore(
        &mut self,
        encoded_name: &str,
        args: &[mir::ValueId],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (store_name, field_name, op, extra_specs) = Self::parse_encoded_filter(encoded_name)?;
        if args.is_empty() {
            return Ok(self.comp.ctx.i64_type().const_int(0, false).into());
        }
        let (sd, st, rec_size, fp) = self.setup_store_access(store_name)?;
        self.comp.store_lock(fp)?;
        let i64t = self.comp.ctx.i64_type();
        let i32t = self.comp.ctx.i32_type();

        let (field_idx, field_ty) = sd
            .fields
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == field_name)
            .map(|(i, f)| (i, f.ty.clone()))
            .ok_or_else(|| format!("unknown field '{field_name}' in store '{store_name}'"))?;

        let filter_val = self.val(args[0]);

        let count = self.comp.store_read_count(fp)?;
        let buf = self.comp.store_load_records(fp, count, rec_size)?;

        // Find the 'deleted' field index
        let deleted_idx = sd
            .fields
            .iter()
            .position(|f| f.name == "deleted")
            .ok_or_else(|| {
                format!("store '{store_name}' has no 'deleted' field (is it @simple?)")
            })?;

        let fv = self.comp.cur_fn.unwrap();
        let idx_ptr = self.comp.entry_alloca(i64t.into(), "restore.idx");
        b!(self.comp.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.comp.ctx.append_basic_block(fv, "restore.loop");
        let body_bb = self.comp.ctx.append_basic_block(fv, "restore.body");
        let update_bb = self.comp.ctx.append_basic_block(fv, "restore.update");
        let next_bb = self.comp.ctx.append_basic_block(fv, "restore.next");
        let done_bb = self.comp.ctx.append_basic_block(fv, "restore.done");

        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(loop_bb);
        let idx = b!(self.comp.bld.build_load(i64t, idx_ptr, "restore.i")).into_int_value();
        let cmp = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::ULT,
            idx,
            count,
            "restore.cmp"
        ));
        b!(self
            .comp
            .bld
            .build_conditional_branch(cmp, body_bb, done_bb));

        self.comp.bld.position_at_end(body_bb);
        let offset =
            b!(self
                .comp
                .bld
                .build_int_mul(idx, i64t.const_int(rec_size, false), "restore.off"));
        let rec_ptr = unsafe {
            b!(self
                .comp
                .bld
                .build_gep(self.comp.ctx.i8_type(), buf, &[offset], "restore.rec"))
        };

        // Only consider records that are actually soft-deleted
        let del_check_gep =
            b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, deleted_idx as u32, "restore.del.chk"));
        let del_check_val = b!(self
            .comp
            .bld
            .build_load(i64t, del_check_gep, "restore.del.v"))
        .into_int_value();
        let is_deleted = b!(self.comp.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            del_check_val,
            i64t.const_int(0, false),
            "restore.is_del"
        ));
        let filter_bb = self.comp.ctx.append_basic_block(fv, "restore.filter");
        b!(self
            .comp
            .bld
            .build_conditional_branch(is_deleted, filter_bb, next_bb));

        self.comp.bld.position_at_end(filter_bb);
        let extras: Vec<(
            crate::ast::LogicalOp,
            usize,
            Type,
            crate::ast::BinOp,
            BasicValueEnum<'ctx>,
        )> = extra_specs
            .iter()
            .enumerate()
            .map(|(ei, (lop, efield, eop))| {
                let (fi, ft) = sd
                    .fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name == *efield)
                    .map(|(i, f)| (i, f.ty.clone()))
                    .unwrap_or((0, Type::I64));
                let ev = self.val(args[1 + ei]);
                (*lop, fi, ft, *eop, ev)
            })
            .collect();
        let cond = self
            .comp
            .eval_store_filter(rec_ptr, st, field_idx, &field_ty, op, filter_val, &extras)?;
        b!(self
            .comp
            .bld
            .build_conditional_branch(cond, update_bb, next_bb));

        self.comp.bld.position_at_end(update_bb);
        // Set 'deleted' field to 0
        let del_gep =
            b!(self
                .comp
                .bld
                .build_struct_gep(st, rec_ptr, deleted_idx as u32, "restore.del"));
        b!(self.comp.bld.build_store(del_gep, i64t.const_int(0, false)));
        // WAL: log the restore as an update
        self.comp.wal_write_update(store_name, rec_ptr, rec_size)?;
        b!(self.comp.bld.build_unconditional_branch(next_bb));

        self.comp.bld.position_at_end(next_bb);
        let next_idx =
            b!(self
                .comp
                .bld
                .build_int_add(idx, i64t.const_int(1, false), "restore.next"));
        b!(self.comp.bld.build_store(idx_ptr, next_idx));
        b!(self.comp.bld.build_unconditional_branch(loop_bb));

        self.comp.bld.position_at_end(done_bb);
        // Write records back
        let fseek_fn = self.comp.module.get_function("fseek").unwrap();
        b!(self.comp.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(super::super::stores::HEADER_SIZE, false).into(),
                i32t.const_int(0, false).into()
            ],
            ""
        ));
        let fwrite_fn = self.comp.module.get_function("fwrite").unwrap();
        b!(self.comp.bld.build_call(
            fwrite_fn,
            &[
                buf.into(),
                i64t.const_int(rec_size, false).into(),
                count.into(),
                fp.into()
            ],
            ""
        ));

        let free_fn = self.comp.ensure_free();
        b!(self.comp.bld.build_call(free_fn, &[buf.into()], ""));
        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));
        self.comp.store_unlock(fp)?;

        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }

    // ── StoreSave: flush the store file ──
    pub(super) fn emit_store_save(&mut self, store_name: &str) -> Result<BasicValueEnum<'ctx>, String> {
        let (_sd, _st, _rec_size, fp) = self.setup_store_access(store_name)?;
        let fflush_fn = self.comp.module.get_function("fflush").unwrap();
        b!(self.comp.bld.build_call(fflush_fn, &[fp.into()], ""));
        // Checkpoint WAL on save
        self.comp.wal_checkpoint(store_name)?;
        Ok(self.comp.ctx.i8_type().const_int(0, false).into())
    }
}
