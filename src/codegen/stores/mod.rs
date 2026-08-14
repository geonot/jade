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

pub(crate) const HEADER_SIZE: u64 = 40;

mod handles;
mod indexing;
mod runtime;

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

fn fnv1a(acc: u64, bytes: &[u8]) -> u64 {
    let mut h = acc;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

fn type_fingerprint_tag(ty: &Type) -> String {
    format!("{ty:?}")
}

pub(crate) fn store_schema_fingerprint(sd: &hir::StoreDef) -> i64 {
    let mut h = FNV_OFFSET;
    let mut decs: Vec<String> = sd
        .decorators
        .iter()
        .filter(|d| {
            !matches!(
                d,
                crate::ast::StoreDecorator::Durable
                    | crate::ast::StoreDecorator::Relaxed
                    | crate::ast::StoreDecorator::Volatile
            )
        })
        .map(|d| format!("{d:?}"))
        .collect();
    decs.sort();
    for d in &decs {
        h = fnv1a(h, d.as_bytes());
        h = fnv1a(h, &[0]);
    }
    for f in &sd.fields {
        h = fnv1a(h, f.name.as_str().as_bytes());
        h = fnv1a(h, &[0x1f]);
        h = fnv1a(h, type_fingerprint_tag(&f.ty).as_bytes());
        h = fnv1a(h, &[0x1f]);
        let mut fdecs: Vec<String> = f.decorators.iter().map(|d| format!("{d:?}")).collect();
        fdecs.sort();
        for d in &fdecs {
            h = fnv1a(h, d.as_bytes());
            h = fnv1a(h, &[0x1e]);
        }
        h = fnv1a(h, if f.is_relation { b"R" } else { b"_" });
        h = fnv1a(h, if f.is_has_many { b"M" } else { b"_" });
        h = fnv1a(h, &[0]);
    }
    h as i64
}
