//! Sub-pass of perceus/uses split.

use std::collections::HashMap;

use crate::hir::*;

use super::super::{PerceusPass, UseInfo};

impl PerceusPass {
    pub(in crate::perceus) fn count_uses_block(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        for stmt in block {
            self.count_uses_stmt(stmt, uses);
        }
    }

    pub(in crate::perceus) fn count_uses_stmt(&mut self, stmt: &Stmt, uses: &mut HashMap<DefId, UseInfo>) {
        match stmt {
            Stmt::Bind(b) => {
                self.count_uses_expr(&b.value, uses);
                uses.insert(b.def_id, UseInfo::new(b.ty.clone(), b.ownership));
                self.hints.stats.total_bindings_analyzed += 1;
            }
            Stmt::TupleBind(bindings, value, _) => {
                self.count_uses_expr(value, uses);
                for (def_id, _, ty) in bindings {
                    uses.insert(*def_id, UseInfo::new(ty.clone(), ty.default_ownership()));
                    self.hints.stats.total_bindings_analyzed += 1;
                }
            }
            Stmt::Assign(target, value, _) => {
                self.count_uses_expr(target, uses);
                self.count_uses_expr(value, uses);
            }
            Stmt::Expr(e) => {
                self.count_uses_expr(e, uses);
            }
            Stmt::If(i) => {
                self.count_uses_expr(&i.cond, uses);
                self.count_uses_block(&i.then, uses);
                for (ec, eb) in &i.elifs {
                    self.count_uses_expr(ec, uses);
                    self.count_uses_block(eb, uses);
                }
                if let Some(els) = &i.els {
                    self.count_uses_block(els, uses);
                }
            }
            Stmt::While(w) => {
                self.count_uses_expr(&w.cond, uses);
                self.count_uses_block_conservative(&w.body, uses);
            }
            Stmt::For(f) => {
                self.count_uses_expr(&f.iter, uses);
                if let Some(end) = &f.end {
                    self.count_uses_expr(end, uses);
                }
                if let Some(step) = &f.step {
                    self.count_uses_expr(step, uses);
                }
                uses.insert(f.bind_id, UseInfo::new(f.bind_ty.clone(), Ownership::Owned));
                if let (Some(id2), Some(ty2)) = (f.bind2_id, &f.bind2_ty) {
                    uses.insert(id2, UseInfo::new(ty2.clone(), Ownership::Owned));
                }
                self.count_uses_block_conservative(&f.body, uses);
            }
            Stmt::Loop(l) => {
                self.count_uses_block_conservative(&l.body, uses);
            }
            Stmt::Ret(val, _, _) => {
                if let Some(v) = val {
                    self.count_uses_expr_escaping(v, uses);
                }
            }
            Stmt::Break(val, _) => {
                if let Some(v) = val {
                    self.count_uses_expr(v, uses);
                }
            }
            Stmt::Continue(_) => {}
            Stmt::Match(m) => {
                self.count_uses_expr(&m.subject, uses);
                for arm in &m.arms {
                    self.count_uses_pat(&arm.pat, uses);
                    if let Some(ref g) = arm.guard {
                        self.count_uses_expr(g, uses);
                    }
                    self.count_uses_block(&arm.body, uses);
                }
            }
            Stmt::Asm(a) => {
                for (_, e) in &a.inputs {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::Drop(def_id, _, _, _) => {
                if let Some(info) = uses.get_mut(def_id) {
                    info.use_count += 1;
                }
            }
            Stmt::ErrReturn(e, _, _) => {
                self.count_uses_expr_escaping(e, uses);
            }
            Stmt::Defer(body, _) => self.count_uses_block(body, uses),
            Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::StoreDelete(_, _, _) => {}
            Stmt::StoreDestroy(_, _, _) => {}
            Stmt::StoreRestore(_, _, _) => {}
            Stmt::StoreSave(_, _) => {}
            Stmt::StoreSet(_, assigns, _, _) => {
                for (_, e) in assigns {
                    self.count_uses_expr_escaping(e, uses);
                }
            }
            Stmt::Transaction(body, _) => {
                self.count_uses_block(body, uses);
            }
            Stmt::ChannelClose(e, _) => {
                self.count_uses_expr(e, uses);
            }
            Stmt::Stop(e, _) => {
                self.count_uses_expr(e, uses);
            }
            Stmt::SimFor(f, _) => {
                self.count_uses_expr(&f.iter, uses);
                if let Some(end) = &f.end {
                    self.count_uses_expr(end, uses);
                }
                if let Some(step) = &f.step {
                    self.count_uses_expr(step, uses);
                }
                uses.insert(f.bind_id, UseInfo::new(f.bind_ty.clone(), Ownership::Owned));
                self.count_uses_block_conservative(&f.body, uses);
            }
            Stmt::SimBlock(b, _) => {
                self.count_uses_block(b, uses);
            }
            Stmt::UseLocal(_, _, _, _) => {}
            Stmt::GlobalStore(_, e, _) => {
                self.count_uses_expr_escaping(e, uses);
            }
        }
    }

    pub(in crate::perceus) fn count_uses_block_conservative(&mut self, block: &Block, uses: &mut HashMap<DefId, UseInfo>) {
        let mut refs = Vec::new();
        self.collect_refs_block(block, &mut refs);
        for def_id in &refs {
            if let Some(info) = uses.get_mut(def_id) {
                info.use_count = info.use_count.saturating_add(2);
                info.escapes = true;
            }
        }
        self.count_uses_block(block, uses);
    }

}
