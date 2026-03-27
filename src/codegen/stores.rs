use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{FunctionValue, PointerValue};

use crate::hir;
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
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        self.structs.insert(struct_name.clone(), fields);

        let ptr_ty = self.ctx.ptr_type(AddressSpace::default());
        let global = self
            .module
            .add_global(ptr_ty, None, &format!("__store_{}_fp", sd.name));
        global.set_linkage(Linkage::Internal);
        global.set_initializer(&ptr_ty.const_null());

        Ok(())
    }

    fn store_field_llvm_ty(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match ty {
            Type::String => self.ctx.i8_type().array_type(STRING_BUF_SIZE as u32).into(),
            other => self.llvm_ty(other),
        }
    }

    pub(crate) fn store_record_size(&self, sd: &hir::StoreDef) -> u64 {
        let rec_name = format!("__store_{}_rec", sd.name);
        let st = self.module.get_struct_type(&rec_name).unwrap();
        self.type_store_size(st.into())
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
        b!(self.bld.build_conditional_branch(is_null, open_bb, done_bb));

        self.bld.position_at_end(open_bb);
        let filename = format!("{name}.store\0");
        let file_str = b!(self.bld.build_global_string_ptr(&filename, "store.path"));
        let mode_rw = b!(self.bld.build_global_string_ptr("r+b\0", "mode.rw"));
        let fopen_fn = self.module.get_function("fopen").unwrap();
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
        open_bb.get_terminator().unwrap().erase_from_basic_block();
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

        let fwrite_fn = self.module.get_function("fwrite").unwrap();

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

        let fflush_fn = self.module.get_function("fflush").unwrap();
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

    const LOCK_EX: u64 = 2;
    const LOCK_UN: u64 = 8;

    pub(crate) fn store_lock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        self.store_flock(fp, Self::LOCK_EX)
    }

    pub(crate) fn store_unlock(&mut self, fp: PointerValue<'ctx>) -> Result<(), String> {
        self.store_flock(fp, Self::LOCK_UN)
    }

    fn store_flock(&mut self, fp: PointerValue<'ctx>, op: u64) -> Result<(), String> {
        let fileno_fn = self.module.get_function("fileno").unwrap();
        let flock_fn = self.module.get_function("flock").unwrap();
        let fd = self.call_result(b!(self.bld.build_call(fileno_fn, &[fp.into()], "fd")));
        let lock_op = self.ctx.i32_type().const_int(op, false);
        b!(self
            .bld
            .build_call(flock_fn, &[fd.into(), lock_op.into()], ""));
        Ok(())
    }
}
