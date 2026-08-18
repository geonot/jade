use crate::hir;

mod fold;

use fold::{fold_block, fold_expr};

pub fn fold_program(prog: &mut hir::Program) {
    for f in &mut prog.fns {
        fold_block(&mut f.body);
    }
    for td in &mut prog.types {
        for m in &mut td.methods {
            fold_block(&mut m.body);
        }
    }
    for actor in &mut prog.actors {
        for m in &mut actor.handlers {
            fold_block(&mut m.body);
            if let Some(sleep_ms) = &mut m.loop_sleep_ms {
                fold_expr(sleep_ms);
            }
        }
    }
    for imp in &mut prog.trait_impls {
        for m in &mut imp.methods {
            fold_block(&mut m.body);
        }
    }
}
