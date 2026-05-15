//! Substitution helpers (renaming ValueIds in instructions/terminators).

use super::super::*;
use std::collections::HashMap;

pub(super) fn subst_inst(inst: &mut Instruction, map: &HashMap<ValueId, ValueId>) -> bool {
    let mut hit = false;
    macro_rules! sub {
        ($v:expr) => {
            if let Some(&r) = map.get($v) {
                *$v = r;
                hit = true;
            }
        };
    }
    match &mut inst.kind {
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => {
            sub!(a);
            sub!(b);
        }
        InstKind::UnaryOp(_, v)
        | InstKind::Cast(v, _)
        | InstKind::StrictCast(v, _)
        | InstKind::Ref(v)
        | InstKind::Deref(v)
        | InstKind::Copy(v)
        | InstKind::RcInc(v)
        | InstKind::RcDec(v)
        | InstKind::Alloc(v) => {
            sub!(v);
        }
        InstKind::Drop(v, _) => {
            sub!(v);
        }
        InstKind::DropMany(items) => {
            for (v, _) in items {
                sub!(v);
            }
        }
        InstKind::Call(_, args)
        | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => {
            for a in args {
                sub!(a);
            }
        }
        InstKind::MethodCall(obj, _, args) => {
            sub!(obj);
            for a in args {
                sub!(a);
            }
        }
        InstKind::IndirectCall(f, args) => {
            sub!(f);
            for a in args {
                sub!(a);
            }
        }
        InstKind::FieldGet(o, _) => {
            sub!(o);
        }
        InstKind::FieldSet(o, _, v) => {
            sub!(o);
            sub!(v);
        }
        InstKind::FieldStore(_, _, v) => {
            sub!(v);
        }
        InstKind::Index(a, i) | InstKind::IndexUnchecked(a, i) => {
            sub!(a);
            sub!(i);
        }
        InstKind::IndexSet(a, i, v) => {
            sub!(a);
            sub!(i);
            sub!(v);
        }
        InstKind::IndexStore(_, i, v) => {
            sub!(i);
            sub!(v);
        }
        InstKind::StructInit(_, fs) => {
            for (_, v) in fs {
                sub!(v);
            }
        }
        InstKind::Slice(a, s, e) => {
            sub!(a);
            sub!(s);
            sub!(e);
        }
        InstKind::Store(_, v) => {
            sub!(v);
        }
        InstKind::GlobalStore(_, v) => {
            sub!(v);
        }
        // Collections
        InstKind::VecNew(args) => {
            for a in args {
                sub!(a);
            }
        }
        InstKind::VecPush(vec, val) | InstKind::ChanSend(vec, val) => {
            sub!(vec);
            sub!(val);
        }
        InstKind::VecLen(v)
        | InstKind::ChanRecv(v)
        | InstKind::RcClone(v)
        | InstKind::WeakUpgrade(v)
        | InstKind::Log(v) => {
            sub!(v);
        }
        InstKind::RcNew(v, _) => {
            sub!(v);
        }
        // Closures
        InstKind::ClosureCreate(_, captures) | InstKind::SelectArm(captures, _) => {
            for a in captures {
                sub!(a);
            }
        }
        InstKind::SpawnActor(_, inits) => {
            for (_, a) in inits {
                sub!(a);
            }
        }
        InstKind::ClosureCall(f, args) => {
            sub!(f);
            for a in args {
                sub!(a);
            }
        }
        InstKind::ChanCreate(_, cap) => {
            if let Some(c) = cap {
                sub!(c);
            }
        }
        InstKind::MapInit | InstKind::SetInit | InstKind::PQInit | InstKind::DequeInit => {}
        InstKind::Assert(v, _) => {
            sub!(v);
        }
        InstKind::DynDispatch(obj, _, _, args) => {
            sub!(obj);
            for a in args {
                sub!(a);
            }
        }
        InstKind::DynCoerce(v, _, _) => {
            sub!(v);
        }
        InstKind::InlineAsm(_, args) => {
            for a in args {
                sub!(a);
            }
        }
        InstKind::IntConst(_)
        | InstKind::FloatConst(_)
        | InstKind::BoolConst(_)
        | InstKind::StringConst(_)
        | InstKind::Void
        | InstKind::Load(_)
        | InstKind::GlobalLoad(_)
        | InstKind::FnRef(_) => {}
    }
    hit
}

/// Apply a value→value substitution map to a terminator.
pub(super) fn subst_term(term: &mut Terminator, map: &HashMap<ValueId, ValueId>) -> bool {
    match term {
        Terminator::Branch(c, _, _) => {
            if let Some(&v) = map.get(c) {
                *c = v;
                return true;
            }
        }
        Terminator::Return(Some(v)) => {
            if let Some(&r) = map.get(v) {
                *v = r;
                return true;
            }
        }
        Terminator::Switch(v, _, _) => {
            if let Some(&r) = map.get(v) {
                *v = r;
                return true;
            }
        }
        _ => {}
    }
    false
}
