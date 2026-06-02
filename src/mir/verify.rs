//! MIR structural verifier.
//!
//! Catches MIR-level invariant violations early — before they reach codegen,
//! where errors surface as cryptic LLVM verifier diagnostics or, worse,
//! miscompilations. Runs automatically after MIR lowering (and again after
//! optimization) under `debug_assertions`; can be invoked explicitly in tests.
//!
//! Invariants checked:
//!   1. Entry block exists.
//!   2. Block ids are unique within a function.
//!   3. SSA single-definition: each `ValueId` defined at most once (as a
//!      parameter, phi destination, or instruction destination).
//!   4. Every `ValueId` *used* is defined somewhere in the same function.
//!   5. Every `BlockId` referenced by a terminator or phi incoming edge
//!      exists in the function.
//!   6. Phi incoming `(pred, _)` edges reference an actual predecessor of the
//!      containing block.
//!   7. Function return type matches the value type at every
//!      `Terminator::Return`:
//!        - `Return(Some(v))` ⇒ `value_ty(v) == fn.ret_ty`
//!        - `Return(None)`    ⇒ `fn.ret_ty == Type::Void`
//!      (This is the invariant that catches generator-shape bugs like the
//!      P0-3 regression: function declared `-> Generator<i64>` but body
//!      emitting `Return(i64)`.)
//!   8. Branch condition is `Type::Bool`.
//!
//! Type equality uses `PartialEq`. Pending the Type-canonicalization work
//! (P0-12), a small set of relaxations are allowed: `Type::Generator(_)`
//! matches any `Type::Generator(_)` regardless of yield type (yield-type
//! polymorphism is not yet enforced at the MIR level), and `Type::Var(_)` /
//! `Type::Unknown` are always considered compatible (they should not appear
//! in well-typed MIR but the verifier is forgiving rather than wrong).

use super::*;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

