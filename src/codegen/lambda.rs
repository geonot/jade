use std::collections::HashMap;
use std::collections::HashSet;

use inkwell::module::Linkage;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::BasicValueEnum;

use crate::hir;
use crate::types::Type;

use super::Compiler;
use super::b;

impl<'ctx> Compiler<'ctx> {
    pub(crate) fn compile_lambda(
        &mut self,
        params: &[hir::Param],
        body: &hir::Block,
        fn_ty: &Type,
    ) -> Result<BasicValueEnum<'ctx>, String> {
        let (ptys, ret_ty) = match fn_ty {
            Type::Fn(p, r) => (p.clone(), *r.clone()),
            _ => {
                let ptys: Vec<Type> = params.iter().map(|p| p.ty.clone()).collect();
                let ret_ty = Type::Void;
                (ptys, ret_ty)
            }
        };
        let lambda_name = format!("lambda.{}", self.module.get_functions().count());

        let mut body_ids = HashSet::new();
        Self::collect_var_refs_block(body, &mut body_ids);
        let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();
        let mut cap_globals = Vec::new();
        for id in &body_ids {
            if param_names.contains(id.as_str())
                || self.fns.contains_key(id)
                || self.variant_tags.contains_key(id)
            {
                continue;
            }
            if let Some((ptr, ty)) = self.find_var(id).cloned() {
                let val = b!(self.bld.build_load(self.llvm_ty(&ty), ptr, id));
                let gname = format!("{}.cap.{}", lambda_name, id);
                let lt = self.llvm_ty(&ty);
                let g = self.module.add_global(lt, None, &gname);
                g.set_initializer(&self.default_val(&ty));
                g.set_thread_local(true);
                b!(self.bld.build_store(g.as_pointer_value(), val));
                cap_globals.push((id.clone(), g.as_pointer_value(), ty));
            }
        }
        let lp: Vec<BasicMetadataTypeEnum<'ctx>> =
            ptys.iter().map(|t| self.llvm_ty(t).into()).collect();
        let ft = self.mk_fn_type(&ret_ty, &lp, false);
        let lambda_fv = self.module.add_function(&lambda_name, ft, None);
        lambda_fv.add_attribute(
            inkwell::attributes::AttributeLoc::Function,
            self.attr("nounwind"),
        );
        lambda_fv.set_linkage(Linkage::Internal);
        self.fns.insert(
            lambda_name.clone(),
            (lambda_fv, ptys.clone(), ret_ty.clone()),
        );
        let saved_fn = self.cur_fn;
        let saved_bb = self.bld.get_insert_block();
        self.cur_fn = Some(lambda_fv);
        let entry = self.ctx.append_basic_block(lambda_fv, "entry");
        self.bld.position_at_end(entry);
        self.vars.push(HashMap::new());
        for (name, gptr, ty) in &cap_globals {
            let lt = self.llvm_ty(ty);
            let val = b!(self.bld.build_load(lt, *gptr, name));
            let a = self.entry_alloca(lt, name);
            b!(self.bld.build_store(a, val));
            self.set_var(name, a, ty.clone());
        }
        for (i, p) in params.iter().enumerate() {
            let ty = &ptys[i];
            let a = self.entry_alloca(self.llvm_ty(ty), &p.name);
            b!(self
                .bld
                .build_store(a, lambda_fv.get_nth_param(i as u32).unwrap()));
            self.set_var(&p.name, a, ty.clone());
        }
        let last = self.compile_block(body)?;
        if self.no_term() {
            match &ret_ty {
                Type::Void => {
                    b!(self.bld.build_return(None));
                }
                _ => {
                    let rty = self.llvm_ty(&ret_ty);
                    let v = match last {
                        Some(v) if v.get_type() == rty => v,
                        _ => self.default_val(&ret_ty),
                    };
                    b!(self.bld.build_return(Some(&v)));
                }
            }
        }
        self.vars.pop();
        self.cur_fn = saved_fn;
        if let Some(bb) = saved_bb {
            self.bld.position_at_end(bb);
        }
        Ok(lambda_fv.as_global_value().as_pointer_value().into())
    }

    pub(crate) fn collect_var_refs_block(block: &hir::Block, out: &mut HashSet<String>) {
        for stmt in block {
            match stmt {
                hir::Stmt::Expr(e) | hir::Stmt::Bind(hir::Bind { value: e, .. }) => {
                    Self::collect_var_refs_expr(e, out);
                }
                hir::Stmt::TupleBind(_, v, _) => Self::collect_var_refs_expr(v, out),
                hir::Stmt::Assign(t, v, _) => {
                    Self::collect_var_refs_expr(t, out);
                    Self::collect_var_refs_expr(v, out);
                }
                hir::Stmt::Ret(Some(e), _, _) | hir::Stmt::Break(Some(e), _) => {
                    Self::collect_var_refs_expr(e, out);
                }
                hir::Stmt::If(i) => {
                    Self::collect_var_refs_expr(&i.cond, out);
                    Self::collect_var_refs_block(&i.then, out);
                    for (c, b) in &i.elifs {
                        Self::collect_var_refs_expr(c, out);
                        Self::collect_var_refs_block(b, out);
                    }
                    if let Some(b) = &i.els {
                        Self::collect_var_refs_block(b, out);
                    }
                }
                hir::Stmt::While(w) => {
                    Self::collect_var_refs_expr(&w.cond, out);
                    Self::collect_var_refs_block(&w.body, out);
                }
                hir::Stmt::For(f) => {
                    Self::collect_var_refs_expr(&f.iter, out);
                    Self::collect_var_refs_block(&f.body, out);
                }
                hir::Stmt::Loop(l) => Self::collect_var_refs_block(&l.body, out),
                hir::Stmt::Match(m) => {
                    Self::collect_var_refs_expr(&m.subject, out);
                    for arm in &m.arms {
                        if let Some(ref g) = arm.guard {
                            Self::collect_var_refs_expr(g, out);
                        }
                        Self::collect_var_refs_block(&arm.body, out);
                    }
                }
                hir::Stmt::ErrReturn(e, _, _) => Self::collect_var_refs_expr(e, out),
                _ => {}
            }
        }
    }

    fn collect_var_refs_expr(e: &hir::Expr, out: &mut HashSet<String>) {
        match &e.kind {
            hir::ExprKind::Var(_, n) => {
                out.insert(n.clone());
            }
            hir::ExprKind::BinOp(l, _, r) => {
                Self::collect_var_refs_expr(l, out);
                Self::collect_var_refs_expr(r, out);
            }
            hir::ExprKind::UnaryOp(_, e)
            | hir::ExprKind::Coerce(e, _)
            | hir::ExprKind::Cast(e, _)
            | hir::ExprKind::Ref(e)
            | hir::ExprKind::Deref(e) => Self::collect_var_refs_expr(e, out),
            hir::ExprKind::Call(_, _, args)
            | hir::ExprKind::Builtin(_, args)
            | hir::ExprKind::Syscall(args) => {
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                Self::collect_var_refs_expr(callee, out);
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::Method(obj, _, _, args)
            | hir::ExprKind::StringMethod(obj, _, args)
            | hir::ExprKind::VecMethod(obj, _, args)
            | hir::ExprKind::MapMethod(obj, _, args) => {
                Self::collect_var_refs_expr(obj, out);
                for a in args {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::Field(e, _, _) => Self::collect_var_refs_expr(e, out),
            hir::ExprKind::Index(a, b) => {
                Self::collect_var_refs_expr(a, out);
                Self::collect_var_refs_expr(b, out);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                Self::collect_var_refs_expr(c, out);
                Self::collect_var_refs_expr(t, out);
                Self::collect_var_refs_expr(f, out);
            }
            hir::ExprKind::Array(es) | hir::ExprKind::Tuple(es) | hir::ExprKind::VecNew(es) => {
                for e in es {
                    Self::collect_var_refs_expr(e, out);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for fi in inits {
                    Self::collect_var_refs_expr(&fi.value, out);
                }
            }
            hir::ExprKind::IfExpr(i) => {
                Self::collect_var_refs_expr(&i.cond, out);
                Self::collect_var_refs_block(&i.then, out);
                for (c, b) in &i.elifs {
                    Self::collect_var_refs_expr(c, out);
                    Self::collect_var_refs_block(b, out);
                }
                if let Some(b) = &i.els {
                    Self::collect_var_refs_block(b, out);
                }
            }
            hir::ExprKind::Block(b) => Self::collect_var_refs_block(b, out),
            hir::ExprKind::Lambda(_, body) => Self::collect_var_refs_block(body, out),
            hir::ExprKind::Pipe(left, _, _, extra) => {
                Self::collect_var_refs_expr(left, out);
                for a in extra {
                    Self::collect_var_refs_expr(a, out);
                }
            }
            hir::ExprKind::ListComp(body, _, _, iter, end, cond) => {
                Self::collect_var_refs_expr(body, out);
                Self::collect_var_refs_expr(iter, out);
                if let Some(e) = end {
                    Self::collect_var_refs_expr(e, out);
                }
                if let Some(c) = cond {
                    Self::collect_var_refs_expr(c, out);
                }
            }
            _ => {}
        }
    }
}
