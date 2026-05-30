use super::super::*;
use crate::ast::Span;
use crate::hir::{self, ExprKind};
use crate::intern::Symbol;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

/// Context active while lowering an actor handler body. Actor fields are
/// referenced in handler bodies as bare `Var`s carrying the field's canonical
/// `DefId` (see `hir::ActorDef::field_def_ids`). Those reads/writes must be
/// redirected to load/store through the actor's heap-allocated state struct
/// (pointed to by `self_state`) rather than treated as SSA locals, because
/// the state persists across handler invocations.
pub(super) struct FieldCtx {
    /// The handler's first parameter: a pointer to the actor state struct.
    pub(super) self_state: ValueId,
    /// `Type::Struct(<actor>_state, _)` — the type tag used so codegen can
    /// resolve the struct layout for `FieldGet`/`FieldSet`.
    pub(super) state_ty: Type,
    /// Maps each field's canonical `DefId` to its (name, type).
    pub(super) map: HashMap<crate::hir::DefId, (Symbol, Type)>,
}

pub(super) struct Lowerer {
    pub(super) func: Function,
    pub(super) current_block: BlockId,
    // Declared local/param names -> their type. Tracks membership ("is this a
    // known local?") and supplies the element type to `read_var` at capture,
    // iterator, and drop sites where the type is not otherwise in hand. The SSA
    // *value* of each name lives in `current_def` (Braun 2013); this map holds
    // names and types only, never values.
    pub(super) var_types: HashMap<Symbol, Type>,
    pub(super) loop_stack: Vec<(BlockId, BlockId)>,
    pub(super) label_stack: Vec<(Symbol, BlockId, BlockId)>,
    pub(super) lambda_fns: Vec<Function>,
    pub(super) function_defers: Vec<crate::hir::Block>,
    // Braun et al. SSA construction state:
    // Per-block, per-variable definitions written during lowering.
    pub(super) current_def: HashMap<BlockId, HashMap<Symbol, ValueId>>,
    // Blocks whose set of predecessors is finalized — safe to look back through.
    pub(super) sealed_blocks: HashSet<BlockId>,
    // Phis created in unsealed blocks awaiting operand completion at seal time.
    // (name, phi.dest, value type)
    pub(super) incomplete_phis: HashMap<BlockId, Vec<(Symbol, ValueId, Type)>>,
    // Incrementally maintained reverse-edge map.
    pub(super) preds: HashMap<BlockId, Vec<BlockId>>,
    // Value-forwarding map for phis collapsed by `try_remove_trivial_phi`.
    // When a trivial phi `p` is replaced by `same`, we record `p -> same`.
    // A cascade may later replace `same` itself, so any value returned up the
    // SSA-read recursion must be canonicalized through `resolve` before it is
    // cached or installed as a phi operand — otherwise a stale (removed) id
    // leaks into live phis as an "undefined value".
    pub(super) value_subst: HashMap<ValueId, ValueId>,
    // Blocks that are unreachable continuations created after a diverging
    // statement (`after.ret` / `after.break` / `after.continue` /
    // `after.err_return`). They must never be wired into the CFG as live
    // predecessors of merge blocks: `set_terminator` is a no-op for them, so
    // they stay `Unreachable` and do not contribute spurious phi operands.
    pub(super) unreachable_blocks: HashSet<BlockId>,
    // Set only while lowering an actor handler body; redirects field
    // reads/writes to the actor state struct. `None` for normal functions.
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
                // Entry has no predecessors and is implicitly sealed at function start.
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

    /// If `def_id` names an actor field in the active handler context, returns
    /// its `(field_name, field_type)`. `None` for normal functions or for
    /// `Var`s that resolve to handler params / locals (which shadow fields by
    /// `DefId`, so the lookup correctly misses).
    pub(super) fn field_lookup(&self, def_id: crate::hir::DefId) -> Option<(Symbol, Type)> {
        self.field_ctx
            .as_ref()
            .and_then(|fc| fc.map.get(&def_id).cloned())
    }

    /// The `self_state` pointer value for the active handler context.
    pub(super) fn field_self(&self) -> ValueId {
        self.field_ctx
            .as_ref()
            .expect("field_self called outside handler context")
            .self_state
    }

    /// The actor state struct type tag for `FieldSet` instructions.
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
        // In an actor handler, a write through the `self` state pointer (the
        // explicit-self form `self.field = v`) must carry the actor state
        // struct type so codegen can resolve the layout — the HIR type of
        // `self` may be the actor type rather than `<actor>_state`.
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
        // A dead continuation block (created after a diverging `return`/`break`/
        // `continue`) is unreachable by construction: nothing branches to it.
        // Enclosing control-flow lowering (if/loop/match/select) unconditionally
        // tries to wire the "current" block into its merge with a `Goto`, but
        // doing so here would make the dead block a live predecessor of the
        // merge and inject undefined phi operands during SSA construction.
        // Leave it `Unreachable` and record no edges.
        if self.unreachable_blocks.contains(&block) {
            return;
        }
        // Remove this block from the predecessor lists of any current successors.
        let old_succs = self.func.block(block).terminator.successors();
        for s in old_succs {
            if let Some(v) = self.preds.get_mut(&s) {
                v.retain(|&b| b != block);
            }
        }
        // Install new terminator and register the new edges.
        let new_succs = term.successors();
        self.func.block_mut(block).terminator = term;
        for s in new_succs {
            self.preds.entry(s).or_default().push(block);
        }
    }

    pub(super) fn switch_to(&mut self, block: BlockId) {
        self.current_block = block;
    }

    /// Mark `block` as an unreachable continuation created after a diverging
    /// statement. It will never gain predecessors, so we seal it immediately:
    /// any variable reads in dead code lowered into it then resolve through the
    /// zero-predecessor path (a synthesized load) instead of leaving dangling
    /// incomplete phis that would later collapse to undefined values.
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
