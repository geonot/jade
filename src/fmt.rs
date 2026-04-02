use crate::ast::*;
use crate::lexer::Lexer;
use crate::parser::Parser;

pub fn format_source(src: &str) -> Result<String, String> {
    let tokens = Lexer::new(src).tokenize().map_err(|e| e.to_string())?;
    let prog = Parser::new(tokens)
        .parse_program()
        .map_err(|e| e.to_string())?;
    Ok(format_program(&prog))
}

fn format_program(prog: &Program) -> String {
    let mut out = String::new();
    for (i, decl) in prog.decls.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        format_decl(&mut out, decl, 0);
        out.push('\n');
    }
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn format_decl(out: &mut String, decl: &Decl, level: usize) {
    match decl {
        Decl::Fn(f) => format_fn(out, f, level),
        Decl::Type(t) => {
            indent(out, level);
            out.push_str(&format!("type {} is\n", t.name));
            for field in &t.fields {
                indent(out, level + 1);
                out.push_str(&field.name);
                if let Some(ref ty) = field.ty {
                    out.push_str(&format!(" is {}", format_type(ty)));
                }
                out.push('\n');
            }
        }
        Decl::Enum(e) => {
            indent(out, level);
            out.push_str(&format!("enum {}\n", e.name));
            for v in &e.variants {
                indent(out, level + 1);
                out.push_str(&v.name);
                if !v.fields.is_empty() {
                    out.push_str(" of ");
                    let fields: Vec<String> = v.fields.iter().map(|f| {
                        if let Some(ref name) = f.name {
                            format!("{name} is {}", format_type(&f.ty))
                        } else {
                            format_type(&f.ty)
                        }
                    }).collect();
                    out.push_str(&fields.join(", "));
                }
                out.push('\n');
            }
        }
        Decl::Extern(e) => {
            indent(out, level);
            out.push_str(&format!("extern {}", e.name));
            if !e.params.is_empty() {
                let params: Vec<String> = e.params.iter().map(|(name, ty)| {
                    format!("{name} {}", format_type(ty))
                }).collect();
                out.push_str(&format!(" {}", params.join(", ")));
            }
            out.push_str(&format!(" -> {}\n", format_type(&e.ret)));
        }
        Decl::Use(u) => {
            indent(out, level);
            out.push_str("use ");
            out.push_str(&u.path.join("."));
            if let Some(ref imports) = u.imports {
                out.push_str(" import ");
                out.push_str(&imports.join(", "));
            }
            if let Some(ref alias) = u.alias {
                out.push_str(&format!(" as {alias}"));
            }
            out.push('\n');
        }
        Decl::Trait(t) => {
            indent(out, level);
            out.push_str(&format!("trait {}\n", t.name));
            for m in &t.methods {
                indent(out, level + 1);
                out.push_str(&format!("*{}", m.name));
                for p in &m.params {
                    out.push_str(&format!(" {}", p.name));
                    if let Some(ref ty) = p.ty {
                        out.push_str(&format!(" {}", format_type(ty)));
                    }
                }
                if let Some(ref ret) = m.ret {
                    out.push_str(&format!(" -> {}", format_type(ret)));
                }
                out.push('\n');
                if let Some(ref body) = m.default_body {
                    format_block(out, body, level + 2);
                }
            }
        }
        Decl::Impl(im) => {
            indent(out, level);
            if let Some(ref trait_name) = im.trait_name {
                out.push_str(&format!("impl {trait_name} for {}\n", im.type_name));
            } else {
                out.push_str(&format!("impl {}\n", im.type_name));
            }
            for m in &im.methods {
                format_fn(out, m, level + 1);
            }
        }
        Decl::Const(name, expr, _) => {
            indent(out, level);
            out.push_str(&format!("const {} is {}\n", name, format_expr(expr)));
        }
        Decl::Test(t) => {
            indent(out, level);
            out.push_str(&format!("test '{}'\n", t.name));
            format_block(out, &t.body, level + 1);
        }
        Decl::Actor(a) => {
            indent(out, level);
            out.push_str(&format!("actor {}\n", a.name));
            for h in &a.handlers {
                indent(out, level + 1);
                out.push_str(&format!("*{}", h.name));
                for p in &h.params {
                    out.push_str(&format!(" {}", p.name));
                    if let Some(ref ty) = p.ty {
                        out.push_str(&format!(" {}", format_type(ty)));
                    }
                }
                out.push('\n');
                format_block(out, &h.body, level + 2);
            }
        }
        Decl::Store(s) => {
            indent(out, level);
            out.push_str(&format!("store {}\n", s.name));
            for field in &s.fields {
                indent(out, level + 1);
                out.push_str(&field.name);
                if let Some(ref ty) = field.ty {
                    out.push_str(&format!(" is {}", format_type(ty)));
                }
                out.push('\n');
            }
        }
        Decl::ErrDef(e) => {
            indent(out, level);
            out.push_str(&format!("error {}\n", e.name));
            for v in &e.variants {
                indent(out, level + 1);
                out.push_str(&v.name);
                if !v.fields.is_empty() {
                    let ts: Vec<String> = v.fields.iter().map(format_type).collect();
                    out.push_str(&format!(" of {}", ts.join(", ")));
                }
                out.push('\n');
            }
        }
        Decl::Supervisor(s) => {
            indent(out, level);
            out.push_str(&format!("supervisor {}\n", s.name));
        }
        Decl::TypeAlias(name, ty, _) => {
            indent(out, level);
            out.push_str(&format!("alias {} is {}\n", name, format_type(ty)));
        }
        Decl::Newtype(name, ty, _) => {
            indent(out, level);
            out.push_str(&format!("type {} is {}\n", name, format_type(ty)));
        }
        Decl::TopStmt(stmt) => {
            format_stmt(out, stmt, level);
        }
    }
}

