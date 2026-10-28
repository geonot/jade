use std::collections::{HashMap, HashSet};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Range {
    lo: i128,
    hi: i128,
}

impl Range {
    const EMPTY: Range = Range { lo: 1, hi: 0 };
    const FULL: Range = Range {
        lo: i128::MIN,
        hi: i128::MAX,
    };

    fn of(ty: &Type) -> Option<Range> {
        if !ty.is_int() {
            return None;
        }
        let bits = ty.bits();
        Some(if ty.is_unsigned() {
            Range {
                lo: 0,
                hi: (1i128 << bits) - 1,
            }
        } else {
            Range {
                lo: -(1i128 << (bits - 1)),
                hi: (1i128 << (bits - 1)) - 1,
            }
        })
    }

    fn point(n: i128) -> Range {
        Range { lo: n, hi: n }
    }

    fn is_empty(self) -> bool {
        self.lo > self.hi
    }

    fn join(self, o: Range) -> Range {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return self;
        }
        Range {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
        }
    }

    fn meet(self, o: Range) -> Range {
        Range {
            lo: self.lo.max(o.lo),
            hi: self.hi.min(o.hi),
        }
    }

    fn within(self, o: Range) -> bool {
        !self.is_empty() && self.lo >= o.lo && self.hi <= o.hi
    }

    fn fit(self, bound: Range) -> Range {
        if self.within(bound) { self } else { bound }
    }

    fn constant(n: i64, ty: &Type) -> Range {
        let bits = ty.bits();
        let mut m = (n as i128) & ((1i128 << bits) - 1);
        if ty.is_signed() && m >= 1i128 << (bits - 1) {
            m -= 1i128 << bits;
        }
        Range::point(m)
    }

    fn corners(a: Range, b: Range, f: impl Fn(i128, i128) -> i128) -> Range {
        if a.is_empty() || b.is_empty() {
            return Range::EMPTY;
        }
        let c = [f(a.lo, b.lo), f(a.lo, b.hi), f(a.hi, b.lo), f(a.hi, b.hi)];
        Range {
            lo: *c.iter().min().unwrap(),
            hi: *c.iter().max().unwrap(),
        }
    }
}

struct Analysis<'f> {
    func: &'f Function,
    tys: HashMap<ValueId, Type>,
    ranges: HashMap<ValueId, Range>,
    facts: HashMap<BlockId, Vec<(ValueId, CmpOp, ValueId)>>,
    order: Vec<BlockId>,
    live: HashSet<BlockId>,
}

pub fn proven_no_overflow(func: &Function) -> HashSet<ValueId> {
    let mut a = Analysis::new(func);
    if !a.solve() {
        return HashSet::new();
    }
    let mut out = HashSet::new();
    for &b in &a.order {
        for inst in &func.block(b).insts {
            let (Some(dest), Some(bound)) = (inst.dest, Range::of(&inst.ty)) else {
                continue;
            };
            let same = |v: &ValueId| a.tys.get(v) == Some(&inst.ty);
            let exact = match inst.kind {
                InstKind::BinOp(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), l, r)
                    if same(&l) && same(&r) =>
                {
                    a.exact(op, a.refined(l, b), a.refined(r, b))
                }
                InstKind::UnaryOp(UnaryOp::Neg, v) if same(&v) => a.negate(a.refined(v, b)),
                _ => continue,
            };
            if exact.within(bound) {
                out.insert(dest);
            }
        }
    }
    out
}

impl<'f> Analysis<'f> {
    fn new(func: &'f Function) -> Self {
        let mut tys = HashMap::new();
        for p in &func.params {
            tys.insert(p.value, p.ty.clone());
        }
        for bb in &func.blocks {
            for phi in &bb.phis {
                tys.insert(phi.dest, phi.ty.clone());
            }
            for inst in &bb.insts {
                if let Some(d) = inst.dest {
                    tys.insert(d, inst.ty.clone());
                }
            }
        }
        let order = reverse_postorder(func);
        let facts = branch_facts(func, &order, &tys);
        Analysis {
            func,
            tys,
            ranges: HashMap::new(),
            facts,
            live: order.iter().copied().collect(),
            order,
        }
    }

    fn range(&self, v: ValueId) -> Range {
        match self.ranges.get(&v) {
            Some(r) => *r,
            None => self.tys.get(&v).and_then(Range::of).unwrap_or(Range::FULL),
        }
    }

    fn refined(&self, v: ValueId, b: BlockId) -> Range {
        let mut r = self.range(v);
        for &(x, op, y) in self.facts.get(&b).map(Vec::as_slice).unwrap_or(&[]) {
            let (op, other) = if x == v {
                (op, y)
            } else if y == v {
                (flip(op), x)
            } else {
                continue;
            };
            let o = self.range(other);
            if o.is_empty() {
                continue;
            }
            r = r.meet(match op {
                CmpOp::Lt => Range {
                    lo: i128::MIN,
                    hi: o.hi - 1,
                },
                CmpOp::Le => Range {
                    lo: i128::MIN,
                    hi: o.hi,
                },
                CmpOp::Gt => Range {
                    lo: o.lo + 1,
                    hi: i128::MAX,
                },
                CmpOp::Ge => Range {
                    lo: o.lo,
                    hi: i128::MAX,
                },
                CmpOp::Eq => o,
                CmpOp::Ne => continue,
            });
        }
        r
    }

