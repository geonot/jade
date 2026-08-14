use super::*;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

pub fn verify_program(prog: &Program) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for func in &prog.functions {
        if let Err(es) = verify_function(func) {
            for e in es {
                errors.push(format!("[fn {}] {}", func.name.as_str(), e));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn verify_function(f: &Function) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

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

    let preds = f.predecessors();

    let check_block = |b: BlockId, ctx: &str, errors: &mut Vec<String>| {
        if !block_ids.contains(&b) {
            errors.push(format!("{} references undefined block {}", ctx, b));
        }
    };

    let check_val =
        |v: ValueId, ctx: &str, value_ty: &HashMap<ValueId, Type>, errors: &mut Vec<String>| {
            if !value_ty.contains_key(&v) {
                errors.push(format!("{} uses undefined value {}", ctx, v));
            }
        };

    for bb in &f.blocks {
        let bb_label = bb.id;

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
                if let Some(ty) = value_ty.get(val)
                    && !ty_compatible(ty, &phi.ty)
                {
                    errors.push(format!(
                        "phi {} in {} has type {:?} but its incoming value {} from {} has type {:?}",
                        phi.dest, bb_label, phi.ty, val, pred_bb, ty
                    ));
                }
            }
        }

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

        match &bb.terminator {
            Terminator::Goto(b) => {
                check_block(*b, &format!("goto in {}", bb_label), &mut errors);
            }
            Terminator::Branch(cond, t, fl) => {
                check_val(
                    *cond,
                    &format!("branch cond in {}", bb_label),
                    &value_ty,
                    &mut errors,
                );

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
                check_val(
                    *scr,
                    &format!("switch scrutinee in {}", bb_label),
                    &value_ty,
                    &mut errors,
                );
                for (_, b) in cases {
                    check_block(*b, &format!("switch case in {}", bb_label), &mut errors);
                }
                check_block(
                    *default,
                    &format!("switch default in {}", bb_label),
                    &mut errors,
                );
            }
            Terminator::Return(opt) => match opt {
                Some(v) => {
                    check_val(
                        *v,
                        &format!("return in {}", bb_label),
                        &value_ty,
                        &mut errors,
                    );

                    if let Some(ty) = value_ty.get(v)
                        && !(f.is_coroutine && f.scheduler_task)
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

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn ty_compatible(a: &Type, b: &Type) -> bool {
    use Type::*;

    let a = unwrap_transparent(a);
    let b = unwrap_transparent(b);
    if a == b {
        return true;
    }
    match (a, b) {
        (Generator(_), Generator(_)) => true,

        (Coroutine(_), Coroutine(_)) => true,

        (Struct(n1, _), Struct(n2, _))
        | (Enum(n1), Enum(n2))
        | (Struct(n1, _), Enum(n2))
        | (Enum(n1), Struct(n2, _)) => n1 == n2,

        (TypeVar(_), _) | (_, TypeVar(_)) => true,
        _ => false,
    }
}

fn unwrap_transparent(t: &Type) -> &Type {
    match t {
        Type::Alias(_, inner) | Type::Newtype(_, inner) | Type::Frozen(inner) => {
            unwrap_transparent(inner)
        }
        other => other,
    }
}

fn is_truthy_compatible(t: &Type) -> bool {
    use Type::*;
    let t = unwrap_transparent(t);
    matches!(
        t,
        Bool | I8
            | I16
            | I32
            | I64
            | U8
            | U16
            | U32
            | U64
            | Ptr(_)
            | String
            | Enum(_)
            | Struct(_, _)
    )
}

fn inst_used_values(k: &InstKind) -> Vec<ValueId> {
    use InstKind::*;
    match k {
        IntConst(_) | FloatConst(_) | BoolConst(_) | StringConst(_) | Void | Load(_) | FnRef(_)
        | MapInit | GlobalLoad(_) => Vec::new(),

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
        FieldClear(o, _) => vec![*o],
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
        Log(v) | Eprint(v) | Assert(v, _) => vec![*v],
        InlineAsm(_, args) => args.clone(),
        GlobalStore(_, v) => vec![*v],
    }
}

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
        FieldClear(..) => "FieldClear",
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
        Eprint(_) => "Eprint",
        Assert(..) => "Assert",
        InlineAsm(..) => "InlineAsm",
        GlobalLoad(_) => "GlobalLoad",
        GlobalStore(..) => "GlobalStore",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FnAttrs, Span};
    use crate::hir::DefId;
    use crate::intern::Symbol;
    use crate::mir::{BasicBlock, DropMeta, Function, Instruction, Param, Phi};

    fn func(blocks: Vec<BasicBlock>, ret_ty: Type) -> Function {
        Function {
            name: Symbol::intern("test_fn"),
            def_id: DefId(0),
            pkg_id: None,
            params: vec![Param {
                value: ValueId(0),
                name: Symbol::intern("p"),
                ty: Type::I64,
                by_ref: false,
            }],
            ret_ty,
            blocks,
            entry: BlockId(0),
            span: Span::new(0, 0, 1, 1),
            next_value: 100,
            next_block: 100,
            attrs: FnAttrs::default(),
            is_coroutine: false,
            scheduler_task: false,
            cancel_cleanup: None,
            drops: DropMeta::default(),
        }
    }

    fn block(id: u32, insts: Vec<Instruction>, term: Terminator) -> BasicBlock {
        BasicBlock {
            id: BlockId(id),
            label: Symbol::intern("bb"),
            phis: vec![],
            insts,
            terminator: term,
        }
    }

    fn inst(dest: u32, kind: InstKind, ty: Type) -> Instruction {
        Instruction {
            dest: Some(ValueId(dest)),
            kind,
            ty,
            span: Span::new(0, 0, 1, 1),
            def_id: None,
        }
    }

    #[test]
    fn accepts_well_formed_function() {
        let f = func(
            vec![block(
                0,
                vec![inst(1, InstKind::IntConst(42), Type::I64)],
                Terminator::Return(Some(ValueId(1))),
            )],
            Type::I64,
        );
        assert!(verify_function(&f).is_ok());
    }

    #[test]
    fn rejects_use_of_undefined_value() {
        let f = func(
            vec![block(
                0,
                vec![inst(
                    1,
                    InstKind::BinOp(BinOp::Add, ValueId(0), ValueId(99)),
                    Type::I64,
                )],
                Terminator::Return(Some(ValueId(1))),
            )],
            Type::I64,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("undefined value")));
    }

    #[test]
    fn rejects_return_type_mismatch() {
        let f = func(
            vec![block(
                0,
                vec![inst(1, InstKind::StringConst("x".into()), Type::String)],
                Terminator::Return(Some(ValueId(1))),
            )],
            Type::I64,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("ret_ty")));
    }

    #[test]
    fn rejects_return_void_from_nonvoid_function() {
        let f = func(vec![block(0, vec![], Terminator::Return(None))], Type::I64);
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("return-void")));
    }

    #[test]
    fn rejects_goto_to_missing_block() {
        let f = func(
            vec![block(0, vec![], Terminator::Goto(BlockId(7)))],
            Type::Void,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("undefined block")));
    }

    #[test]
    fn rejects_duplicate_block_ids() {
        let f = func(
            vec![
                block(0, vec![], Terminator::Return(None)),
                block(0, vec![], Terminator::Return(None)),
            ],
            Type::Void,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("duplicate block")));
    }

    #[test]
    fn rejects_missing_entry_block() {
        let mut f = func(vec![block(0, vec![], Terminator::Return(None))], Type::Void);
        f.entry = BlockId(3);
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("entry block")));
    }

    #[test]
    fn rejects_multiply_defined_value() {
        let f = func(
            vec![block(
                0,
                vec![
                    inst(1, InstKind::IntConst(1), Type::I64),
                    inst(1, InstKind::IntConst(2), Type::I64),
                ],
                Terminator::Return(Some(ValueId(1))),
            )],
            Type::I64,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("multiply defined")));
    }

    #[test]
    fn rejects_phi_incoming_from_non_predecessor() {
        let mut entry = block(
            0,
            vec![inst(1, InstKind::IntConst(1), Type::I64)],
            Terminator::Goto(BlockId(1)),
        );
        entry.phis = vec![];
        let mut join = block(1, vec![], Terminator::Return(Some(ValueId(2))));
        join.phis = vec![Phi {
            dest: ValueId(2),
            ty: Type::I64,
            incoming: vec![(BlockId(2), ValueId(1))],
        }];
        let f = func(
            vec![entry, join, block(2, vec![], Terminator::Return(None))],
            Type::I64,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("not a predecessor")));
    }

    #[test]
    fn rejects_phi_incoming_type_mismatch() {
        let entry = block(
            0,
            vec![inst(1, InstKind::IntConst(1), Type::I64)],
            Terminator::Branch(ValueId(1), BlockId(1), BlockId(2)),
        );
        let then_bb = block(
            1,
            vec![inst(3, InstKind::IntConst(7), Type::I64)],
            Terminator::Goto(BlockId(3)),
        );
        let else_bb = block(
            2,
            vec![inst(4, InstKind::StringConst("x".into()), Type::String)],
            Terminator::Goto(BlockId(3)),
        );
        let mut join = block(3, vec![], Terminator::Return(Some(ValueId(5))));
        join.phis = vec![Phi {
            dest: ValueId(5),
            ty: Type::I64,
            incoming: vec![(BlockId(1), ValueId(3)), (BlockId(2), ValueId(4))],
        }];
        let f = func(vec![entry, then_bb, else_bb, join], Type::I64);
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("incoming value")));
    }

    #[test]
    fn rejects_branch_on_non_truthy_type() {
        let f = func(
            vec![
                block(
                    0,
                    vec![inst(1, InstKind::FloatConst(1.0), Type::F64)],
                    Terminator::Branch(ValueId(1), BlockId(1), BlockId(2)),
                ),
                block(1, vec![], Terminator::Return(None)),
                block(2, vec![], Terminator::Return(None)),
            ],
            Type::Void,
        );
        let errs = verify_function(&f).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("truthy")));
    }

    #[test]
    fn program_errors_carry_function_name() {
        let f = func(vec![block(0, vec![], Terminator::Return(None))], Type::I64);
        let prog = Program {
            functions: vec![f],
            types: vec![],
            externs: vec![],
            globals: vec![],
            pkg_id: None,
            item_pkgs: std::collections::HashMap::new(),
        };
        let errs = verify_program(&prog).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("[fn test_fn]")));
    }
}
