//! Store index hashing, version helpers, migrations, and field sizing.

use super::*;

impl<'ctx> Compiler<'ctx> {
    /// Compute the hash of a field value for index operations.
    /// Returns i64 hash value.
    pub(crate) fn idx_hash_field(
        &mut self,
        field_val: inkwell::values::BasicValueEnum<'ctx>,
        field_ty: &Type,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        // Normalize Struct("I64", []) → I64, etc.
        let resolved = match field_ty {
            Type::Struct(name, params) if params.is_empty() => match &*name.as_str() {
                "I8" => Type::I8,
                "I16" => Type::I16,
                "I32" => Type::I32,
                "I64" => Type::I64,
                "U8" => Type::U8,
                "U16" => Type::U16,
                "U32" => Type::U32,
                "U64" => Type::U64,
                "F32" => Type::F32,
                "F64" => Type::F64,
                "Bool" => Type::Bool,
                "String" => Type::String,
                _ => field_ty.clone(),
            },
            _ => field_ty.clone(),
        };
        Ok(match &resolved {
            Type::I64
            | Type::I32
            | Type::I16
            | Type::I8
            | Type::U64
            | Type::U32
            | Type::U16
            | Type::U8
            | Type::Bool => {
                let i64t = self.ctx.i64_type();
                let val = if field_val.is_int_value() {
                    let iv = field_val.into_int_value();
                    if iv.get_type().get_bit_width() < 64 {
                        b!(self.bld.build_int_z_extend(iv, i64t, "idx.ext"))
                    } else {
                        iv
                    }
                } else {
                    i64t.const_int(0, false)
                };
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_i64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "idx.hash")))
                    .into_int_value()
            }
            Type::F64 | Type::F32 => {
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_f64");
                let fval = if field_val.is_float_value() {
                    let fv = field_val.into_float_value();
                    if fv.get_type() == self.ctx.f32_type() {
                        b!(self
                            .bld
                            .build_float_ext(fv, self.ctx.f64_type(), "idx.fext"))
                    } else {
                        fv
                    }
                } else {
                    self.ctx.f64_type().const_float(0.0)
                };
                self.call_result(b!(self.bld.build_call(hash_fn, &[fval.into()], "idx.hash")))
                    .into_int_value()
            }
            Type::String => {
                // Use SSO-aware helpers to get data pointer and length
                let str_data = self.string_data(field_val)?;
                let str_len = self.string_len(field_val)?;
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_str");
                self.call_result(b!(self.bld.build_call(
                    hash_fn,
                    &[str_data.into(), str_len.into()],
                    "idx.hash"
                )))
                .into_int_value()
            }
            _ => {
                // Fallback: hash as i64(0)
                self.ctx.i64_type().const_int(0, false)
            }
        })
    }

    /// Hash a field value loaded from a store record on disk.
    /// For strings: store records use fixed 256-byte buffers [8B len][248B data].
    /// For numerics: load the value and hash it.
    pub(crate) fn hash_store_field_from_gep(
        &mut self,
        field_gep: inkwell::values::PointerValue<'ctx>,
        field_ty: &Type,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        // Normalize Struct("I64", []) → I64, etc.
        let resolved = match field_ty {
            Type::Struct(name, params) if params.is_empty() => match &*name.as_str() {
                "I8" => Type::I8,
                "I16" => Type::I16,
                "I32" => Type::I32,
                "I64" => Type::I64,
                "U8" => Type::U8,
                "U16" => Type::U16,
                "U32" => Type::U32,
                "U64" => Type::U64,
                "F32" => Type::F32,
                "F64" => Type::F64,
                "Bool" => Type::Bool,
                "String" => Type::String,
                _ => field_ty.clone(),
            },
            _ => field_ty.clone(),
        };
        let i64t = self.ctx.i64_type();
        Ok(match &resolved {
            Type::I64
            | Type::I32
            | Type::I16
            | Type::I8
            | Type::U64
            | Type::U32
            | Type::U16
            | Type::U8
            | Type::Bool => {
                let val = b!(self.bld.build_load(i64t, field_gep, "dist.ival")).into_int_value();
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_i64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "dist.hash")))
                    .into_int_value()
            }
            Type::F64 | Type::F32 => {
                let f64t = self.ctx.f64_type();
                let val = b!(self.bld.build_load(f64t, field_gep, "dist.fval")).into_float_value();
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_f64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "dist.hash")))
                    .into_int_value()
            }
            Type::String => {
                // Store records use fixed 256B: [8B len][248B data]
                let len = b!(self.bld.build_load(i64t, field_gep, "dist.slen")).into_int_value();
                let i8t = self.ctx.i8_type();
                let data_ptr = unsafe {
                    b!(self.bld.build_gep(
                        i8t,
                        field_gep,
                        &[i64t.const_int(8, false)],
                        "dist.sdata"
                    ))
                };
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_hash_str");
                self.call_result(b!(self.bld.build_call(
                    hash_fn,
                    &[data_ptr.into(), len.into()],
                    "dist.hash"
                )))
                .into_int_value()
            }
            _ => i64t.const_int(0, false),
        })
    }

    /// Check if a store has the @versioned decorator
    pub(crate) fn store_is_versioned(sd: &hir::StoreDef) -> bool {
        sd.decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Versioned)
    }

    /// Lazily open the version file for a @versioned store.
    /// Global: __store_{name}_ver
    pub(crate) fn load_store_ver(
        &mut self,
        store_name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_ver");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no version global for '{store_name}'"))?;
        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "ver.cur"))
        .into_pointer_value();

        let is_null = b!(self.bld.build_is_null(current, "ver.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "ver.open");
        let cont_bb = self.ctx.append_basic_block(fv, "ver.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let ver_path = format!("{store_name}.versions\0");
        let ver_str = b!(self.bld.build_global_string_ptr(&ver_path, "ver.path"));
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_ver_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[ver_str.as_pointer_value().into()],
                "ver.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "ver.fp2"))
        .into_pointer_value();
        Ok(result)
    }

    /// Generate a migration function that checks & applies a migration at startup.
    /// Returns the function value so it can be called from main.
    pub(crate) fn gen_migration(
        &mut self,
        mig: &crate::ast::MigrationDef,
    ) -> Result<inkwell::values::FunctionValue<'ctx>, String> {
        let fn_name = format!("__migrate_{}", mig.name);
        let void_ty = self.ctx.void_type();
        let ft = void_ty.fn_type(&[], false);
        let fv = self
            .module
            .add_function(&fn_name, ft, Some(Linkage::Internal));
        self.tag_fn(fv);

        let old_fn = self.cur_fn;
        self.cur_fn = Some(fv);

        let entry = self.ctx.append_basic_block(fv, "entry");
        let apply_bb = self.ctx.append_basic_block(fv, "apply");
        let done_bb = self.ctx.append_basic_block(fv, "done");

        self.bld.position_at_end(entry);

        let i64t = self.ctx.i64_type();
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());

        // Determine the log file path from the first alter op's store name
        // (or use a generic name based on migration name)
        let log_path = if let Some(op) = mig.up.first() {
            format!("{}.migrations.log\0", op.store_name)
        } else {
            format!("{}.migrations.log\0", mig.name)
        };
        let log_str = b!(self.bld.build_global_string_ptr(&log_path, "mig.path"));

        // Open migration log
        let log_open = crate::codegen::fn_or_die(&self.module, "jade_mig_log_open");
        let log_fp = self
            .call_result(b!(self.bld.build_call(
                log_open,
                &[log_str.as_pointer_value().into()],
                "mig.log"
            )))
            .into_pointer_value();

        // Check if already applied
        let log_applied = crate::codegen::fn_or_die(&self.module, "jade_mig_log_applied");
        let applied = self
            .call_result(b!(self.bld.build_call(
                log_applied,
                &[
                    log_fp.into(),
                    i64t.const_int(mig.version as u64, false).into()
                ],
                "mig.applied"
            )))
            .into_int_value();

        let is_applied = b!(self.bld.build_int_compare(
            inkwell::IntPredicate::NE,
            applied,
            i64t.const_int(0, false),
            "mig.done"
        ));
        b!(self
            .bld
            .build_conditional_branch(is_applied, done_bb, apply_bb));

        // Apply migration
        self.bld.position_at_end(apply_bb);

        // For each alter op, check if the store file exists. If it doesn't,
        // this is a fresh install — the store will be created with the new
        // schema, so we skip the migration body and just record it as applied.
        let record_bb = self.ctx.append_basic_block(fv, "record");

        for op in &mig.up {
            let store_name = &op.store_name;
            let fp_global_name = format!("__store_{store_name}_fp");
            let store_path_lit = format!("{store_name}.store\0");
            let store_path_str = b!(self
                .bld
                .build_global_string_ptr(&store_path_lit, "mig.spath"));

            // Check if store file exists: fopen(path, "rb")
            let fopen_fn = crate::codegen::fn_or_die(&self.module, "fopen");
            let rb_str = b!(self.bld.build_global_string_ptr("rb\0", "mig.rb"));
            let test_fp = self
                .call_result(b!(self.bld.build_call(
                    fopen_fn,
                    &[
                        store_path_str.as_pointer_value().into(),
                        rb_str.as_pointer_value().into()
                    ],
                    "mig.test"
                )))
                .into_pointer_value();
            let is_null = b!(self.bld.build_is_null(test_fp, "mig.nofile"));

            let exists_bb = self.ctx.append_basic_block(fv, "mig.exists");
            let next_bb = self.ctx.append_basic_block(fv, "mig.next");
            b!(self
                .bld
                .build_conditional_branch(is_null, record_bb, exists_bb));

            // Store exists — close the test handle, open via ensure, then apply
            self.bld.position_at_end(exists_bb);
            let fclose_fn = crate::codegen::fn_or_die(&self.module, "fclose");
            b!(self.bld.build_call(fclose_fn, &[test_fp.into()], ""));

            // Ensure the store is open (sets the global FILE*)
            let ensure_fn_name = format!("__store_ensure_{store_name}");
            if let Some(ensure_fn) = self.module.get_function(&ensure_fn_name) {
                b!(self.bld.build_call(ensure_fn, &[], ""));
            }

            for action in &op.actions {
                match action {
                    crate::ast::AlterAction::Add {
                        name: _,
                        ty,
                        default: _,
                    } => {
                        let field_size = self.field_byte_size(ty);

                        let fp_global = self.module.get_global(&fp_global_name);
                        if let Some(fp_g) = fp_global {
                            let fp =
                                b!(self
                                    .bld
                                    .build_load(ptr_ty, fp_g.as_pointer_value(), "mig.fp"))
                                .into_pointer_value();
                            // Read current rec_size from header offset 16
                            let fseek = crate::codegen::fn_or_die(&self.module, "fseek");
                            b!(self.bld.build_call(
                                fseek,
                                &[
                                    fp.into(),
                                    i64t.const_int(16, false).into(),
                                    self.ctx.i32_type().const_int(0, false).into()
                                ],
                                ""
                            ));
                            let rec_size_buf = self.entry_alloca(i64t.into(), "mig.rsz");
                            let fread = crate::codegen::fn_or_die(&self.module, "fread");
                            b!(self.bld.build_call(
                                fread,
                                &[
                                    rec_size_buf.into(),
                                    i64t.const_int(8, false).into(),
                                    i64t.const_int(1, false).into(),
                                    fp.into()
                                ],
                                ""
                            ));
                            let field_offset =
                                b!(self.bld.build_load(i64t, rec_size_buf, "mig.off"))
                                    .into_int_value();

                            let add_fn =
                                crate::codegen::fn_or_die(&self.module, "jade_mig_add_field");
                            b!(self.bld.build_call(
                                add_fn,
                                &[
                                    fp_g.as_pointer_value().into(),
                                    store_path_str.as_pointer_value().into(),
                                    field_offset.into(),
                                    i64t.const_int(field_size, false).into(),
                                    ptr_ty.const_null().into(),
                                ],
                                ""
                            ));
                        }
                    }
                    crate::ast::AlterAction::Drop { name: field_name } => {
                        if let Some(sd) = self.store_defs.get(store_name) {
                            let sd = sd.clone();
                            let mut offset: u64 = 0;
                            let mut field_size: u64 = 0;
                            for f in &sd.fields {
                                let sz = self.field_byte_size(&f.ty);
                                if f.name == *field_name {
                                    field_size = sz;
                                    break;
                                }
                                offset += sz;
                            }
                            if field_size > 0 {
                                let fp_global = self.module.get_global(&fp_global_name);
                                if let Some(fp_g) = fp_global {
                                    let drop_fn = crate::codegen::fn_or_die(
                                        &self.module,
                                        "jade_mig_drop_field",
                                    );
                                    b!(self.bld.build_call(
                                        drop_fn,
                                        &[
                                            fp_g.as_pointer_value().into(),
                                            store_path_str.as_pointer_value().into(),
                                            i64t.const_int(offset, false).into(),
                                            i64t.const_int(field_size, false).into(),
                                        ],
                                        ""
                                    ));
                                }
                            }
                        }
                    }
                    crate::ast::AlterAction::Rename { .. } => {
                        // Rename is a compile-time operation — no runtime action needed
                    }
                }
            }

            b!(self.bld.build_unconditional_branch(next_bb));
            self.bld.position_at_end(next_bb);
        }

        // Fall through to record
        b!(self.bld.build_unconditional_branch(record_bb));

        // Record the migration as applied
        self.bld.position_at_end(record_bb);
        let log_record = crate::codegen::fn_or_die(&self.module, "jade_mig_log_record");
        b!(self.bld.build_call(
            log_record,
            &[
                log_fp.into(),
                i64t.const_int(mig.version as u64, false).into(),
                i64t.const_int(1, false).into(), // direction = up
            ],
            ""
        ));

        let log_close = crate::codegen::fn_or_die(&self.module, "jade_mig_log_close");
        b!(self.bld.build_call(log_close, &[log_fp.into()], ""));
        b!(self.bld.build_return(None));

        // Done (skip path — migration was already applied)
        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(log_close, &[log_fp.into()], ""));
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(fv)
    }

    /// Get the byte size of a store field type.
    pub(in crate::codegen) fn field_byte_size(&self, ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::String => 256, // fixed-size store string buffer
            Type::Struct(name, _) => match &*name.as_str() {
                "I8" | "U8" | "Bool" => 1,
                "I16" | "U16" => 2,
                "I32" | "U32" | "F32" => 4,
                "I64" | "U64" | "F64" => 8,
                "String" => 256,
                _ => 8,
            },
            _ => 8,
        }
    }
}