    fn exact(&self, op: BinOp, l: Range, r: Range) -> Range {
        match op {
            BinOp::Add => Range::corners(l, r, i128::saturating_add),
            BinOp::Sub => Range::corners(l, r, i128::saturating_sub),
            _ => Range::corners(l, r, i128::saturating_mul),
        }
    }

    fn negate(&self, r: Range) -> Range {
        if r.is_empty() {
            r
        } else {
            Range {
                lo: -r.hi,
                hi: -r.lo,
            }
        }
    }

    fn transfer(&self, inst: &Instruction, b: BlockId, bound: Range) -> Range {
        let same = |v: &ValueId| self.tys.get(v) == Some(&inst.ty);
        let r = match inst.kind {
            InstKind::IntConst(n) => Range::constant(n, &inst.ty),
            InstKind::Copy(v) => self.refined(v, b).fit(bound),
            InstKind::VecLen(_) => Range {
                lo: 0,
                hi: i64::MAX as i128,
            },
            InstKind::Cast(v, _) | InstKind::StrictCast(v, _)
                if self.tys.get(&v).is_some_and(Type::is_int) =>
            {
                self.refined(v, b).fit(bound)
            }
            InstKind::UnaryOp(UnaryOp::Neg, v) if same(&v) => {
                self.negate(self.refined(v, b)).fit(bound)
            }
            InstKind::BinOp(op, l, r) if same(&l) && same(&r) => {
                let (l, r) = (self.refined(l, b), self.refined(r, b));
                if l.is_empty() || r.is_empty() {
                    return Range::EMPTY;
                }
                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul => self.exact(op, l, r),
                    BinOp::Div if r.lo > 0 || r.hi < 0 => Range::corners(l, r, |a, b| a / b),
                    BinOp::Mod if r.lo > 0 => {
                        let m = r.hi - 1;
                        if l.lo >= 0 {
                            Range {
                                lo: 0,
                                hi: l.hi.min(m),
                            }
                        } else if l.hi <= 0 {
                            Range {
                                lo: l.lo.max(-m),
                                hi: 0,
                            }
                        } else {
                            Range { lo: -m, hi: m }
                        }
                    }
                    BinOp::BitAnd if l.lo >= 0 && r.lo >= 0 => Range {
                        lo: 0,
                        hi: l.hi.min(r.hi),
                    },
                    BinOp::BitAnd if l.lo >= 0 => Range { lo: 0, hi: l.hi },
                    BinOp::BitAnd if r.lo >= 0 => Range { lo: 0, hi: r.hi },
                    BinOp::BitOr | BinOp::BitXor if l.lo >= 0 && r.lo >= 0 => {
                        let top = l.hi.max(r.hi);
                        let bits = 128 - top.leading_zeros();
                        Range {
                            lo: 0,
                            hi: (1i128 << bits) - 1,
                        }
                    }
                    BinOp::Shr | BinOp::Ushr if l.lo >= 0 && r.lo >= 0 && r.hi < 64 => Range {
                        lo: l.lo >> r.hi,
                        hi: l.hi >> r.lo,
                    },
                    _ => bound,
                }
            }
            _ => bound,
        };
        if r.is_empty() { r } else { r.meet(bound) }
    }

    fn sweep(&mut self, widen: Option<&mut HashMap<ValueId, u32>>) -> bool {
        let mut widen = widen;
        let mut changed = false;
        let func = self.func;
        for &b in &self.order.clone() {
            let bb = func.block(b);
            for phi in &bb.phis {
                let Some(bound) = Range::of(&phi.ty) else {
                    continue;
                };
                let mut r = Range::EMPTY;
                for &(p, v) in &phi.incoming {
                    if self.live.contains(&p) {
                        r = r.join(self.refined(v, p));
                    }
                }
                r = if r.is_empty() { r } else { r.meet(bound) };
                let old = self.ranges.get(&phi.dest).copied().unwrap_or(Range::EMPTY);
                if let Some(counts) = widen.as_deref_mut() {
                    r = r.join(old);
                    if r != old && !old.is_empty() {
                        let n = counts.entry(phi.dest).or_default();
                        *n += 1;
                        if *n > 2 {
                            if r.lo < old.lo {
                                r.lo = bound.lo;
                            }
                            if r.hi > old.hi {
                                r.hi = bound.hi;
                            }
                        }
                    }
                }
                if r != old {
                    self.ranges.insert(phi.dest, r);
                    changed = true;
                }
            }
            for inst in &bb.insts {
                let (Some(d), Some(bound)) = (inst.dest, Range::of(&inst.ty)) else {
                    continue;
                };
                let mut r = self.transfer(inst, b, bound);
                let old = self.ranges.get(&d).copied().unwrap_or(Range::EMPTY);
                if widen.is_some() {
                    r = r.join(old);
                }
                if r != old {
                    self.ranges.insert(d, r);
                    changed = true;
                }
            }
        }
        changed
    }

    fn solve(&mut self) -> bool {
        for &b in &self.order {
            for phi in &self.func.block(b).phis {
                if Range::of(&phi.ty).is_some() {
                    self.ranges.insert(phi.dest, Range::EMPTY);
                }
            }
            for inst in &self.func.block(b).insts {
                if let Some(d) = inst.dest
                    && Range::of(&inst.ty).is_some()
                {
                    self.ranges.insert(d, Range::EMPTY);
                }
            }
        }
        let mut counts = HashMap::new();
        let mut stable = false;
        for _ in 0..64 {
            if !self.sweep(Some(&mut counts)) {
                stable = true;
                break;
            }
        }
        if !stable {
            return false;
        }
        for _ in 0..3 {
            if !self.sweep(None) {
                break;
            }
        }
        true
    }
}

