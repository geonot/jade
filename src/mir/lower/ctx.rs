use super::super::*;
use crate::ast::Span;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

pub(super) struct FieldCtx {
    pub(super) self_state: ValueId,

    pub(super) state_ty: Type,

    pub(super) map: HashMap<crate::hir::DefId, (Symbol, Type)>,
}

pub(super) struct Lowerer {
    pub(super) func: Function,
    pub(super) current_block: BlockId,

    pub(super) var_types: HashMap<Symbol, Type>,
    pub(super) loop_stack: Vec<(BlockId, BlockId)>,
    pub(super) label_stack: Vec<(Symbol, BlockId, BlockId)>,
    pub(super) lambda_fns: Vec<Function>,
    pub(super) function_defers: Vec<crate::hir::Block>,

    pub(super) current_def: HashMap<BlockId, HashMap<Symbol, ValueId>>,

    pub(super) sealed_blocks: HashSet<BlockId>,

    pub(super) incomplete_phis: HashMap<BlockId, Vec<(Symbol, ValueId, Type)>>,

    pub(super) preds: HashMap<BlockId, Vec<BlockId>>,

    pub(super) value_subst: HashMap<ValueId, ValueId>,

    pub(super) unreachable_blocks: HashSet<BlockId>,

    pub(super) field_ctx: Option<FieldCtx>,
}

impl Lowerer {
    pub(super) fn new(name: &str, def_id: crate::hir::DefId, span: Span) -> Self {
        let entry = BlockId(0);
        let func = Function {
            name: name.into(),
            def_id,
            params: Vec::new(),
            ret_ty: Type::Void,
            blocks: vec![BasicBlock {
                id: entry,
                label: Symbol::intern("entry"),
                phis: Vec::new(),
                insts: Vec::new(),
                terminator: Terminator::Unreachable,
            }],
            entry,
            span,
            next_value: 0,
            next_block: 1,
            attrs: crate::ast::FnAttrs::default(),
            perceus: crate::mir::PerceusMeta::default(),
            is_coroutine: false,
        };
        Lowerer {
            func,
            current_block: entry,
            var_types: HashMap::new(),
            loop_stack: Vec::new(),
            label_stack: Vec::new(),
            lambda_fns: Vec::new(),
            function_defers: Vec::new(),
            current_def: {
                let mut m = HashMap::new();
                m.insert(entry, HashMap::new());
                m
            },
            sealed_blocks: {
                let mut s = HashSet::new();

                s.insert(entry);
                s
            },
            incomplete_phis: HashMap::new(),
            preds: {
                let mut p = HashMap::new();
                p.insert(entry, Vec::new());
                p
            },
            value_subst: HashMap::new(),
            unreachable_blocks: HashSet::new(),
            field_ctx: None,
        }
    }

    pub(super) fn field_lookup(&self, def_id: crate::hir::DefId) -> Option<(Symbol, Type)> {
        self.field_ctx
            .as_ref()
            .and_then(|fc| fc.map.get(&def_id).cloned())
    }

    pub(super) fn field_self(&self) -> ValueId {
        self.field_ctx
            .as_ref()
            .expect("field_self called outside handler context")
            .self_state
    }

    pub(super) fn field_state_ty(&self) -> Type {
        self.field_ctx
            .as_ref()
            .expect("field_state_ty called outside handler context")
            .state_ty
            .clone()
    }

    pub(super) fn lower_deferred_in_reverse(&mut self) {
        let defers: Vec<crate::hir::Block> = self.function_defers.clone();
        for block in defers.into_iter().rev() {
            for stmt in &block {
                let _ = self.lower_stmt(stmt);
            }
        }
    }

    pub(super) fn new_value(&mut self) -> ValueId {
        self.func.new_value()
    }

    pub(super) fn new_block(&mut self, label: &str) -> BlockId {
        let id = self.func.new_block(label);
        self.preds.entry(id).or_default();
        self.current_def.entry(id).or_default();
        id
    }

