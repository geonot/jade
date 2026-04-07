use inkwell::values::BasicValueEnum;
use inkwell::{AddressSpace, IntPredicate};

use crate::hir;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_map_new(&mut self) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();
        let malloc = self.ensure_malloc();

        let header_size = i64t.const_int(24, false);
        let header_ptr = b!(self
            .bld
            .build_call(malloc, &[header_size.into()], "map.hdr"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let init_cap = 16u64;
        let entry_size = 48u64;
        let calloc = self.ensure_calloc();
        let buf = b!(self.bld.build_call(
            calloc,
            &[
                i64t.const_int(init_cap, false).into(),
                i64t.const_int(entry_size, false).into()
            ],
            "map.buf"
        ))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "map.ptr"));
        b!(self.bld.build_store(ptr_gep, buf));

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "map.len"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "map.cap"));
        b!(self
            .bld
            .build_store(cap_gep, i64t.const_int(init_cap, false)));

        Ok(header_ptr.into())
    }

    pub(crate) fn compile_map_method(
        &mut self,
        obj: &hir::Expr,
        method: &str,
        args: &[hir::Expr],
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let obj_val = self.compile_expr(obj)?;
        let header_ptr = obj_val.into_pointer_value();

        match method {
            "len" => self.vec_len(header_ptr),
            "set" => {
                if args.len() < 2 { return Err("map.set() requires key and value".into()); }
                let key_val = self.compile_expr(&args[0])?;
                let val_val = self.compile_expr(&args[1])?;
                self.map_set_val(header_ptr, key_val, val_val)
            }
            "get" => {
                if args.is_empty() { return Err("map.get() requires a key".into()); }
                let key_val = self.compile_expr(&args[0])?;
                self.map_get_val(header_ptr, key_val)
            }
            "has" | "contains" => {
                if args.is_empty() { return Err("map.has() requires a key".into()); }
                let key_val = self.compile_expr(&args[0])?;
                self.map_has_val(header_ptr, key_val)
            }
            "remove" => {
                if args.is_empty() { return Err("map.remove() requires a key".into()); }
                let key_val = self.compile_expr(&args[0])?;
                self.map_remove_val(header_ptr, key_val)
            }
            "clear" => self.map_clear(header_ptr),
            "keys" => self.map_keys(header_ptr),
            "values" => self.map_values(header_ptr),
            _ => Err(format!("no method '{method}' on Map")),
        }
    }

    fn map_probe(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        hash: inkwell::values::IntValue<'ctx>,
        match_bb: inkwell::basic_block::BasicBlock<'ctx>,
        empty_bb: inkwell::basic_block::BasicBlock<'ctx>,
    ) -> Result<
        (
            inkwell::values::PointerValue<'ctx>,
            inkwell::values::PointerValue<'ctx>,
        ),
        String,
    > {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mp.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mp.cap")).into_int_value();
        let mask = b!(self
            .bld
            .build_int_nsw_sub(cap, i64t.const_int(1, false), "mp.mask"));
        let start_idx = b!(self.bld.build_and(hash, mask, "mp.idx"));
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mp.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mp.entries"
        ))
        .into_pointer_value();
        let entry_size = i64t.const_int(48, false);

        let loop_bb = self.ctx.append_basic_block(fv, "mp.loop");
        let check_bb = self.ctx.append_basic_block(fv, "mp.check");
        let next_bb = self.ctx.append_basic_block(fv, "mp.next");
        let pre_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(loop_bb);
        let phi_idx = b!(self.bld.build_phi(i64t, "mp.i"));
        phi_idx.add_incoming(&[(&start_idx, pre_bb)]);
        let idx = phi_idx.as_basic_value().into_int_value();
        let byte_off = b!(self.bld.build_int_nsw_mul(idx, entry_size, "mp.off"));
        let entry_ptr = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "mp.eptr")) };
        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(40, false)], "mp.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "mp.occ")).into_int_value();
        let is_occ =
            b!(self
                .bld
                .build_int_compare(IntPredicate::NE, occ, i8t.const_int(0, false), "mp.io"));
        b!(self
            .bld
            .build_conditional_branch(is_occ, check_bb, empty_bb));

        self.bld.position_at_end(check_bb);
        let shp = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "mp.shp"))
        };
        let sh = b!(self.bld.build_load(i64t, shp, "mp.sh")).into_int_value();
        let heq = b!(self
            .bld
            .build_int_compare(IntPredicate::EQ, sh, hash, "mp.heq"));
        b!(self.bld.build_conditional_branch(heq, match_bb, next_bb));

        self.bld.position_at_end(next_bb);
        let ni = b!(self
            .bld
            .build_int_nsw_add(idx, i64t.const_int(1, false), "mp.ni"));
        let wi = b!(self.bld.build_and(ni, mask, "mp.wi"));
        phi_idx.add_incoming(&[(&wi, next_bb)]);
        b!(self.bld.build_unconditional_branch(loop_bb));

        self.bld.position_at_end(match_bb);
        Ok((entry_ptr, occ_ptr))
    }

    pub(crate) fn map_set_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        key_val: BasicValueEnum<'ctx>,
        val_val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();
        let hash = self.fnv_hash_string(key_val)?;

        let overwrite_bb = self.ctx.append_basic_block(fv, "ms.overwrite");
        let empty_bb = self.ctx.append_basic_block(fv, "ms.empty");
        let done_bb = self.ctx.append_basic_block(fv, "ms.done");

        let (entry_ptr, occ_ptr) = self.map_probe(header_ptr, hash, overwrite_bb, empty_bb)?;

        let val_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "ms.vp"))
        };
        b!(self.bld.build_store(val_ptr, val_val));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(empty_bb);
        let hash_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(0, false)], "ms.hp2"))
        };
        b!(self.bld.build_store(hash_ptr, hash));
        let key_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(8, false)], "ms.kp"))
        };
        let memcpy = self.ensure_memcpy();
        let key_alloca = self.entry_alloca(self.string_type().into(), "ms.ktmp");
        b!(self.bld.build_store(key_alloca, key_val));
        b!(self.bld.build_call(
            memcpy,
            &[
                key_ptr.into(),
                key_alloca.into(),
                i64t.const_int(24, false).into()
            ],
            ""
        ));
        let val_ptr2 = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "ms.vp2"))
        };
        b!(self.bld.build_store(val_ptr2, val_val));
        b!(self.bld.build_store(occ_ptr, i8t.const_int(1, false)));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "ms.lenp"));
        let len = b!(self.bld.build_load(i64t, len_gep, "ms.len")).into_int_value();
        let new_len = b!(self
            .bld
            .build_int_nsw_add(len, i64t.const_int(1, false), "ms.nl"));
        b!(self.bld.build_store(len_gep, new_len));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(crate) fn map_get_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        key_val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fv = self.cur_fn.unwrap();
        let hash = self.fnv_hash_string(key_val)?;

        let found_bb = self.ctx.append_basic_block(fv, "mg.found");
        let nf_bb = self.ctx.append_basic_block(fv, "mg.nf");
        let merge_bb = self.ctx.append_basic_block(fv, "mg.merge");

        let (entry_ptr, _) = self.map_probe(header_ptr, hash, found_bb, nf_bb)?;

        let val_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, entry_ptr, &[i64t.const_int(32, false)], "mg.vp"))
        };
        let found_val = b!(self.bld.build_load(i64t, val_ptr, "mg.fv"));
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(nf_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let phi = b!(self.bld.build_phi(i64t, "mg.v"));
        phi.add_incoming(&[(&found_val, found_bb), (&i64t.const_int(0, false), nf_bb)]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn map_has_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        key_val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let fv = self.cur_fn.unwrap();
        let hash = self.fnv_hash_string(key_val)?;

        let found_bb = self.ctx.append_basic_block(fv, "mh.found");
        let nf_bb = self.ctx.append_basic_block(fv, "mh.nf");
        let merge_bb = self.ctx.append_basic_block(fv, "mh.merge");

        self.map_probe(header_ptr, hash, found_bb, nf_bb)?;

        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(nf_bb);
        b!(self.bld.build_unconditional_branch(merge_bb));

        self.bld.position_at_end(merge_bb);
        let bool_t = self.ctx.bool_type();
        let phi = b!(self.bld.build_phi(bool_t, "mh.v"));
        phi.add_incoming(&[
            (&bool_t.const_int(1, false), found_bb),
            (&bool_t.const_int(0, false), nf_bb),
        ]);
        Ok(phi.as_basic_value())
    }

    pub(crate) fn map_remove_val(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        key_val: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();
        let hash = self.fnv_hash_string(key_val)?;

        let found_bb = self.ctx.append_basic_block(fv, "mr.found");
        let done_bb = self.ctx.append_basic_block(fv, "mr.done");

        let (_, occ_ptr) = self.map_probe(header_ptr, hash, found_bb, done_bb)?;

        b!(self.bld.build_store(occ_ptr, i8t.const_int(0, false)));
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "mr.lenp"));
        let cur_len = b!(self.bld.build_load(i64t, len_gep, "mr.len")).into_int_value();
        let new_len = b!(self
            .bld
            .build_int_nsw_sub(cur_len, i64t.const_int(1, false), "mr.nl"));
        b!(self.bld.build_store(len_gep, new_len));
        b!(self.bld.build_unconditional_branch(done_bb));

        self.bld.position_at_end(done_bb);
        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    pub(crate) fn map_clear(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let header_ty = self.vec_header_type();

        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mc.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mc.entries"
        ))
        .into_pointer_value();
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mc.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mc.cap")).into_int_value();
        let total = b!(self
            .bld
            .build_int_nsw_mul(cap, i64t.const_int(48, false), "mc.total"));
        let memset = self.ensure_memset();
        let zero_i32 = self.ctx.i32_type().const_int(0, false);
        b!(self
            .bld
            .build_call(memset, &[entries.into(), zero_i32.into(), total.into()], ""));

        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "mc.lenp"));
        b!(self.bld.build_store(len_gep, i64t.const_int(0, false)));

        Ok(self.ctx.i8_type().const_int(0, false).into())
    }

    fn map_keys(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.map_collect(header_ptr, 8) // key offset = 8
    }

    fn map_values(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        self.map_collect(header_ptr, 32) // value offset = 32
    }

    /// Iterate all occupied entries in the map and collect the field at
    /// `field_offset` bytes into each entry into a new Vec.
    fn map_collect(
        &mut self,
        header_ptr: inkwell::values::PointerValue<'ctx>,
        field_offset: u64,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let header_ty = self.vec_header_type();
        let fv = self.cur_fn.unwrap();

        // Read map capacity and entries pointer
        let cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 2, "mk.capp"));
        let cap = b!(self.bld.build_load(i64t, cap_gep, "mk.cap")).into_int_value();
        let ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 0, "mk.ptrp"));
        let entries = b!(self.bld.build_load(
            self.ctx.ptr_type(AddressSpace::default()),
            ptr_gep,
            "mk.entries"
        ))
        .into_pointer_value();

        // Read map length (number of occupied entries)
        let len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, header_ptr, 1, "mk.lenp"));
        let map_len = b!(self.bld.build_load(i64t, len_gep, "mk.len")).into_int_value();

        // Allocate result Vec header
        let result_hdr = self.compile_vec_new(&[])?;
        let result_ptr = result_hdr.into_pointer_value();

        // Pre-allocate buffer for result vec
        let elem_size = i64t.const_int(8, false);
        let buf_bytes = b!(self.bld.build_int_nsw_mul(map_len, elem_size, "mk.bufsz"));
        let malloc = self.ensure_malloc();
        let buf = b!(self
            .bld
            .build_call(malloc, &[buf_bytes.into()], "mk.buf"))
        .try_as_basic_value()
        .basic()
        .unwrap()
        .into_pointer_value();
        let r_ptr_gep = b!(self
            .bld
            .build_struct_gep(header_ty, result_ptr, 0, "mk.rptrp"));
        b!(self.bld.build_store(r_ptr_gep, buf));
        let r_cap_gep = b!(self
            .bld
            .build_struct_gep(header_ty, result_ptr, 2, "mk.rcapp"));
        b!(self.bld.build_store(r_cap_gep, map_len));

        // Loop through all entries
        let entry_size = i64t.const_int(48, false);
        let idx_alloca = self.entry_alloca(i64t.into(), "mk.idx");
        b!(self
            .bld
            .build_store(idx_alloca, i64t.const_int(0, false)));
        let out_idx_alloca = self.entry_alloca(i64t.into(), "mk.oidx");
        b!(self
            .bld
            .build_store(out_idx_alloca, i64t.const_int(0, false)));

        let cond_bb = self.ctx.append_basic_block(fv, "mk.cond");
        let body_bb = self.ctx.append_basic_block(fv, "mk.body");
        let store_bb = self.ctx.append_basic_block(fv, "mk.store");
        let inc_bb = self.ctx.append_basic_block(fv, "mk.inc");
        let done_bb = self.ctx.append_basic_block(fv, "mk.done");

        b!(self.bld.build_unconditional_branch(cond_bb));
        self.bld.position_at_end(cond_bb);
        let cur_idx = b!(self.bld.build_load(i64t, idx_alloca, "mk.i")).into_int_value();
        let cmp = b!(self
            .bld
            .build_int_compare(IntPredicate::SLT, cur_idx, cap, "mk.cmp"));
        b!(self.bld.build_conditional_branch(cmp, body_bb, done_bb));

        self.bld.position_at_end(body_bb);
        let byte_off = b!(self
            .bld
            .build_int_nsw_mul(cur_idx, entry_size, "mk.off"));
        let ep = unsafe { b!(self.bld.build_gep(i8t, entries, &[byte_off], "mk.ep")) };
        let occ_ptr = unsafe {
            b!(self
                .bld
                .build_gep(i8t, ep, &[i64t.const_int(40, false)], "mk.occp"))
        };
        let occ = b!(self.bld.build_load(i8t, occ_ptr, "mk.occ")).into_int_value();
        let is_occ = b!(self.bld.build_int_compare(
            IntPredicate::NE,
            occ,
            i8t.const_int(0, false),
            "mk.isocc"
        ));
        b!(self.bld.build_conditional_branch(is_occ, store_bb, inc_bb));

        self.bld.position_at_end(store_bb);
        let field_ptr = unsafe {
            b!(self.bld.build_gep(
                i8t,
                ep,
                &[i64t.const_int(field_offset, false)],
                "mk.fp"
            ))
        };
        // For keys (offset 8), load 24 bytes (String SSO). For values (offset 32), load i64.
        let field_val = if field_offset == 8 {
            // Key is a String (24 bytes SSO) — copy into result vec as String
            let st = self.string_type();
            b!(self.bld.build_load(st, field_ptr, "mk.key"))
        } else {
            b!(self.bld.build_load(i64t, field_ptr, "mk.val"))
        };

        let out_idx = b!(self
            .bld
            .build_load(i64t, out_idx_alloca, "mk.oi"))
        .into_int_value();
        let out_off = b!(self
            .bld
            .build_int_nsw_mul(out_idx, elem_size, "mk.ooff"));
        let dest = unsafe { b!(self.bld.build_gep(i8t, buf, &[out_off], "mk.dest")) };

        if field_offset == 8 {
            let memcpy = self.ensure_memcpy();
            let tmp = self.entry_alloca(self.string_type().into(), "mk.stmp");
            b!(self.bld.build_store(tmp, field_val));
            b!(self.bld.build_call(
                memcpy,
                &[
                    dest.into(),
                    tmp.into(),
                    i64t.const_int(24, false).into()
                ],
                ""
            ));
        } else {
            b!(self.bld.build_store(dest, field_val));
        }

        let new_oi = b!(self.bld.build_int_nsw_add(
            out_idx,
            i64t.const_int(1, false),
            "mk.noi"
        ));
        b!(self.bld.build_store(out_idx_alloca, new_oi));
        b!(self.bld.build_unconditional_branch(inc_bb));

        self.bld.position_at_end(inc_bb);
        let next = b!(self.bld.build_int_nsw_add(
            cur_idx,
            i64t.const_int(1, false),
            "mk.next"
        ));
        b!(self.bld.build_store(idx_alloca, next));
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        // Set result vec length
        let r_len_gep = b!(self
            .bld
            .build_struct_gep(header_ty, result_ptr, 1, "mk.rlenp"));
        let final_oi = b!(self
            .bld
            .build_load(i64t, out_idx_alloca, "mk.flen"))
        .into_int_value();
        b!(self.bld.build_store(r_len_gep, final_oi));

        Ok(result_ptr.into())
    }

    fn fnv_hash_string(
        &mut self,
        str_val: BasicValueEnum<'ctx>,
    ) -> Result<inkwell::values::IntValue<'ctx>, String> {
        let i64t = self.ctx.i64_type();
        let i8t = self.ctx.i8_type();
        let fv = self.cur_fn.unwrap();

        let data = self.string_data(str_val)?.into_pointer_value();
        let len = self.string_len(str_val)?.into_int_value();

        let basis = i64t.const_int(0xcbf29ce484222325, false);
        let prime = i64t.const_int(0x00000100000001B3, false);

        let cond_bb = self.ctx.append_basic_block(fv, "fnv.cond");
        let body_bb = self.ctx.append_basic_block(fv, "fnv.body");
        let done_bb = self.ctx.append_basic_block(fv, "fnv.done");
        let entry_bb = self.bld.get_insert_block().unwrap();
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(cond_bb);
        let phi_hash = b!(self.bld.build_phi(i64t, "fnv.h"));
        phi_hash.add_incoming(&[(&basis, entry_bb)]);
        let phi_i = b!(self.bld.build_phi(i64t, "fnv.i"));
        phi_i.add_incoming(&[(&i64t.const_int(0, false), entry_bb)]);
        let cur_i = phi_i.as_basic_value().into_int_value();
        let cur_hash = phi_hash.as_basic_value().into_int_value();
        let done = b!(self
            .bld
            .build_int_compare(IntPredicate::SGE, cur_i, len, "fnv.done"));
        b!(self.bld.build_conditional_branch(done, done_bb, body_bb));

        self.bld.position_at_end(body_bb);
        let byte_ptr = unsafe { b!(self.bld.build_gep(i8t, data, &[cur_i], "fnv.bp")) };
        let byte = b!(self.bld.build_load(i8t, byte_ptr, "fnv.byte")).into_int_value();
        let ext = b!(self.bld.build_int_z_extend(byte, i64t, "fnv.ext"));
        let xored = b!(self.bld.build_xor(cur_hash, ext, "fnv.xor"));
        let mulled = b!(self.bld.build_int_nsw_mul(xored, prime, "fnv.mul"));
        let next_i = b!(self
            .bld
            .build_int_nsw_add(cur_i, i64t.const_int(1, false), "fnv.ni"));
        phi_hash.add_incoming(&[(&mulled, body_bb)]);
        phi_i.add_incoming(&[(&next_i, body_bb)]);
        b!(self.bld.build_unconditional_branch(cond_bb));

        self.bld.position_at_end(done_bb);
        Ok(phi_hash.as_basic_value().into_int_value())
    }
}
