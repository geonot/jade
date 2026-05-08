use super::super::*;
use super::Lowerer;
use crate::ast::Span;
use crate::hir;
use crate::types::Type;

impl Lowerer {
    pub(super) fn lower_stmt_effects(&mut self, stmt: &hir::Stmt) -> ValueId {
        match stmt {
            hir::Stmt::ChannelClose(ch, span) => {
                let c = self.lower_expr(ch);
                self.emit(
                    InstKind::Call("__chan_close".into(), vec![c]),
                    Type::Void,
                    *span,
                )
            }
            hir::Stmt::Stop(expr, span) => {
                let v = self.lower_expr(expr);
                self.emit(InstKind::Call("__stop".into(), vec![v]), Type::Void, *span)
            }
            hir::Stmt::Asm(asm) => {
                let input_vals: Vec<_> =
                    asm.inputs.iter().map(|(_, e)| self.lower_expr(e)).collect();
                self.emit(
                    InstKind::InlineAsm(asm.template.clone(), input_vals),
                    Type::Void,
                    asm.span,
                )
            }
            hir::Stmt::SimBlock(body, span) => {
                self.lower_block_stmts(body);
                self.emit(InstKind::Void, Type::Void, *span)
            }
            hir::Stmt::UseLocal(_, _, _, _) => {
                // No-op in MIR — use declarations are resolved at HIR level
                self.emit(InstKind::Void, Type::Void, Span::dummy())
            }
            hir::Stmt::GlobalStore(name, value, _span) => {
                let val = self.lower_expr(value);
                self.emit(
                    InstKind::GlobalStore(name.clone(), val),
                    Type::Void,
                    Span::dummy(),
                )
            }
            _ => unreachable!("statement dispatched to wrong MIR lowering module"),
        }
    }
}
