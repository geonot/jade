use super::Compiler;
use crate::{hir, mir, types::Type};
use inkwell::types::BasicType;

pub(crate) struct VariantLayout {
    pub offsets: Vec<u64>,
    pub size: u64,
}

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn variant_layout(&self, name: &str, fields: &[Type]) -> VariantLayout {
        let mut size = 0u64;
        let offsets = fields
            .iter()
            .map(|ty| {
                let offset = size;
                let native = if Self::is_recursive_field(ty, name) {
                    self.ctx.ptr_type(inkwell::AddressSpace::default()).into()
                } else {
                    self.llvm_ty(ty)
                };
                size = size
                    .checked_add(self.type_store_size(native).next_multiple_of(8))
                    .expect("native variant layout exceeds address space");
                offset
            })
            .collect();
        VariantLayout { offsets, size }
    }

    fn layout_ready(&self, ty: &Type) -> bool {
        match ty {
            Type::Struct(name, args) => {
                args.is_empty()
                    && self
                        .module
                        .get_struct_type(&name.as_str())
                        .is_some_and(|s| s.is_sized())
            }
            Type::Enum(name) => self
                .module
                .get_struct_type(&name.as_str())
                .is_some_and(|s| s.is_sized()),
            Type::Row(name) => self
                .module
                .get_struct_type(&format!("__store_{name}"))
                .is_some_and(|s| s.is_sized()),
            Type::Array(t, n) => u32::try_from(*n).is_ok() && self.layout_ready(t),
            Type::Alias(_, t) | Type::Newtype(_, t) | Type::Frozen(t) => self.layout_ready(t),
            Type::Tuple(ts) => ts.iter().all(|t| self.layout_ready(t)),
            Type::Param(_) | Type::TypeVar(_) => false,
            _ => true,
        }
    }

    fn validate_native_type(&self, ty: &Type) -> Result<(), String> {
        if !self.layout_ready(ty) {
            return Err(format!("unsupported or unresolved native layout: {ty}"));
        }
        match ty {
            Type::Array(t, _)
            | Type::Alias(_, t)
            | Type::Newtype(_, t)
            | Type::Frozen(t)
            | Type::Vec(t)
            | Type::Ptr(t)
            | Type::Coroutine(t)
            | Type::Generator(t)
            | Type::Channel(t)
            | Type::View(t) => self.validate_native_type(t),
            Type::Tuple(ts) => ts.iter().try_for_each(|t| self.validate_native_type(t)),
            Type::Map(k, v) => {
                self.validate_native_type(k)?;
                self.validate_native_type(v)
            }
            Type::Fn(ps, r) => {
                ps.iter().try_for_each(|t| self.validate_native_type(t))?;
                self.validate_native_type(r)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn validate_native_program(&self, prog: &mir::Program) -> Result<(), String> {
        for td in &prog.types {
            for (_, ty) in &td.fields {
                self.validate_native_type(ty)?;
            }
        }
        for f in &prog.functions {
            self.validate_native_type(&f.ret_ty)?;
            for p in &f.params {
                self.validate_native_type(&p.ty)?;
            }
            for bb in &f.blocks {
                for phi in &bb.phis {
                    self.validate_native_type(&phi.ty)?;
                }
                for inst in &bb.insts {
                    self.validate_native_type(&inst.ty)?;
                    if let mir::InstKind::SizeOf(t) = &inst.kind {
                        self.validate_native_type(t)?;
                    }
                }
            }
        }
        for ext in &prog.externs {
            self.validate_native_type(&ext.ret)?;
            for ty in &ext.params {
                self.validate_native_type(ty)?;
            }
        }
        for global in &prog.globals {
            self.validate_native_type(&global.ty)?;
        }
        Ok(())
    }

    pub(crate) fn declare_native_types(
        &mut self,
        prog: &mir::Program,
        hir: &hir::Program,
    ) -> Result<(), String> {
        for name in prog
            .types
            .iter()
            .map(|t| t.name)
            .chain(hir.enums.iter().map(|e| e.name))
            .chain(hir.err_defs.iter().map(|e| e.name))
        {
            if self.module.get_struct_type(&name.as_str()).is_none() {
                self.ctx.opaque_struct_type(&name.as_str());
            }
        }
        let mut structs: Vec<_> = prog.types.iter().collect();
        let mut enums: Vec<_> = hir.enums.iter().collect();
        let mut errors: Vec<_> = hir.err_defs.iter().collect();
        while !structs.is_empty() || !enums.is_empty() || !errors.is_empty() {
            let before = structs.len() + enums.len() + errors.len();
            let mut next = Vec::new();
            for td in structs {
                if !td.fields.iter().all(|(_, t)| self.layout_ready(t)) {
                    next.push(td);
                    continue;
                }
                let fields: Vec<_> = td.fields.iter().map(|(_, t)| self.llvm_ty(t)).collect();
                self.module
                    .get_struct_type(&td.name.as_str())
                    .unwrap()
                    .set_body(&fields, false);
                self.structs.insert(
                    td.name,
                    td.fields
                        .iter()
                        .map(|(n, t)| (n.as_str(), t.clone()))
                        .collect(),
                );
            }
            structs = next;
            let mut next = Vec::new();
            for ed in enums {
                if !ed.variants.iter().flat_map(|v| &v.fields).all(|f| {
                    Self::is_recursive_field(&f.ty, &ed.name.as_str()) || self.layout_ready(&f.ty)
                }) {
                    next.push(ed);
                    continue;
                }
                self.declare_enum(ed)?;
            }
            enums = next;
            let mut next = Vec::new();
            for ed in errors {
                if !ed
                    .variants
                    .iter()
                    .flat_map(|v| &v.fields)
                    .all(|t| Self::is_recursive_field(t, &ed.name.as_str()) || self.layout_ready(t))
                {
                    next.push(ed);
                    continue;
                }
                self.declare_err_def(ed)?;
            }
            errors = next;
            if structs.len() + enums.len() + errors.len() == before {
                let names: Vec<_> = structs
                    .iter()
                    .map(|t| t.name)
                    .chain(enums.iter().map(|e| e.name))
                    .chain(errors.iter().map(|e| e.name))
                    .collect();
                return Err(format!(
                    "unsupported recursive or unresolved native layout: {}",
                    crate::intern::Symbol::join_vec(&names, ", ")
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::Lexer, parser::Parser, typer::Typer};
    use inkwell::{context::Context, targets::TargetData};

    fn compiler<'ctx>(ctx: &'ctx Context, bits: u32) -> Compiler<'ctx> {
        let mut c = Compiler::new(ctx, "layout");
        let data = match bits {
            32 => "e-p:32:32-i64:32-f64:32-n8:16:32-S128",
            64 => "e-p:64:64-i64:64-f64:64-n8:16:32:64-S128",
            _ => unreachable!(),
        };
        c.target_data = Some(TargetData::create(data));
        c.module.set_data_layout(&c.layout().get_data_layout());
        c
    }

    fn programs(source: &str) -> (mir::Program, hir::Program) {
        let tokens = Lexer::new(source).tokenize().unwrap();
        let ast = Parser::new(tokens).parse_program().unwrap();
        let hir = Typer::new().lower_program(&ast).unwrap();
        (mir::lower::lower_program(&hir), hir)
    }

    #[test]
    fn target_data_controls_aggregate_padding_and_stride() {
        for (bits, closure, tuple, view) in [(32, 8, 16, 12), (64, 16, 24, 16)] {
            let ctx = Context::create();
            let c = compiler(&ctx, bits);
            let tuple_ty = Type::Tuple(vec![Type::I8, Type::I64, Type::I8]);
            assert_eq!(
                c.type_size_of(&Type::Fn(vec![], Box::new(Type::Void))),
                closure
            );
            assert_eq!(c.type_size_of(&Type::Array(Box::new(Type::U8), 3)), 3);
            assert_eq!(c.type_size_of(&tuple_ty), tuple);
            assert_eq!(
                c.type_size_of(&Type::Array(Box::new(tuple_ty.clone()), 3)),
                tuple * 3
            );
            assert_eq!(c.type_size_of(&Type::View(Box::new(Type::I8))), view);
            let native = c.llvm_ty(&tuple_ty).into_struct_type();
            assert_eq!(
                c.layout().offset_of_element(&native, 1),
                Some(u64::from(bits / 8))
            );
            assert_eq!(c.type_abi_align(native.into()), u64::from(bits / 8));
            let packed = ctx.struct_type(&[ctx.i8_type().into(), ctx.i64_type().into()], true);
            assert_eq!(c.type_store_size(packed.into()), 9);
            assert_eq!(c.type_abi_align(packed.into()), 1);
        }
    }

    #[test]
    fn variant_slots_use_abi_sizes_including_multiword_closures() {
        for (bits, offsets, size) in [
            (32, vec![0, 8, 24, 32, 40], 48),
            (64, vec![0, 16, 40, 48, 56], 64),
        ] {
            let ctx = Context::create();
            let c = compiler(&ctx, bits);
            let fields = vec![
                Type::Fn(vec![], Box::new(Type::Void)),
                Type::Tuple(vec![Type::I8, Type::I64, Type::I8]),
                Type::Array(Box::new(Type::I8), 3),
                Type::Enum("Chain".into()),
                Type::I64,
            ];
            let layout = c.variant_layout("Chain", &fields);
            assert_eq!(layout.offsets, offsets);
            assert_eq!(layout.size, size);
        }
    }

    #[test]
    fn native_declarations_resolve_forward_references_and_rows() {
        let (mut mir, hir) =
            programs("type Outer\n    inner as Inner\n\ntype Inner\n    value as i64\n");
        mir.types[1]
            .fields
            .push(("row".into(), Type::Row("Items".into())));
        let ctx = Context::create();
        let mut c = compiler(&ctx, 64);
        let row = ctx.opaque_struct_type("__store_Items");
        row.set_body(&[ctx.i64_type().into()], false);
        c.declare_native_types(&mir, &hir).unwrap();
        c.validate_native_program(&mir).unwrap();
        assert_eq!(c.type_size_of(&Type::Struct("Outer".into(), vec![])), 16);
    }

    #[test]
    fn unsupported_nominal_cycles_and_array_lengths_fail_without_panicking() {
        let (_, hir) = programs("");
        for ty in [
            Type::Struct("Broken".into(), vec![]),
            Type::Enum("Missing".into()),
            Type::Row("Missing".into()),
            Type::Param("T".into()),
            Type::TypeVar(0),
            Type::Array(Box::new(Type::I8), u32::MAX as usize + 1),
        ] {
            let ctx = Context::create();
            let mut c = compiler(&ctx, 64);
            let (mut mir, _) = programs("");
            mir.types.push(mir::TypeDef {
                name: "Broken".into(),
                fields: vec![("field".into(), ty)],
            });
            assert!(
                c.declare_native_types(&mir, &hir)
                    .unwrap_err()
                    .contains("native layout")
            );
        }
        let ctx = Context::create();
        let mut c = compiler(&ctx, 64);
        let (mut mir, _) = programs("");
        mir.types = vec![
            mir::TypeDef {
                name: "Left".into(),
                fields: vec![("right".into(), Type::Struct("Right".into(), vec![]))],
            },
            mir::TypeDef {
                name: "Right".into(),
                fields: vec![("left".into(), Type::Struct("Left".into(), vec![]))],
            },
        ];
        assert!(
            c.declare_native_types(&mir, &hir)
                .unwrap_err()
                .contains("Left, Right")
        );
    }

    #[test]
    fn preflight_rejects_unresolved_function_and_global_types() {
        for ty in [
            Type::Struct("Missing".into(), vec![]),
            Type::Array(Box::new(Type::I8), u32::MAX as usize + 1),
            Type::Fn(vec![Type::Param("T".into())], Box::new(Type::Void)),
            Type::Vec(Box::new(Type::Enum("Missing".into()))),
        ] {
            let ctx = Context::create();
            let c = compiler(&ctx, 64);
            let (mut mir, _) = programs("*main\n    log(1)\n");
            mir.functions[0].ret_ty = ty.clone();
            assert!(
                c.validate_native_program(&mir)
                    .unwrap_err()
                    .contains("native layout")
            );
            let (mut mir, _) = programs("");
            mir.globals.push(mir::GlobalDef {
                name: "global".into(),
                ty,
            });
            assert!(
                c.validate_native_program(&mir)
                    .unwrap_err()
                    .contains("native layout")
            );
        }
    }

    #[test]
    fn recursive_nominal_clone_eligibility_matches_backend() {
        let (mir, hir) = programs(
            "enum Chain\n    End\n    Link(String, Chain)\n\nenum Action\n    Apply((i64) returns i64)\n\ntype Wrapper\n    action as Action\n",
        );
        let ctx = Context::create();
        let mut c = compiler(&ctx, 64);
        c.declare_native_types(&mir, &hir).unwrap();
        assert!(c.is_value_clonable(&Type::Enum("Chain".into())));
        assert!(!c.is_value_clonable(&Type::Enum("Action".into())));
        assert!(!c.is_value_clonable(&Type::Struct("Wrapper".into(), vec![])));
        assert!(!c.is_value_clonable(&Type::Enum("Missing".into())));
    }
}
