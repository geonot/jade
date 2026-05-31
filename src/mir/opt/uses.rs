use super::super::*;
use std::collections::HashSet;

pub(super) fn collect_used(func: &Function) -> HashSet<ValueId> {
    let mut used = HashSet::new();
    for bb in &func.blocks {
        for phi in &bb.phis {
            for (_, v) in &phi.incoming {
                used.insert(*v);
            }
        }
        for inst in &bb.insts {
            collect_inst_uses(&inst.kind, &mut used);
        }
        collect_term_uses(&bb.terminator, &mut used);
    }
    used
}

fn collect_inst_uses(kind: &InstKind, s: &mut HashSet<ValueId>) {
    match kind {
        InstKind::BinOp(_, a, b) | InstKind::Cmp(_, a, b, _) => {
            s.insert(*a);
            s.insert(*b);
        }
        InstKind::UnaryOp(_, v)
        | InstKind::Cast(v, _, _)
        | InstKind::StrictCast(v, _, _)
        | InstKind::Ref(v)
        | InstKind::Deref(v)
        | InstKind::Copy(v)
        | InstKind::Alloc(v) => {
            s.insert(*v);
        }
        InstKind::Drop(v, _) | InstKind::Clone(v, _) => {
            s.insert(*v);
        }
        InstKind::DropMany(items) => {
            for (v, _) in items {
                s.insert(*v);
            }
        }
        InstKind::Call(_, args)
        | InstKind::RuntimeOp(_, args)
        | InstKind::ArrayInit(args)
        | InstKind::VariantInit(_, _, _, args) => {
            for a in args {
                s.insert(*a);
            }
        }
        InstKind::MethodCall(obj, _, args, _) => {
            s.insert(*obj);
            for a in args {
                s.insert(*a);
            }
        }
        InstKind::IndirectCall(f, args) => {
            s.insert(*f);
            for a in args {
                s.insert(*a);
            }
        }
        InstKind::FieldGet(o, _) => {
            s.insert(*o);
        }
        InstKind::FieldSet(o, _, v) => {
            s.insert(*o);
            s.insert(*v);
        }
        InstKind::FieldStore(_, _, v) => {
            s.insert(*v);
        }
        InstKind::FieldClear(o, _) => {
            s.insert(*o);
        }
        InstKind::Index(a, i) | InstKind::IndexUnchecked(a, i) => {
            s.insert(*a);
            s.insert(*i);
        }
        InstKind::IndexSet(a, i, v) => {
            s.insert(*a);
            s.insert(*i);
            s.insert(*v);
        }
        InstKind::IndexStore(_, i, v) => {
            s.insert(*i);
            s.insert(*v);
        }
        InstKind::StructInit(_, fs) => {
            for (_, v) in fs {
                s.insert(*v);
            }
        }
        InstKind::Slice(a, lo, hi) => {
            s.insert(*a);
            s.insert(*lo);
            s.insert(*hi);
        }
        InstKind::Store(_, v) => {
            s.insert(*v);
        }
        InstKind::GlobalStore(_, v) => {
            s.insert(*v);
        }

        InstKind::VecNew(args) => {
            for a in args {
                s.insert(*a);
            }
        }
        InstKind::VecPush(vec, val) | InstKind::ChanSend(vec, val) => {
            s.insert(*vec);
            s.insert(*val);
        }
        InstKind::VecLen(v) | InstKind::ChanRecv(v) | InstKind::Log(v) | InstKind::Eprint(v) => {
            s.insert(*v);
        }
        InstKind::ClosureCreate(_, captures) | InstKind::SelectArm(captures, _) => {
            for a in captures {
                s.insert(*a);
            }
        }
        InstKind::SpawnActor(_, inits) => {
            for (_, a) in inits {
                s.insert(*a);
            }
        }
        InstKind::ClosureCall(f, args) => {
            s.insert(*f);
            for a in args {
                s.insert(*a);
            }
        }
        InstKind::ChanCreate(_, cap) => {
            if let Some(c) = cap {
                s.insert(*c);
            }
        }
        InstKind::MapInit => {}
        InstKind::Assert(v, _) => {
            s.insert(*v);
        }
        InstKind::InlineAsm(_, args) => {
            for a in args {
                s.insert(*a);
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
}

fn collect_term_uses(term: &Terminator, s: &mut HashSet<ValueId>) {
    match term {
        Terminator::Branch(c, _, _) | Terminator::Switch(c, _, _) => {
            s.insert(*c);
        }
        Terminator::Return(Some(v)) => {
            s.insert(*v);
        }
        _ => {}
    }
}

pub(super) fn is_pure(kind: &InstKind) -> bool {
    matches!(
        kind,
        InstKind::IntConst(_)
            | InstKind::FloatConst(_)
            | InstKind::BoolConst(_)
            | InstKind::StringConst(_)
            | InstKind::Void
            | InstKind::FnRef(_)
            | InstKind::BinOp(..)
            | InstKind::UnaryOp(..)
            | InstKind::Cmp(..)
            | InstKind::Cast(..)
            | InstKind::StrictCast(..)
            | InstKind::Copy(..)
            | InstKind::ArrayInit(_)
            | InstKind::StructInit(..)
            | InstKind::VariantInit(..)
            | InstKind::MapInit
    )
}
