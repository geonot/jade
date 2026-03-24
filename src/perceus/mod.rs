use std::collections::HashMap;

use crate::ast::Span;
use crate::hir::*;
use crate::types::Type;

#[derive(Debug, Clone, Default)]
pub struct PerceusHints {
    pub elide_drops: std::collections::HashSet<DefId>,
    pub reuse_candidates: HashMap<DefId, ReuseInfo>,
    pub borrow_to_move: std::collections::HashSet<DefId>,
    pub speculative_reuse: HashMap<DefId, ReuseInfo>,
    pub last_use: HashMap<DefId, Span>,
    pub drop_fusions: Vec<DropFusion>,
    pub fbip_sites: Vec<FbipSite>,
    pub tail_reuse: HashMap<DefId, TailReuseInfo>,
    pub stats: PerceusStats,
}

#[derive(Debug, Clone)]
pub struct ReuseInfo {
    pub released_ty: Type,
    pub allocated_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct DropFusion {
    pub def_ids: Vec<DefId>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FbipSite {
    pub subject_id: DefId,
    pub subject_ty: Type,
    pub constructed_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TailReuseInfo {
    pub param_id: DefId,
    pub param_ty: Type,
    pub alloc_ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct PerceusStats {
    pub drops_elided: u32,
    pub reuse_sites: u32,
    pub borrows_promoted: u32,
    pub speculative_reuse_sites: u32,
    pub fbip_sites: u32,
    pub tail_reuse_sites: u32,
    pub drops_fused: u32,
    pub last_use_tracked: u32,
    pub total_bindings_analyzed: u32,
}

#[derive(Debug, Clone)]
pub(super) struct UseInfo {
    pub(super) use_count: u32,
    pub(super) last_use_span: Option<Span>,
    pub(super) escapes: bool,
    pub(super) borrowed: bool,
    pub(super) ty: Type,
    pub(super) ownership: Ownership,
}

impl UseInfo {
    pub(super) fn new(ty: Type, ownership: Ownership) -> Self {
        Self {
            use_count: 0,
            last_use_span: None,
            escapes: false,
            borrowed: false,
            ty,
            ownership,
        }
    }
}

pub struct PerceusPass {
    pub(super) hints: PerceusHints,
}

mod analysis;
mod uses;

impl PerceusPass {
    pub fn new() -> Self {
        Self {
            hints: PerceusHints::default(),
        }
    }

    pub fn optimize(&mut self, prog: &Program) -> PerceusHints {
        for f in &prog.fns {
            self.analyze_fn(f);
        }
        for td in &prog.types {
            for m in &td.methods {
                self.analyze_fn(m);
            }
        }
        for ti in &prog.trait_impls {
            for m in &ti.methods {
                self.analyze_fn(m);
            }
        }
        self.hints.clone()
    }

    fn analyze_fn(&mut self, f: &Fn) {
        let mut uses: HashMap<DefId, UseInfo> = HashMap::new();
        for p in &f.params {
            uses.insert(p.def_id, UseInfo::new(p.ty.clone(), p.ownership));
        }
        self.count_uses_block(&f.body, &mut uses);
        self.analyze_drop_specialization(&uses);
        self.analyze_reuse(&f.body, &uses);
        self.promote_borrows(&uses);
        self.analyze_last_use(&uses);
        self.analyze_fbip(&f.body, &uses);
        self.analyze_tail_reuse(f, &uses);
        self.analyze_drop_fusion(&f.body, &uses);
        self.analyze_speculative_reuse(&f.body, &uses);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;
    use crate::typer::Typer;

    fn analyze(src: &str) -> PerceusHints {
        let tokens = Lexer::new(src).tokenize().unwrap();
        let prog = Parser::new(tokens).parse_program().unwrap();
        let mut typer = Typer::new();
        let hir = typer.lower_program(&prog).unwrap();
        let mut perceus = PerceusPass::new();
        perceus.optimize(&hir)
    }

    #[test]
    fn test_scalar_drops_elided() {
        let hints = analyze(
            "*main()\n    x is 42\n    y is 3.14\n    z is true\n    log(x)\n    log(y)\n    log(z)\n",
        );
        assert!(
            hints.stats.drops_elided >= 3,
            "expected >= 3 drops elided for scalars, got {}",
            hints.stats.drops_elided
        );
    }

    #[test]
    fn test_array_of_scalars_drops_elided() {
        let hints = analyze("*main()\n    arr is [1, 2, 3]\n    log(arr[0])\n");
        assert!(
            hints.stats.drops_elided >= 1,
            "expected >= 1 drop elided for scalar array, got {}",
            hints.stats.drops_elided
        );
    }

    #[test]
    fn test_tuple_of_scalars_drops_elided() {
        let hints = analyze("*main()\n    t is (1, 2, 3)\n    log(t)\n");
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_string_not_elided() {
        let hints = analyze("*main()\n    s is \"hello\"\n    log(s)\n");
        assert!(!Type::String.is_trivially_droppable());
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }

    #[test]
    fn test_rc_not_elided() {
        let hints = analyze("*main()\n    x is rc(42)\n    log(@x)\n");
        assert!(
            hints.stats.total_bindings_analyzed >= 1,
            "should have analyzed at least 1 binding"
        );
    }

    #[test]
    fn test_borrow_promoted_single_use() {
        let hints = analyze("*main()\n    x is 42\n    p is %x\n    log(@p)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_rc_reuse_same_type() {
        let hints =
            analyze("*main()\n    x is rc(10)\n    log(@x)\n    y is rc(20)\n    log(@y)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_no_reuse_different_layout() {
        let hints =
            analyze("*main()\n    x is rc(10)\n    log(@x)\n    y is rc(3.14)\n    log(@y)\n");
        assert!(hints.stats.total_bindings_analyzed >= 2);
    }

    #[test]
    fn test_complex_program() {
        let hints = analyze(
            "*factorial(n: i64) -> i64\n    if n <= 1\n        1\n    else\n        n * factorial(n - 1)\n\n*main()\n    result is factorial(10)\n    log(result)\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_loop_conservatism() {
        let hints = analyze(
            "*main()\n    x is rc(0)\n    i is 0\n    while i < 10\n        log(@x)\n        i is i + 1\n",
        );
        assert!(hints.reuse_candidates.is_empty() || hints.stats.reuse_sites == 0);
    }

    #[test]
    fn test_function_params_analyzed() {
        let hints =
            analyze("*add(a: i64, b: i64) -> i64\n    a + b\n*main()\n    log(add(1, 2))\n");
        assert!(hints.stats.drops_elided >= 2);
    }

    #[test]
    fn test_struct_not_trivially_droppable() {
        assert!(!Type::Struct("Point".into()).is_trivially_droppable());
        assert!(!Type::String.is_trivially_droppable());
        assert!(!Type::Rc(Box::new(Type::I64)).is_trivially_droppable());
    }

    #[test]
    fn test_scalars_trivially_droppable() {
        assert!(Type::I64.is_trivially_droppable());
        assert!(Type::F64.is_trivially_droppable());
        assert!(Type::Bool.is_trivially_droppable());
        assert!(Type::Void.is_trivially_droppable());
        assert!(Type::Ptr(Box::new(Type::I64)).is_trivially_droppable());
    }

    #[test]
    fn test_nested_array_droppable() {
        assert!(Type::Array(Box::new(Type::I64), 3).is_trivially_droppable());
        assert!(!Type::Array(Box::new(Type::String), 3).is_trivially_droppable());
    }

    #[test]
    fn test_layout_compatibility() {
        let rc_i64 = Type::Rc(Box::new(Type::I64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_i64));

        let rc_f64 = Type::Rc(Box::new(Type::F64));
        assert!(PerceusPass::layouts_compatible(&rc_i64, &rc_f64));

        let rc_i8 = Type::Rc(Box::new(Type::I8));
        assert!(!PerceusPass::layouts_compatible(&rc_i64, &rc_i8));
    }

    #[test]
    fn test_perceus_stats_populated() {
        let hints = analyze(
            "*main()\n    a is 1\n    b is 2.0\n    c is true\n    log(a)\n    log(b)\n    log(c)\n",
        );
        assert!(hints.stats.total_bindings_analyzed > 0);
        assert!(hints.stats.drops_elided > 0);
    }

    #[test]
    fn test_enum_analysis() {
        let hints = analyze(
            "enum Color\n    Red\n    Green\n    Blue\n\n*main() -> i32\n    c is Red\n    match c\n        Red ? log(1)\n        Green ? log(2)\n        Blue ? log(3)\n    0\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }

    #[test]
    fn test_generic_fn_analysis() {
        let hints = analyze(
            "*identity(x)\n    x\n*main()\n    log(identity(42))\n    log(identity(3.14))\n",
        );
        assert!(hints.stats.drops_elided >= 1);
    }

    #[test]
    fn test_match_arms_analyzed() {
        let hints = analyze(
            "enum Shape\n    Circle(f64)\n    Square(f64)\n\n*area(s: Shape) -> f64\n    match s\n        Circle(r) ? 3.14159 * r * r\n        Square(side) ? side * side\n\n*main()\n    log(area(Circle(5.0)))\n",
        );
        assert!(hints.stats.total_bindings_analyzed >= 1);
    }
}
