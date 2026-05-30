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
}
