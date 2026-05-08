//! Store schema codegen: declarations, runtime initialization, handle accessors,
//! and WAL helpers. Consumed by `mir_codegen/store.rs` and `mir_codegen/store_ext.rs`.

use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{FunctionValue, PointerValue};

use crate::hir;
use crate::intern::Symbol;
use crate::types::Type;

use super::Compiler;
use super::b;

pub(crate) const STRING_BUF_SIZE: u64 = 256;

pub(crate) const HEADER_SIZE: u64 = 24;

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

        // WAL runtime functions
        if self.module.get_function("jade_wal_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_wal_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_wal_write").is_none() {
            let void_ty = self.ctx.void_type();
            let u8t = self.ctx.i8_type();
            let ft = void_ty.fn_type(&[ptr.into(), u8t.into(), ptr.into(), i32t.into()], false);
            self.module
                .add_function("jade_wal_write", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_wal_checkpoint").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_wal_checkpoint", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_wal_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_wal_close", ft, Some(Linkage::External));
        }

        // Index runtime functions
        if self.module.get_function("jade_idx_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_idx_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_idx_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_insert").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_idx_insert", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_lookup").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_idx_lookup", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_contains").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_idx_contains", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_delete").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_idx_delete", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_hash_i64").is_none() {
            let ft = i64t.fn_type(&[i64t.into()], false);
            self.module
                .add_function("jade_idx_hash_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_hash_str").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_idx_hash_str", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_idx_hash_f64").is_none() {
            let ft = i64t.fn_type(&[self.ctx.f64_type().into()], false);
            self.module
                .add_function("jade_idx_hash_f64", ft, Some(Linkage::External));
        }

        // Version runtime functions
        if self.module.get_function("jade_ver_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_ver_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_ver_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_ver_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_ver_append").is_none() {
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
                .add_function("jade_ver_append", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_ver_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_ver_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_ver_at").is_none() {
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
                .add_function("jade_ver_at", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_ver_history").is_none() {
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
                .add_function("jade_ver_history", ft, Some(Linkage::External));
        }

        // Migration runtime functions
        if self.module.get_function("jade_mig_log_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_mig_log_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_mig_log_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_mig_log_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_mig_log_applied").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_mig_log_applied", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_mig_log_record").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_mig_log_record", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_mig_add_field").is_none() {
            let ft = i64t.fn_type(
                &[ptr.into(), ptr.into(), i64t.into(), i64t.into(), ptr.into()],
                false,
            );
            self.module
                .add_function("jade_mig_add_field", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_mig_drop_field").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_mig_drop_field", ft, Some(Linkage::External));
        }

        // KV store runtime functions
        if self.module.get_function("jade_kv_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_kv_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_kv_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_set").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_kv_set", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_get").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_kv_get", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_has").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_kv_has", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_del").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_kv_del", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_incr").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into(), i64t.into(), i64t.into()], false);
            self.module
                .add_function("jade_kv_incr", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_kv_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_kv_persist").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_kv_persist", ft, Some(Linkage::External));
        }

        // Vector store runtime functions
        if self.module.get_function("jade_vec_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_vec_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_vec_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_vec_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_vec_insert").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("jade_vec_insert", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_vec_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_vec_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_vec_nearest").is_none() {
            // (JadeVec*, query_ptr, k, out_indices) -> count
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into(), ptr.into()], false);
            self.module
                .add_function("jade_vec_nearest", ft, Some(Linkage::External));
        }

        // Column store runtime functions
        if self.module.get_function("jade_col_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_col_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_col_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_append").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("jade_col_append", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_col_count", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_sum_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_col_sum_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_min_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_col_min_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_col_max_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_col_max_i64", ft, Some(Linkage::External));
        }

        // Bloom filter runtime functions
        if self.module.get_function("jade_bloom_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_bloom_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_bloom_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_bloom_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_bloom_add_i64").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_bloom_add_i64", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_bloom_test_i64").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_bloom_test_i64", ft, Some(Linkage::External));
        }

        // Full-text search runtime functions
        if self.module.get_function("jade_fts_open").is_none() {
            let ft = ptr.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_fts_open", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_fts_close").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_fts_close", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_fts_add").is_none() {
            let void_ty = self.ctx.void_type();
            let ft = void_ty.fn_type(&[ptr.into(), i64t.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_fts_add", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_fts_search_n").is_none() {
            let ft = i64t.fn_type(&[ptr.into(), ptr.into(), i64t.into()], false);
            self.module
                .add_function("jade_fts_search_n", ft, Some(Linkage::External));
        }
        if self.module.get_function("jade_fts_posting_count").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("jade_fts_posting_count", ft, Some(Linkage::External));
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

        let jade_field_tys: Vec<BasicTypeEnum<'ctx>> =
            sd.fields.iter().map(|f| self.llvm_ty(&f.ty)).collect();
        let jade_st = self.ctx.opaque_struct_type(&struct_name);
        jade_st.set_body(&jade_field_tys, false);

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

        // WAL file pointer global
        let wal_global = self
            .module
            .add_global(ptr_ty, None, &format!("__store_{}_wal", sd.name));
        wal_global.set_linkage(Linkage::Internal);
        wal_global.set_initializer(&ptr_ty.const_null());

        // Per-field index globals for @index and @unique fields
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

        // Version file pointer global for @versioned stores
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

        // KV handle global for @kv stores
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

        // Vector handle global for @vector stores
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

    /// Lazily open a @kv store handle, returning the JadeKV pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_kv_open");
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

    /// Lazily open a @vector store handle, returning the JadeVec pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_vec_open");
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

impl<'ctx> Compiler<'ctx> {
    /// Lazily open a column file for a specific field. Returns JadeCol* pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_col_open");
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

    /// Lazily open a bloom filter for a specific field. Returns JadeBloom* pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_bloom_open");
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

    /// Lazily open an FTS index for a specific field. Returns JadeFts* pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_fts_open");
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
        let wal_open_fn = crate::codegen::fn_or_die(&self.module, "jade_wal_open");
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
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jade_wal_write");
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
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jade_wal_write");
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
        let wal_write_fn = crate::codegen::fn_or_die(&self.module, "jade_wal_write");
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
        let wal_cp_fn = crate::codegen::fn_or_die(&self.module, "jade_wal_checkpoint");
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

        // Build a Jade String from the buffer
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

    /// Lazily open an index file, returning the JadeIndex pointer.
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
        let open_fn = crate::codegen::fn_or_die(&self.module, "jade_idx_open");
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