fn format_fn(out: &mut String, f: &Fn, level: usize) {
    indent(out, level);
    out.push('*');
    out.push_str(&f.name);
    for p in &f.params {
        out.push(' ');
        out.push_str(&p.name);
        if let Some(ref ty) = p.ty {
            out.push(' ');
            out.push_str(&format_type(ty));
        }
    }
    if let Some(ref ret) = f.ret {
        out.push_str(&format!(" -> {}", format_type(ret)));
    }
    out.push('\n');
    format_block(out, &f.body, level + 1);
}

fn format_block(out: &mut String, stmts: &[Stmt], level: usize) {
    for stmt in stmts {
        format_stmt(out, stmt, level);
    }
}

fn format_stmt(out: &mut String, stmt: &Stmt, level: usize) {
    match stmt {
        Stmt::Bind(b) => {
            indent(out, level);
            out.push_str(&b.name);
            out.push_str(" is ");
            out.push_str(&format_expr(&b.value));
            out.push('\n');
        }
        Stmt::Assign(lhs, rhs, _) => {
            indent(out, level);
            out.push_str(&format_expr(lhs));
            out.push_str(" is ");
            out.push_str(&format_expr(rhs));
            out.push('\n');
        }
        Stmt::Expr(e) => {
            indent(out, level);
            out.push_str(&format_expr(e));
            out.push('\n');
        }
        Stmt::Ret(expr, _) => {
            indent(out, level);
            out.push_str("return");
            if let Some(e) = expr {
                out.push(' ');
                out.push_str(&format_expr(e));
            }
            out.push('\n');
        }
        Stmt::If(i) => format_if(out, i, level),
        Stmt::While(w) => {
            indent(out, level);
            out.push_str("while ");
            out.push_str(&format_expr(&w.cond));
            out.push('\n');
            format_block(out, &w.body, level + 1);
        }
        Stmt::For(f) => {
            indent(out, level);
            if let Some(ref label) = f.label {
                out.push_str(&format!("{label} is "));
            }
            out.push_str("for ");
            out.push_str(&f.bind);
            out.push_str(" in ");
            out.push_str(&format_expr(&f.iter));
            out.push('\n');
            format_block(out, &f.body, level + 1);
        }
        Stmt::SimFor(f, _) => {
            indent(out, level);
            out.push_str("sim for ");
            out.push_str(&f.bind);
            out.push_str(" in ");
            out.push_str(&format_expr(&f.iter));
            out.push('\n');
            format_block(out, &f.body, level + 1);
        }
        Stmt::SimBlock(b, _) => {
            indent(out, level);
            out.push_str("sim\n");
            format_block(out, b, level + 1);
        }
        Stmt::Loop(l) => {
            indent(out, level);
            out.push_str("loop\n");
            format_block(out, &l.body, level + 1);
        }
        Stmt::Break(_, _) => {
            indent(out, level);
            out.push_str("break\n");
        }
        Stmt::Continue(_) => {
            indent(out, level);
            out.push_str("continue\n");
        }
        Stmt::Match(m) => {
            indent(out, level);
            out.push_str("match ");
            out.push_str(&format_expr(&m.subject));
            out.push('\n');
            for arm in &m.arms {
                indent(out, level + 1);
                out.push_str(&format_pat(&arm.pat));
                if let Some(ref guard) = arm.guard {
                    out.push_str(" if ");
                    out.push_str(&format_expr(guard));
                }
                out.push_str(" =>\n");
                format_block(out, &arm.body, level + 2);
            }
        }
        Stmt::TupleBind(bindings, expr, _) => {
            indent(out, level);
            out.push_str(&format!("({}) is {}\n", bindings.join(", "), format_expr(expr)));
        }
        Stmt::StoreInsert(name, exprs, _) => {
            indent(out, level);
            out.push_str(&format!("insert into {name}"));
            for e in exprs {
                out.push(' ');
                out.push_str(&format_expr(e));
            }
            out.push('\n');
        }
        Stmt::StoreDelete(name, _filter, _) => {
            indent(out, level);
            out.push_str(&format!("delete from {name}\n"));
        }
        Stmt::StoreSet(name, assignments, _filter, _) => {
            indent(out, level);
            out.push_str(&format!("set {name}"));
            for (k, v) in assignments {
                out.push_str(&format!(" {k} is {}", format_expr(v)));
            }
            out.push('\n');
        }
        Stmt::Asm(_) => {
            indent(out, level);
            out.push_str("asm ...\n");
        }
        Stmt::ErrReturn(e, _) => {
            indent(out, level);
            out.push_str(&format!("throw {}\n", format_expr(e)));
        }
        Stmt::Transaction(body, _) => {
            indent(out, level);
            out.push_str("transaction\n");
            format_block(out, body, level + 1);
        }
        Stmt::ChannelClose(e, _) => {
            indent(out, level);
            out.push_str(&format!("close {}\n", format_expr(e)));
        }
        Stmt::Stop(e, _) => {
            indent(out, level);
            out.push_str(&format!("stop {}\n", format_expr(e)));
        }
        Stmt::UseLocal(u) => {
            indent(out, level);
            out.push_str("use ");
            out.push_str(&u.path.join("."));
            if let Some(ref imports) = u.imports {
                out.push_str(" import ");
                out.push_str(&imports.join(", "));
            }
            out.push('\n');
        }
    }
}