    pub(super) fn lower_field_assign(
        &mut self,
        obj: &hir::Expr,
        field: &str,
        val: ValueId,
        span: Span,
    ) {
        let obj_val = self.lower_expr(obj);
        let mut obj_ty = obj.ty.clone();

        if self.field_ctx.is_some() && obj_val == self.field_self() {
            obj_ty = self.field_state_ty();
            let st = obj_ty.clone();
            self.emit_void_typed(
                InstKind::FieldSet(obj_val, Symbol::intern(&field.to_string()), val),
                st,
                span,
            );
            return;
        }
        let updated = self.emit(
            InstKind::FieldSet(obj_val, Symbol::intern(&field.to_string()), val),
            obj_ty.clone(),
            span,
        );

        if matches!(obj_ty, Type::Ptr(_)) {
            return;
        }
        match &obj.kind {
            ExprKind::Var(_, name) => {
                self.write_var(name.clone(), self.current_block, updated);
            }
            ExprKind::Field(parent, parent_field, _) => {
                self.lower_field_assign(parent, &parent_field.as_str(), updated, span);
            }
            _ => {}
        }
    }

    pub(super) fn emit(&mut self, kind: InstKind, ty: Type, span: Span) -> ValueId {
        let dest = self.new_value();
        self.func
            .block_mut(self.current_block)
            .insts
            .push(Instruction {
                dest: Some(dest),
                kind,
                ty,
                span,
                def_id: None,
            });
        dest
    }

    pub(super) fn emit_void(&mut self, kind: InstKind, span: Span) {
        self.func
            .block_mut(self.current_block)
            .insts
            .push(Instruction {
                dest: None,
                kind,
                ty: Type::Void,
                span,
                def_id: None,
            });
    }

    pub(super) fn emit_void_typed(&mut self, kind: InstKind, ty: Type, span: Span) {
        self.func
            .block_mut(self.current_block)
            .insts
            .push(Instruction {
                dest: None,
                kind,
                ty,
                span,
                def_id: None,
            });
    }

    pub(super) fn set_terminator(&mut self, term: Terminator) {
        let block = self.current_block;

        if self.unreachable_blocks.contains(&block) {
            return;
        }

        let old_succs = self.func.block(block).terminator.successors();
        for s in old_succs {
            if let Some(v) = self.preds.get_mut(&s) {
                v.retain(|&b| b != block);
            }
        }

        let new_succs = term.successors();
        self.func.block_mut(block).terminator = term;
        for s in new_succs {
            self.preds.entry(s).or_default().push(block);
        }
    }

    pub(super) fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
    }

    pub(super) fn mark_dead_block(&mut self, block: BlockId) {
        self.unreachable_blocks.insert(block);
        self.seal_block(block);
    }

    pub(super) fn current_block_has_terminator(&self) -> bool {
        !matches!(
            self.func.block(self.current_block).terminator,
            Terminator::Unreachable
        )
    }

    pub(super) fn try_extract_int_const(&self, val: ValueId) -> Option<i64> {
        for bb in &self.func.blocks {
            for inst in &bb.insts {
                if inst.dest == Some(val) {
                    if let InstKind::IntConst(n) = &inst.kind {
                        return Some(*n);
                    }
                    if let InstKind::BoolConst(b) = &inst.kind {
                        return Some(*b as i64);
                    }
                    return None;
                }
            }
        }
        None
    }

    pub(super) fn value_type(&self, val: ValueId) -> Type {
        for p in &self.func.params {
            if p.value == val {
                return p.ty.clone();
            }
        }
        for bb in &self.func.blocks {
            for phi in &bb.phis {
                if phi.dest == val {
                    return phi.ty.clone();
                }
            }
            for inst in &bb.insts {
                if inst.dest == Some(val) {
                    return inst.ty.clone();
                }
            }
        }

        panic!(
            "MIR lower: cannot resolve type for {:?} — this is a compiler bug",
            val
        )
    }
}
