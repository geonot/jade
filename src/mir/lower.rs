//! HIR → MIR lowering.
//!
//! Converts HIR functions into MIR basic blocks with explicit control flow.

use std::collections::HashMap;
use crate::ast::{self, Span};
use crate::hir::{self, ExprKind};
use crate::types::Type;
use super::*;

/// Lowers an HIR program into MIR.
pub fn lower_program(prog: &hir::Program) -> Program {
    let mut functions = Vec::new();
    for f in &prog.fns {
        functions.push(lower_function(f));
    }
    // Also lower type methods
    for td in &prog.types {
        for m in &td.methods {
            functions.push(lower_function(m));
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
    loop_stack: Vec<(BlockId, BlockId)>, // (continue_target, break_target)
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
            loop_stack: Vec::new(),
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
        });
        dest
    }

    fn emit_void(&mut self, kind: InstKind, span: Span) {
        self.func.block_mut(self.current_block).insts.push(Instruction {
            dest: None,
            kind,
            ty: Type::Void,
            span,
        });
    }

    fn set_terminator(&mut self, term: Terminator) {
        self.func.block_mut(self.current_block).terminator = term;
    }

    fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
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
                let l = self.lower_expr(lhs);
                let r = self.lower_expr(rhs);
                match op {
                    ast::BinOp::Eq => self.emit(InstKind::Cmp(CmpOp::Eq, l, r), ty, span),
                    ast::BinOp::Ne => self.emit(InstKind::Cmp(CmpOp::Ne, l, r), ty, span),
                    ast::BinOp::Lt => self.emit(InstKind::Cmp(CmpOp::Lt, l, r), ty, span),
                    ast::BinOp::Gt => self.emit(InstKind::Cmp(CmpOp::Gt, l, r), ty, span),
                    ast::BinOp::Le => self.emit(InstKind::Cmp(CmpOp::Le, l, r), ty, span),
                    ast::BinOp::Ge => self.emit(InstKind::Cmp(CmpOp::Ge, l, r), ty, span),
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
                self.emit(InstKind::Load(name.clone()), ty, span)
            }

            ExprKind::VariantRef(enum_name, variant_name, tag) => {
                self.emit(InstKind::VariantInit(enum_name.clone(), variant_name.clone(), *tag, vec![]), ty, span)
            }

            ExprKind::Block(stmts) => {
                self.lower_block_expr(stmts)
            }

            ExprKind::Lambda(_params, _body) => {
                // TODO: closure lowering
                self.emit(InstKind::Void, ty, span)
            }

            ExprKind::Builtin(_, args) => {
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::Call("__builtin".into(), vals), ty, span)
            }

            ExprKind::Coerce(inner, _) => self.lower_expr(inner),

            ExprKind::Pipe(inner, _def_id, name, extra_args) => {
                let mut args = vec![self.lower_expr(inner)];
                args.extend(extra_args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(name.clone(), args), ty, span)
            }

            // Collection methods — all follow the same pattern
            ExprKind::StringMethod(obj, name, args) | ExprKind::VecMethod(obj, name, args)
            | ExprKind::MapMethod(obj, name, args) | ExprKind::SetMethod(obj, name, args)
            | ExprKind::PQMethod(obj, name, args) | ExprKind::DequeMethod(obj, name, args) => {
                let obj_val = self.lower_expr(obj);
                let vals: Vec<_> = args.iter().map(|a| self.lower_expr(a)).collect();
                self.emit(InstKind::MethodCall(obj_val, name.clone(), vals), ty, span)
            }

            ExprKind::VecNew(elems) | ExprKind::NDArrayNew(elems) | ExprKind::SIMDNew(elems) => {
                let vals: Vec<ValueId> = elems.iter().map(|e| self.lower_expr(e)).collect();
                self.emit(InstKind::ArrayInit(vals), ty, span)
            }

            ExprKind::MapNew | ExprKind::SetNew | ExprKind::PQNew | ExprKind::DequeNew => {
                self.emit(InstKind::Void, ty, span)
            }

            ExprKind::ListComp(_body, _def_id, _bind, _iter, _cond, _map) => {
                // TODO: desugar into loop + append
                self.emit(InstKind::Void, ty, span)
            }

            // Concurrency primitives — lower as opaque calls
            ExprKind::Spawn(name) => {
                self.emit(InstKind::Call(format!("__spawn_{name}"), vec![]), ty, span)
            }
            ExprKind::Send(target, _type_name, handler, _tag, args) => {
                let mut all = vec![self.lower_expr(target)];
                all.extend(args.iter().map(|a| self.lower_expr(a)));
                self.emit(InstKind::Call(format!("__send_{handler}"), all), ty, span)
            }
            ExprKind::ChannelCreate(_, cap) => {
                let c = self.lower_expr(cap);
                self.emit(InstKind::Call("__chan_create".into(), vec![c]), ty, span)
            }
            ExprKind::ChannelSend(chan, val) => {
                let args = vec![self.lower_expr(chan), self.lower_expr(val)];
                self.emit(InstKind::Call("__chan_send".into(), args), ty, span)
            }
            ExprKind::ChannelRecv(chan) => {
                let c = self.lower_expr(chan);
                self.emit(InstKind::Call("__chan_recv".into(), vec![c]), ty, span)
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

            // Fallback for any expression kind we don't lower specifically
            _ => self.emit(InstKind::Void, ty, span),
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
                self.var_map.insert(b.name.clone(), val);
                val
            }

            hir::Stmt::Assign(target, value, _span) => {
                let val = self.lower_expr(value);
                match &target.kind {
                    ExprKind::Var(_, name) => {
                        self.var_map.insert(name.clone(), val);
                    }
                    ExprKind::Field(obj, field, _) => {
                        let obj_val = self.lower_expr(obj);
                        self.emit_void(InstKind::FieldSet(obj_val, field.clone(), val), target.span);
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
                let cond = self.lower_expr(&if_stmt.cond);
                let then_bb = self.new_block("if.then");
                let merge_bb = self.new_block("if.merge");
                let has_else = if_stmt.els.is_some();
                let else_bb = if has_else {
                    self.new_block("if.else")
                } else {
                    merge_bb
                };

                self.set_terminator(Terminator::Branch(cond, then_bb, else_bb));

                self.switch_to(then_bb);
                self.lower_block_stmts(&if_stmt.then);
                self.set_terminator(Terminator::Goto(merge_bb));

                if let Some(els) = &if_stmt.els {
                    self.switch_to(else_bb);
                    self.lower_block_stmts(els);
                    self.set_terminator(Terminator::Goto(merge_bb));
                }

                for (elif_cond, elif_body) in &if_stmt.elifs {
                    let elif_test = self.new_block("elif.test");
                    let elif_body_bb = self.new_block("elif.body");
                    self.switch_to(elif_test);
                    let c = self.lower_expr(elif_cond);
                    self.set_terminator(Terminator::Branch(c, elif_body_bb, merge_bb));

                    self.switch_to(elif_body_bb);
                    self.lower_block_stmts(elif_body);
                    self.set_terminator(Terminator::Goto(merge_bb));
                }

                self.switch_to(merge_bb);
                self.emit(InstKind::Void, Type::Void, if_stmt.span)
            }

            hir::Stmt::While(w) => {
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
                self.set_terminator(Terminator::Goto(cond_bb));
                self.loop_stack.pop();

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, w.span)
            }

            hir::Stmt::For(f) => {
                // TODO: proper iterator protocol — for now, lower body in a loop
                let _iter = self.lower_expr(&f.iter);
                let cond_bb = self.new_block("for.cond");
                let body_bb = self.new_block("for.body");
                let exit_bb = self.new_block("for.exit");

                self.set_terminator(Terminator::Goto(cond_bb));
                self.switch_to(cond_bb);
                let cond = self.emit(InstKind::BoolConst(false), Type::Bool, f.span);
                self.set_terminator(Terminator::Branch(cond, body_bb, exit_bb));

                self.loop_stack.push((cond_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&f.body);
                self.set_terminator(Terminator::Goto(cond_bb));
                self.loop_stack.pop();

                self.switch_to(exit_bb);
                self.emit(InstKind::Void, Type::Void, f.span)
            }

            hir::Stmt::Loop(l) => {
                let body_bb = self.new_block("loop.body");
                let exit_bb = self.new_block("loop.exit");

                self.set_terminator(Terminator::Goto(body_bb));
                self.loop_stack.push((body_bb, exit_bb));
                self.switch_to(body_bb);
                self.lower_block_stmts(&l.body);
                self.set_terminator(Terminator::Goto(body_bb));
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
                let subj = self.lower_expr(&m.subject);
                let merge_bb = self.new_block("match.merge");

                if m.arms.is_empty() {
                    self.switch_to(merge_bb);
                    return self.emit(InstKind::Void, Type::Void, m.span);
                }

                let mut next_test = self.current_block;
                for arm in &m.arms {
                    let arm_bb = self.new_block("match.arm");
                    let next_bb = self.new_block("match.next");

                    self.switch_to(next_test);
                    self.set_terminator(Terminator::Goto(arm_bb));

                    self.switch_to(arm_bb);
                    self.lower_block_stmts(&arm.body);
                    self.set_terminator(Terminator::Goto(merge_bb));

                    next_test = next_bb;
                }
                self.switch_to(next_test);
                self.set_terminator(Terminator::Goto(merge_bb));

                self.switch_to(merge_bb);
                let _ = subj;
                self.emit(InstKind::Void, Type::Void, m.span)
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

            _ => self.emit(InstKind::Void, Type::Void, Span::dummy()),
        }
    }

    fn lower_block_stmts(&mut self, stmts: &[hir::Stmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }
}

fn lower_function(f: &hir::Fn) -> Function {
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

    lowerer.func
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
        // Comparisons handled separately in lower_expr
        ast::BinOp::Eq | ast::BinOp::Ne | ast::BinOp::Lt
        | ast::BinOp::Gt | ast::BinOp::Le | ast::BinOp::Ge => BinOp::Add, // unreachable
    }
}

fn lower_unaryop(op: &ast::UnaryOp) -> UnaryOp {
    match op {
        ast::UnaryOp::Neg => UnaryOp::Neg,
        ast::UnaryOp::Not => UnaryOp::Not,
        ast::UnaryOp::BitNot => UnaryOp::BitNot,
    }
}