fn format_if(out: &mut String, i: &If, level: usize) {
    indent(out, level);
    out.push_str("if ");
    out.push_str(&format_expr(&i.cond));
    out.push('\n');
    format_block(out, &i.then, level + 1);
    for (cond, body) in &i.elifs {
        indent(out, level);
        out.push_str("else if ");
        out.push_str(&format_expr(cond));
        out.push('\n');
        format_block(out, body, level + 1);
    }
    if let Some(ref els) = i.els {
        indent(out, level);
        out.push_str("else\n");
        format_block(out, els, level + 1);
    }
}

fn format_expr(e: &Expr) -> String {
    match e {
        Expr::None(_) => "none".into(),
        Expr::Void(_) => "void".into(),
        Expr::Int(n, _) => n.to_string(),
        Expr::Float(f, _) => format!("{f}"),
        Expr::Str(s, _) => format!("'{s}'"),
        Expr::Bool(true, _) => "true".into(),
        Expr::Bool(false, _) => "false".into(),
        Expr::Ident(name, _) => name.clone(),
        Expr::BinOp(l, op, r, _) => {
            let ops = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Mod => "%",
                BinOp::Eq => "equals",
                BinOp::Ne => "not equals",
                BinOp::Lt => "<",
                BinOp::Le => "<=",
                BinOp::Gt => ">",
                BinOp::Ge => ">=",
                BinOp::And => "and",
                BinOp::Or => "or",
                BinOp::BitAnd => "&",
                BinOp::BitOr => "|",
                BinOp::BitXor => "^",
                BinOp::Shl => "<<",
                BinOp::Shr => ">>",
                BinOp::Exp => "**",
            };
            format!("{} {} {}", format_expr(l), ops, format_expr(r))
        }
        Expr::UnaryOp(op, e, _) => {
            let ops = match op {
                UnaryOp::Neg => "-",
                UnaryOp::Not => "not ",
                UnaryOp::BitNot => "~",
            };
            format!("{}{}", ops, format_expr(e))
        }
        Expr::Call(callee, args, _) => {
            let arg_strs: Vec<String> = args.iter().map(format_expr).collect();
            if arg_strs.is_empty() {
                format!("{}()", format_expr(callee))
            } else {
                format!("{} {}", format_expr(callee), arg_strs.join(", "))
            }
        }
        Expr::Method(obj, method, args, _) => {
            let arg_strs: Vec<String> = args.iter().map(format_expr).collect();
            if arg_strs.is_empty() {
                format!("{}.{method}()", format_expr(obj))
            } else {
                format!("{}.{method} {}", format_expr(obj), arg_strs.join(", "))
            }
        }
        Expr::Field(obj, field, _) => format!("{}.{field}", format_expr(obj)),
        Expr::Index(arr, idx, _) => format!("{}[{}]", format_expr(arr), format_expr(idx)),
        Expr::Ternary(c, t, f, _) => {
            format!("if {} then {} else {}", format_expr(c), format_expr(t), format_expr(f))
        }
        Expr::As(e, ty, _) => format!("{} as {}", format_expr(e), format_type(ty)),
        Expr::Array(elems, _) => {
            let es: Vec<String> = elems.iter().map(format_expr).collect();
            format!("[{}]", es.join(", "))
        }
        Expr::Tuple(elems, _) => {
            let es: Vec<String> = elems.iter().map(format_expr).collect();
            format!("({})", es.join(", "))
        }
        Expr::Struct(name, fields, _) => {
            let fs: Vec<String> = fields.iter().map(|fi| {
                if let Some(ref name) = fi.name {
                    format!("{name} is {}", format_expr(&fi.value))
                } else {
                    format_expr(&fi.value)
                }
            }).collect();
            format!("{name} {{ {} }}", fs.join(", "))
        }
        Expr::IfExpr(i) => {
            format!("if {} then {} else {}",
                format_expr(&i.cond),
                if i.then.len() == 1 { format_expr_from_stmt(&i.then[0]) } else { "...".into() },
                if let Some(ref els) = i.els {
                    if els.len() == 1 { format_expr_from_stmt(&els[0]) } else { "...".into() }
                } else {
                    "none".into()
                }
            )
        }
        Expr::Pipe(l, r, _, _) => format!("{} |> {}", format_expr(l), format_expr(r)),
        Expr::Block(_, _) => "do ... end".into(),
        Expr::Lambda(params, _, _, _) => {
            let ps: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();
            format!("({}) => ...", ps.join(", "))
        }
        Expr::Placeholder(_) => "$".into(),
        Expr::Ref(e, _) => format!("&{}", format_expr(e)),
        Expr::Deref(e, _) => format!("*{}", format_expr(e)),
        Expr::Embed(path, _) => format!("embed '{path}'"),
        Expr::ListComp(body, bind, iter, _, _, _) => {
            format!("[{} for {bind} in {}]", format_expr(body), format_expr(iter))
        }
        Expr::Unreachable(_) => "unreachable".into(),
        Expr::AsFormat(e, fmt, _) => format!("{} as {fmt}", format_expr(e)),
        Expr::StrictCast(e, ty, _) => format!("{} as strict {}", format_expr(e), format_type(ty)),
        Expr::Slice(obj, from, to, _) => {
            format!("{} from {} to {}", format_expr(obj), format_expr(from), format_expr(to))
        }
        _ => "...".into(),
    }
}