fn flip(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Gt,
        CmpOp::Gt => CmpOp::Lt,
        CmpOp::Le => CmpOp::Ge,
        CmpOp::Ge => CmpOp::Le,
        o => o,
    }
}

fn negate(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Lt => CmpOp::Ge,
        CmpOp::Ge => CmpOp::Lt,
        CmpOp::Gt => CmpOp::Le,
        CmpOp::Le => CmpOp::Gt,
        CmpOp::Eq => CmpOp::Ne,
        CmpOp::Ne => CmpOp::Eq,
    }
}

fn reverse_postorder(func: &Function) -> Vec<BlockId> {
    let mut seen = HashSet::new();
    let mut post = Vec::new();
    let mut stack = vec![(func.entry, 0usize)];
    seen.insert(func.entry);
    while let Some((b, i)) = stack.pop() {
        let succs = func.block(b).terminator.successors();
        if i < succs.len() {
            stack.push((b, i + 1));
            if seen.insert(succs[i]) {
                stack.push((succs[i], 0));
            }
        } else {
            post.push(b);
        }
    }
    post.reverse();
    post
}

fn dominators(func: &Function, order: &[BlockId]) -> HashMap<BlockId, BlockId> {
    let index: HashMap<BlockId, usize> = order.iter().enumerate().map(|(i, b)| (*b, i)).collect();
    let preds = func.predecessors();
    let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
    idom.insert(func.entry, func.entry);
    let intersect = |idom: &HashMap<BlockId, BlockId>, mut a: BlockId, mut b: BlockId| {
        while a != b {
            while index[&a] > index[&b] {
                a = idom[&a];
            }
            while index[&b] > index[&a] {
                b = idom[&b];
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for &b in order.iter().skip(1) {
            let mut new: Option<BlockId> = None;
            for p in preds.get(&b).map(Vec::as_slice).unwrap_or(&[]) {
                if !idom.contains_key(p) {
                    continue;
                }
                new = Some(match new {
                    None => *p,
                    Some(n) => intersect(&idom, *p, n),
                });
            }
            if let Some(n) = new
                && idom.get(&b) != Some(&n)
            {
                idom.insert(b, n);
                changed = true;
            }
        }
    }
    idom
}

fn branch_facts(
    func: &Function,
    order: &[BlockId],
    tys: &HashMap<ValueId, Type>,
) -> HashMap<BlockId, Vec<(ValueId, CmpOp, ValueId)>> {
    let idom = dominators(func, order);
    let preds = func.predecessors();
    let mut cmps: HashMap<ValueId, (CmpOp, ValueId, ValueId, bool)> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            let Some(d) = inst.dest else { continue };
            match inst.kind {
                InstKind::Cmp(op, l, r, ref ty)
                    if ty.is_int() && tys.get(&l) == Some(ty) && tys.get(&r) == Some(ty) =>
                {
                    cmps.insert(d, (op, l, r, false));
                }
                InstKind::UnaryOp(UnaryOp::Not, v) => {
                    if let Some(&(op, l, r, neg)) = cmps.get(&v) {
                        cmps.insert(d, (op, l, r, !neg));
                    }
                }
                _ => {}
            }
        }
    }
    let mut facts: HashMap<BlockId, Vec<(ValueId, CmpOp, ValueId)>> = HashMap::new();
    for &b in order {
        let mut f = match idom.get(&b) {
            Some(&d) if d != b => facts.get(&d).cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        if b != func.entry
            && let [p] = preds.get(&b).map(Vec::as_slice).unwrap_or(&[])
            && let Terminator::Branch(c, t, e) = func.block(*p).terminator
            && t != e
            && let Some(&(op, l, r, neg)) = cmps.get(&c)
        {
            let op = if (b == e) != neg { negate(op) } else { op };
            f.push((l, op, r));
        }
        facts.insert(b, f);
    }
    facts
}
