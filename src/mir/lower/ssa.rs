//! Braun et al. 2013 — "Simple and Efficient Construction of Static Single
//! Assignment Form" — incremental SSA construction during MIR lowering.
//!
//! This is the sole variable model: `write_var` records a definition in the
//! per-block `current_def` map; `read_var` resolves a use, building phis on
//! demand (and recursively across predecessors / loop back-edges) via
//! `read_var_recursive`, `seal_block`, and `try_remove_trivial_phi`. Function
//! parameters, closure captures, and actor `self`/message params are seeded
//! into `current_def[entry]` at construction time, so entry-block reads
//! resolve directly without any side table.
//!
//! The auxiliary `var_types` map (on `Lowerer`) records each declared name's
//! type so capture/iterator/drop sites can test membership and recover the
//! element type for `read_var`; it never holds SSA values.

use super::super::*;
use super::Lowerer;
use crate::ast::Span;
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    /// Record a write of `val` to `name` in `block` (Braun per-block def map).
    /// Also records the variable's type in `var_types` so capture/iterator/drop
    /// sites can recover it without a separate value table.
    pub(super) fn write_var(&mut self, name: Symbol, block: BlockId, val: ValueId) {
        let ty = self.value_type(val);
        self.var_types.insert(name.clone(), ty);
        self.current_def
            .entry(block)
            .or_default()
            .insert(name, val);
    }

    /// Read the value of `name` as observed at `block`.
    ///
    /// `ty` is the static type of the variable; required for phi insertion.
    /// `span` is used only if we must synthesize a `Load` fallback (legacy path).
    pub(super) fn read_var(
        &mut self,
        name: Symbol,
        block: BlockId,
        ty: Type,
        span: Span,
    ) -> ValueId {
        if let Some(&v) = self
            .current_def
            .get(&block)
            .and_then(|m| m.get(&name))
        {
            return v;
        }

        // Braun recursive case.
        let v = self.read_var_recursive(name.clone(), block, ty.clone(), span);
        let v = self.resolve(v);
        self.current_def
            .entry(block)
            .or_default()
            .insert(name, v);
        v
    }

    /// Canonicalize a value id through the phi-collapse forwarding map.
    /// `try_remove_trivial_phi` records `removed_phi -> replacement`; following
    /// the chain yields the live value that survived all cascaded collapses.
    pub(super) fn resolve(&self, mut v: ValueId) -> ValueId {
        let mut guard = 0usize;
        while let Some(&next) = self.value_subst.get(&v) {
            if next == v {
                break;
            }
            v = next;
            guard += 1;
            if guard > 1_000_000 {
                break; // defensive: never spin on a corrupt cycle
            }
        }
        v
    }

    fn read_var_recursive(
        &mut self,
        name: Symbol,
        block: BlockId,
        ty: Type,
        span: Span,
    ) -> ValueId {
        // Block not yet sealed: insert an incomplete phi and record it for
        // later operand population at seal time.
        if !self.sealed_blocks.contains(&block) {
            let phi_dest = self.new_value();
            self.func.block_mut(block).phis.push(Phi {
                dest: phi_dest,
                ty: ty.clone(),
                incoming: Vec::new(),
            });
            self.incomplete_phis
                .entry(block)
                .or_default()
                .push((name, phi_dest, ty));
            return phi_dest;
        }

        let preds = self.preds.get(&block).cloned().unwrap_or_default();
        match preds.len() {
            0 => {
                // Sealed with no predecessors. Either the entry block (params
                // and captures are pre-seeded into `current_def[entry]`, so a
                // genuine variable never reaches here) or an unreachable block
                // created after a diverging statement. A read that lands here
                // is therefore either dead code or a lowering bug; synthesize
                // an undef-like `Load` so codegen of a truly unreachable path
                // stays harmless, while a reachable miss surfaces as bad codegen.
                let prev = self.current_block;
                self.current_block = block;
                let v = self.emit(InstKind::Load(name), ty, span);
                self.current_block = prev;
                v
            }
            1 => {
                // Single predecessor — no phi needed.
                let pred = preds[0];
                self.read_var(name, pred, ty, span)
            }
            _ => {
                // Multiple predecessors — insert a phi to break the recursion
                // cycle (important for loops), then fill operands and try to
                // collapse if trivial.
                let phi_dest = self.new_value();
                self.func.block_mut(block).phis.push(Phi {
                    dest: phi_dest,
                    ty: ty.clone(),
                    incoming: Vec::new(),
                });
                // Cache so recursive reads through cycles see this phi.
                self.current_def
                    .entry(block)
                    .or_default()
                    .insert(name.clone(), phi_dest);
                self.add_phi_operands(name, block, phi_dest, ty, span);
                let r = self.try_remove_trivial_phi(block, phi_dest).unwrap_or(phi_dest);
                self.resolve(r)
            }
        }
    }

    fn add_phi_operands(
        &mut self,
        name: Symbol,
        block: BlockId,
        phi_dest: ValueId,
        ty: Type,
        span: Span,
    ) {
        let preds = self.preds.get(&block).cloned().unwrap_or_default();
        let mut incoming = Vec::with_capacity(preds.len());
        for pred in preds {
            let v = self.read_var(name.clone(), pred, ty.clone(), span);
            incoming.push((pred, v));
        }
        // Find the phi we created and install the operands.
        if let Some(phi) = self
            .func
            .block_mut(block)
            .phis
            .iter_mut()
            .find(|p| p.dest == phi_dest)
        {
            phi.incoming = incoming;
        }
    }

    /// Try to collapse a phi whose operands all reference the same value
    /// (modulo self-references). Returns the replacement value, or `None`
    /// if the phi is genuinely needed.
    fn try_remove_trivial_phi(&mut self, block: BlockId, phi_dest: ValueId) -> Option<ValueId> {
        let same = {
            let phi = self
                .func
                .block(block)
                .phis
                .iter()
                .find(|p| p.dest == phi_dest)?;
            let mut same: Option<ValueId> = None;
            for &(_, v) in &phi.incoming {
                if v == phi_dest {
                    continue; // self-reference, skip
                }
                match same {
                    Some(s) if s == v => continue,
                    Some(_) => return None, // ≥2 distinct non-self operands
                    None => same = Some(v),
                }
            }
            same?
        };

        // Remove the phi from the block.
        self.func.block_mut(block).phis.retain(|p| p.dest != phi_dest);

        // Replace all uses of `phi_dest` with `same` throughout the function.
        // Also update current_def cache entries.
        self.replace_value_uses(phi_dest, same);

        // Recursively check phis that referenced this one — they may now be trivial.
        // Collect candidates first to avoid borrow issues.
        let candidates: Vec<(BlockId, ValueId)> = self
            .func
            .blocks
            .iter()
            .flat_map(|bb| {
                let bid = bb.id;
                bb.phis
                    .iter()
                    .filter(|p| p.incoming.iter().any(|&(_, v)| v == same))
                    .map(move |p| (bid, p.dest))
            })
            .collect();
        for (b, p) in candidates {
            let _ = self.try_remove_trivial_phi(b, p);
        }

        Some(same)
    }

    fn replace_value_uses(&mut self, old: ValueId, new: ValueId) {
        // Record the forwarding so any value already in flight up the SSA-read
        // recursion (or about to be returned) can be canonicalized via
        // `resolve`. `new` is canonicalized first to keep chains short.
        let new = self.resolve(new);
        if old != new {
            self.value_subst.insert(old, new);
        }
        let mut map = std::collections::HashMap::new();
        map.insert(old, new);
        for bb in &mut self.func.blocks {
            for phi in &mut bb.phis {
                for (_, v) in &mut phi.incoming {
                    if *v == old {
                        *v = new;
                    }
                }
            }
            for inst in &mut bb.insts {
                crate::mir::opt::subst::subst_inst(inst, &map);
            }
            crate::mir::opt::subst::subst_term(&mut bb.terminator, &map);
        }
        for defs in self.current_def.values_mut() {
            for v in defs.values_mut() {
                if *v == old {
                    *v = new;
                }
            }
        }
    }

    /// Mark `block` as sealed: its predecessor set is now finalized. Fill in
    /// operands for any phi previously inserted while the block was open.
    pub(super) fn seal_block(&mut self, block: BlockId) {
        if self.sealed_blocks.contains(&block) {
            return;
        }
        self.sealed_blocks.insert(block);

        let incomplete = self.incomplete_phis.remove(&block).unwrap_or_default();
        for (name, phi_dest, ty) in incomplete {
            self.add_phi_operands(name.clone(), block, phi_dest, ty, Span::dummy());
            if let Some(replacement) = self.try_remove_trivial_phi(block, phi_dest) {
                // Phi collapsed — update the cached def to point at the
                // replacement (canonicalized: a cascade may have moved it on).
                let replacement = self.resolve(replacement);
                if let Some(m) = self.current_def.get_mut(&block) {
                    if m.get(&name) == Some(&phi_dest) {
                        m.insert(name, replacement);
                    }
                }
            }
        }
    }
}
