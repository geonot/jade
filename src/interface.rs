//! `.jadei` — Jade Interface File Format
//!
//! After type inference resolves all types, function signatures, type/enum/trait
//! definitions can be serialized to `.jadei` files. When a module is imported
//! via `use`, the compiler can load the `.jadei` file instead of re-parsing and
//! re-typing the source, enabling separate compilation.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Current interface file format version.
const INTERFACE_VERSION: u32 = 1;

// ────────────────────────────────────────────────────────────────────────
// Serializable type representation (mirrors crate::types::Type)
// ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Void,
    String,
    Array(Box<IType>, usize),
    Vec(Box<IType>),
    Map(Box<IType>, Box<IType>),
    Tuple(Vec<IType>),
    Struct(std::string::String, Vec<IType>),
    Enum(std::string::String),
    Fn(Vec<IType>, Box<IType>),
    Param(std::string::String),
    Ptr(Box<IType>),
    Rc(Box<IType>),
    Weak(Box<IType>),
    ActorRef(std::string::String),
    Coroutine(Box<IType>),
    Channel(Box<IType>),
    DynTrait(std::string::String),
}

// ────────────────────────────────────────────────────────────────────────
// Interface file data structures
// ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceFile {
    pub version: u32,
    pub module: String,
    pub functions: Vec<FnSig>,
    pub types: Vec<TypeSig>,
    pub enums: Vec<EnumSig>,
    pub traits: Vec<TraitSig>,
    pub impls: Vec<ImplSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnSig {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<ParamSig>,
    pub ret: IType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamSig {
    pub name: String,
    pub ty: IType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeSig {
    pub name: String,
    pub type_params: Vec<String>,
    pub fields: Vec<FieldSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSig {
    pub name: String,
    pub ty: IType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumSig {
    pub name: String,
    pub type_params: Vec<String>,
    pub variants: Vec<VariantSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantSig {
    pub name: String,
    pub fields: Vec<IType>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitSig {
    pub name: String,
    pub type_params: Vec<String>,
    pub methods: Vec<FnSig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplSig {
    pub trait_name: String,
    pub type_name: String,
}

// ────────────────────────────────────────────────────────────────────────
// Conversion: Type ↔ IType
// ────────────────────────────────────────────────────────────────────────

impl From<&crate::types::Type> for IType {
    fn from(ty: &crate::types::Type) -> Self {
        use crate::types::Type;
        match ty {
            Type::I8 => IType::I8,
            Type::I16 => IType::I16,
            Type::I32 => IType::I32,
            Type::I64 => IType::I64,
            Type::U8 => IType::U8,
            Type::U16 => IType::U16,
            Type::U32 => IType::U32,
            Type::U64 => IType::U64,
            Type::F32 => IType::F32,
            Type::F64 => IType::F64,
            Type::Bool => IType::Bool,
            Type::Void => IType::Void,
            Type::String => IType::String,
            Type::Array(inner, len) => IType::Array(Box::new(inner.as_ref().into()), *len),
            Type::Vec(inner) => IType::Vec(Box::new(inner.as_ref().into())),
            Type::Map(k, v) => IType::Map(Box::new(k.as_ref().into()), Box::new(v.as_ref().into())),
            Type::Tuple(tys) => IType::Tuple(tys.iter().map(|t| t.into()).collect()),
            Type::Struct(n, params) => {
                IType::Struct(n.clone(), params.iter().map(|t| t.into()).collect())
            }
            Type::Enum(n) => IType::Enum(n.clone()),
            Type::Fn(params, ret) => {
                IType::Fn(params.iter().map(|t| t.into()).collect(), Box::new(ret.as_ref().into()))
            }
            Type::Param(n) => IType::Param(n.clone()),
            Type::Ptr(inner) => IType::Ptr(Box::new(inner.as_ref().into())),
            Type::Rc(inner) => IType::Rc(Box::new(inner.as_ref().into())),
            Type::Weak(inner) => IType::Weak(Box::new(inner.as_ref().into())),
            Type::ActorRef(n) => IType::ActorRef(n.clone()),
            Type::Coroutine(inner) => IType::Coroutine(Box::new(inner.as_ref().into())),
            Type::Channel(inner) => IType::Channel(Box::new(inner.as_ref().into())),
            Type::DynTrait(n) => IType::DynTrait(n.clone()),
            Type::TypeVar(_) => IType::I64, // unsolved vars default to i64
        }
    }
}

impl From<&IType> for crate::types::Type {
    fn from(ity: &IType) -> Self {
        use crate::types::Type;
        match ity {
            IType::I8 => Type::I8,
            IType::I16 => Type::I16,
            IType::I32 => Type::I32,
            IType::I64 => Type::I64,
            IType::U8 => Type::U8,
            IType::U16 => Type::U16,
            IType::U32 => Type::U32,
            IType::U64 => Type::U64,
            IType::F32 => Type::F32,
            IType::F64 => Type::F64,
            IType::Bool => Type::Bool,
            IType::Void => Type::Void,
            IType::String => Type::String,
            IType::Array(inner, len) => Type::Array(Box::new(inner.as_ref().into()), *len),
            IType::Vec(inner) => Type::Vec(Box::new(inner.as_ref().into())),
            IType::Map(k, v) => Type::Map(Box::new(k.as_ref().into()), Box::new(v.as_ref().into())),
            IType::Tuple(tys) => Type::Tuple(tys.iter().map(|t| t.into()).collect()),
            IType::Struct(n, params) => {
                Type::Struct(n.clone(), params.iter().map(|t| t.into()).collect())
            }
            IType::Enum(n) => Type::Enum(n.clone()),
            IType::Fn(params, ret) => {
                Type::Fn(params.iter().map(|t| t.into()).collect(), Box::new(ret.as_ref().into()))
            }
            IType::Param(n) => Type::Param(n.clone()),
            IType::Ptr(inner) => Type::Ptr(Box::new(inner.as_ref().into())),
            IType::Rc(inner) => Type::Rc(Box::new(inner.as_ref().into())),
            IType::Weak(inner) => Type::Weak(Box::new(inner.as_ref().into())),
            IType::ActorRef(n) => Type::ActorRef(n.clone()),
            IType::Coroutine(inner) => Type::Coroutine(Box::new(inner.as_ref().into())),
            IType::Channel(inner) => Type::Channel(Box::new(inner.as_ref().into())),
            IType::DynTrait(n) => Type::DynTrait(n.clone()),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────
// Building an InterfaceFile from AST declarations
// ────────────────────────────────────────────────────────────────────────

impl InterfaceFile {
    /// Create a new empty interface file for the given module name.
    pub fn new(module: &str) -> Self {
        InterfaceFile {
            version: INTERFACE_VERSION,
            module: module.to_string(),
            functions: Vec::new(),
            types: Vec::new(),
            enums: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
        }
    }

    /// Build an interface file from AST declarations.
    /// Types should already be resolved (via inference) before calling this.
    pub fn from_decls(module: &str, decls: &[crate::ast::Decl]) -> Self {
        let mut iface = Self::new(module);
        for decl in decls {
            match decl {
                crate::ast::Decl::Fn(f) => {
                    iface.functions.push(FnSig {
                        name: f.name.clone(),
                        type_params: f.type_params.clone(),
                        params: f
                            .params
                            .iter()
                            .map(|p| ParamSig {
                                name: p.name.clone(),
                                ty: p.ty.as_ref().map(|t| t.into()).unwrap_or(IType::I64),
                            })
                            .collect(),
                        ret: f.ret.as_ref().map(|t| t.into()).unwrap_or(IType::Void),
                    });
                }
                crate::ast::Decl::Type(td) => {
                    iface.types.push(TypeSig {
                        name: td.name.clone(),
                        type_params: td.type_params.clone(),
                        fields: td
                            .fields
                            .iter()
                            .map(|field| FieldSig {
                                name: field.name.clone(),
                                ty: field.ty.as_ref().map(|t| t.into()).unwrap_or(IType::I64),
                            })
                            .collect(),
                    });
                }
                crate::ast::Decl::Enum(ed) => {
                    iface.enums.push(EnumSig {
                        name: ed.name.clone(),
                        type_params: ed.type_params.clone(),
                        variants: ed
                            .variants
                            .iter()
                            .map(|v| VariantSig {
                                name: v.name.clone(),
                                fields: v.fields.iter().map(|vf| (&vf.ty).into()).collect(),
                            })
                            .collect(),
                    });
                }
                crate::ast::Decl::Trait(td) => {
                    iface.traits.push(TraitSig {
                        name: td.name.clone(),
                        type_params: td.type_params.clone(),
                        methods: td
                            .methods
                            .iter()
                            .map(|m| FnSig {
                                name: m.name.clone(),
                                type_params: Vec::new(),
                                params: m
                                    .params
                                    .iter()
                                    .map(|p| ParamSig {
                                        name: p.name.clone(),
                                        ty: p.ty.as_ref().map(|t| t.into()).unwrap_or(IType::I64),
                                    })
                                    .collect(),
                                ret: m.ret.as_ref().map(|t| t.into()).unwrap_or(IType::Void),
                            })
                            .collect(),
                    });
                }
                crate::ast::Decl::Impl(ib) => {
                    if let Some(trait_name) = &ib.trait_name {
                        iface.impls.push(ImplSig {
                            trait_name: trait_name.clone(),
                            type_name: ib.type_name.clone(),
                        });
                    }
                }
                _ => {} // Use, Extern, Test, Actor, Store, Const, ErrDef
            }
        }
        iface
    }

    // ────────────────────────────────────────────────────────────────────
    // Serialization / Deserialization
    // ────────────────────────────────────────────────────────────────────

    /// Serialize to JSON and write to the given path.
    pub fn write_to(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize interface: {e}"))?;
        std::fs::write(path, json)
            .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
        Ok(())
    }

    /// Read and deserialize from the given path.
    pub fn read_from(path: &Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let iface: InterfaceFile = serde_json::from_str(&json)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        if iface.version != INTERFACE_VERSION {
            return Err(format!(
                "{}: interface version {} (expected {})",
                path.display(),
                iface.version,
                INTERFACE_VERSION
            ));
        }
        Ok(iface)
    }

    /// Convert this interface back to AST declarations for merging into a program.
    pub fn to_decls(&self) -> Vec<crate::ast::Decl> {
        use crate::ast::*;
        let dummy = Span::dummy();
        let mut decls = Vec::new();

        for ts in &self.types {
            decls.push(Decl::Type(TypeDef {
                name: ts.name.clone(),
                type_params: ts.type_params.clone(),
                fields: ts
                    .fields
                    .iter()
                    .map(|f| Field {
                        name: f.name.clone(),
                        ty: Some((&f.ty).into()),
                        default: None,
                        span: dummy,
                    })
                    .collect(),
                methods: Vec::new(),
                layout: LayoutAttrs::default(),
                span: dummy,
            }));
        }

        for es in &self.enums {
            decls.push(Decl::Enum(EnumDef {
                name: es.name.clone(),
                type_params: es.type_params.clone(),
                variants: es
                    .variants
                    .iter()
                    .map(|v| Variant {
                        name: v.name.clone(),
                        fields: v
                            .fields
                            .iter()
                            .map(|ft| VField {
                                name: None,
                                ty: ft.into(),
                            })
                            .collect(),
                        span: dummy,
                    })
                    .collect(),
                span: dummy,
            }));
        }

        for ts in &self.traits {
            decls.push(Decl::Trait(TraitDef {
                name: ts.name.clone(),
                type_params: ts.type_params.clone(),
                assoc_types: Vec::new(),
                methods: ts
                    .methods
                    .iter()
                    .map(|m| TraitMethod {
                        name: m.name.clone(),
                        params: m
                            .params
                            .iter()
                            .map(|p| Param {
                                name: p.name.clone(),
                                ty: Some((&p.ty).into()),
                                default: None,
                                literal: None,
                                span: dummy,
                            })
                            .collect(),
                        ret: Some((&m.ret).into()),
                        default_body: None,
                        span: dummy,
                    })
                    .collect(),
                span: dummy,
            }));
        }

        // Note: Function bodies are not included in interface files.
        // Only extern-style declarations could be synthesized, but for now
        // we rely on the fact that imported functions are already compiled
        // in a separate object file and linked at the LLVM level.

        decls
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_empty() {
        let iface = InterfaceFile::new("test_mod");
        let json = serde_json::to_string(&iface).unwrap();
        let back: InterfaceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.version, INTERFACE_VERSION);
        assert_eq!(back.module, "test_mod");
    }

    #[test]
    fn roundtrip_function() {
        let mut iface = InterfaceFile::new("math");
        iface.functions.push(FnSig {
            name: "add".into(),
            type_params: vec![],
            params: vec![
                ParamSig { name: "a".into(), ty: IType::I64 },
                ParamSig { name: "b".into(), ty: IType::I64 },
            ],
            ret: IType::I64,
        });
        let json = serde_json::to_string_pretty(&iface).unwrap();
        let back: InterfaceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.functions.len(), 1);
        assert_eq!(back.functions[0].name, "add");
        assert_eq!(back.functions[0].params.len(), 2);
        assert_eq!(back.functions[0].ret, IType::I64);
    }

    #[test]
    fn roundtrip_struct_type() {
        let mut iface = InterfaceFile::new("geom");
        iface.types.push(TypeSig {
            name: "Point".into(),
            type_params: vec![],
            fields: vec![
                FieldSig { name: "x".into(), ty: IType::F64 },
                FieldSig { name: "y".into(), ty: IType::F64 },
            ],
        });
        let json = serde_json::to_string(&iface).unwrap();
        let back: InterfaceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.types.len(), 1);
        assert_eq!(back.types[0].fields.len(), 2);
    }

    #[test]
    fn roundtrip_enum() {
        let mut iface = InterfaceFile::new("colors");
        iface.enums.push(EnumSig {
            name: "Color".into(),
            type_params: vec![],
            variants: vec![
                VariantSig { name: "Red".into(), fields: vec![] },
                VariantSig { name: "Rgb".into(), fields: vec![IType::U8, IType::U8, IType::U8] },
            ],
        });
        let json = serde_json::to_string(&iface).unwrap();
        let back: InterfaceFile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.enums.len(), 1);
        assert_eq!(back.enums[0].variants.len(), 2);
        assert_eq!(back.enums[0].variants[1].fields.len(), 3);
    }

    #[test]
    fn type_conversion_roundtrip() {
        use crate::types::Type;
        let cases = vec![
            Type::I64,
            Type::String,
            Type::Vec(Box::new(Type::I32)),
            Type::Fn(vec![Type::I64, Type::String], Box::new(Type::Bool)),
            Type::Struct("Point".into(), vec![Type::F64, Type::F64]),
            Type::Map(Box::new(Type::String), Box::new(Type::I64)),
            Type::Tuple(vec![Type::I8, Type::Bool]),
        ];
        for ty in &cases {
            let ity: IType = ty.into();
            let back: Type = (&ity).into();
            assert_eq!(&back, ty, "roundtrip failed for {ty}");
        }
    }

    #[test]
    fn file_roundtrip() {
        let mut iface = InterfaceFile::new("test");
        iface.functions.push(FnSig {
            name: "greet".into(),
            type_params: vec![],
            params: vec![ParamSig { name: "name".into(), ty: IType::String }],
            ret: IType::String,
        });
        iface.types.push(TypeSig {
            name: "Pair".into(),
            type_params: vec!["T".into()],
            fields: vec![
                FieldSig { name: "first".into(), ty: IType::Param("T".into()) },
                FieldSig { name: "second".into(), ty: IType::Param("T".into()) },
            ],
        });

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.jadei");
        iface.write_to(&path).unwrap();
        let loaded = InterfaceFile::read_from(&path).unwrap();
        assert_eq!(loaded.module, "test");
        assert_eq!(loaded.functions.len(), 1);
        assert_eq!(loaded.types.len(), 1);
    }
}
