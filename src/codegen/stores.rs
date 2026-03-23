//! Store codegen: persistent binary file-backed record stores.
//!
//! Each store compiles to:
//!   - A struct type `__store_<name>` for the record layout
//!   - A global `FILE*` handle (opened lazily)
//!   - Runtime helper functions for insert, count, query, all, delete
//!
//! File format (v1):
//!   [8 bytes: magic "JADESTR\0"]
//!   [8 bytes: record count (i64)]
//!   [8 bytes: record size (i64)]
//!   [N * record_size bytes: records...]
//!
//! String fields use 256-byte fixed buffers: [8 bytes len][248 bytes data].

use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::ast::BinOp;
use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

/// Fixed buffer size for String fields in store records.
const STRING_BUF_SIZE: u64 = 256;

/// Header size: 8 (magic) + 8 (count) + 8 (record_size) = 24 bytes.
const HEADER_SIZE: u64 = 24;

impl<'ctx> Compiler<'ctx> {
    /// Declare stdio and helper externs needed by stores.
    pub(crate) fn declare_store_runtime(&mut self) {
        let ptr = self.ctx.ptr_type(AddressSpace::default());
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        // FILE *fopen(const char *path, const char *mode)
        if self.module.get_function("fopen").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), ptr.into()], false);
            self.module
                .add_function("fopen", ft, Some(Linkage::External));
        }
        // int fclose(FILE *)
        if self.module.get_function("fclose").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fclose", ft, Some(Linkage::External));
        }
        // size_t fread(void *ptr, size_t size, size_t count, FILE *stream)
        if self.module.get_function("fread").is_none() {
            let ft = i64t.fn_type(
                &[ptr.into(), i64t.into(), i64t.into(), ptr.into()],
                false,
            );
            self.module
                .add_function("fread", ft, Some(Linkage::External));
        }
        // size_t fwrite(const void *ptr, size_t size, size_t count, FILE *stream)
        if self.module.get_function("fwrite").is_none() {
            let ft = i64t.fn_type(
                &[ptr.into(), i64t.into(), i64t.into(), ptr.into()],
                false,
            );
            self.module
                .add_function("fwrite", ft, Some(Linkage::External));
        }
        // int fseek(FILE *, long offset, int whence)
        if self.module.get_function("fseek").is_none() {
            let ft = i32t.fn_type(&[ptr.into(), i64t.into(), i32t.into()], false);
            self.module
                .add_function("fseek", ft, Some(Linkage::External));
        }
        // long ftell(FILE *)
        if self.module.get_function("ftell").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("ftell", ft, Some(Linkage::External));
        }
        // int fflush(FILE *)
        if self.module.get_function("fflush").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fflush", ft, Some(Linkage::External));
        }
        // int remove(const char *)
        if self.module.get_function("remove").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("remove", ft, Some(Linkage::External));
        }
        // void *memset(void *, int, size_t)
        if self.module.get_function("memset").is_none() {
            let ft = ptr.fn_type(&[ptr.into(), i32t.into(), i64t.into()], false);
            self.module
                .add_function("memset", ft, Some(Linkage::External));
        }
        // void *memcpy(void *, const void *, size_t)
        self.ensure_memcpy();
        // void *malloc(size_t)
        self.ensure_malloc();
        // void free(void *)
        self.ensure_free();
        // int memcmp(const void *, const void *, size_t)
        self.ensure_memcmp();
        // int strlen(const char *)
        if self.module.get_function("strlen").is_none() {
            let ft = i64t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("strlen", ft, Some(Linkage::External));
        }
        // int fileno(FILE *)
        if self.module.get_function("fileno").is_none() {
            let ft = i32t.fn_type(&[ptr.into()], false);
            self.module
                .add_function("fileno", ft, Some(Linkage::External));
        }
        // int flock(int fd, int operation)
        if self.module.get_function("flock").is_none() {
            let ft = i32t.fn_type(&[i32t.into(), i32t.into()], false);
            self.module
                .add_function("flock", ft, Some(Linkage::External));
        }
    }

    /// Declare the struct type and global FILE* for a store.
    pub(crate) fn declare_store(&mut self, sd: &hir::StoreDef) -> Result<(), String> {
        let struct_name = format!("__store_{}", sd.name);
        let rec_name = format!("__store_{}_rec", sd.name);

        // Build the on-disk record struct (String → [256 x i8] fixed buffer)
        let rec_field_tys: Vec<BasicTypeEnum<'ctx>> = sd
            .fields
            .iter()
            .map(|f| self.store_field_llvm_ty(&f.ty))
            .collect();
        let rec_st = self.ctx.opaque_struct_type(&rec_name);
        rec_st.set_body(&rec_field_tys, false);

        // Build the Jade-facing struct (String fields use proper Jade String type)
        let jade_field_tys: Vec<BasicTypeEnum<'ctx>> = sd
            .fields
            .iter()
            .map(|f| self.llvm_ty(&f.ty))
            .collect();
        let jade_st = self.ctx.opaque_struct_type(&struct_name);
        jade_st.set_body(&jade_field_tys, false);

        let fields: Vec<(String, Type)> = sd
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        self.structs.insert(struct_name.clone(), fields);

        // Global FILE* pointer for this store, initialized to null
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global = self.module.add_global(ptr_ty, None, &format!("__store_{}_fp", sd.name));
        global.set_linkage(Linkage::Internal);
        global.set_initializer(&ptr_ty.const_null());

        Ok(())
    }

    /// LLVM type for a store field: String fields become [256 x i8] fixed buffers.
    fn store_field_llvm_ty(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::String => self.ctx.i8_type().array_type(STRING_BUF_SIZE as u32).into(),
            other => self.llvm_ty(other),
        }
    }

    /// Get the record size for a store by summing field sizes.
    fn store_record_size(&self, sd: &hir::StoreDef) -> u64 {
        let rec_name = format!("__store_{}_rec", sd.name);
        let st = self.module.get_struct_type(&rec_name).unwrap();
        self.type_store_size(st.into())
    }

    /// Generate the ensure_open function that lazily opens the store file.
    /// Returns the function value for `__store_<name>_ensure_open`.
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

        // void __store_<name>_ensure_open()
        let ft = self.ctx.void_type().fn_type(&[], false);
        let fv = self.module.add_function(&fn_name, ft, Some(Linkage::Internal));
        self.tag_fn(fv);

        let entry = self.ctx.append_basic_block(fv, "entry");
        let open_bb = self.ctx.append_basic_block(fv, "do_open");
        let init_bb = self.ctx.append_basic_block(fv, "init_file");
        let done_bb = self.ctx.append_basic_block(fv, "done");

        let old_fn = self.cur_fn;
        let old_bb = self.bld.get_insert_block();
        self.cur_fn = Some(fv);

        // Entry: check if FILE* is null
        self.bld.position_at_end(entry);
        let global_name = format!("__store_{name}_fp");
        let global = self.module.get_global(&global_name).unwrap();
        let fp = b!(self.bld.build_load(ptr_ty, global.as_pointer_value(), "fp"));
        let is_null = b!(self.bld.build_is_null(fp.into_pointer_value(), "is_null"));
        b!(self.bld.build_conditional_branch(is_null, open_bb, done_bb));

        // do_open: fopen("<name>.store", "r+b"), if null → create new
        self.bld.position_at_end(open_bb);
        let filename = format!("{name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "store.path"));
        let mode_rw = b!(self.bld.build_global_string_ptr("r+b\0", "mode.rw"));
        let fopen_fn = self.module.get_function("fopen").unwrap();
        let fp_val = self.call_result(b!(self.bld.build_call(
            fopen_fn,
            &[file_str.as_pointer_value().into(), mode_rw.as_pointer_value().into()],
            "fp"
        )));
        let fp_null = b!(self.bld.build_is_null(fp_val.into_pointer_value(), "fp.null"));
        b!(self.bld.build_conditional_branch(fp_null, init_bb, done_bb));

        // Before jumping to done after successful r+b open, store the fp
        // We need a new block for that
        let store_existing_bb = self.ctx.append_basic_block(fv, "store_existing");
        // Rebuild the branch: open_bb should branch to init_bb if null, store_existing if not
        // Remove the last terminator and rebuild
        open_bb.get_terminator().unwrap().erase_from_basic_block();
        self.bld.position_at_end(open_bb);
        b!(self.bld.build_conditional_branch(fp_null, init_bb, store_existing_bb));

        self.bld.position_at_end(store_existing_bb);
        b!(self.bld.build_store(global.as_pointer_value(), fp_val));
        b!(self.bld.build_unconditional_branch(done_bb));

        // init_file: create file with header
        self.bld.position_at_end(init_bb);
        let mode_wb = b!(self.bld.build_global_string_ptr("w+b\0", "mode.wb"));
        let new_fp = self.call_result(b!(self.bld.build_call(
            fopen_fn,
            &[file_str.as_pointer_value().into(), mode_wb.as_pointer_value().into()],
            "new_fp"
        )));
        b!(self.bld.build_store(global.as_pointer_value(), new_fp));

        // Write header: magic (8 bytes) + count (8 bytes) + record_size (8 bytes)
        let fwrite_fn = self.module.get_function("fwrite").unwrap();

        // Write magic "JADESTR\0"
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

        // Write count = 0
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

        // Write record_size
        let rec_size = self.store_record_size(sd);
        let rec_size_alloca = self.entry_alloca(i64t.into(), "hdr.recsz");
        b!(self.bld.build_store(rec_size_alloca, i64t.const_int(rec_size, false)));
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

        // Flush
        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        b!(self.bld.build_unconditional_branch(done_bb));

        // done: return
        self.bld.position_at_end(done_bb);
        b!(self.bld.build_return(None));

        self.cur_fn = old_fn;
        if let Some(bb) = old_bb {
            self.bld.position_at_end(bb);
        }

        Ok(fv)
    }

    /// Get the FILE* pointer for a store (loads from global).
    fn load_store_fp(&mut self, store_name: &str) -> Result<PointerValue<'ctx>, String> {
        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global_name = format!("__store_{store_name}_fp");
        let global = self
            .module
            .get_global(&global_name)
            .ok_or_else(|| format!("no store global for '{store_name}'"))?;
        let fp = b!(self.bld.build_load(ptr_ty, global.as_pointer_value(), "store.fp"));
        Ok(fp.into_pointer_value())
    }

    /// Acquire exclusive advisory lock on a store file (LOCK_EX = 2).
    pub(crate) fn store_lock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        let fileno_fn = self.module.get_function("fileno").unwrap();
        let flock_fn = self.module.get_function("flock").unwrap();
        let fd = self.call_result(b!(self.bld.build_call(fileno_fn, &[fp.into()], "fd")));
        let lock_ex = self.ctx.i32_type().const_int(2, false); // LOCK_EX
        b!(self.bld.build_call(flock_fn, &[fd.into(), lock_ex.into()], ""));
        Ok(())
    }

    /// Release advisory lock on a store file (LOCK_UN = 8).
    pub(crate) fn store_unlock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        let fileno_fn = self.module.get_function("fileno").unwrap();
        let flock_fn = self.module.get_function("flock").unwrap();
        let fd = self.call_result(b!(self.bld.build_call(fileno_fn, &[fp.into()], "fd")));
        let lock_un = self.ctx.i32_type().const_int(8, false); // LOCK_UN
        b!(self.bld.build_call(flock_fn, &[fd.into(), lock_un.into()], ""));
        Ok(())
    }

    /// Compile `insert <store> val1, val2, ...`
    pub(crate) fn compile_store_insert(
        &mut self,
        store_name: &str,
        values: &[hir::Expr],
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        // Ensure store is open
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self
            .module
            .get_struct_type(&rec_name)
            .ok_or_else(|| format!("no store rec struct '{rec_name}'"))?;

        // Allocate a record buffer and zero it
        let rec_ptr = self.entry_alloca(st.into(), "store.rec");
        let rec_size = self.store_record_size(sd);
        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[
                rec_ptr.into(),
                i32t.const_int(0, false).into(),
                i64t.const_int(rec_size, false).into(),
            ],
            ""
        ));

        // Fill in each field
        for (i, (field_def, val_expr)) in sd.fields.iter().zip(values.iter()).enumerate() {
            let gep = b!(self.bld.build_struct_gep(st, rec_ptr, i as u32, &field_def.name));
            match &field_def.ty {
                Type::String => {
                    // Compile the string value, extract data+len, copy into fixed buffer
                    let val = self.compile_expr(val_expr)?;
                    self.copy_string_to_fixed_buf(val, gep)?;
                }
                _ => {
                    let val = self.compile_expr(val_expr)?;
                    b!(self.bld.build_store(gep, val));
                }
            }
        }

        // Seek to end of file and write the record
        let fseek_fn = self.module.get_function("fseek").unwrap();
        // SEEK_END = 2
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(0, false).into(),
                i32t.const_int(2, false).into(),
            ],
            ""
        ));

        let fwrite_fn = self.module.get_function("fwrite").unwrap();
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                rec_ptr.into(),
                i64t.const_int(rec_size, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        // Update record count in header: seek to offset 8, read count, increment, write back
        // Seek to count field (offset 8)
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(), // SEEK_SET = 0
            ],
            ""
        ));

        let count_buf = self.entry_alloca(i64t.into(), "count.buf");
        let fread_fn = self.module.get_function("fread").unwrap();
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

        let old_count = b!(self.bld.build_load(i64t, count_buf, "old.count"));
        let new_count = b!(self.bld.build_int_add(
            old_count.into_int_value(),
            i64t.const_int(1, false),
            "new.count"
        ));
        b!(self.bld.build_store(count_buf, new_count));

        // Seek back to offset 8 and write new count
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[
                count_buf.into(),
                i64t.const_int(8, false).into(),
                i64t.const_int(1, false).into(),
                fp.into(),
            ],
            ""
        ));

        // Flush
        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }

    /// Compile `count <store>` → i64
    pub(crate) fn compile_store_count(
        &mut self,
        store_name: &str,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        // Seek to offset 8 (count field in header)
        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[
                fp.into(),
                i64t.const_int(8, false).into(),
                i32t.const_int(0, false).into(),
            ],
            ""
        ));

        let count_buf = self.entry_alloca(i64t.into(), "store.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
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

        let count = b!(self.bld.build_load(i64t, count_buf, "count"));
        Ok(count)
    }

    /// Compile `<store> where <field> <op> <val>` → first matching record as struct
    pub(crate) fn compile_store_query(
        &mut self,
        store_name: &str,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        // Pre-compile filter values BEFORE the loop
        let (fi, ft, fv, extras) = self.precompile_filter_values(filter, sd)?;

        // Read count from header
        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "q.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
        b!(self.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        // Seek to start of records (offset HEADER_SIZE)
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""
        ));

        // Bulk read all records into memory
        let total = b!(self.bld.build_int_mul(count, i64t.const_int(rec_size, false), "q.total"));
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(IntPredicate::EQ, total, i64t.const_int(0, false), "q.isz")),
            one, total, "q.alloc"
        )).into_int_value();
        let malloc_fn = self.ensure_malloc();
        let buf = self.call_result(b!(self.bld.build_call(
            malloc_fn, &[alloc_size.into()], "q.buf"
        ))).into_pointer_value();
        b!(self.bld.build_call(
            fread_fn,
            &[buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""
        ));

        // Allocate result struct (zero-initialized)
        let result_ptr = self.entry_alloca(st.into(), "q.result");
        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[result_ptr.into(), i32t.const_int(0, false).into(), i64t.const_int(rec_size, false).into()],
            ""
        ));

        // Loop over in-memory buffer: for i = 0..count, check filter, if match → copy and break
        let fv_fn = self.cur_fn.unwrap();
        let idx_ptr = self.entry_alloca(i64t.into(), "q.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv_fn, "q.loop");
        let body_bb = self.ctx.append_basic_block(fv_fn, "q.body");
        let match_bb = self.ctx.append_basic_block(fv_fn, "q.match");
        let next_bb = self.ctx.append_basic_block(fv_fn, "q.next");
        let done_bb = self.ctx.append_basic_block(fv_fn, "q.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        // Loop header: check idx < count
        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "idx")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, idx, count, "q.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        // Body: get pointer to record in buffer
        self.bld.position_at_end(body_bb);
        let offset = b!(self.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "q.off"));
        let rec_ptr = unsafe {
            b!(self.bld.build_gep(self.ctx.i8_type(), buf, &[offset], "q.rec"))
        };

        // Evaluate the filter (values already compiled)
        let cond = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fv, &extras)?;
        b!(self.bld.build_conditional_branch(cond, match_bb, next_bb));

        // Match: copy record to result
        self.bld.position_at_end(match_bb);
        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[result_ptr.into(), rec_ptr.into(), i64t.const_int(rec_size, false).into()],
            ""
        ));
        b!(self.bld.build_unconditional_branch(done_bb));

        // Next: increment idx
        self.bld.position_at_end(next_bb);
        let next_idx = b!(self.bld.build_int_add(idx, i64t.const_int(1, false), "q.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Done: free buffer, load result struct and convert
        self.bld.position_at_end(done_bb);
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));
        let result = self.load_store_record_as_jade(st, result_ptr, sd)?;
        Ok(result)
    }

    /// Compile `all <store>` → pointer to array of Jade structs (with string conversion).
    pub(crate) fn compile_store_all(
        &mut self,
        store_name: &str,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let rec_st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        let jade_name = format!("__store_{store_name}");
        let jade_st = self.module.get_struct_type(&jade_name).unwrap();
        let jade_size = self.type_store_size(jade_st.into());

        // Read count from header
        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "all.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
        b!(self.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        // Seek to start of records
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""
        ));

        // Allocate a raw buffer for reading and a Jade output buffer
        let raw_total = b!(self.bld.build_int_mul(count, i64t.const_int(rec_size, false), "all.raw_total"));
        let jade_total = b!(self.bld.build_int_mul(count, i64t.const_int(jade_size, false), "all.jade_total"));
        let malloc_fn = self.ensure_malloc();
        let raw_buf = self.call_result(b!(self.bld.build_call(
            malloc_fn,
            &[raw_total.into()],
            "all.raw"
        ))).into_pointer_value();
        let jade_buf = self.call_result(b!(self.bld.build_call(
            malloc_fn,
            &[jade_total.into()],
            "all.jade"
        ))).into_pointer_value();

        // Read all raw records at once
        b!(self.bld.build_call(
            fread_fn,
            &[raw_buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""
        ));

        // Check if any string fields need conversion
        let has_strings = sd.fields.iter().any(|f| matches!(f.ty, Type::String));

        if has_strings {
            // Loop: convert each raw record to Jade struct
            let fv = self.cur_fn.unwrap();
            let idx_ptr = self.entry_alloca(i64t.into(), "all.idx");
            b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

            let loop_bb = self.ctx.append_basic_block(fv, "all.loop");
            let body_bb = self.ctx.append_basic_block(fv, "all.body");
            let done_bb = self.ctx.append_basic_block(fv, "all.done");

            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(loop_bb);
            let idx = b!(self.bld.build_load(i64t, idx_ptr, "all.i")).into_int_value();
            let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, idx, count, "all.cmp"));
            b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

            self.bld.position_at_end(body_bb);
            // Get pointer to raw record: raw_buf + idx * rec_size
            let raw_off = b!(self.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "all.roff"));
            let raw_ptr = unsafe {
                b!(self.bld.build_gep(self.ctx.i8_type(), raw_buf, &[raw_off], "all.rptr"))
            };
            // Convert to Jade struct
            let jade_val = self.load_store_record_as_jade(rec_st, raw_ptr, sd)?;
            // Store to output: jade_buf + idx * jade_size
            let jade_off = b!(self.bld.build_int_mul(idx, i64t.const_int(jade_size, false), "all.joff"));
            let jade_ptr = unsafe {
                b!(self.bld.build_gep(self.ctx.i8_type(), jade_buf, &[jade_off], "all.jptr"))
            };
            b!(self.bld.build_store(jade_ptr, jade_val));

            // Increment
            let next_idx = b!(self.bld.build_int_add(idx, i64t.const_int(1, false), "all.next"));
            b!(self.bld.build_store(idx_ptr, next_idx));
            b!(self.bld.build_unconditional_branch(loop_bb));

            self.bld.position_at_end(done_bb);
        } else {
            // No string fields — raw layout matches Jade layout, just memcpy
            let memcpy_fn = self.ensure_memcpy();
            b!(self.bld.build_call(
                memcpy_fn,
                &[jade_buf.into(), raw_buf.into(), raw_total.into()],
                ""
            ));
        }

        // Free the raw buffer
        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[raw_buf.into()], ""));

        Ok(jade_buf.into())
    }

    /// Compile `delete <store> where <field> <op> <val>`
    /// Strategy: read all records, rewrite file without matching ones.
    pub(crate) fn compile_store_delete(
        &mut self,
        store_name: &str,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        // Pre-compile filter values BEFORE the loop
        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;

        // Read count
        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "del.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
        b!(self.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        // Seek to records
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""
        ));

        // Read all records into buffer
        let total = b!(self.bld.build_int_mul(count, i64t.const_int(rec_size, false), "del.total"));
        let malloc_fn = self.ensure_malloc();
        let buf = self.call_result(b!(self.bld.build_call(
            malloc_fn,
            &[total.into()],
            "del.buf"
        ))).into_pointer_value();

        b!(self.bld.build_call(
            fread_fn,
            &[buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""
        ));

        // Close the current file
        let fclose_fn = self.module.get_function("fclose").unwrap();
        b!(self.bld.build_call(fclose_fn, &[fp.into()], ""));

        // Reopen with "w+b" to truncate
        let filename = format!("{store_name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "del.path"));
        let mode_wb = b!(self.bld.build_global_string_ptr("w+b\0", "del.mode"));
        let fopen_fn = self.module.get_function("fopen").unwrap();
        let new_fp = self.call_result(b!(self.bld.build_call(
            fopen_fn,
            &[file_str.as_pointer_value().into(), mode_wb.as_pointer_value().into()],
            "del.fp"
        ))).into_pointer_value();

        // Update global
        let global_name = format!("__store_{store_name}_fp");
        let global = self.module.get_global(&global_name).unwrap();
        b!(self.bld.build_store(global.as_pointer_value(), new_fp));

        // Write header with magic and record_size (count will be updated after)
        let fwrite_fn = self.module.get_function("fwrite").unwrap();
        let magic = b!(self.bld.build_global_string_ptr("JADESTR\0", "del.magic"));
        b!(self.bld.build_call(
            fwrite_fn,
            &[magic.as_pointer_value().into(), i64t.const_int(1, false).into(), i64t.const_int(8, false).into(), new_fp.into()],
            ""
        ));

        // Write placeholder count = 0
        let new_count_ptr = self.entry_alloca(i64t.into(), "del.newcount");
        b!(self.bld.build_store(new_count_ptr, i64t.const_int(0, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[new_count_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""
        ));

        // Write record_size
        let rec_size_ptr = self.entry_alloca(i64t.into(), "del.recsz");
        b!(self.bld.build_store(rec_size_ptr, i64t.const_int(rec_size, false)));
        b!(self.bld.build_call(
            fwrite_fn,
            &[rec_size_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""
        ));

        // Loop over old records, write non-matching ones
        let fv_fn = self.cur_fn.unwrap();
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
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, idx, count, "del.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        // rec_ptr = buf + idx * rec_size
        let offset = b!(self.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "del.off"));
        let rec_ptr = unsafe {
            b!(self.bld.build_gep(self.ctx.i8_type(), buf, &[offset], "del.rec"))
        };

        let matches = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fval, &extras)?;
        // If matches, skip (delete); if not, keep
        b!(self.bld.build_conditional_branch(matches, skip_bb, keep_bb));

        self.bld.position_at_end(keep_bb);
        // Write this record to new file
        b!(self.bld.build_call(
            fwrite_fn,
            &[rec_ptr.into(), i64t.const_int(rec_size, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""
        ));
        // Increment kept count
        let kept = b!(self.bld.build_load(i64t, new_count_ptr, "kept")).into_int_value();
        let kept_inc = b!(self.bld.build_int_add(kept, i64t.const_int(1, false), "kept.inc"));
        b!(self.bld.build_store(new_count_ptr, kept_inc));
        b!(self.bld.build_unconditional_branch(skip_bb));

        self.bld.position_at_end(skip_bb);
        let next_idx = b!(self.bld.build_int_add(idx, i64t.const_int(1, false), "del.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Done: update count in header, free buffer, flush
        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(
            fseek_fn,
            &[new_fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        b!(self.bld.build_call(
            fwrite_fn,
            &[new_count_ptr.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), new_fp.into()],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[new_fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }

    /// Compile `set <store> <field> <val> [, ...] where <filter>`
    /// Strategy: read all records, find matching ones, update fields in-place,
    /// write the entire buffer back.
    pub(crate) fn compile_store_set(
        &mut self,
        store_name: &str,
        assignments: &[(String, hir::Expr)],
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<(), String> {
        let ensure_fn = self.gen_store_ensure_open(sd)?;
        b!(self.bld.build_call(ensure_fn, &[], ""));

        let fp = self.load_store_fp(store_name)?;
        self.store_lock(fp)?;
        let i64t = self.ctx.i64_type();
        let i32t = self.ctx.i32_type();

        let rec_name = format!("__store_{store_name}_rec");
        let st = self.module.get_struct_type(&rec_name).unwrap();
        let rec_size = self.store_record_size(sd);

        // Compile all assignment values
        let mut assign_vals = Vec::new();
        for (fname, fexpr) in assignments {
            let idx = sd.fields.iter().position(|f| f.name == *fname).unwrap();
            let val = self.compile_expr(fexpr)?;
            assign_vals.push((idx, fname.clone(), val));
        }

        // Read count
        let fseek_fn = self.module.get_function("fseek").unwrap();
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(8, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let count_buf = self.entry_alloca(i64t.into(), "set.count");
        b!(self.bld.build_store(count_buf, i64t.const_int(0, false)));
        let fread_fn = self.module.get_function("fread").unwrap();
        b!(self.bld.build_call(
            fread_fn,
            &[count_buf.into(), i64t.const_int(8, false).into(), i64t.const_int(1, false).into(), fp.into()],
            ""
        ));
        let count = b!(self.bld.build_load(i64t, count_buf, "count")).into_int_value();

        // Seek to records
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""
        ));

        // Read all records into buffer
        let total = b!(self.bld.build_int_mul(count, i64t.const_int(rec_size, false), "set.total"));
        let malloc_fn = self.ensure_malloc();
        let buf = self.call_result(b!(self.bld.build_call(
            malloc_fn,
            &[total.into()],
            "set.buf"
        ))).into_pointer_value();
        b!(self.bld.build_call(
            fread_fn,
            &[buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""
        ));

        // Loop over records, modify matching ones in the buffer

        // Precompile filter values before loop
        let (fi, ft, fval, extras) = self.precompile_filter_values(filter, sd)?;
        let fv = self.cur_fn.unwrap();
        let idx_ptr = self.entry_alloca(i64t.into(), "set.idx");
        b!(self.bld.build_store(idx_ptr, i64t.const_int(0, false)));

        let loop_bb = self.ctx.append_basic_block(fv, "set.loop");
        let body_bb = self.ctx.append_basic_block(fv, "set.body");
        let update_bb = self.ctx.append_basic_block(fv, "set.update");
        let next_bb = self.ctx.append_basic_block(fv, "set.next");
        let done_bb = self.ctx.append_basic_block(fv, "set.done");

        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let idx = b!(self.bld.build_load(i64t, idx_ptr, "set.i")).into_int_value();
        let cmp = b!(self.bld.build_int_compare(IntPredicate::ULT, idx, count, "set.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let offset = b!(self.bld.build_int_mul(idx, i64t.const_int(rec_size, false), "set.off"));
        let rec_ptr = unsafe {
            b!(self.bld.build_gep(self.ctx.i8_type(), buf, &[offset], "set.rec"))
        };
        let matches = self.eval_store_filter(rec_ptr, st, fi, &ft, filter.op, fval, &extras)?;
        b!(self.bld.build_conditional_branch(matches, update_bb, next_bb));

        // Update matching record fields in-place
        self.bld.position_at_end(update_bb);
        for (field_idx, _fname, val) in &assign_vals {
            let fty = &sd.fields[*field_idx].ty;
            let gep = b!(self.bld.build_struct_gep(st, rec_ptr, *field_idx as u32, "set.assign"));
            match fty {
                Type::String => {
                    self.copy_string_to_fixed_buf(*val, gep)?;
                }
                _ => {
                    b!(self.bld.build_store(gep, *val));
                }
            }
        }
        b!(self.bld.build_unconditional_branch(next_bb));

        self.bld.position_at_end(next_bb);
        let next_idx = b!(self.bld.build_int_add(idx, i64t.const_int(1, false), "set.next"));
        b!(self.bld.build_store(idx_ptr, next_idx));
        b!(self.bld.build_unconditional_branch(loop_bb));

        // Done: write all records back to file
        self.bld.position_at_end(done_bb);
        b!(self.bld.build_call(
            fseek_fn,
            &[fp.into(), i64t.const_int(HEADER_SIZE, false).into(), i32t.const_int(0, false).into()],
            ""
        ));
        let fwrite_fn = self.module.get_function("fwrite").unwrap();
        b!(self.bld.build_call(
            fwrite_fn,
            &[buf.into(), i64t.const_int(rec_size, false).into(), count.into(), fp.into()],
            ""
        ));

        let free_fn = self.ensure_free();
        b!(self.bld.build_call(free_fn, &[buf.into()], ""));

        let fflush_fn = self.module.get_function("fflush").unwrap();
        b!(self.bld.build_call(fflush_fn, &[fp.into()], ""));

        self.store_unlock(fp)?;
        Ok(())
    }

    /// Copy a Jade String value into a fixed [256 x i8] buffer at `buf_ptr`.
    /// Layout: [8 bytes len][248 bytes data, zero-padded]
    fn copy_string_to_fixed_buf(
        &mut self,
        string_val: BasicValueEnum<'ctx>,
        buf_ptr: PointerValue<'ctx>,
    ) -> Result<(), String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let i32t = self.ctx.i32_type();

        // Zero out the buffer first
        let memset_fn = self.module.get_function("memset").unwrap();
        b!(self.bld.build_call(
            memset_fn,
            &[buf_ptr.into(), i32t.const_int(0, false).into(), i64t.const_int(STRING_BUF_SIZE, false).into()],
            ""
        ));

        // Get string length and data pointer
        let len = self.string_len(string_val)?.into_int_value();
        let data = self.string_data(string_val)?.into_pointer_value();

        // Clamp len to 248 (leave 8 bytes for length prefix)
        let max_data = i64t.const_int(STRING_BUF_SIZE - 8, false);
        let clamped = b!(self.bld.build_select(
            b!(self.bld.build_int_compare(IntPredicate::UGT, len, max_data, "str.clamp")),
            max_data,
            len,
            "str.len"
        ));

        // Store length at buf_ptr[0..8]
        b!(self.bld.build_store(buf_ptr, clamped));

        // Copy data at buf_ptr + 8
        let data_dst = unsafe {
            b!(self.bld.build_gep(i8t, buf_ptr, &[i64t.const_int(8, false)], "str.dst"))
        };
        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[data_dst.into(), data.into(), clamped.into()],
            ""
        ));

        Ok(())
    }

    /// Read a fixed [256 x i8] buffer and produce a Jade String value.
    /// Layout: [8 bytes len][248 bytes data]
    fn read_string_from_fixed_buf(
        &mut self,
        buf_ptr: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();

        // Read length from first 8 bytes
        let len = b!(self.bld.build_load(i64t, buf_ptr, "str.len")).into_int_value();

        // Data starts at offset 8
        let data_src = unsafe {
            b!(self.bld.build_gep(i8t, buf_ptr, &[i64t.const_int(8, false)], "str.src"))
        };

        // Allocate heap buffer for the string data and copy
        let malloc_fn = self.ensure_malloc();
        let one = i64t.const_int(1, false);
        let alloc_size = b!(self.bld.build_int_add(len, one, "str.alloc"));
        let heap = self.call_result(b!(self.bld.build_call(
            malloc_fn,
            &[alloc_size.into()],
            "str.heap"
        ))).into_pointer_value();

        let memcpy_fn = self.ensure_memcpy();
        b!(self.bld.build_call(
            memcpy_fn,
            &[heap.into(), data_src.into(), len.into()],
            ""
        ));

        // Null-terminate
        let end = unsafe {
            b!(self.bld.build_gep(i8t, heap, &[len], "str.end"))
        };
        b!(self.bld.build_store(end, i8t.const_int(0, false)));

        // Build Jade String struct {ptr, len, cap=0}
        self.build_string(heap, len, i64t.const_int(0, false), "str.from_store")
    }

    /// Load a store record from a raw struct pointer, converting fixed-buf
    /// string fields back into Jade String values, and return as a Jade struct value.
    fn load_store_record_as_jade(
        &mut self,
        st: inkwell::types::StructType<'ctx>,
        raw_ptr: PointerValue<'ctx>,
        sd: &hir::StoreDef,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let jade_struct_name = format!("__store_{}", sd.name);
        let jade_st = self
            .module
            .get_struct_type(&jade_struct_name)
            .ok_or_else(|| format!("no jade store struct '{jade_struct_name}'"))?;
        let jade_ptr = self.entry_alloca(jade_st.into(), "jade.rec");

        for (i, field) in sd.fields.iter().enumerate() {
            let src_gep = b!(self.bld.build_struct_gep(st, raw_ptr, i as u32, &format!("raw.{}", field.name)));
            let dst_gep = b!(self.bld.build_struct_gep(jade_st, jade_ptr, i as u32, &format!("jade.{}", field.name)));
            match &field.ty {
                Type::String => {
                    let s = self.read_string_from_fixed_buf(src_gep)?;
                    b!(self.bld.build_store(dst_gep, s));
                }
                ty => {
                    let lty = self.llvm_ty(ty);
                    let val = b!(self.bld.build_load(lty, src_gep, &field.name));
                    b!(self.bld.build_store(dst_gep, val));
                }
            }
        }

        Ok(b!(self.bld.build_load(jade_st, jade_ptr, "jade.result")))
    }

    /// Pre-compile all filter values (must be called BEFORE entering any loop).
    /// Returns the primary (field_idx, field_ty, compiled_val) and a vec of extras.
    fn precompile_filter_values(
        &mut self,
        filter: &hir::StoreFilter,
        sd: &hir::StoreDef,
    ) -> Result<(
        usize, Type, BasicValueEnum<'ctx>,
        Vec<(crate::ast::LogicalOp, usize, Type, BinOp, BasicValueEnum<'ctx>)>,
    ), String> {
        let (field_idx, field_ty) = sd.fields.iter().enumerate()
            .find(|(_, f)| f.name == filter.field)
            .map(|(i, f)| (i, f.ty.clone()))
            .unwrap();
        let filter_val = self.compile_expr(&filter.value)?;
        let mut extras = Vec::new();
        for (lop, cond) in &filter.extra {
            let (ci, ct) = sd.fields.iter().enumerate()
                .find(|(_, f)| f.name == cond.field)
                .map(|(i, f)| (i, f.ty.clone()))
                .unwrap();
            let cv = self.compile_expr(&cond.value)?;
            extras.push((*lop, ci, ct, cond.op, cv));
        }
        Ok((field_idx, field_ty, filter_val, extras))
    }

    /// Evaluate a pre-compiled filter against a record pointer (safe to call inside loops).
    /// Returns an i1 (bool).
    fn eval_store_filter(
        &mut self,
        rec_ptr: PointerValue<'ctx>,
        rec_st: inkwell::types::StructType<'ctx>,
        primary_idx: usize,
        primary_ty: &Type,
        primary_op: BinOp,
        primary_val: BasicValueEnum<'ctx>,
        extras: &[(crate::ast::LogicalOp, usize, Type, BinOp, BasicValueEnum<'ctx>)],
    ) -> Result<IntValue<'ctx>, String> {
        let field_gep = b!(self.bld.build_struct_gep(rec_st, rec_ptr, primary_idx as u32, "sf.field"));
        let mut result = self.store_compare_field(field_gep, primary_ty, primary_op, primary_val)?;
        for (lop, ci, ct, op, cv) in extras {
            let cg = b!(self.bld.build_struct_gep(rec_st, rec_ptr, *ci as u32, "sf.efield"));
            let ecmp = self.store_compare_field(cg, ct, *op, *cv)?;
            result = match lop {
                crate::ast::LogicalOp::And => b!(self.bld.build_and(result, ecmp, "sf.and")),
                crate::ast::LogicalOp::Or => b!(self.bld.build_or(result, ecmp, "sf.or")),
            };
        }
        Ok(result)
    }

    /// Compare a store field value with a filter value. Returns an i1 (bool).
    fn store_compare_field(
        &mut self,
        field_ptr: PointerValue<'ctx>,
        field_ty: &Type,
        op: BinOp,
        filter_val: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, String> {
        match field_ty {
            Type::String => {
                // Compare string: read the fixed buf, compare with filter string
                let i64t = self.ctx.i64_type();
                let i8t = self.ctx.i8_type();
                let i32t = self.ctx.i32_type();

                // Get the stored string's length and data
                let stored_len = b!(self.bld.build_load(i64t, field_ptr, "cmp.slen")).into_int_value();
                let stored_data = unsafe {
                    b!(self.bld.build_gep(i8t, field_ptr, &[i64t.const_int(8, false)], "cmp.sdata"))
                };

                // Get filter string's length and data
                let filter_len = self.string_len(filter_val)?.into_int_value();
                let filter_data = self.string_data(filter_val)?.into_pointer_value();

                // For Eq/Ne: compare lengths first, then memcmp
                let fv = self.cur_fn.unwrap();
                let len_eq_bb = self.ctx.append_basic_block(fv, "cmp.len_eq");
                let result_bb = self.ctx.append_basic_block(fv, "cmp.result");

                let len_match = b!(self.bld.build_int_compare(
                    IntPredicate::EQ, stored_len, filter_len, "cmp.leneq"
                ));

                match op {
                    BinOp::Eq => {
                        let false_bb = self.ctx.append_basic_block(fv, "cmp.false");
                        b!(self.bld.build_conditional_branch(len_match, len_eq_bb, false_bb));

                        self.bld.position_at_end(false_bb);
                        b!(self.bld.build_unconditional_branch(result_bb));

                        self.bld.position_at_end(len_eq_bb);
                        let memcmp_fn = self.ensure_memcmp();
                        let cmp_result = self.call_result(b!(self.bld.build_call(
                            memcmp_fn,
                            &[stored_data.into(), filter_data.into(), stored_len.into()],
                            "cmp.mc"
                        ))).into_int_value();
                        let is_eq = b!(self.bld.build_int_compare(
                            IntPredicate::EQ, cmp_result, i32t.const_int(0, false), "cmp.eq"
                        ));
                        b!(self.bld.build_unconditional_branch(result_bb));
                        let len_eq_end = self.bld.get_insert_block().unwrap();

                        self.bld.position_at_end(result_bb);
                        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "cmp.str"));
                        phi.add_incoming(&[
                            (&self.ctx.bool_type().const_int(0, false), false_bb),
                            (&is_eq, len_eq_end),
                        ]);
                        Ok(phi.as_basic_value().into_int_value())
                    }
                    BinOp::Ne => {
                        let true_bb = self.ctx.append_basic_block(fv, "cmp.true");
                        b!(self.bld.build_conditional_branch(len_match, len_eq_bb, true_bb));

                        self.bld.position_at_end(true_bb);
                        b!(self.bld.build_unconditional_branch(result_bb));

                        self.bld.position_at_end(len_eq_bb);
                        let memcmp_fn = self.ensure_memcmp();
                        let cmp_result = self.call_result(b!(self.bld.build_call(
                            memcmp_fn,
                            &[stored_data.into(), filter_data.into(), stored_len.into()],
                            "cmp.mc"
                        ))).into_int_value();
                        let is_ne = b!(self.bld.build_int_compare(
                            IntPredicate::NE, cmp_result, i32t.const_int(0, false), "cmp.ne"
                        ));
                        b!(self.bld.build_unconditional_branch(result_bb));
                        let len_eq_end = self.bld.get_insert_block().unwrap();

                        self.bld.position_at_end(result_bb);
                        let phi = b!(self.bld.build_phi(self.ctx.bool_type(), "cmp.str"));
                        phi.add_incoming(&[
                            (&self.ctx.bool_type().const_int(1, false), true_bb),
                            (&is_ne, len_eq_end),
                        ]);
                        Ok(phi.as_basic_value().into_int_value())
                    }
                    _ => {
                        // For ordered comparisons on strings, use memcmp result
                        // (not fully correct for Unicode but good enough for v1)
                        // Use min of the two lengths for memcmp, then break ties by length
                        result_bb.remove_from_function().unwrap();
                        len_eq_bb.remove_from_function().unwrap();

                        let min_len = b!(self.bld.build_select(
                            b!(self.bld.build_int_compare(IntPredicate::ULT, stored_len, filter_len, "min.cmp")),
                            stored_len,
                            filter_len,
                            "min.len"
                        )).into_int_value();

                        let memcmp_fn = self.ensure_memcmp();
                        let cmp_result = self.call_result(b!(self.bld.build_call(
                            memcmp_fn,
                            &[stored_data.into(), filter_data.into(), min_len.into()],
                            "cmp.mc"
                        ))).into_int_value();

                        let pred = match op {
                            BinOp::Lt => IntPredicate::SLT,
                            BinOp::Gt => IntPredicate::SGT,
                            BinOp::Le => IntPredicate::SLE,
                            BinOp::Ge => IntPredicate::SGE,
                            _ => unreachable!(),
                        };
                        Ok(b!(self.bld.build_int_compare(
                            pred, cmp_result, i32t.const_int(0, false), "cmp.ord"
                        )))
                    }
                }
            }
            Type::I64 | Type::U64 | Type::I32 | Type::U32 | Type::I16 | Type::U16 | Type::I8 | Type::U8 => {
                let lty = self.llvm_ty(field_ty);
                let stored = b!(self.bld.build_load(lty, field_ptr, "cmp.ival")).into_int_value();
                let filter_int = filter_val.into_int_value();
                let pred = match op {
                    BinOp::Eq => IntPredicate::EQ,
                    BinOp::Ne => IntPredicate::NE,
                    BinOp::Lt => IntPredicate::SLT,
                    BinOp::Gt => IntPredicate::SGT,
                    BinOp::Le => IntPredicate::SLE,
                    BinOp::Ge => IntPredicate::SGE,
                    _ => return Err(format!("unsupported store filter op: {:?}", op)),
                };
                Ok(b!(self.bld.build_int_compare(pred, stored, filter_int, "cmp.i")))
            }
            Type::F64 | Type::F32 => {
                let lty = self.llvm_ty(field_ty);
                let stored = b!(self.bld.build_load(lty, field_ptr, "cmp.fval")).into_float_value();
                let filter_float = filter_val.into_float_value();
                use inkwell::FloatPredicate;
                let pred = match op {
                    BinOp::Eq => FloatPredicate::OEQ,
                    BinOp::Ne => FloatPredicate::ONE,
                    BinOp::Lt => FloatPredicate::OLT,
                    BinOp::Gt => FloatPredicate::OGT,
                    BinOp::Le => FloatPredicate::OLE,
                    BinOp::Ge => FloatPredicate::OGE,
                    _ => return Err(format!("unsupported store filter op: {:?}", op)),
                };
                Ok(b!(self.bld.build_float_compare(pred, stored, filter_float, "cmp.f")))
            }
            Type::Bool => {
                let stored = b!(self.bld.build_load(self.ctx.bool_type(), field_ptr, "cmp.bval")).into_int_value();
                let filter_bool = filter_val.into_int_value();
                let pred = match op {
                    BinOp::Eq => IntPredicate::EQ,
                    BinOp::Ne => IntPredicate::NE,
                    _ => return Err("bool fields only support equals/isnt comparisons".into()),
                };
                Ok(b!(self.bld.build_int_compare(pred, stored, filter_bool, "cmp.b")))
            }
            _ => Err(format!("unsupported store field type for filtering: {:?}", field_ty)),
        }
    }
}