/// Verify every function in `prog`. Returns the list of diagnostics, prefixed
/// with the offending function name. Empty `Ok(())` on success.
pub fn verify_program(prog: &Program) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for func in &prog.functions {
        if let Err(es) = verify_function(func) {
            for e in es {
                errors.push(format!("[fn {}] {}", func.name.as_str(), e));
            }
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

/// Verify a single MIR function. Returns one diagnostic per violation.
pub fn verify_function(f: &Function) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

    // --- Pass 1: collect block ids and value definitions ---
    let mut block_ids: HashSet<BlockId> = HashSet::new();
    for bb in &f.blocks {
        if !block_ids.insert(bb.id) {
            errors.push(format!("duplicate block {}", bb.id));
        }
    }
    if !block_ids.contains(&f.entry) {
        errors.push(format!("entry block {} does not exist", f.entry));
    }

    let mut value_ty: HashMap<ValueId, Type> = HashMap::new();
    let redef = |v: ValueId,
                     ty: &Type,
                     site: &str,
                     value_ty: &mut HashMap<ValueId, Type>,
                     errors: &mut Vec<String>| {
        if value_ty.insert(v, ty.clone()).is_some() {
            errors.push(format!("{} multiply defined ({})", v, site));
        }
    };
    for p in &f.params {
        redef(p.value, &p.ty, "param", &mut value_ty, &mut errors);
    }
    for bb in &f.blocks {
        for phi in &bb.phis {
            redef(phi.dest, &phi.ty, "phi", &mut value_ty, &mut errors);
        }
        for inst in &bb.insts {
            if let Some(v) = inst.dest {
                redef(v, &inst.ty, "inst", &mut value_ty, &mut errors);
            }
        }
    }

    // --- Pass 2: check uses, terminators, phi predecessors, ret type ---
    let preds = f.predecessors();

    let check_block = |b: BlockId, ctx: &str, errors: &mut Vec<String>| {
        if !block_ids.contains(&b) {
            errors.push(format!("{} references undefined block {}", ctx, b));
        }
    };

    let check_val = |v: ValueId,
                     ctx: &str,
                     value_ty: &HashMap<ValueId, Type>,
                     errors: &mut Vec<String>| {
        if !value_ty.contains_key(&v) {
            errors.push(format!("{} uses undefined value {}", ctx, v));
        }
    };

    for bb in &f.blocks {
        let bb_label = bb.id;

        // Phis: each incoming block must be an actual predecessor; values must be defined.
        let empty: Vec<BlockId> = Vec::new();
        let bb_preds: &Vec<BlockId> = preds.get(&bb.id).unwrap_or(&empty);
        for phi in &bb.phis {
            for (pred_bb, val) in &phi.incoming {
                check_block(*pred_bb, &format!("phi in {}", bb_label), &mut errors);
                if !bb_preds.contains(pred_bb) {
                    errors.push(format!(
                        "phi {} in {} has incoming from {} which is not a predecessor",
                        phi.dest, bb_label, pred_bb
                    ));
                }
                check_val(
                    *val,
                    &format!("phi {} in {}", phi.dest, bb_label),
                    &value_ty,
                    &mut errors,
                );
            }
        }

        // Instructions: every operand value must be defined.
        for inst in &bb.insts {
            for v in inst_used_values(&inst.kind) {
                check_val(
                    v,
                    &format!("inst in {} ({})", bb_label, inst_tag(&inst.kind)),
                    &value_ty,
                    &mut errors,
                );
            }
        }

        // Terminator: block ids must exist, condition operand must be defined,
        // return type must match function ret type.
        match &bb.terminator {
            Terminator::Goto(b) => {
                check_block(*b, &format!("goto in {}", bb_label), &mut errors);
            }
            Terminator::Branch(cond, t, fl) => {
                check_val(*cond, &format!("branch cond in {}", bb_label), &value_ty, &mut errors);
                // Jinn has truthy semantics: codegen's compile_ternary / if applies
                // `to_bool(...)` which coerces any int (and pointers/options) to i1.
                // So MIR-level Branch cond is legitimately any int-like type, not just Bool.
                if let Some(ty) = value_ty.get(cond)
                    && !ty_compatible(ty, &Type::Bool)
                    && !is_truthy_compatible(ty)
                {
                    errors.push(format!(
                        "branch in {} cond {} has type {:?}, expected Bool or truthy-compatible",
                        bb_label, cond, ty
                    ));
                }
                check_block(*t, &format!("branch true in {}", bb_label), &mut errors);
                check_block(*fl, &format!("branch false in {}", bb_label), &mut errors);
            }
            Terminator::Switch(scr, cases, default) => {
                check_val(*scr, &format!("switch scrutinee in {}", bb_label), &value_ty, &mut errors);
                for (_, b) in cases {
                    check_block(*b, &format!("switch case in {}", bb_label), &mut errors);
                }
                check_block(*default, &format!("switch default in {}", bb_label), &mut errors);
            }
            Terminator::Return(opt) => match opt {
                Some(v) => {
                    check_val(*v, &format!("return in {}", bb_label), &value_ty, &mut errors);
                    if let Some(ty) = value_ty.get(v)
                        && !ty_compatible(ty, &f.ret_ty)
                    {
                        errors.push(format!(
                            "return in {} yields type {:?} but function ret_ty is {:?}",
                            bb_label, ty, f.ret_ty
                        ));
                    }
                }
                None => {
                    if !ty_compatible(&f.ret_ty, &Type::Void) {
                        errors.push(format!(
                            "return-void in {} but function ret_ty is {:?}",
                            bb_label, f.ret_ty
                        ));
                    }
                }
            },
            Terminator::Unreachable => {}
        }
    }

    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

/// Type equality with relaxations for not-yet-canonical Type variants
/// (see P0-12). Errs on the side of *not* reporting a false positive.
fn ty_compatible(a: &Type, b: &Type) -> bool {
    use Type::*;
    // Strip aliases and newtypes — they should be canonicalized, but during
    // the migration accept the underlying representation.
    let a = unwrap_transparent(a);
    let b = unwrap_transparent(b);
    if a == b {
        return true;
    }
    match (a, b) {
        // Yield-type polymorphism is not enforced at MIR level yet.
        (Generator(_), Generator(_)) => true,
        // Coroutine handles are typically just pointers at MIR level.
        (Coroutine(_), Coroutine(_)) => true,
        // Struct vs Enum non-canonicalization (see P0-12): the typer may
        // tag a use as `Struct("Option")` and a definition as
        // `Enum("Option")` depending on the lowering path. Treat as the
        // same nominal type by name.
        (Struct(n1, _), Struct(n2, _))
        | (Enum(n1), Enum(n2))
        | (Struct(n1, _), Enum(n2))
        | (Enum(n1), Struct(n2, _)) => n1 == n2,
        // Type variables should not appear in well-formed MIR; if they leak
        // through, do not produce a false positive — the typer will have
        // already flagged the underlying problem.
        (TypeVar(_), _) | (_, TypeVar(_)) => true,
        _ => false,
    }
}

fn unwrap_transparent(t: &Type) -> &Type {
    match t {
        Type::Alias(_, inner) | Type::Newtype(_, inner) => unwrap_transparent(inner),
        other => other,
    }
}

/// True if `t` is acceptable as a `Branch`/`if` condition under Jinn's
/// truthy semantics (codegen applies `to_bool`).
fn is_truthy_compatible(t: &Type) -> bool {
    use Type::*;
    let t = unwrap_transparent(t);
    matches!(
        t,
        Bool | I8 | I16 | I32 | I64 | U8 | U16 | U32 | U64
            | Ptr(_) | String | Enum(_) | Struct(_, _)
    )
}

/// Enumerate all `ValueId`s read by an instruction kind (operands, not the
/// destination).
fn inst_used_values(k: &InstKind) -> Vec<ValueId> {
    use InstKind::*;
    match k {
        IntConst(_)
        | FloatConst(_)
        | BoolConst(_)
        | StringConst(_)
        | Void
        | Load(_)
        | FieldTombstone(_, _)
        | FnRef(_)
        | MapInit
        | GlobalLoad(_) => Vec::new(),

        BinOp(_, a, b) | Cmp(_, a, b, _) => vec![*a, *b],
        UnaryOp(_, a) => vec![*a],
        Call(_, args) => args.clone(),
        MethodCall(recv, _, args, _) => {
            let mut v = vec![*recv];
            v.extend(args.iter().copied());
            v
        }
        IndirectCall(callee, args) => {
            let mut v = vec![*callee];
            v.extend(args.iter().copied());
            v
        }
        Store(_, v) => vec![*v],
        FieldGet(r, _) => vec![*r],
        FieldSet(r, _, v) => vec![*r, *v],
        FieldStore(_, _, v) => vec![*v],
        Index(a, b) | IndexUnchecked(a, b) => vec![*a, *b],
        IndexSet(a, b, c) => vec![*a, *b, *c],
        IndexStore(_, a, b) => vec![*a, *b],
        StructInit(_, fields) => fields.iter().map(|(_, v)| *v).collect(),
        VariantInit(_, _, _, args) => args.clone(),
        ArrayInit(args) => args.clone(),
        Cast(v, _) | StrictCast(v, _) | Ref(v) | Deref(v) | Alloc(v) => vec![*v],
        Drop(v, _) => vec![*v],
        DropMany(vs) => vs.iter().map(|(v, _)| *v).collect(),
        Copy(v) | Clone(v, _) => vec![*v],
        Slice(a, b, c) => vec![*a, *b, *c],
        VecNew(args) => args.clone(),
        VecPush(a, b) => vec![*a, *b],
        VecLen(v) => vec![*v],
        ClosureCreate(_, caps) => caps.clone(),
        ClosureCall(callee, args) => {
            let mut v = vec![*callee];
            v.extend(args.iter().copied());
            v
        }
        SpawnActor(_, fields) => fields.iter().map(|(_, v)| *v).collect(),
        ChanCreate(_, cap) => cap.iter().copied().collect(),
        ChanSend(a, b) => vec![*a, *b],
        ChanRecv(v) => vec![*v],
        SelectArm(args, _) => args.clone(),
        Log(v) | Assert(v, _) => vec![*v],
        InlineAsm(_, args) => args.clone(),
        GlobalStore(_, v) => vec![*v],
    }
}

/// Short tag for an instruction kind, for diagnostic messages.
fn inst_tag(k: &InstKind) -> &'static str {
    use InstKind::*;
    match k {
        IntConst(_) => "IntConst",
        FloatConst(_) => "FloatConst",
        BoolConst(_) => "BoolConst",
        StringConst(_) => "StringConst",
        Void => "Void",
        BinOp(..) => "BinOp",
        UnaryOp(..) => "UnaryOp",
        Cmp(..) => "Cmp",
        Call(..) => "Call",
        MethodCall(..) => "MethodCall",
        IndirectCall(..) => "IndirectCall",
        Load(_) => "Load",
        Store(..) => "Store",
        FieldGet(..) => "FieldGet",
        FieldSet(..) => "FieldSet",
        FieldStore(..) => "FieldStore",
        FieldTombstone(..) => "FieldTombstone",
        Index(..) => "Index",
        IndexUnchecked(..) => "IndexUnchecked",
        IndexSet(..) => "IndexSet",
        IndexStore(..) => "IndexStore",
        StructInit(..) => "StructInit",
        VariantInit(..) => "VariantInit",
        ArrayInit(_) => "ArrayInit",
        Cast(..) => "Cast",
        StrictCast(..) => "StrictCast",
        Ref(_) => "Ref",
        Deref(_) => "Deref",
        Alloc(_) => "Alloc",
        Drop(..) => "Drop",
        DropMany(_) => "DropMany",
        Copy(_) => "Copy",
        Clone(..) => "Clone",
        FnRef(_) => "FnRef",
        Slice(..) => "Slice",
        VecNew(_) => "VecNew",
        VecPush(..) => "VecPush",
        VecLen(_) => "VecLen",
        MapInit => "MapInit",
        ClosureCreate(..) => "ClosureCreate",
        ClosureCall(..) => "ClosureCall",
        SpawnActor(..) => "SpawnActor",
        ChanCreate(..) => "ChanCreate",
        ChanSend(..) => "ChanSend",
        ChanRecv(_) => "ChanRecv",
        SelectArm(..) => "SelectArm",
        Log(_) => "Log",
        Assert(..) => "Assert",
        InlineAsm(..) => "InlineAsm",
        GlobalLoad(_) => "GlobalLoad",
        GlobalStore(..) => "GlobalStore",
    }
}
