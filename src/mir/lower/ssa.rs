use super::super::*;
use super::Lowerer;
use crate::ast::Span;
use crate::intern::Symbol;
use crate::types::Type;

impl Lowerer {
    pub(super) fn write_var(&mut self, name: Symbol, block: BlockId, val: ValueId) {
        let ty = self.value_type(val);
        self.var_types.insert(name.clone(), ty);
        self.current_def.entry(block).or_default().insert(name, val);
    }

    pub(super) fn read_var(
        &mut self,
        name: Symbol,
        block: BlockId,
        ty: Type,
        span: Span,
    ) -> ValueId {
        if let Some(&v) = self.current_def.get(&block).and_then(|m| m.get(&name)) {
            return v;
        }

        let v = self.read_var_recursive(name.clone(), block, ty.clone(), span);
        let v = self.resolve(v);
        self.current_def.entry(block).or_default().insert(name, v);
        v
    }

    pub(super) fn resolve(&self, mut v: ValueId) -> ValueId {
        let mut guard = 0usize;
        while let Some(&next) = self.value_subst.get(&v) {
            if next == v {
                break;
            }
            v = next;
            guard += 1;
            if guard > 1_000_000 {
                break;
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
                let prev = self.current_block;
                self.current_block = block;
                let v = self.emit(InstKind::Load(name), ty, span);
                self.current_block = prev;
                v
            }
            1 => {
                let pred = preds[0];
                self.read_var(name, pred, ty, span)
            }
            _ => {
                let phi_dest = self.new_value();
                self.func.block_mut(block).phis.push(Phi {
                    dest: phi_dest,
                    ty: ty.clone(),
                    incoming: Vec::new(),
                });

                self.current_def
                    .entry(block)
                    .or_default()
                    .insert(name.clone(), phi_dest);
                self.add_phi_operands(name, block, phi_dest, ty, span);
                let r = self
                    .try_remove_trivial_phi(block, phi_dest)
                    .unwrap_or(phi_dest);
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
                    continue;
                }
                match same {
                    Some(s) if s == v => continue,
                    Some(_) => return None,
                    None => same = Some(v),
                }
            }
            same?
        };

        self.func
            .block_mut(block)
            .phis
            .retain(|p| p.dest != phi_dest);

        self.replace_value_uses(phi_dest, same);

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

    pub(super) fn seal_block(&mut self, block: BlockId) {
        if self.sealed_blocks.contains(&block) {
            return;
        }
        self.sealed_blocks.insert(block);

        let incomplete = self.incomplete_phis.remove(&block).unwrap_or_default();
        for (name, phi_dest, ty) in incomplete {
            self.add_phi_operands(name.clone(), block, phi_dest, ty, Span::dummy());
            if let Some(replacement) = self.try_remove_trivial_phi(block, phi_dest) {
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
