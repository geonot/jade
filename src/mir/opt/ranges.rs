use super::super::*;
use std::collections::HashMap;

pub struct IntRange {
    pub lo: i64,
    pub hi: i64,
}

impl IntRange {
    pub const FULL: Self = Self {
        lo: i64::MIN,
        hi: i64::MAX,
    };
    pub fn constant(n: i64) -> Self {
        Self { lo: n, hi: n }
    }
    pub fn is_non_negative(&self) -> bool {
        self.lo >= 0
    }

    pub fn intersect(self, other: Self) -> Option<Self> {
        let lo = self.lo.max(other.lo);
        let hi = self.hi.min(other.hi);
        if lo <= hi {
            Some(Self { lo, hi })
        } else {
            None
        }
    }
}

pub fn compute_ranges(func: &Function) -> HashMap<ValueId, IntRange> {
    let mut ranges: HashMap<ValueId, IntRange> = HashMap::new();
    for bb in &func.blocks {
        for inst in &bb.insts {
            if let Some(d) = inst.dest {
                let r = match &inst.kind {
                    InstKind::IntConst(n) => Some(IntRange::constant(*n)),
                    InstKind::BinOp(BinOp::Add, l, r) => match (ranges.get(l), ranges.get(r)) {
                        (Some(a), Some(b)) => Some(IntRange {
                            lo: a.lo.saturating_add(b.lo),
                            hi: a.hi.saturating_add(b.hi),
                        }),
                        _ => None,
                    },
                    InstKind::BinOp(BinOp::Mul, l, r) => match (ranges.get(l), ranges.get(r)) {
                        (Some(a), Some(b)) => {
                            let ps = [
                                a.lo.saturating_mul(b.lo),
                                a.lo.saturating_mul(b.hi),
                                a.hi.saturating_mul(b.lo),
                                a.hi.saturating_mul(b.hi),
                            ];
                            Some(IntRange {
                                lo: *ps.iter().min().unwrap(),
                                hi: *ps.iter().max().unwrap(),
                            })
                        }
                        _ => None,
                    },
                    InstKind::BinOp(BinOp::Sub, l, r) => match (ranges.get(l), ranges.get(r)) {
                        (Some(a), Some(b)) => Some(IntRange {
                            lo: a.lo.saturating_sub(b.hi),
                            hi: a.hi.saturating_sub(b.lo),
                        }),
                        _ => None,
                    },
                    InstKind::BinOp(BinOp::BitAnd, l, r) => match (ranges.get(l), ranges.get(r)) {
                        (Some(a), Some(b)) if a.is_non_negative() && b.is_non_negative() => {
                            Some(IntRange {
                                lo: 0,
                                hi: a.hi.min(b.hi),
                            })
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(range) = r {
                    ranges.insert(d, range);
                }
            }
        }
    }
    ranges
}
