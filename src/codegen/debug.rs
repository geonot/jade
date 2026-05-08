//! Debug-info helpers for source locations and local variable declarations.

use super::*;

impl<'ctx> Compiler<'ctx> {
    pub(in crate::codegen) fn finalize_debug(&self) {
        if let Some(ref di) = self.di_builder {
            di.finalize();
        }
    }

    pub(crate) fn pop_debug_scope(&mut self) {
        if self.debug {
            self.di_scope_stack.pop();
        }
    }

    pub(crate) fn set_debug_location(&self, line: u32, col: u32) {
        if !self.debug {
            return;
        }
        if let Some(scope) = self.di_scope_stack.last() {
            let di = ice!(
                self.di_builder.as_ref(),
                "debug info builder not initialized"
            );
            let loc = di.create_debug_location(self.ctx, line, col, *scope, None);
            self.bld.set_current_debug_location(loc);
        }
    }

    /// R15: emit `llvm.dbg.declare` for an alloca'd local so debuggers
    /// (lldb, gdb) can resolve `frame variable <name>`. Uses an opaque
    /// 64-bit basic type as a stand-in DIType — accurate enough for
    /// integers/pointers and gives the variable a name binding.
    /// No-op when debug info is disabled.
    pub(crate) fn attach_dbg_declare(&self, ptr: PointerValue<'ctx>, name: &str, line: u32) {
        if !self.debug {
            return;
        }
        let Some(ref di) = self.di_builder else {
            return;
        };
        let Some(scope) = self.di_scope_stack.last().copied() else {
            return;
        };
        let Some(ref cu) = self.di_compile_unit else {
            return;
        };
        let file = cu.get_file();
        // Use a generic 64-bit unsigned DI type. This is a stand-in:
        // proper per-Type DI metadata is a follow-up. lldb still prints
        // the address and bytes, which is the main thing the user gets.
        let di_ty = di.create_basic_type(
            "__jinn_local",
            64,
            0x07, /* DW_ATE_unsigned */
            DIFlags::PUBLIC,
        );
        let Ok(di_ty) = di_ty else { return };
        let var_info = di.create_auto_variable(
            scope,
            name,
            file,
            line,
            di_ty.as_type(),
            true,
            DIFlags::PUBLIC,
            0,
        );
        let loc = di.create_debug_location(self.ctx, line, 1, scope, None);
        let Some(bb) = self.bld.get_insert_block() else {
            return;
        };
        di.insert_declare_at_end(ptr, Some(var_info), None, loc, bb);
    }
}
