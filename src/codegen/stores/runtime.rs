use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn declare_store_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        if self.module.get_function("fopen").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("fopen", ft, Some(Linkage::External));
        }
        if self.module.get_function("fclose").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fclose", ft, Some(Linkage::External));
        }
        if self.module.get_function("fread").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into(), i64t.into(), ptr.into()], false);
            self.module
                .add_function("fread", ft, Some(Linkage::External));
        }
        if self.module.get_function("fwrite").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into(), i64t.into(), ptr.into()], false);
            self.module
                .add_function("fwrite", ft, Some(Linkage::External));
        }
        if self.module.get_function("fseek").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), i64t.into(), i32t.into()], false);
            self.module
                .add_function("fseek", ft, Some(Linkage::External));
        }
        if self.module.get_function("ftell").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("ftell", ft, Some(Linkage::External));
        }
        if self.module.get_function("fflush").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fflush", ft, Some(Linkage::External));
        }
        if self.module.get_function("remove").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("remove", ft, Some(Linkage::External));
        }
        if self.module.get_function("memset").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External));
        }
        self.ensure_memcpy();
        self.ensure_malloc();
        self.ensure_free();
        self.ensure_memcmp();
        if self.module.get_function("strlen").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("strlen", ft, Some(Linkage::External));
        }
        if self.module.get_function("fileno").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fileno", ft, Some(Linkage::External));
        }
        if self.module.get_function("flock").is_none() {
            let ft = i32t.fn_type(&[i32t.into(), i32t.into()], false);
            self.module
                .add_function("flock", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_wal_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_wal_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_wal_write").is_none() {
            let void_ty = self.ctx.void_type();
            let u8t = self.ctx.i8_type();
            let ft = void_ty.fn_type(&[ptr.into(), u8t.into(), ptr.into(), i32t.into()], false);
            self.module
                .add_function("jinn_wal_write", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_wal_checkpoint").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_wal_checkpoint", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_wal_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_wal_close", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_idx_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_idx_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_idx_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_insert").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_idx_insert", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_lookup").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_idx_lookup", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_contains").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_idx_contains", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_delete").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_idx_delete", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_hash_i64").is_none() {
            let ft = i64t.fn_type(&[i64t.into()], false);
            self.module
                .add_function("jinn_idx_hash_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_hash_str").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_idx_hash_str", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_idx_hash_f64").is_none() {
            let ft = i64t.fn_type(&[self.ctx.f64_type().into()], false);
            self.module
                .add_function("jinn_idx_hash_f64", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_ver_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_ver_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_ver_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_ver_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_ver_append").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(
                &[
                    ptr.into(),
                    i64t.into(),
                    i64t.into(),
                    ptr.into(),
                    i64t.into(),
                ],
                false,
            );
            self.module
                .add_function("jinn_ver_append", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_ver_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_ver_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_ver_at").is_none() {
            let ft = i64t.fn_type(
                &[
                    ptr.into(),
                    i64t.into(),
                    i64t.into(),
                    ptr.into(),
                    i64t.into(),
                ],
                false,
            );
            self.module
                .add_function("jinn_ver_at", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_ver_history").is_none() {
            let ft = i64t.fn_type(
                &[
                    ptr.into(),
                    i64t.into(),
                    ptr.into(),
                    i64t.into(),
                    i64t.into(),
                ],
                false,
            );
            self.module
                .add_function("jinn_ver_history", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_mig_log_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_mig_log_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_mig_log_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_mig_log_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_mig_log_applied").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_mig_log_applied", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_mig_log_record").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_mig_log_record", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_mig_add_field").is_none() {
            let ft = i64t.fn_type(
                &[ptr.into(), ptr.into(), i64t.into(), i64t.into(), ptr.into()],
                false,
            );
            self.module
                .add_function("jinn_mig_add_field", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_mig_drop_field").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_mig_drop_field", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_kv_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_kv_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_kv_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_set").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_kv_set", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_get").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_kv_get", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_has").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_kv_has", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_del").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_kv_del", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_incr").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jinn_kv_incr", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_kv_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_kv_persist").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_kv_persist", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_vec_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_vec_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_vec_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_vec_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_vec_insert").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("jinn_vec_insert", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_vec_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_vec_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_vec_nearest").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into(), ptr.into()], false);
            self.module
                .add_function("jinn_vec_nearest", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_col_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_col_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_col_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_append").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("jinn_col_append", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_col_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_sum_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_col_sum_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_min_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_col_min_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_col_max_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_col_max_i64", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_bloom_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_bloom_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_bloom_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_bloom_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_bloom_add_i64").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_bloom_add_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_bloom_test_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_bloom_test_i64", ft, Some(Linkage::External));
        }

        if self.module.get_function("jinn_fts_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_fts_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_fts_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_fts_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_fts_add").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_fts_add", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_fts_search_n").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jinn_fts_search_n", ft, Some(Linkage::External));
        }
        if self.module.get_function("jinn_fts_posting_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jinn_fts_posting_count", ft, Some(Linkage::External));
        }
    }

    pub(crate) fn declare_store(&mut self, sd: &hir::StoreDef) -> Result<(), String> {
        let struct_name = format!("__store_{}", sd.name);
        let rec_name = format!("__store_{}_rec", sd.name);

        let rec_field_tys: Vec<BasicTypeEnum<'ctx>> = sd
            .fields
            .iter()
            .map(|f| self.store_field_llvm_ty(&f.ty))
            .collect();
        let rec_st = self.ctx.opaque_struct_type(&rec_name);
        rec_st.set_body(&rec_field_tys, false);

        let jinn_field_tys: Vec<BasicTypeEnum<'ctx>> =
            sd.fields.iter().map(|f| self.llvm_ty(&f.ty)).collect();
        let jinn_st = self.ctx.opaque_struct_type(&struct_name);
        jinn_st.set_body(&jinn_field_tys, false);

        let fields: Vec<(String, Type)> = sd
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty.clone()))
            .collect();
        self.structs.insert(Symbol::intern(&struct_name), fields);

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global = self
            .module
            .add_global(ptr_ty, None, &format!("__store_{}_fp", sd.name));
        global.set_linkage(Linkage::Internal);
        global.set_initializer(&ptr_ty.const_null());

        let wal_global = self
            .module
            .add_global(ptr_ty, None, &format!("__store_{}_wal", sd.name));
        wal_global.set_linkage(Linkage::Internal);
        wal_global.set_initializer(&ptr_ty.const_null());

        for field in &sd.fields {
            let has_index = field.decorators.iter().any(|d| {
                matches!(
                    d,
                    crate::ast::FieldDecorator::Index | crate::ast::FieldDecorator::Unique
                )
            });
            if has_index {
                let idx_global = self.module.add_global(
                    ptr_ty,
                    None,
                    &format!("__store_{}_idx_{}", sd.name, field.name),
                );
                idx_global.set_linkage(Linkage::Internal);
                idx_global.set_initializer(&ptr_ty.const_null());
            }
        }

        let is_versioned = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Versioned);
        if is_versioned {
            let ver_global =
                self.module
                    .add_global(ptr_ty, None, &format!("__store_{}_ver", sd.name));
            ver_global.set_linkage(Linkage::Internal);
            ver_global.set_initializer(&ptr_ty.const_null());
        }

        let is_kv = sd
            .decorators
            .iter()
            .any(|d| *d == crate::ast::StoreDecorator::Kv);
        if is_kv {
            let kv_global =
                self.module
                    .add_global(ptr_ty, None, &format!("__store_{}_kv", sd.name));
            kv_global.set_linkage(Linkage::Internal);
            kv_global.set_initializer(&ptr_ty.const_null());
        }

        let is_vector = sd
            .decorators
            .iter()
            .any(|d| matches!(d, crate::ast::StoreDecorator::Vector(_)));
        if is_vector {
            let vec_global =
                self.module
                    .add_global(ptr_ty, None, &format!("__store_{}_vec", sd.name));
            vec_global.set_linkage(Linkage::Internal);
            vec_global.set_initializer(&ptr_ty.const_null());
        }

        Ok(())
    }

    pub(crate) fn load_kv_handle(
        &mut self,
        store_name: &str,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_kv");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no kv global for '{store_name}' — is it declared @kv?"))?;
        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "kv.cur"))
        .into_pointer_value();

        let is_null = b!(self.bld.build_is_null(current, "kv.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "kv.open");
        let cont_bb = self.ctx.append_basic_block(fv, "kv.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let kv_path = format!("{store_name}.kv\0");
        let kv_str = b!(self.bld.build_global_string_ptr(&kv_path, "kv.path"));
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_kv_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[kv_str.as_pointer_value().into()],
                "kv.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "kv.ptr"))
        .into_pointer_value();
        Ok(result)
    }

    pub(crate) fn load_vec_handle(
        &mut self,
        store_name: &str,
        dims: u64,
    ) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let global_name = format!("__store_{store_name}_vec");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no vec global for '{store_name}' — is it declared @vector?"))?;
        let current = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "vec.cur"))
        .into_pointer_value();

        let is_null = b!(self.bld.build_is_null(current, "vec.null"));
        let fv = self.current_fn();
        let open_bb = self.ctx.append_basic_block(fv, "vec.open");
        let cont_bb = self.ctx.append_basic_block(fv, "vec.cont");
        b!(self.bld.build_conditional_branch(is_null, open_bb, cont_bb));

        self.bld.position_at_end(open_bb);
        let vec_path = format!("{store_name}.vec\0");
        let vec_str = b!(self.bld.build_global_string_ptr(&vec_path, "vec.path"));
        let dims_val = i64t.const_int(dims, false);
        let open_fn = crate::codegen::fn_or_die(&self.module, "jinn_vec_open");
        let opened = self
            .call_result(b!(self.bld.build_call(
                open_fn,
                &[vec_str.as_pointer_value().into(), dims_val.into()],
                "vec.new"
            )))
            .into_pointer_value();
        b!(self.bld.build_store(global.as_pointer_value(), opened));
        b!(self.bld.build_unconditional_branch(cont_bb));

        self.bld.position_at_end(cont_bb);
        let result = b!(self
            .bld
            .build_load(ptr_ty, global.as_pointer_value(), "vec.ptr"))
        .into_pointer_value();
        Ok(result)
    }

    pub(in crate::codegen) fn store_field_llvm_ty(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::String => self.ctx.i8_type().array_type(STRING_BUF_SIZE as u32).into(),
            other => self.llvm_ty(other),
        }
    }
}
