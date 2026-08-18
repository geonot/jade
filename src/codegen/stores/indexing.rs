use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn idx_hash_field(
        &mut self,
        field_val: inkwell::values::BasicValueEnum<'ctx>,
        field_ty: &Type,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
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
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_i64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "idx.hash")))
                    .into_int_value()
            }
            Type::F64 | Type::F32 => {
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_f64");
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
                let str_data = self.string_data(field_val)?;
                let str_len = self.string_len(field_val)?;
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_str");
                self.call_result(b!(self.bld.build_call(
                    hash_fn,
                    &[str_data.into(), str_len.into()],
                    "idx.hash"
                )))
                .into_int_value()
            }
            _ => self.ctx.i64_type().const_int(0, false),
        })
    }

    pub(crate) fn hash_store_field_from_gep(
        &mut self,
        field_gep: inkwell::values::PointerValue<'ctx>,
        field_ty: &Type,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
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
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_i64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "dist.hash")))
                    .into_int_value()
            }
            Type::F64 | Type::F32 => {
                let f64t = self.ctx.f64_type();
                let val = b!(self.bld.build_load(f64t, field_gep, "dist.fval")).into_float_value();
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_f64");
                self.call_result(b!(self.bld.build_call(hash_fn, &[val.into()], "dist.hash")))
                    .into_int_value()
            }
            Type::String => {
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
                let hash_fn = crate::codegen::fn_or_die(&self.module, "jinn_idx_hash_str");
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

    pub(crate) fn store_is_versioned(sd: &hir::StoreDef) -> bool {
        sd.decorators
            .contains(&crate::ast::StoreDecorator::Versioned)
    }

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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_ver_open");
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

        let log_path = if let Some(op) = mig.up.first() {
            format!("{}.migrations.log\0", op.store_name)
        } else {
            format!("{}.migrations.log\0", mig.name)
        };
        let log_str = b!(self.bld.build_global_string_ptr(&log_path, "mig.path"));

        let log_open = crate::codegen::fn_or_die(&self.module, "jinn_mig_log_open");
        let log_fp = self
            .call_result(b!(self.bld.build_call(
                log_open,
                &[log_str.as_pointer_value().into()],
                "mig.log"
            )))
            .into_pointer_value();

        let log_applied = crate::codegen::fn_or_die(&self.module, "jinn_mig_log_applied");
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

        self.bld.position_at_end(apply_bb);
        let mig_enter = crate::codegen::fn_or_die(&self.module, "jinn_migration_enter");
        b!(self.bld.build_call(mig_enter, &[], ""));

        let record_bb = self.ctx.append_basic_block(fv, "record");

        for op in &mig.up {
            let store_name = &op.store_name;
            let fp_global_name = format!("__store_{store_name}_fp");
            let store_path_lit = format!("{store_name}.store\0");
            let store_path_str = b!(self
                .bld
                .build_global_string_ptr(&store_path_lit, "mig.spath"));

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

            self.bld.position_at_end(exists_bb);
            let fclose_fn = crate::codegen::fn_or_die(&self.module, "fclose");
            b!(self.bld.build_call(fclose_fn, &[test_fp.into()], ""));

            if let Some(sd) = self.store_defs.get(store_name).cloned() {
                let ensure_fn = self.gen_store_ensure_open(&sd)?;
                b!(self.bld.build_call(ensure_fn, &[], ""));
            }

            for action in &op.actions {
                match action {
                    crate::ast::AlterAction::Add {
                        name: field_name,
                        ty,
                        default,
                    } => {
                        let field_size = self.field_byte_size(ty);
                        let default_ptr: inkwell::values::BasicMetadataValueEnum = match default {
                            None => ptr_ty.const_null().into(),
                            Some(expr) => {
                                let bytes = Self::migration_default_bytes(expr, ty, field_size)
                                    .map_err(|e| {
                                        format!(
                                            "migration `{}`: `add {}` on store '{}': {}",
                                            mig.name, field_name, store_name, e
                                        )
                                    })?;
                                let i8t = self.ctx.i8_type();
                                let vals: Vec<_> = bytes
                                    .iter()
                                    .map(|b| i8t.const_int(*b as u64, false))
                                    .collect();
                                let arr = i8t.const_array(&vals);
                                let g = self.module.add_global(
                                    i8t.array_type(field_size as u32),
                                    None,
                                    "mig.defv",
                                );
                                g.set_initializer(&arr);
                                g.set_linkage(Linkage::Private);
                                g.as_pointer_value().into()
                            }
                        };

                        let fp_global = self.module.get_global(&fp_global_name);
                        if let Some(fp_g) = fp_global {
                            let fp =
                                b!(self
                                    .bld
                                    .build_load(ptr_ty, fp_g.as_pointer_value(), "mig.fp"))
                                .into_pointer_value();

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
                                crate::codegen::fn_or_die(&self.module, "jinn_mig_add_field");
                            b!(self.bld.build_call(
                                add_fn,
                                &[
                                    fp_g.as_pointer_value().into(),
                                    store_path_str.as_pointer_value().into(),
                                    field_offset.into(),
                                    i64t.const_int(field_size, false).into(),
                                    default_ptr,
                                ],
                                ""
                            ));
                        }
                    }
                    crate::ast::AlterAction::Drop { name: field_name } => {
                        let down_ty = mig
                            .down
                            .iter()
                            .filter(|dop| dop.store_name == *store_name)
                            .flat_map(|dop| dop.actions.iter())
                            .find_map(|a| match a {
                                crate::ast::AlterAction::Add { name, ty, .. }
                                    if name == field_name =>
                                {
                                    Some(ty.clone())
                                }
                                _ => None,
                            });
                        let Some(dropped_ty) = down_ty else {
                            return Err(format!(
                                "migration `{}`: `drop {}` on store '{}' cannot determine \
                                 the field's position in the pre-migration record; alpha \
                                 removes the record's final field — declare the matching \
                                 `add {} as <type>` in the `down` block to supply its type",
                                mig.name, field_name, store_name, field_name
                            ));
                        };
                        let field_size = self.field_byte_size(&dropped_ty);
                        if let Some(fp_g) = self.module.get_global(&fp_global_name) {
                            let fp =
                                b!(self
                                    .bld
                                    .build_load(ptr_ty, fp_g.as_pointer_value(), "mig.dfp"))
                                .into_pointer_value();
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
                            let rec_size_buf = self.entry_alloca(i64t.into(), "mig.drsz");
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
                            let old_rec_size =
                                b!(self.bld.build_load(i64t, rec_size_buf, "mig.dsz"))
                                    .into_int_value();
                            let field_offset = b!(self.bld.build_int_sub(
                                old_rec_size,
                                i64t.const_int(field_size, false),
                                "mig.doff"
                            ));
                            let drop_fn =
                                crate::codegen::fn_or_die(&self.module, "jinn_mig_drop_field");
                            b!(self.bld.build_call(
                                drop_fn,
                                &[
                                    fp_g.as_pointer_value().into(),
                                    store_path_str.as_pointer_value().into(),
                                    field_offset.into(),
                                    i64t.const_int(field_size, false).into(),
                                ],
                                ""
                            ));
                        }
                    }
                    crate::ast::AlterAction::Rename { .. } => {}
                }
            }

            if let Some(sd) = self.store_defs.get(store_name) {
                let fingerprint = super::store_schema_fingerprint(sd);
                let version = self
                    .store_schema_versions
                    .get(store_name)
                    .copied()
                    .unwrap_or(0);
                if let Some(fp_g) = self.module.get_global(&fp_global_name) {
                    let stamp_fn =
                        crate::codegen::fn_or_die(&self.module, "jinn_store_stamp_schema");
                    b!(self.bld.build_call(
                        stamp_fn,
                        &[
                            fp_g.as_pointer_value().into(),
                            i64t.const_int(fingerprint as u64, false).into(),
                            i64t.const_int(version as u64, false).into(),
                        ],
                        ""
                    ));
                }
            }

            b!(self.bld.build_unconditional_branch(next_bb));
            self.bld.position_at_end(next_bb);
        }

        b!(self.bld.build_unconditional_branch(record_bb));

        self.bld.position_at_end(record_bb);
        let mig_leave = crate::codegen::fn_or_die(&self.module, "jinn_migration_leave");
        b!(self.bld.build_call(mig_leave, &[], ""));
        let log_record = crate::codegen::fn_or_die(&self.module, "jinn_mig_log_record");
        b!(self.bld.build_call(
            log_record,
            &[
                log_fp.into(),
                i64t.const_int(mig.version as u64, false).into(),
                i64t.const_int(1, false).into(),
            ],
            ""
        ));

        let log_close = crate::codegen::fn_or_die(&self.module, "jinn_mig_log_close");
        b!(self.bld.build_call(log_close, &[log_fp.into()], ""));
        b!(self.bld.build_return(None));

        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(log_close, &[log_fp.into()], ""));
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        Ok(fv)
    }

    pub(in crate::codegen) fn field_byte_size(&self, ty: &Type) -> u64 {
        match ty {
            Type::I8 | Type::U8 | Type::Bool => 1,
            Type::I16 | Type::U16 => 2,
            Type::I32 | Type::U32 | Type::F32 => 4,
            Type::I64 | Type::U64 | Type::F64 => 8,
            Type::String => super::STRING_BUF_SIZE,
            Type::Struct(name, _) => match &*name.as_str() {
                "I8" | "U8" | "Bool" => 1,
                "I16" | "U16" => 2,
                "I32" | "U32" | "F32" => 4,
                "I64" | "U64" | "F64" => 8,
                "String" => super::STRING_BUF_SIZE,
                _ => 8,
            },
            _ => 8,
        }
    }

    fn migration_default_bytes(
        expr: &crate::ast::Expr,
        ty: &Type,
        field_size: u64,
    ) -> Result<Vec<u8>, String> {
        enum Lit {
            Int(i64),
            Float(f64),
            Bool(bool),
            Str(String),
        }
        fn lit(e: &crate::ast::Expr) -> Option<Lit> {
            match e {
                crate::ast::Expr::Int(n, _) => Some(Lit::Int(*n)),
                crate::ast::Expr::Float(f, _) => Some(Lit::Float(*f)),
                crate::ast::Expr::Bool(b, _) => Some(Lit::Bool(*b)),
                crate::ast::Expr::Str(s, _) => Some(Lit::Str(s.clone())),
                crate::ast::Expr::UnaryOp(crate::ast::UnaryOp::Neg, inner, _) => {
                    match lit(inner)? {
                        Lit::Int(n) => Some(Lit::Int(n.wrapping_neg())),
                        Lit::Float(f) => Some(Lit::Float(-f)),
                        _ => None,
                    }
                }
                _ => None,
            }
        }
        let Some(v) = lit(expr) else {
            return Err("alpha supports only literal `default` values in migrations".into());
        };
        let kind: String = match ty {
            Type::I8 => "I8".into(),
            Type::U8 => "U8".into(),
            Type::I16 => "I16".into(),
            Type::U16 => "U16".into(),
            Type::I32 => "I32".into(),
            Type::U32 => "U32".into(),
            Type::I64 => "I64".into(),
            Type::U64 => "U64".into(),
            Type::F32 => "F32".into(),
            Type::F64 => "F64".into(),
            Type::Bool => "Bool".into(),
            Type::String => "String".into(),
            Type::Struct(name, _) => name.as_str(),
            other => {
                return Err(format!(
                    "unsupported field type `{other:?}` for a migration default"
                ));
            }
        };
        let int_range = |k: &str| -> Option<(i64, i64)> {
            Some(match k {
                "I8" => (i8::MIN as i64, i8::MAX as i64),
                "I16" => (i16::MIN as i64, i16::MAX as i64),
                "I32" => (i32::MIN as i64, i32::MAX as i64),
                "I64" => (i64::MIN, i64::MAX),
                "U8" => (0, u8::MAX as i64),
                "U16" => (0, u16::MAX as i64),
                "U32" => (0, u32::MAX as i64),
                "U64" => (0, i64::MAX),
                _ => return None,
            })
        };
        let mut out = vec![0u8; field_size as usize];
        match (kind.as_str(), v) {
            (k @ ("I8" | "U8" | "I16" | "U16" | "I32" | "U32" | "I64" | "U64"), Lit::Int(n)) => {
                if let Some((lo, hi)) = int_range(k)
                    && (n < lo || n > hi)
                {
                    return Err(format!("default {n} does not fit in `{k}`"));
                }
                let sz = (field_size as usize).min(8);
                out[..sz].copy_from_slice(&n.to_le_bytes()[..sz]);
            }
            ("Bool", Lit::Bool(b)) => out[0] = b as u8,
            ("Bool", Lit::Int(n)) => out[0] = (n != 0) as u8,
            ("F32", Lit::Float(f)) => out[..4].copy_from_slice(&(f as f32).to_le_bytes()),
            ("F32", Lit::Int(n)) => out[..4].copy_from_slice(&(n as f32).to_le_bytes()),
            ("F64", Lit::Float(f)) => out[..8].copy_from_slice(&f.to_le_bytes()),
            ("F64", Lit::Int(n)) => out[..8].copy_from_slice(&(n as f64).to_le_bytes()),
            ("String", Lit::Str(s)) => {
                let bytes = s.as_bytes();
                let cap = (field_size as usize).saturating_sub(8);
                let n = bytes.len().min(cap);
                out[..8].copy_from_slice(&(n as i64).to_le_bytes());
                out[8..8 + n].copy_from_slice(&bytes[..n]);
            }
            (k, _) => {
                return Err(format!(
                    "the `default` literal does not match field type `{k}`"
                ));
            }
        }
        Ok(out)
    }
}
