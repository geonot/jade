use super::super::*;
use crate::ast::Span;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

pub(super) struct Lowerer {
    pub(super) func: Function,
    pub(super) current_block: BlockId,
    pub(super) var_map: HashMap<Symbol, ValueId>,
    pub(super) mem_vars: HashSet<Symbol>,
    pub(super) loop_stack: Vec<(BlockId, BlockId)>,
    pub(super) label_stack: Vec<(Symbol, BlockId, BlockId)>,
    pub(super) lambda_fns: Vec<Function>,
    pub(super) function_defers: Vec<crate::hir::Block>,
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
        };
        Lowerer {
            func,
            current_block: entry,
            var_map: HashMap::new(),
            mem_vars: HashSet::new(),
            loop_stack: Vec::new(),
            label_stack: Vec::new(),
            lambda_fns: Vec::new(),
            function_defers: Vec::new(),
        }
    }

    /// Lower all registered defer blocks at the current insertion point in
    /// LIFO order. Used before function exits to run cleanup code.
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
        self.func.new_block(label)
    }

    /// Recursively lower a field assignment on an SSA (non-mem_var) struct.
    /// For `o.inner.val is 42`:
    ///   inner = FieldGet(o, "inner")
    ///   updated_inner = FieldSet(inner, "val", 42)
    ///   updated_o = FieldSet(o, "inner", updated_inner)
    ///   var_map["o"] = updated_o
    pub(super) fn lower_field_assign(
        &mut self,
        obj: &hir::Expr,
        field: &str,
        val: ValueId,
        span: Span,
    ) {
        let obj_val = self.lower_expr(obj);
        let obj_ty = obj.ty.clone();
        let updated = self.emit(
            InstKind::FieldSet(obj_val, Symbol::intern(&field.to_string()), val),
            obj_ty.clone(),
            span,
        );
        // For pointer types, the FieldSet stores through the pointer and
        // returns the same pointer — the var_map entry must NOT be
        // updated.  Updating it creates an SSA value that is only
        // available in the current block, which breaks references from
        // blocks where FieldSet didn't execute (e.g. after a while loop).
        if matches!(obj_ty, Type::Ptr(_)) {
            return;
        }
        match &obj.kind {
            ExprKind::Var(_, name) => {
                self.var_map.insert(name.clone(), updated);
            }
            ExprKind::Field(parent, parent_field, _) => {
                // Propagate the update up: parent.parent_field = updated
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

    /// Emit an instruction with no destination but carrying a type annotation.
    /// Used for Store instructions so the variable type is preserved.
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
        self.func.block_mut(self.current_block).terminator = term;
    }

    pub(super) fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
    }

    pub(super) fn current_block_has_terminator(&self) -> bool {
        !matches!(
            self.func.block(self.current_block).terminator,
            Terminator::Unreachable
        )
    }

    /// Try to extract an integer constant from a ValueId by scanning the
    /// current function's instructions.
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

    /// Look up the type of a ValueId by scanning instructions and params.
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
        // All value types should be resolvable. If not, it indicates a compiler bug
        // in MIR lowering.
        panic!(
            "MIR lower: cannot resolve type for {:?} — this is a compiler bug",
            val
        )
    }
}
