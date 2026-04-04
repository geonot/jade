//! HIR → MIR lowering.
//!
//! Converts HIR functions into MIR basic blocks with explicit control flow.

use std::collections::{HashMap, HashSet};
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind, Pat};
use crate::types::Type;
use super::*;

/// Lowers an HIR program into MIR.
pub fn lower_program(prog: &hir::Program) -> Program {
    let mut functions = Vec::new();
    for f in &prog.fns {
        functions.extend(lower_function(f));
    }
    // Also lower type methods
    for td in &prog.types {
        for m in &td.methods {
            functions.extend(lower_function(m));
        }
    }
    // Also lower trait impl methods
    for ti in &prog.trait_impls {
        for m in &ti.methods {
            functions.extend(lower_function(m));
        }
    }
    let types = prog.types.iter().map(|td| TypeDef {
        name: td.name.clone(),
        fields: td.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect(),
    }).collect();
    let externs = prog.externs.iter().map(|ef| ExternDecl {
        name: ef.name.clone(),
        params: ef.params.iter().map(|p| p.1.clone()).collect(),
        ret: ef.ret.clone(),
    }).collect();
    Program { functions, types, externs }
}

struct Lowerer {
    func: Function,
    current_block: BlockId,
    var_map: HashMap<String, ValueId>,
    /// Variables that are backed by memory (Store/Load) instead of var_map.
    /// Used for variables reassigned inside loops or branches.
    mem_vars: HashSet<String>,
    loop_stack: Vec<(BlockId, BlockId)>, // (continue_target, break_target)
    /// Lambda functions generated during lowering (added to the program).
    lambda_fns: Vec<Function>,
}

impl Lowerer {
    fn new(name: &str, def_id: crate::hir::DefId, span: Span) -> Self {
        let entry = BlockId(0);
        let func = Function {
            name: name.into(),
            def_id,
            params: Vec::new(),
            ret_ty: Type::Void,
            blocks: vec![BasicBlock {
                id: entry,
                label: "entry".to_string(),
                phis: Vec::new(),
                insts: Vec::new(),
                terminator: Terminator::Unreachable,
            }],
            entry,
            span,
            next_value: 0,
            next_block: 1,
        };
        Lowerer {
            func,
            current_block: entry,
            var_map: HashMap::new(),
            mem_vars: HashSet::new(),
            loop_stack: Vec::new(),
            lambda_fns: Vec::new(),
        }
    }

    fn new_value(&mut self) -> ValueId {
        self.func.new_value()
    }

    fn new_block(&mut self, label: &str) -> BlockId {
        self.func.new_block(label)
    }

    fn emit(&mut self, kind: InstKind, ty: Type, span: Span) -> ValueId {
        let dest = self.new_value();
        self.func.block_mut(self.current_block).insts.push(Instruction {
            dest: Some(dest),
            kind,
            ty,
            span,
            def_id: None,
        });
        dest
    }

    fn emit_with_def_id(&mut self, kind: InstKind, ty: Type, span: Span, def_id: crate::hir::DefId) -> ValueId {
        let dest = self.new_value();
        self.func.block_mut(self.current_block).insts.push(Instruction {
            dest: Some(dest),
            kind,
            ty,
            span,
            def_id: Some(def_id),
        });
        dest
    }

    fn emit_void(&mut self, kind: InstKind, span: Span) {
        self.func.block_mut(self.current_block).insts.push(Instruction {
            dest: None,
            kind,
            ty: Type::Void,
            span,
            def_id: None,
        });
    }

    /// Emit an instruction with no destination but carrying a type annotation.
    /// Used for Store instructions so the variable type is preserved.
    fn emit_void_typed(&mut self, kind: InstKind, ty: Type, span: Span) {
        self.func.block_mut(self.current_block).insts.push(Instruction {
            dest: None,
            kind,
            ty,
            span,
            def_id: None,
        });
    }

    fn set_terminator(&mut self, term: Terminator) {
        self.func.block_mut(self.current_block).terminator = term;
    }

    fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
    }

    fn current_block_has_terminator(&self) -> bool {
        !matches!(
            self.func.block(self.current_block).terminator,
            Terminator::Unreachable
        )
    }

    /// Try to extract an integer constant from a ValueId by scanning the
    /// current function's instructions.
    fn try_extract_int_const(&self, val: ValueId) -> Option<i64> {
        for bb in &self.func.blocks {
            for inst in &bb.insts {
                if inst.dest == Some(val) {
                    if let InstKind::IntConst(n) = &inst.kind {
                        return Some(*n);
                    }
                    return None;
                }
            }
        }
        None
    }

    /// Look up the type of a ValueId by scanning instructions and params.
    fn value_type(&self, val: ValueId) -> Type {
        for p in &self.func.params {
            if p.value == val { return p.ty.clone(); }
        }
        for bb in &self.func.blocks {
            for phi in &bb.phis {
                if phi.dest == val { return phi.ty.clone(); }
            }
            for inst in &bb.insts {
                if inst.dest == Some(val) { return inst.ty.clone(); }
            }
        }
        eprintln!("MIR lower: value_type fallback to I64 for {:?}", val);
        Type::I64 // fallback
    }

    fn lower_expr(&mut self, expr: &hir::Expr) -> ValueId {
        let span = expr.span;
        let ty = expr.ty.clone();
        match &expr.kind {
            ExprKind::Int(n) => self.emit(InstKind::IntConst(*n), ty, span),
            ExprKind::Float(f) => self.emit(InstKind::FloatConst(*f), ty, span),
            ExprKind::Bool(b) => self.emit(InstKind::BoolConst(*b), ty, span),
            ExprKind::Str(s) => self.emit(InstKind::StringConst(s.clone()), ty, span),
            ExprKind::Void => self.emit(InstKind::Void, Type::Void, span),
            ExprKind::None => self.emit(InstKind::IntConst(0), ty, span),

            ExprKind::Var(_, name) => {
                if let Some(&val) = self.var_map.get(name) {
                    val
                } else {
                    self.emit(InstKind::Load(name.clone()), ty, span)
                }
            }

            // HIR uses ast::BinOp which includes comparison operators
            ExprKind::BinOp(lhs, op, rhs) => {
                // Short-circuit And/Or: evaluate rhs only if needed.
                if *op == ast::BinOp::And {
                    let l = self.lower_expr(lhs);
                    let false_val = self.emit(InstKind::BoolConst(false), Type::Bool, span);
                    let rhs_bb = self.new_block("and.rhs");
                    let merge_bb = self.new_block("and.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, rhs_bb, merge_bb));
                    self.switch_to(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, false_val), (rhs_end, r)],
                    });
                    return phi;
                }
                if *op == ast::BinOp::Or {
                    let l = self.lower_expr(lhs);
                    let true_val = self.emit(InstKind::BoolConst(true), Type::Bool, span);
                    let rhs_bb = self.new_block("or.rhs");
                    let merge_bb = self.new_block("or.merge");
                    let cur_bb = self.current_block;
                    self.set_terminator(Terminator::Branch(l, merge_bb, rhs_bb));
                    self.switch_to(rhs_bb);
                    let r = self.lower_expr(rhs);
                    let rhs_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    self.switch_to(merge_bb);
                    let phi = self.func.new_value();
                    self.func.block_mut(merge_bb).phis.push(crate::mir::Phi {
                        dest: phi,
                        ty: Type::Bool,
                        incoming: vec![(cur_bb, true_val), (rhs_end, r)],
                    });
                    return phi;
                }
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                let operand_ty = lhs.ty.clone();
                match op {
                    ast::BinOp::Eq => self.emit(InstKind::Cmp(CmpOp::Eq, l, r, operand_ty.clone()), ty, span),
                    ast::BinOp::Ne => self.emit(InstKind::Cmp(CmpOp::Ne, l, r, operand_ty.clone()), ty, span),
                    ast::BinOp::Lt => self.emit(InstKind::Cmp(CmpOp::Lt, l, r, operand_ty.clone()), ty, span),
                    ast::BinOp::Gt => self.emit(InstKind::Cmp(CmpOp::Gt, l, r, operand_ty.clone()), ty, span),
                    ast::BinOp::Le => self.emit(InstKind::Cmp(CmpOp::Le, l, r, operand_ty.clone()), ty, span),
                    ast::BinOp::Ge => self.emit(InstKind::Cmp(CmpOp::Ge, l, r, operand_ty), ty, span),
                    _ => {
                        let mir_op = lower_binop(op);
                        self.emit(InstKind::BinOp(mir_op, l, r), ty, span)
                    }
                }
            }

            ExprKind::UnaryOp(op, inner) => {
                let v = self.lower_expr(inner);
                let mir_op = lower_unaryop(op);
                self.emit(InstKind::UnaryOp(mir_op, v), ty, span)
            }

            ExprKind::Call(_, name, args) => {
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call(name.clone(), arg_vals), ty, span)
            }

            ExprKind::IndirectCall(callee, args) => {
                let f = self.lower_expr(callee);
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::IndirectCall(f, arg_vals), ty, span)
            }

            // Method(obj, _type_name, method_name, args)
            ExprKind::Method(obj, _type_name, method_name, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, method_name.clone(), arg_vals), ty, span)
            }

            ExprKind::Field(obj, field, _idx) => {
                let obj_val = self.lower_expr(obj);
                self.emit(InstKind::FieldGet(obj_val, field.clone()), ty, span)
            }

            ExprKind::Index(arr, idx) => {
                let a = self.lower_expr(arr);
                let i = self.lower_expr(idx);
                self.emit(InstKind::Index(a, i), ty, span)
            }

            ExprKind::Struct(name, inits) => {
                let fields: Vec<(String, ValueId)> = inits.iter().map(|fi| {
                    let v = self.lower_expr(&fi.value);
                    (fi.name.clone().unwrap_or_default(), v)
                }).collect();
                self.emit(InstKind::StructInit(name.clone(), fields), ty, span)
            }

            ExprKind::VariantCtor(enum_name, variant_name, tag, inits) => {
                let arg_vals: Vec<ValueId> = inits.iter().map(|fi| self.lower_expr(&fi.value)).collect();
                self.emit(InstKind::VariantInit(enum_name.clone(), variant_name.clone(), *tag, arg_vals), ty, span)
            }

            ExprKind::IfExpr(if_expr) => {
                let cond_val = self.lower_expr(&if_expr.cond);
                let then_bb = self.new_block("if.then");
                let else_bb = self.new_block("if.else");
                let merge_bb = self.new_block("if.merge");

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                // Then branch
                self.switch_to(then_bb);
                let then_val = self.lower_block_expr(&if_expr.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                // Else branch
                self.switch_to(else_bb);
                let else_val = if let Some(els) = &if_expr.els {
                    self.lower_block_expr(els)
                } else {
                    self.emit(InstKind::Void, Type::Void, span)
                };
                let else_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                // Merge
                self.switch_to(merge_bb);
                if !matches!(ty, Type::Void) {
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: ty.clone(),
                        incoming: vec![(then_end, then_val), (else_end, else_val)],
                    });
                    result
                } else {
                    self.emit(InstKind::Void, Type::Void, span)
                }
            }

            ExprKind::Ternary(cond, then_expr, else_expr) => {
                let cond_val = self.lower_expr(cond);
                let then_bb = self.new_block("ternary.then");
                let else_bb = self.new_block("ternary.else");
                let merge_bb = self.new_block("ternary.merge");

                self.set_terminator(Terminator::Branch(cond_val, then_bb, else_bb));

                self.switch_to(then_bb);
                let then_val = self.lower_expr(then_expr);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(else_bb);
                let else_val = self.lower_expr(else_expr);
                let else_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(merge_bb);
                let result = self.new_value();
                self.func.block_mut(merge_bb).phis.push(Phi {
                    dest: result,
                    ty: ty.clone(),
                    incoming: vec![(then_end, then_val), (else_end, else_val)],
                });
                result
            }

            ExprKind::Cast(inner, target_ty) | ExprKind::StrictCast(inner, target_ty) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Cast(v, target_ty.clone()), ty, span)
            }

            ExprKind::Ref(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Ref(v), ty, span)
            }

            ExprKind::Deref(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Deref(v), ty, span)
            }

            ExprKind::Array(elems) | ExprKind::Tuple(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::ArrayInit(vals), ty, span)
            }

            ExprKind::Slice(arr, start, end) => {
                let a = self.lower_expr(arr);
                let s = self.lower_expr(start);
                let e = self.lower_expr(end);
                self.emit(InstKind::Slice(a, s, e), ty, span)
            }

            ExprKind::FnRef(_, name) => {
                self.emit(InstKind::FnRef(name.clone()), ty, span)
            }

            ExprKind::VariantRef(enum_name, variant_name, tag) => {
                self.emit(InstKind::VariantInit(enum_name.clone(), variant_name.clone(), *tag, vec![]), ty, span)
            }

            ExprKind::Block(stmts) => {
                self.lower_block_expr(stmts)
            }

            ExprKind::Lambda(params, body) => {
                // Lower the lambda body as a separate MIR function.
                // Captured variables become leading parameters; declared params follow.
                let lambda_name = format!("lambda.{}", self.func.next_value);

                // Collect captured variable (name, ValueId, Type) triples.
                let param_names: std::collections::HashSet<&str> =
                    params.iter().map(|p| p.name.as_str()).collect();
                let mut refs = std::collections::HashSet::new();
                Self::collect_expr_var_refs_block(body, &mut refs);
                let mut capture_info: Vec<(String, ValueId, Type)> = Vec::new();
                for name in &refs {
                    if !param_names.contains(name.as_str()) {
                        if let Some(&val) = self.var_map.get(name) {
                            let cap_ty = self.value_type(val);
                            capture_info.push((name.clone(), val, cap_ty));
                        }
                    }
                }
                let capture_vals: Vec<ValueId> = capture_info.iter().map(|(_, v, _)| *v).collect();

                // Determine return type from the closure's type annotation.
                let ret_ty = if let Type::Fn(_, r) = &ty { *r.clone() } else { Type::I64 };

                // Create a new Lowerer for the lambda function.
                let mut lambda_lowerer = Lowerer::new(&lambda_name, crate::hir::DefId(0), span);
                lambda_lowerer.func.ret_ty = ret_ty;

                // Add capture parameters first.
                for (cap_name, _, cap_ty) in &capture_info {
                    let val = lambda_lowerer.new_value();
                    lambda_lowerer.func.params.push(Param {
                        value: val,
                        name: cap_name.clone(),
                        ty: cap_ty.clone(),
                    });
                    lambda_lowerer.var_map.insert(cap_name.clone(), val);
                }

                // Add declared parameters.
                for p in params {
                    let val = lambda_lowerer.new_value();
                    lambda_lowerer.func.params.push(Param {
                        value: val,
                        name: p.name.clone(),
                        ty: p.ty.clone(),
                    });
                    lambda_lowerer.var_map.insert(p.name.clone(), val);
                }

                // Lower the lambda body.
                let mut last = lambda_lowerer.emit(InstKind::Void, Type::Void, span);
                for stmt in body {
                    last = lambda_lowerer.lower_stmt(stmt);
                }
                if !lambda_lowerer.current_block_has_terminator() {
                    lambda_lowerer.set_terminator(Terminator::Return(Some(last)));
                }

                // Collect the lambda function and any nested lambdas.
                self.lambda_fns.push(lambda_lowerer.func);
                self.lambda_fns.append(&mut lambda_lowerer.lambda_fns);

                self.emit(InstKind::ClosureCreate(lambda_name, capture_vals), ty, span)
            }

            ExprKind::Coerce(inner, _) => self.lower_expr(inner),

            ExprKind::Pipe(inner, _def_id, name, extra_args) => {
                let mut args = vec![self.lower_expr(inner)];
                args.extend(extra_args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(name.clone(), args), ty, span)
            }

            // Collection methods — all follow the same pattern
            ExprKind::StringMethod(obj, name, args) | ExprKind::DeferredMethod(obj, name, args)
            | ExprKind::VecMethod(obj, name, args)
            | ExprKind::MapMethod(obj, name, args) | ExprKind::SetMethod(obj, name, args)
            | ExprKind::PQMethod(obj, name, args) | ExprKind::DequeMethod(obj, name, args) => {
                let obj_val = self.lower_expr(obj);
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, name.clone(), vals), ty, span)
            }

            ExprKind::VecNew(elems) | ExprKind::NDArrayNew(elems) | ExprKind::SIMDNew(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::VecNew(vals), ty, span)
            }

            ExprKind::MapNew => {
                self.emit(InstKind::MapInit, ty, span)
            }

            ExprKind::SetNew | ExprKind::PQNew | ExprKind::DequeNew => {
                self.emit(InstKind::SetInit, ty, span)
            }

            ExprKind::ListComp(body_expr, _def_id, bind, iter, end, cond) => {
                // Desugar: vec = VecNew(); for bind in iter..end { if cond { VecPush(vec, body) } }
                let vec_val = self.emit(InstKind::VecNew(vec![]), ty.clone(), span);
                let iter_val = self.lower_expr(iter);

                let cond_bb = self.new_block("listcomp.cond");
                let body_bb = self.new_block("listcomp.body");
                let inc_bb = self.new_block("listcomp.inc");
                let exit_bb = self.new_block("listcomp.exit");

                // Create loop index using Store/Load.
                let zero = self.emit(InstKind::IntConst(0), Type::I64, span);
                let one = self.emit(InstKind::IntConst(1), Type::I64, span);
                let end_val = if let Some(e) = end {
                    self.lower_expr(e)
                } else {
                    self.emit(InstKind::VecLen(iter_val), Type::I64, span)
                };
                let idx_name = format!("__listcomp_idx_{bind}");
                self.emit_void_typed(InstKind::Store(idx_name.clone(), zero), Type::I64, span);

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, span);
                let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, idx, end_val, Type::I64), Type::Bool, span);
                self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                self.switch_to(body_bb);
                // Bind the loop variable.
                self.var_map.insert(bind.clone(), idx);
                let elem_val = self.lower_expr(body_expr);
                if let Some(c) = cond {
                    let filter_bb = self.new_block("listcomp.filter");
                    let push_bb = self.new_block("listcomp.push");
                    let cond_val = self.lower_expr(c);
                    self.set_terminator(Terminator::Branch(cond_val, push_bb, filter_bb));

                    self.switch_to(push_bb);
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));

                    self.switch_to(filter_bb);
                    self.set_terminator(Terminator::Goto(inc_bb));
                } else {
                    self.emit_void(InstKind::VecPush(vec_val, elem_val), span);
                    self.set_terminator(Terminator::Goto(inc_bb));
                }

                // Increment index.
                self.switch_to(inc_bb);
                let cur_idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, span);
                let next_idx = self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, span);
                self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, span);
                self.set_terminator(Terminator::Goto(cond_bb));

                self.switch_to(exit_bb);
                vec_val
            }

            // Concurrency primitives — lower as dedicated MIR instructions
            ExprKind::Spawn(name) => {
                self.emit(InstKind::SpawnActor(name.clone(), vec![]), ty, span)
            }
            ExprKind::Send(target, _type_name, handler, _tag, args) => {
                let mut all = vec![self.lower_expr(target)];
                all.extend(args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(format!("__send_{handler}"), all), ty, span)
            }
            ExprKind::ChannelCreate(elem_ty, cap) => {
                let _c = self.lower_expr(cap);
                self.emit(InstKind::ChanCreate(elem_ty.clone()), ty, span)
            }
            ExprKind::ChannelSend(chan, val) => {
                let ch = self.lower_expr(chan);
                let v = self.lower_expr(val);
                self.emit(InstKind::ChanSend(ch, v), ty, span)
            }
            ExprKind::ChannelRecv(chan) => {
                let c = self.lower_expr(chan);
                self.emit(InstKind::ChanRecv(c), ty, span)
            }

            ExprKind::Select(arms, default) => {
                // Demote variables assigned in any arm to memory (Store/Load)
                // so the merge point sees correct values.
                let mut assigned = HashSet::new();
                for arm in arms.iter() {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }
                if let Some(def_body) = default {
                    Self::collect_assigned_vars(def_body, &mut assigned);
                }
                let pre_existing: HashSet<String> = assigned.iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, span);

                // Lower select as a SelectArm with all channel values
                let ch_vals: Vec<ValueId> = arms.iter().map(|arm| {
                    self.lower_expr(&arm.chan)
                }).collect();
                let has_default = default.is_some();
                let select_val = self.emit(InstKind::SelectArm(ch_vals.clone(), has_default), ty.clone(), span);
                // Lower bodies as a switch on the selected arm index
                if !arms.is_empty() {
                    let merge_bb = self.new_block("select.merge");
                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    for (i, arm) in arms.iter().enumerate() {
                        let arm_bb = self.new_block(&format!("select.arm{i}"));
                        cases.push((i as i64, arm_bb));
                        self.switch_to(arm_bb);
                        if let Some(bind_name) = &arm.binding {
                            // Use __select_recv instead of ChanRecv — jade_select
                            // already received the data into the case data buffer.
                            let idx_val = self.emit(InstKind::IntConst(i as i64), Type::I64, span);
                            let recv_val = self.emit(InstKind::Call(
                                "__select_recv".to_string(),
                                vec![select_val, idx_val],
                            ), arm.elem_ty.clone(), span);
                            self.var_map.insert(bind_name.clone(), recv_val);
                        }
                        self.lower_block_stmts(&arm.body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                    }
                    let default_bb = if let Some(def_body) = default {
                        let db = self.new_block("select.default");
                        self.switch_to(db);
                        self.lower_block_stmts(def_body);
                        self.set_terminator(Terminator::Goto(merge_bb));
                        db
                    } else {
                        merge_bb
                    };
                    // We need to go back and set the switch terminator
                    // The select_val block ended where we emitted SelectArm
                    // Find the block that contains the select inst
                    let select_block = self.func.blocks.iter()
                        .find(|b| b.insts.iter().any(|i| i.dest == Some(select_val)))
                        .map(|b| b.id)
                        .unwrap_or(self.current_block);
                    self.func.block_mut(select_block).terminator = Terminator::Switch(select_val, cases, default_bb);
                    self.switch_to(merge_bb);
                }
                select_val
            }

            // Atomics — all lowered as intrinsic calls
            ExprKind::AtomicLoad(p) => {
                let v = self.lower_expr(p);
                self.emit(InstKind::Call("__atomic_load".into(), vec![v]), ty, span)
            }
            ExprKind::AtomicStore(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_store".into(), args), ty, span)
            }
            ExprKind::AtomicAdd(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_add".into(), args), ty, span)
            }
            ExprKind::AtomicSub(p, val) => {
                let args = vec![self.lower_expr(p), self.lower_expr(val)];
                self.emit(InstKind::Call("__atomic_sub".into(), args), ty, span)
            }
            ExprKind::AtomicCas(p, expected, desired) => {
                let args = vec![
                    self.lower_expr(p), self.lower_expr(expected), self.lower_expr(desired),
                ];
                self.emit(InstKind::Call("__atomic_cas".into(), args), ty, span)
            }

            // Builtin functions — dedicated MIR instructions for optimizable ones
            ExprKind::Builtin(builtin, args) => {
                use crate::hir::BuiltinFn;
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                match builtin {
                    BuiltinFn::Log => {
                        let arg_ty = args.first().map(|a| a.ty.clone()).unwrap_or(Type::I64);
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::Log(v), arg_ty, span)
                    }
                    BuiltinFn::Assert => {
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::Assert(v, "assertion failed".into()), ty, span)
                    }
                    BuiltinFn::RcAlloc => {
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcNew(v, ty.clone()), ty, span)
                    }
                    BuiltinFn::RcRetain => {
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcClone(v), ty, span)
                    }
                    BuiltinFn::RcRelease => {
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::RcDec(v), ty, span)
                    }
                    BuiltinFn::WeakUpgrade => {
                        let v = vals.into_iter().next().unwrap_or_else(|| self.emit(InstKind::Void, Type::Void, span));
                        self.emit(InstKind::WeakUpgrade(v), ty, span)
                    }
                    _ => {
                        let name = format!("__builtin_{builtin:?}");
                        self.emit(InstKind::Call(name, vals), ty, span)
                    }
                }
            }

            // Syscall — opaque call
            ExprKind::Syscall(args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__syscall".into(), vals), ty, span)
            }

            // Coroutines — opaque calls
            ExprKind::CoroutineCreate(name, body) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Call(format!("__coro_create_{name}"), vec![]), ty, span)
            }
            ExprKind::CoroutineNext(coro) => {
                let c = self.lower_expr(coro);
                self.emit(InstKind::Call("__coro_next".into(), vec![c]), ty, span)
            }
            ExprKind::Yield(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__yield".into(), vec![v]), ty, span)
            }

            // Dynamic dispatch
            ExprKind::DynDispatch(obj, trait_name, method, args) => {
                let obj_val = self.lower_expr(obj);
                let arg_vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::DynDispatch(obj_val, trait_name.clone(), method.clone(), arg_vals), ty, span)
            }
            ExprKind::DynCoerce(inner, _type_name, _trait_name) => {
                self.lower_expr(inner)
            }

            // Store operations — opaque calls
            ExprKind::StoreQuery(store_name, filter) => {
                let filter_val = self.lower_expr(&filter.value);
                // Encode field name and op in the call name for codegen
                let op_str = match filter.op {
                    ast::BinOp::Eq => "eq",
                    ast::BinOp::Ne => "ne",
                    ast::BinOp::Lt => "lt",
                    ast::BinOp::Le => "le",
                    ast::BinOp::Gt => "gt",
                    ast::BinOp::Ge => "ge",
                    _ => "eq",
                };
                self.emit(InstKind::Call(
                    format!("__store_query_{store_name}__{}__{op_str}", filter.field),
                    vec![filter_val],
                ), ty, span)
            }
            ExprKind::StoreCount(store_name) => {
                self.emit(InstKind::Call(format!("__store_count_{store_name}"), vec![]), ty, span)
            }
            ExprKind::StoreAll(store_name) => {
                self.emit(InstKind::Call(format!("__store_all_{store_name}"), vec![]), ty, span)
            }

            // Iterator
            ExprKind::IterNext(iter_var, type_name, method_name) => {
                if let Some(&v) = self.var_map.get(iter_var) {
                    self.emit(InstKind::MethodCall(v, format!("{type_name}_{method_name}"), vec![]), ty, span)
                } else {
                    self.emit(InstKind::Call(format!("__iter_{type_name}_{method_name}"), vec![]), ty, span)
                }
            }

            ExprKind::Unreachable => {
                self.set_terminator(Terminator::Unreachable);
                let dead = self.new_block("after.unreachable");
                self.switch_to(dead);
                self.emit(InstKind::Void, ty, span)
            }

            ExprKind::AsFormat(inner, _fmt_str) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__as_format".into(), vec![v]), ty, span)
            }

            ExprKind::Builder(name, fields) => {
                // Desugar builder into StructInit + field sets
                let inits: Vec<(String, ValueId)> = fields.iter().map(|(n, e)| {
                    (n.clone(), self.lower_expr(e))
                }).collect();
                self.emit(InstKind::StructInit(name.clone(), inits), ty, span)
            }

            ExprKind::CowWrap(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__cow_wrap".into(), vec![v]), ty, span)
            }
            ExprKind::CowClone(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__cow_clone".into(), vec![v]), ty, span)
            }

            ExprKind::GeneratorCreate(_def_id, name, body) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Call(format!("__gen_create_{name}"), vec![]), ty, span)
            }
            ExprKind::GeneratorNext(gen_expr) => {
                let g = self.lower_expr(gen_expr);
                self.emit(InstKind::Call("__gen_next".into(), vec![g]), ty, span)
            }

            ExprKind::Grad(inner) => {
                let v = self.lower_expr(inner);
                self.emit(InstKind::Call("__grad".into(), vec![v]), ty, span)
            }
            ExprKind::Einsum(_pattern, args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__einsum".into(), vals), ty, span)
            }
        }
    }

    fn lower_block_expr(&mut self, stmts: &[hir::Stmt]) -> ValueId {
        let mut last = self.emit(InstKind::Void, Type::Void, Span::dummy());
        for stmt in stmts {
            last = self.lower_stmt(stmt);
        }
        last
    }

    fn lower_stmt(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::Bind(b) => {
                let val = self.lower_expr(&b.value);
                // Store the DefId on the instruction that produced this value,
                // so MIR Perceus can track binding → value relationships.
                if let Some(inst) = self.func.block_mut(self.current_block)
                    .insts.iter_mut().rev()
                    .find(|i| i.dest == Some(val))
                {
                    inst.def_id = Some(b.def_id);
                }
                if self.mem_vars.contains(&b.name) {
                    // Variable is memory-backed (reassigned in a loop/branch).
                    // Emit Store with the variable's type so codegen allocas are correct.
                    self.func.block_mut(self.current_block).insts.push(Instruction {
                        dest: None,
                        kind: InstKind::Store(b.name.clone(), val),
                        ty: b.ty.clone(),
                        span: b.span,
                        def_id: None,
                    });
                } else {
                    self.var_map.insert(b.name.clone(), val);
                }
                val
            }

            hir::Stmt::Assign(target, value, _span) => {
                let val = self.lower_expr(value);
                match &target.kind {
                    ExprKind::Var(_, name) => {
                        if self.mem_vars.contains(name) {
                            // Use the value's type from the expression.
                            self.func.block_mut(self.current_block).insts.push(Instruction {
                                dest: None,
                                kind: InstKind::Store(name.clone(), val),
                                ty: value.ty.clone(),
                                span: target.span,
                                def_id: None,
                            });
                        } else {
                            self.var_map.insert(name.clone(), val);
                        }
                    }
                    ExprKind::Field(obj, field, _) => {
                        // If the object is a mem_var, emit a direct field store
                        // on the variable name so codegen can GEP into the alloca.
                        if let ExprKind::Var(_, name) = &obj.kind {
                            if self.mem_vars.contains(name) {
                                let obj_ty = obj.ty.clone();
                                self.func.block_mut(self.current_block).insts.push(Instruction {
                                    dest: None,
                                    kind: InstKind::FieldStore(name.clone(), field.clone(), val),
                                    ty: obj_ty,
                                    span: target.span,
                                    def_id: None,
                                });
                                return val;
                            }
                        }
                        let obj_val = self.lower_expr(obj);
                        // Pass the obj's type so codegen can find the struct layout.
                        let obj_ty = obj.ty.clone();
                        self.func.block_mut(self.current_block).insts.push(Instruction {
                            dest: None,
                            kind: InstKind::FieldSet(obj_val, field.clone(), val),
                            ty: obj_ty,
                            span: target.span,
                            def_id: None,
                        });
                    }
                    ExprKind::Index(arr, idx) => {
                        let a = self.lower_expr(arr);
                        let i = self.lower_expr(idx);
                        self.emit_void(InstKind::IndexSet(a, i, val), target.span);
                    }
                    _ => {}
                }
                val
            }

            hir::Stmt::Expr(e) => self.lower_expr(e),

            hir::Stmt::If(if_stmt) => {
                // Demote variables assigned in any branch to memory
                // so the merge point gets the correct value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&if_stmt.then, &mut assigned);
                for (_, elif_body) in &if_stmt.elifs {
                    Self::collect_assigned_vars(elif_body, &mut assigned);
                }
                if let Some(els) = &if_stmt.els {
                    Self::collect_assigned_vars(els, &mut assigned);
                }
                // Only demote vars that already exist in var_map (were defined before if).
                let pre_existing: HashSet<String> = assigned.iter()
                    .filter(|n| self.var_map.contains_key(*n))
                    .cloned()
                    .collect();
                self.demote_vars_to_memory(&pre_existing, if_stmt.span);

                let cond = self.lower_expr(&if_stmt.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");

                // Determine the false-branch target:
                // elif chain first, then else, then merge.
                let first_elif_bb = if !if_stmt.elifs.is_empty() {
                    Some(self.new_block("elif.test"))
                } else {
                    None
                };
                let else_bb = if if_stmt.els.is_some() && first_elif_bb.is_none() {
                    self.new_block("if.else")
                } else {
                    first_elif_bb.unwrap_or(merge_bb)
                };

                self.set_terminator(Terminator::Branch(cond, then_bb, else_bb));

                self.switch_to(then_bb);
                let then_val = self.lower_block_expr(&if_stmt.then);
                let then_end = self.current_block;
                self.set_terminator(Terminator::Goto(merge_bb));

                // Lower elif chains.
                let mut elif_vals: Vec<(BlockId, ValueId)> = Vec::new();
                let mut prev_false_bb = first_elif_bb;
                for (i, (elif_cond, elif_body)) in if_stmt.elifs.iter().enumerate() {
                    let elif_test = prev_false_bb.unwrap();
                    let elif_body_bb = self.new_block("elif.body");

                    // Determine where a false elif branches to.
                    let is_last_elif = i + 1 == if_stmt.elifs.len();
                    let elif_false_bb = if is_last_elif {
                        if if_stmt.els.is_some() {
                            Some(self.new_block("if.else"))
                        } else {
                            None
                        }
                    } else {
                        Some(self.new_block("elif.test"))
                    };

                    self.switch_to(elif_test);
                    let c = self.lower_expr(elif_cond);
                    self.set_terminator(Terminator::Branch(
                        c,
                        elif_body_bb,
                        elif_false_bb.unwrap_or(merge_bb),
                    ));

                    self.switch_to(elif_body_bb);
                    let elif_val = self.lower_block_expr(elif_body);
                    let elif_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    elif_vals.push((elif_end, elif_val));

                    prev_false_bb = elif_false_bb;
                }

                let else_val_info = if let Some(els) = &if_stmt.els {
                    // The else block target was the last false branch.
                    let else_target = prev_false_bb.unwrap_or(else_bb);
                    self.switch_to(else_target);
                    let else_val = self.lower_block_expr(els);
                    let else_end = self.current_block;
                    self.set_terminator(Terminator::Goto(merge_bb));
                    Some((else_end, else_val))
                } else {
                    None
                };

                self.switch_to(merge_bb);

                // If all branches produce non-void values, insert a phi at merge.
                let then_ty = self.value_type(then_val);
                if !matches!(then_ty, Type::Void) && else_val_info.is_some() {
                    let mut incoming = vec![(then_end, then_val)];
                    for &(bb, v) in &elif_vals {
                        incoming.push((bb, v));
                    }
                    if let Some((eb, ev)) = else_val_info {
                        incoming.push((eb, ev));
                    }
                    let result = self.new_value();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest: result,
                        ty: then_ty,
                        incoming,
                    });
                    result
                } else {
                    self.emit(InstKind::Void, Type::Void, if_stmt.span)
                }
            }

            hir::Stmt::While(w) => {
                // Demote variables assigned inside loop body to memory
                // so each iteration re-reads the current value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&w.body, &mut assigned);
                // Also check condition for assigned vars (unlikely but safe)
                self.demote_vars_to_memory(&assigned, w.span);

                let cond_bb = self.new_block("while.cond");
                let body_bb = self.new_block("while.body");
                let exit_bb = self.new_block("while.exit");

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let cond = self.lower_expr(&w.cond);
                self.set_terminator(Terminator::Branch(cond, body_bb, exit_bb));

                self.loop_stack.push((cond_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&w.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(cond_bb));
                }
                self.loop_stack.pop();

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, w.span)
            }

            hir::Stmt::For(f) => {
                // Demote variables assigned inside for loop body to memory
                // so each iteration re-reads the current value via Load.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, f.span);

                // Range-based for: `for i in start..end`
                // If `end` is present, this is a range for; otherwise iterate
                // the collection via index.
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("for.cond");
                let body_bb = self.new_block("for.body");
                let inc_bb = self.new_block("for.inc");
                let exit_bb = self.new_block("for.exit");

                if let Some(ref end_expr) = f.end {
                    // Range for: iter_val = start, end = end_expr
                    let end_val = self.lower_expr(end_expr);
                    let step_val = if let Some(ref step_expr) = f.step {
                        self.lower_expr(step_expr)
                    } else {
                        self.emit(InstKind::IntConst(1), Type::I64, f.span)
                    };

                    // Store counter as a variable.
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), iter_val), f.bind_ty.clone(), f.span);
                    self.var_map.insert(f.bind.clone(), iter_val);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    // Condition: load counter, compare < end.
                    self.switch_to(cond_bb);
                    let counter = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64), Type::Bool, f.span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    // Re-bind the loop variable to the loaded counter so body can use it.
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    // Increment
                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let next = self.emit(InstKind::BinOp(BinOp::Add, cur, step_val), f.bind_ty.clone(), f.span);
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), next), f.bind_ty.clone(), f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64) {
                    // Range for with implicit start=0: `for i in N` means 0..N
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                    let end_val = iter_val;

                    self.emit_void_typed(InstKind::Store(f.bind.clone(), zero), f.bind_ty.clone(), f.span);
                    self.var_map.insert(f.bind.clone(), zero);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64), Type::Bool, f.span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), f.span);
                    let next = self.emit(InstKind::BinOp(BinOp::Add, cur, one), f.bind_ty.clone(), f.span);
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), next), f.bind_ty.clone(), f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::Coroutine(_) | Type::Generator(_)) {
                    // Generator/coroutine for: resume loop.
                    // cond: resume gen; done = __gen_done(gen); branch !done ? body : exit
                    // body: val = __gen_next_val(gen); bind = val; ... body ...; goto cond
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    // Resume the generator
                    let _resume = self.emit(InstKind::Call("__gen_resume".into(), vec![iter_val]), Type::Void, f.span);
                    // Check done flag
                    let done = self.emit(InstKind::Call("__gen_done".into(), vec![iter_val]), Type::Bool, f.span);
                    // Branch: if done, exit; else body
                    self.set_terminator(Terminator::Branch(done, exit_bb, body_bb));

                    // Body: read yielded value and bind it
                    self.loop_stack.push((cond_bb, exit_bb));
                    self.switch_to(body_bb);
                    let val = self.emit(InstKind::Call("__gen_next_val".into(), vec![iter_val]), f.bind_ty.clone(), f.span);
                    self.var_map.insert(f.bind.clone(), val);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(cond_bb));
                    self.loop_stack.pop();
                } else {
                    // Collection for: iterate with index.
                    // Get length.
                    let len = self.emit(InstKind::VecLen(iter_val), Type::I64, f.span);
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, f.span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, f.span);
                    let idx_name = format!("__for_idx_{}", f.bind);
                    self.emit_void_typed(InstKind::Store(idx_name.clone(), zero), Type::I64, f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    // Condition: idx < len.
                    self.switch_to(cond_bb);
                    let idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, f.span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, idx, len, Type::I64), Type::Bool, f.span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    // Body: bind element.
                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    let elem = self.emit(InstKind::Index(iter_val, idx), f.bind_ty.clone(), f.span);
                    self.var_map.insert(f.bind.clone(), elem);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    // Increment index.
                    self.switch_to(inc_bb);
                    let cur_idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, f.span);
                    let next_idx = self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, f.span);
                    self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, f.span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                }

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, f.span)
            }

            hir::Stmt::Loop(l) => {
                // Demote variables assigned inside loop body to memory.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&l.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, l.span);

                let body_bb = self.new_block("loop.body");
                let exit_bb = self.new_block("loop.exit");

                self.set_terminator(Terminator::Goto(body_bb));
                self.loop_stack.push((body_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&l.body);
                if !self.current_block_has_terminator() {
                    self.set_terminator(Terminator::Goto(body_bb));
                }
                self.loop_stack.pop();

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, l.span)
            }

            hir::Stmt::Ret(val, _ret_ty, span) => {
                if let Some(v) = val {
                    let rv = self.lower_expr(v);
                    self.set_terminator(Terminator::Return(Some(rv)));
                } else {
                    self.set_terminator(Terminator::Return(None));
                }
                let dead = self.new_block("after.ret");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Break(val, span) => {
                if let Some((_, exit)) = self.loop_stack.last().copied() {
                    if let Some(v) = val {
                        let _ = self.lower_expr(v);
                    }
                    self.set_terminator(Terminator::Goto(exit));
                }
                let dead = self.new_block("after.break");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Continue(span) => {
                if let Some((cont, _)) = self.loop_stack.last().copied() {
                    self.set_terminator(Terminator::Goto(cont));
                }
                let dead = self.new_block("after.continue");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::Match(m) => {
                // Demote variables assigned in any match arm to memory.
                let mut assigned = HashSet::new();
                for arm in &m.arms {
                    Self::collect_assigned_vars(&arm.body, &mut assigned);
                }
                // Only demote pre-existing variables (new bindings in arms are fine).
                let pre_existing: HashSet<String> = assigned.into_iter()
                    .filter(|v| self.var_map.contains_key(v))
                    .collect();
                self.demote_vars_to_memory(&pre_existing, m.span);

                let subj = self.lower_expr(&m.subject);
                let merge_bb = self.new_block("match.merge");

                if m.arms.is_empty() {
                    self.switch_to(merge_bb);
                    return self.emit(InstKind::Void, Type::Void, m.span);
                }

                // Check if this is an integer/enum tag match (Switch) or
                // needs sequential comparison (if-else chain).
                let is_enum = matches!(m.subject.ty, Type::Enum(_));
                let has_ctor = m.arms.iter().any(|a| matches!(a.pat, Pat::Ctor(..)));
                let all_lit = m.arms.iter().all(|a| matches!(a.pat, Pat::Lit(_) | Pat::Wild(_)));
                let result_ty = m.ty.clone();
                let has_result = !matches!(result_ty, Type::Void);

                // Track (value, block) pairs from each arm for Phi creation.
                let mut phi_entries: Vec<(ValueId, BlockId)> = Vec::new();

                if is_enum || has_ctor || all_lit {
                    // Switch-based match on integer/enum discriminant.
                    let disc = if is_enum || has_ctor {
                        // Extract tag from variant.
                        self.emit(InstKind::FieldGet(subj, "__tag".into()), Type::I64, m.span)
                    } else {
                        subj
                    };

                    let mut cases: Vec<(i64, BlockId)> = Vec::new();
                    let mut has_explicit_default = false;
                    let unreach_bb = self.new_block("match.unreach");
                    let mut default_bb = unreach_bb;
                    let mut arm_blocks = Vec::new();

                    for arm in &m.arms {
                        let arm_bb = self.new_block("match.arm");
                        arm_blocks.push((arm_bb, arm));

                        match &arm.pat {
                            Pat::Lit(lit_expr) => {
                                // Lower the literal to get its constant value.
                                let lit_val = self.lower_expr(lit_expr);
                                // Find the integer constant if possible.
                                if let Some(ival) = self.try_extract_int_const(lit_val) {
                                    cases.push((ival, arm_bb));
                                } else {
                                    // Non-integer literal — fallback, use as default.
                                    default_bb = arm_bb;
                                }
                            }
                            Pat::Ctor(_, tag, _, _) => {
                                cases.push((*tag as i64, arm_bb));
                            }
                            Pat::Wild(_) => {
                                default_bb = arm_bb;
                                has_explicit_default = true;
                            }
                            _ => {
                                default_bb = arm_bb;
                                has_explicit_default = true;
                            }
                        }
                    }

                    self.set_terminator(Terminator::Switch(disc, cases, default_bb));

                    // If no explicit default arm, make the unreachable block dead.
                    if !has_explicit_default {
                        self.switch_to(unreach_bb);
                        // This block should never be reached; leave as Unreachable.
                    }

                    for (arm_bb, arm) in arm_blocks {
                        self.switch_to(arm_bb);
                        // Bind pattern variables for Ctor patterns.
                        if let Pat::Ctor(_, _, sub_pats, _) = &arm.pat {
                            for (i, sp) in sub_pats.iter().enumerate() {
                                if let Pat::Bind(_, name, ty, _) = sp {
                                    let field = self.emit(
                                        InstKind::FieldGet(subj, format!("_{i}")),
                                        ty.clone(),
                                        arm.span,
                                    );
                                    self.var_map.insert(name.clone(), field);
                                }
                            }
                        }
                        if let Pat::Bind(_, name, _ty, _) = &arm.pat {
                            self.var_map.insert(name.clone(), subj);
                        }
                        let mut arm_last = self.emit(InstKind::Void, Type::Void, arm.span);
                        for s in &arm.body {
                            arm_last = self.lower_stmt(s);
                        }
                        if !self.current_block_has_terminator() {
                            if has_result {
                                phi_entries.push((arm_last, self.current_block));
                            }
                            self.set_terminator(Terminator::Goto(merge_bb));
                        }
                    }
                } else {
                    // Sequential if-else chain for complex patterns.
                    let mut next_test = self.current_block;
                    for (i, arm) in m.arms.iter().enumerate() {
                        let arm_bb = self.new_block("match.arm");
                        let is_last = i + 1 == m.arms.len();
                        let next_bb = if is_last {
                            merge_bb
                        } else {
                            self.new_block("match.next")
                        };

                        self.switch_to(next_test);

                        match &arm.pat {
                            Pat::Wild(_) => {
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                            Pat::Bind(_, name, _ty, _) => {
                                // Bind always matches.
                                self.var_map.insert(name.clone(), subj);
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                            Pat::Lit(lit_expr) => {
                                let lit_val = self.lower_expr(lit_expr);
                                let subj_ty = self.value_type(subj);
                                let cmp = self.emit(
                                    InstKind::Cmp(CmpOp::Eq, subj, lit_val, subj_ty),
                                    Type::Bool,
                                    arm.span,
                                );
                                self.set_terminator(Terminator::Branch(cmp, arm_bb, next_bb));
                            }
                            _ => {
                                // Fallback: unconditional (catches Range, Tuple, etc.)
                                self.set_terminator(Terminator::Goto(arm_bb));
                            }
                        }

                        self.switch_to(arm_bb);
                        let mut arm_last = self.emit(InstKind::Void, Type::Void, arm.span);
                        for s in &arm.body {
                            arm_last = self.lower_stmt(s);
                        }
                        if !self.current_block_has_terminator() {
                            if has_result {
                                phi_entries.push((arm_last, self.current_block));
                            }
                            self.set_terminator(Terminator::Goto(merge_bb));
                        }

                        next_test = next_bb;
                    }
                    // If the last arm didn't have a wild/bind, ensure we go to merge.
                    if next_test != merge_bb {
                        self.switch_to(next_test);
                        self.set_terminator(Terminator::Goto(merge_bb));
                    }
                }

                self.switch_to(merge_bb);
                if has_result && !phi_entries.is_empty() {
                    let dest = self.new_value();
                    let incoming: Vec<(BlockId, ValueId)> = phi_entries.iter()
                        .map(|(val, blk)| (*blk, *val))
                        .collect();
                    self.func.block_mut(merge_bb).phis.push(Phi {
                        dest,
                        ty: result_ty,
                        incoming,
                    });
                    dest
                } else {
                    self.emit(InstKind::Void, Type::Void, m.span)
                }
            }

            hir::Stmt::Drop(_, name, ty, span) => {
                if let Some(&val) = self.var_map.get(name) {
                    self.emit_void(InstKind::Drop(val, ty.clone()), *span);
                }
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::TupleBind(bindings, value, _span) => {
                let val = self.lower_expr(value);
                for (i, (_id, name, bind_ty)) in bindings.iter().enumerate() {
                    let idx = self.emit(InstKind::IntConst(i as i64), Type::I64, Span::dummy());
                    let elem = self.emit(InstKind::Index(val, idx), bind_ty.clone(), Span::dummy());
                    self.var_map.insert(name.clone(), elem);
                }
                val
            }

            hir::Stmt::ErrReturn(expr, _ty, span) => {
                let v = self.lower_expr(expr);
                self.set_terminator(Terminator::Return(Some(v)));
                let dead = self.new_block("after.err_return");
                self.switch_to(dead);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::ChannelClose(ch, span) => {
                let c = self.lower_expr(ch);
                self.emit(InstKind::Call("__chan_close".into(), vec![c]), Type::Void, *span)
            }

            hir::Stmt::Stop(expr, span) => {
                let v = self.lower_expr(expr);
                self.emit(InstKind::Call("__stop".into(), vec![v]), Type::Void, *span)
            }

            hir::Stmt::Asm(_asm) => {
                // Inline assembly — lower as opaque call
                self.emit(InstKind::Call("__asm".into(), vec![]), Type::Void, Span::dummy())
            }

            hir::Stmt::StoreInsert(store_name, exprs, span) => {
                let vals: Vec<_> = exprs.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::Call(format!("__store_insert_{store_name}"), vals), Type::Void, *span)
            }

            hir::Stmt::StoreDelete(store_name, _filter, span) => {
                self.emit(InstKind::Call(format!("__store_delete_{store_name}"), vec![]), Type::Void, *span)
            }

            hir::Stmt::StoreSet(store_name, fields, _filter, span) => {
                let vals: Vec<_> = fields.iter().map(|(_, e)| self.lower_expr(e)).collect();
                self.emit(InstKind::Call(format!("__store_set_{store_name}"), vals), Type::Void, *span)
            }

            hir::Stmt::Transaction(body, span) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::SimFor(f, span) => {
                // Demote variables assigned inside sim-for body to memory.
                let mut assigned = HashSet::new();
                Self::collect_assigned_vars(&f.body, &mut assigned);
                self.demote_vars_to_memory(&assigned, *span);

                // Parallel for — lower same as sequential for in MIR.
                let iter_val = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("simfor.cond");
                let body_bb = self.new_block("simfor.body");
                let inc_bb = self.new_block("simfor.inc");
                let exit_bb = self.new_block("simfor.exit");

                if let Some(ref end_expr) = f.end {
                    let end_val = self.lower_expr(end_expr);
                    let step_val = if let Some(ref step_expr) = f.step {
                        self.lower_expr(step_expr)
                    } else {
                        self.emit(InstKind::IntConst(1), Type::I64, *span)
                    };
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), iter_val), f.bind_ty.clone(), *span);
                    self.var_map.insert(f.bind.clone(), iter_val);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64), Type::Bool, *span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let next = self.emit(InstKind::BinOp(BinOp::Add, cur, step_val), f.bind_ty.clone(), *span);
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), next), f.bind_ty.clone(), *span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else if matches!(f.iter.ty, Type::I64 | Type::I32 | Type::F64) {
                    // Implicit range: `sim for i in N` means 0..N
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, *span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, *span);
                    let end_val = iter_val;

                    self.emit_void_typed(InstKind::Store(f.bind.clone(), zero), f.bind_ty.clone(), *span);
                    self.var_map.insert(f.bind.clone(), zero);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let counter = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, counter, end_val, Type::I64), Type::Bool, *span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    self.var_map.insert(f.bind.clone(), counter);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur = self.emit(InstKind::Load(f.bind.clone()), f.bind_ty.clone(), *span);
                    let next = self.emit(InstKind::BinOp(BinOp::Add, cur, one), f.bind_ty.clone(), *span);
                    self.emit_void_typed(InstKind::Store(f.bind.clone(), next), f.bind_ty.clone(), *span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                } else {
                    let len = self.emit(InstKind::VecLen(iter_val), Type::I64, *span);
                    let zero = self.emit(InstKind::IntConst(0), Type::I64, *span);
                    let one = self.emit(InstKind::IntConst(1), Type::I64, *span);
                    let idx_name = format!("__simfor_idx_{}", f.bind);
                    self.emit_void_typed(InstKind::Store(idx_name.clone(), zero), Type::I64, *span);
                    self.set_terminator(Terminator::Goto(cond_bb));

                    self.switch_to(cond_bb);
                    let idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, *span);
                    let cmp = self.emit(InstKind::Cmp(CmpOp::Lt, idx, len, Type::I64), Type::Bool, *span);
                    self.set_terminator(Terminator::Branch(cmp, body_bb, exit_bb));

                    self.loop_stack.push((inc_bb, exit_bb));
                    self.switch_to(body_bb);
                    let elem = self.emit(InstKind::Index(iter_val, idx), f.bind_ty.clone(), *span);
                    self.var_map.insert(f.bind.clone(), elem);
                    self.lower_block_stmts(&f.body);
                    self.set_terminator(Terminator::Goto(inc_bb));
                    self.loop_stack.pop();

                    self.switch_to(inc_bb);
                    let cur_idx = self.emit(InstKind::Load(idx_name.clone()), Type::I64, *span);
                    let next_idx = self.emit(InstKind::BinOp(BinOp::Add, cur_idx, one), Type::I64, *span);
                    self.emit_void_typed(InstKind::Store(idx_name, next_idx), Type::I64, *span);
                    self.set_terminator(Terminator::Goto(cond_bb));
                }

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::SimBlock(body, span) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Void, Type::Void, *span)
            }

            hir::Stmt::UseLocal(_, _, _, _) => {
                // No-op in MIR — use declarations are resolved at HIR level
                self.emit(InstKind::Void, Type::Void, Span::dummy())
            }
        }
    }

    fn lower_block_stmts(&mut self, stmts: &[hir::Stmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

    /// Collect variable names *assigned* or *rebound* in a block of HIR statements.
    fn collect_assigned_vars(body: &[hir::Stmt], assigned: &mut HashSet<String>) {
        for stmt in body {
            match stmt {
                hir::Stmt::Bind(b) => {
                    assigned.insert(b.name.clone());
                }
                hir::Stmt::Assign(target, _, _) => {
                    if let ExprKind::Var(_, name) = &target.kind {
                        assigned.insert(name.clone());
                    }
                }
                hir::Stmt::If(i) => {
                    Self::collect_assigned_vars(&i.then, assigned);
                    for (_, elif_body) in &i.elifs {
                        Self::collect_assigned_vars(elif_body, assigned);
                    }
                    if let Some(els) = &i.els {
                        Self::collect_assigned_vars(els, assigned);
                    }
                }
                hir::Stmt::While(w) => {
                    Self::collect_assigned_vars(&w.body, assigned);
                }
                hir::Stmt::For(f) => {
                    Self::collect_assigned_vars(&f.body, assigned);
                }
                hir::Stmt::Loop(l) => {
                    Self::collect_assigned_vars(&l.body, assigned);
                }
                hir::Stmt::Match(m) => {
                    for arm in &m.arms {
                        Self::collect_assigned_vars(&arm.body, assigned);
                    }
                }
                hir::Stmt::Expr(e) => {
                    Self::collect_assigned_vars_in_expr(e, assigned);
                }
                _ => {}
            }
        }
    }

    /// Walk an expression tree to find Select (or other block-containing)
    /// expressions and collect assigned vars from their bodies.
    fn collect_assigned_vars_in_expr(expr: &hir::Expr, assigned: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Select(arms, default) => {
                for arm in arms {
                    Self::collect_assigned_vars(&arm.body, assigned);
                }
                if let Some(def_body) = default {
                    Self::collect_assigned_vars(def_body, assigned);
                }
            }
            _ => {}
        }
    }

    /// Demote variables to memory (Store/Load) — emit Store for their current
    /// var_map value and remove them from var_map so reads use Load.
    fn demote_vars_to_memory(&mut self, vars: &HashSet<String>, span: Span) {
        for name in vars {
            if let Some(&val) = self.var_map.get(name) {
                // Find the type of this variable from the value.
                let ty = self.func.blocks.iter()
                    .flat_map(|bb| bb.insts.iter())
                    .find(|i| i.dest == Some(val))
                    .map(|i| i.ty.clone())
                    .or_else(|| self.func.params.iter()
                        .find(|p| p.value == val)
                        .map(|p| p.ty.clone()))
                    .unwrap_or(Type::I64);
                // Emit Store with the variable's type (not Void) so codegen
                // creates the alloca with the correct LLVM type.
                self.func.block_mut(self.current_block).insts.push(Instruction {
                    dest: None,
                    kind: InstKind::Store(name.clone(), val),
                    ty,
                    span,
                    def_id: None,
                });
                self.var_map.remove(name);
                self.mem_vars.insert(name.clone());
            }
        }
    }

    /// Collect variable names referenced in a block of HIR statements.
    fn collect_expr_var_refs_block(body: &[hir::Stmt], refs: &mut std::collections::HashSet<String>) {
        for stmt in body {
            Self::collect_expr_var_refs_stmt(stmt, refs);
        }
    }

    fn collect_expr_var_refs_stmt(stmt: &hir::Stmt, refs: &mut std::collections::HashSet<String>) {
        match stmt {
            hir::Stmt::Bind(b) => Self::collect_expr_var_refs_expr(&b.value, refs),
            hir::Stmt::Assign(t, v, _) => {
                Self::collect_expr_var_refs_expr(t, refs);
                Self::collect_expr_var_refs_expr(v, refs);
            }
            hir::Stmt::Expr(e) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::If(i) => {
                Self::collect_expr_var_refs_expr(&i.cond, refs);
                Self::collect_expr_var_refs_block(&i.then, refs);
                for (c, b) in &i.elifs {
                    Self::collect_expr_var_refs_expr(c, refs);
                    Self::collect_expr_var_refs_block(b, refs);
                }
                if let Some(els) = &i.els {
                    Self::collect_expr_var_refs_block(els, refs);
                }
            }
            hir::Stmt::While(w) => {
                Self::collect_expr_var_refs_expr(&w.cond, refs);
                Self::collect_expr_var_refs_block(&w.body, refs);
            }
            hir::Stmt::For(f) => {
                Self::collect_expr_var_refs_expr(&f.iter, refs);
                Self::collect_expr_var_refs_block(&f.body, refs);
            }
            hir::Stmt::Loop(l) => Self::collect_expr_var_refs_block(&l.body, refs),
            hir::Stmt::Ret(Some(e), _, _) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::Match(m) => {
                Self::collect_expr_var_refs_expr(&m.subject, refs);
                for arm in &m.arms {
                    Self::collect_expr_var_refs_block(&arm.body, refs);
                }
            }
            hir::Stmt::Break(Some(e), _) | hir::Stmt::ErrReturn(e, _, _)
            | hir::Stmt::ChannelClose(e, _) | hir::Stmt::Stop(e, _) => {
                Self::collect_expr_var_refs_expr(e, refs);
            }
            hir::Stmt::TupleBind(_, e, _) => Self::collect_expr_var_refs_expr(e, refs),
            hir::Stmt::SimFor(f, _) => {
                Self::collect_expr_var_refs_expr(&f.iter, refs);
                Self::collect_expr_var_refs_block(&f.body, refs);
            }
            hir::Stmt::SimBlock(body, _) | hir::Stmt::Transaction(body, _) => {
                Self::collect_expr_var_refs_block(body, refs);
            }
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs { Self::collect_expr_var_refs_expr(e, refs); }
            }
            hir::Stmt::StoreSet(_, updates, _, _) => {
                for (_, e) in updates { Self::collect_expr_var_refs_expr(e, refs); }
            }
            _ => {}
        }
    }

    fn collect_expr_var_refs_expr(expr: &hir::Expr, refs: &mut std::collections::HashSet<String>) {
        match &expr.kind {
            ExprKind::Var(_, name) => { refs.insert(name.clone()); }
            ExprKind::BinOp(l, _, r) => {
                Self::collect_expr_var_refs_expr(l, refs);
                Self::collect_expr_var_refs_expr(r, refs);
            }
            ExprKind::UnaryOp(_, e) | ExprKind::Ref(e) | ExprKind::Deref(e)
            | ExprKind::Cast(e, _) | ExprKind::StrictCast(e, _)
            | ExprKind::Coerce(e, _) => {
                Self::collect_expr_var_refs_expr(e, refs);
            }
            ExprKind::Call(_, _, args) | ExprKind::Array(args) | ExprKind::Tuple(args)
            | ExprKind::VecNew(args) | ExprKind::NDArrayNew(args) | ExprKind::SIMDNew(args)
            | ExprKind::Syscall(args) => {
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::IndirectCall(f, args) => {
                Self::collect_expr_var_refs_expr(f, refs);
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::Method(obj, _, _, args) | ExprKind::StringMethod(obj, _, args)
            | ExprKind::VecMethod(obj, _, args) | ExprKind::MapMethod(obj, _, args)
            | ExprKind::SetMethod(obj, _, args) | ExprKind::PQMethod(obj, _, args)
            | ExprKind::DequeMethod(obj, _, args) | ExprKind::DeferredMethod(obj, _, args) => {
                Self::collect_expr_var_refs_expr(obj, refs);
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::Field(obj, _, _) => Self::collect_expr_var_refs_expr(obj, refs),
            ExprKind::Index(a, i) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(i, refs);
            }
            ExprKind::IfExpr(i) => {
                Self::collect_expr_var_refs_expr(&i.cond, refs);
                Self::collect_expr_var_refs_block(&i.then, refs);
                if let Some(els) = &i.els { Self::collect_expr_var_refs_block(els, refs); }
            }
            ExprKind::Ternary(c, t, f) => {
                Self::collect_expr_var_refs_expr(c, refs);
                Self::collect_expr_var_refs_expr(t, refs);
                Self::collect_expr_var_refs_expr(f, refs);
            }
            ExprKind::Struct(_, fields) | ExprKind::VariantCtor(_, _, _, fields) => {
                for fi in fields { Self::collect_expr_var_refs_expr(&fi.value, refs); }
            }
            ExprKind::Select(arms, default) => {
                for arm in arms {
                    Self::collect_expr_var_refs_expr(&arm.chan, refs);
                    if let Some(v) = &arm.value { Self::collect_expr_var_refs_expr(v, refs); }
                    Self::collect_expr_var_refs_block(&arm.body, refs);
                }
                if let Some(def) = default { Self::collect_expr_var_refs_block(def, refs); }
            }
            ExprKind::DynDispatch(obj, _, _, args) | ExprKind::Send(obj, _, _, _, args)
            | ExprKind::Pipe(obj, _, _, args) => {
                Self::collect_expr_var_refs_expr(obj, refs);
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::Builtin(_, args) => {
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::ChannelSend(a, b) | ExprKind::AtomicStore(a, b)
            | ExprKind::AtomicAdd(a, b) | ExprKind::AtomicSub(a, b) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(b, refs);
            }
            ExprKind::ChannelRecv(e) | ExprKind::CoroutineNext(e)
            | ExprKind::Yield(e) | ExprKind::DynCoerce(e, _, _)
            | ExprKind::AsFormat(e, _) | ExprKind::AtomicLoad(e)
            | ExprKind::Slice(e, _, _) | ExprKind::Grad(e) => {
                Self::collect_expr_var_refs_expr(e, refs);
                if let ExprKind::Slice(_, lo, hi) = &expr.kind {
                    Self::collect_expr_var_refs_expr(lo, refs);
                    Self::collect_expr_var_refs_expr(hi, refs);
                }
            }
            ExprKind::ChannelCreate(_, cap) => Self::collect_expr_var_refs_expr(cap, refs),
            ExprKind::ListComp(body, _, _, iter, end, cond) => {
                Self::collect_expr_var_refs_expr(body, refs);
                Self::collect_expr_var_refs_expr(iter, refs);
                if let Some(e) = end { Self::collect_expr_var_refs_expr(e, refs); }
                if let Some(c) = cond { Self::collect_expr_var_refs_expr(c, refs); }
            }
            ExprKind::CoroutineCreate(_, stmts) => Self::collect_expr_var_refs_block(stmts, refs),
            ExprKind::AtomicCas(a, b, c) => {
                Self::collect_expr_var_refs_expr(a, refs);
                Self::collect_expr_var_refs_expr(b, refs);
                Self::collect_expr_var_refs_expr(c, refs);
            }
            ExprKind::Einsum(_, args) => {
                for a in args { Self::collect_expr_var_refs_expr(a, refs); }
            }
            ExprKind::Builder(_, fields) => {
                for (_, e) in fields { Self::collect_expr_var_refs_expr(e, refs); }
            }
            ExprKind::Block(stmts) => Self::collect_expr_var_refs_block(stmts, refs),
            ExprKind::Lambda(_, body) => Self::collect_expr_var_refs_block(body, refs),
            _ => {}
        }
    }
}

fn lower_function(f: &hir::Fn) -> Vec<Function> {
    let mut lowerer = Lowerer::new(&f.name, f.def_id, f.span);
    lowerer.func.ret_ty = f.ret.clone();

    // Create value IDs for parameters
    for p in &f.params {
        let val = lowerer.new_value();
        lowerer.func.params.push(Param {
            value: val,
            name: p.name.clone(),
            ty: p.ty.clone(),
        });
        lowerer.var_map.insert(p.name.clone(), val);
    }

    // Lower body
    let mut last = lowerer.emit(InstKind::Void, Type::Void, f.span);
    for stmt in &f.body {
        last = lowerer.lower_stmt(stmt);
    }

    // Add implicit return if not already terminated
    if matches!(lowerer.func.block(lowerer.current_block).terminator, Terminator::Unreachable) {
        if matches!(f.ret, Type::Void) {
            lowerer.set_terminator(Terminator::Return(None));
        } else {
            lowerer.set_terminator(Terminator::Return(Some(last)));
        }
    }

    let mut result = vec![lowerer.func];
    result.append(&mut lowerer.lambda_fns);
    result
}

fn lower_binop(op: &ast::BinOp) -> BinOp {
    match op {
        ast::BinOp::Add => BinOp::Add,
        ast::BinOp::Sub => BinOp::Sub,
        ast::BinOp::Mul => BinOp::Mul,
        ast::BinOp::Div => BinOp::Div,
        ast::BinOp::Mod => BinOp::Mod,
        ast::BinOp::Exp => BinOp::Exp,
        ast::BinOp::BitAnd => BinOp::BitAnd,
        ast::BinOp::BitOr => BinOp::BitOr,
        ast::BinOp::BitXor => BinOp::BitXor,
        ast::BinOp::Shl => BinOp::Shl,
        ast::BinOp::Shr => BinOp::Shr,
        ast::BinOp::And => BinOp::And,
        ast::BinOp::Or => BinOp::Or,
        // Comparisons handled separately in lower_expr; this path is unreachable.
        ast::BinOp::Eq | ast::BinOp::Ne | ast::BinOp::Lt
        | ast::BinOp::Gt | ast::BinOp::Le | ast::BinOp::Ge => {
            unreachable!("comparison ops should be handled by lower_expr, not lower_binop")
        }
    }
}

fn lower_unaryop(op: &ast::UnaryOp) -> UnaryOp {
    match op {
        ast::UnaryOp::Neg => UnaryOp::Neg,
        ast::UnaryOp::Not => UnaryOp::Not,
        ast::UnaryOp::BitNot => UnaryOp::BitNot,
    }
}