fn format_expr_from_stmt(s: &Stmt) -> String {
    match s {
        Stmt::Expr(e) => format_expr(e),
        Stmt::Ret(Some(e), _) => format!("return {}", format_expr(e)),
        Stmt::Ret(None, _) => "return".into(),
        _ => "...".into(),
    }
}

fn format_pat(p: &Pat) -> String {
    match p {
        Pat::Lit(e) => format_expr(e),
        Pat::Ident(name, _) => name.clone(),
        Pat::Wild(_) => "_".into(),
        Pat::Tuple(pats, _) => {
            let ps: Vec<String> = pats.iter().map(format_pat).collect();
            format!("({})", ps.join(", "))
        }
        Pat::Ctor(name, pats, _) => {
            if pats.is_empty() {
                name.clone()
            } else {
                let ps: Vec<String> = pats.iter().map(format_pat).collect();
                format!("{name} of {}", ps.join(", "))
            }
        }
        Pat::Or(pats, _) => {
            let ps: Vec<String> = pats.iter().map(format_pat).collect();
            ps.join(" | ")
        }
        Pat::Array(pats, _) => {
            let ps: Vec<String> = pats.iter().map(format_pat).collect();
            format!("[{}]", ps.join(", "))
        }
        Pat::Range(l, r, _) => format!("{} to {}", format_expr(l), format_expr(r)),
    }
}

fn format_type(ty: &crate::types::Type) -> String {
    format!("{ty}")
}
