use crate::ast::*;
use crate::lexer::Lexer;
use crate::parser::Parser;

pub fn format_source(src: &str) -> Result<String, String> {
    let mut lexer = Lexer::new(src);
    let tokens = lexer.tokenize().map_err(|e| e.to_string())?;
    let mut sink = CommentSink::new(src, lexer.comments());
    let prog = Parser::new(tokens)
        .parse_program()
        .map_err(|e| e.to_string())?;
    Ok(format_program(&prog, &mut sink))
}

/// Comment trivia carried through formatting (task 8-18, decision D5).
///
/// The lexer records every `#` comment's span; the printer flushes
/// pending comments before each declaration/statement whose source
/// position follows them, at the current indent. A comment that shares
/// its line with code re-attaches as a trailing `  # …`; a standalone
/// comment keeps a preceding blank line if the source had one.
/// Comments inside a single expression re-anchor to its statement —
/// position within one logical line is not preserved.
struct CommentSink {
    entries: Vec<CmtEntry>,
    idx: usize,
}

struct CmtEntry {
    start: usize,
    line: u32,
    text: String,
    own_line: bool,
    blank_before: bool,
}

impl CommentSink {
    fn new(src: &str, spans: &[crate::ast::Span]) -> Self {
        let bytes = src.as_bytes();
        let entries = spans
            .iter()
            .map(|sp| {
                let text = src[sp.start..sp.end].trim_end().to_string();
                // Standalone iff only whitespace precedes it on its line.
                let mut i = sp.start;
                let mut own_line = true;
                while i > 0 && bytes[i - 1] != b'\n' {
                    if !bytes[i - 1].is_ascii_whitespace() {
                        own_line = false;
                        break;
                    }
                    i -= 1;
                }
                // Blank line directly above? (two newlines with only
                // whitespace between them)
                let mut blank_before = false;
                if own_line && i > 0 {
                    let mut j = i - 1; // the '\n' ending the previous line
                    if bytes[j] == b'\n' {
                        let mut k = j;
                        let mut saw_content = false;
                        while k > 0 {
                            k -= 1;
                            if bytes[k] == b'\n' {
                                break;
                            }
                            if !bytes[k].is_ascii_whitespace() {
                                saw_content = true;
                                break;
                            }
                        }
                        let _ = j;
                        j = k;
                        let _ = j;
                        blank_before = !saw_content && sp.line > 1;
                    }
                }
                CmtEntry {
                    start: sp.start,
                    line: sp.line,
                    text,
                    own_line,
                    blank_before,
                }
            })
            .collect();
        CommentSink { entries, idx: 0 }
    }

    /// Emit every pending standalone comment positioned before `upto`.
    fn flush_before(&mut self, out: &mut String, upto: usize, level: usize) {
        while self.idx < self.entries.len() && self.entries[self.idx].start < upto {
            let e = &self.entries[self.idx];
            if e.blank_before && !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            indent(out, level);
            out.push_str(&e.text);
            out.push('\n');
            self.idx += 1;
        }
    }

    /// If the next pending comment shares `line` with just-printed code,
    /// re-attach it as a trailing comment (the printed text ends with a
    /// newline; splice before it).
    fn attach_trailing(&mut self, out: &mut String, line: u32) {
        while self.idx < self.entries.len()
            && !self.entries[self.idx].own_line
            && self.entries[self.idx].line == line
        {
            let text = self.entries[self.idx].text.clone();
            if out.ends_with('\n') {
                out.pop();
            }
            out.push_str("  ");
            out.push_str(&text);
            out.push('\n');
            self.idx += 1;
        }
    }

    /// End of input: whatever remains prints standalone at column 0.
    fn flush_rest(&mut self, out: &mut String) {
        let end = usize::MAX;
        self.flush_before(out, end, 0);
    }
}

fn decl_span(d: &Decl) -> crate::ast::Span {
    match d {
        Decl::Fn(f) => f.span,
        Decl::Type(t) => t.span,
        Decl::Enum(e) => e.span,
        Decl::Use(u) => u.span,
        Decl::Extern(e) => e.span,
        Decl::ErrDef(e) => e.span,
        Decl::Actor(a) => a.span,
        Decl::Store(s) => s.span,
        Decl::Trait(t) => t.span,
        Decl::Impl(i) => i.span,
        Decl::Supervisor(s) => s.span,
        Decl::Migration(m) => m.span,
        Decl::View(v) => v.span,
        Decl::Test(t) => t.span,
        Decl::Const(_, _, s)
        | Decl::Global(_, _, s)
        | Decl::TypeAlias(_, _, s)
        | Decl::Newtype(_, _, s) => *s,
        Decl::TopStmt(st) => st.span(),
    }
}

fn format_program(prog: &Program, sink: &mut CommentSink) -> String {
    let mut out = String::new();
    for (i, decl) in prog.decls.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        sink.flush_before(&mut out, decl_span(decl).start, 0);
        format_decl(&mut out, decl, 0, sink);
        out.push('\n');
    }
    sink.flush_rest(&mut out);
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn format_decl(out: &mut String, decl: &Decl, level: usize, sink: &mut CommentSink) {
    match decl {
        Decl::Fn(f) => format_fn(out, f, level, sink),
        Decl::Type(t) => {
            indent(out, level);
            out.push_str(&format!("type {}\n", t.name));
            for field in &t.fields {
                indent(out, level + 1);
                out.push_str(&field.name.to_string());
                if let Some(ref ty) = field.ty {
                    out.push_str(&format!(" as {}", format_type(ty)));
                }
                out.push('\n');
            }
        }
        Decl::Enum(e) => {
            indent(out, level);
            out.push_str(&format!("enum {}\n", e.name));
            for v in &e.variants {
                indent(out, level + 1);
                out.push_str(&v.name.as_str());
                if !v.fields.is_empty() {
                    out.push('(');
                    let fields: Vec<String> = v
                        .fields
                        .iter()
                        .map(|f| {
                            if let Some(ref name) = f.name {
                                format!("{name} as {}", format_type(&f.ty))
                            } else {
                                format_type(&f.ty)
                            }
                        })
                        .collect();
                    out.push_str(&fields.join(", "));
                    out.push(')');
                }
                out.push('\n');
            }
        }
        Decl::Extern(e) => {
            indent(out, level);
            out.push_str(&format!("extern {}", e.name));
            if !e.params.is_empty() {
                let params: Vec<String> = e
                    .params
                    .iter()
                    .map(|(name, ty)| format!("{name} {}", format_type(ty)))
                    .collect();
                out.push_str(&format!(" {}", params.join(", ")));
            }
            out.push_str(&format!(" returns {}\n", format_type(&e.ret)));
        }
        Decl::Use(u) => {
            indent(out, level);
            out.push_str("use ");
            out.push_str(&Symbol::join_vec(&u.path, "."));
            if let Some(ref imports) = u.imports {
                out.push_str(" import ");
                out.push_str(&Symbol::join_vec(imports, ", "));
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
                    out.push_str(&format!(" returns {}", format_type(ret)));
                }
                out.push('\n');
                if let Some(ref body) = m.default_body {
                    format_block(out, body, level + 2, sink);
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
                format_fn(out, m, level + 1, sink);
            }
        }
        Decl::Const(name, expr, _) => {
            indent(out, level);
            out.push_str(&format!("const {} is {}\n", name, format_expr(expr)));
        }
        Decl::Global(name, expr, _) => {
            indent(out, level);
            out.push_str(&format!("global {} is {}\n", name, format_expr(expr)));
        }
        Decl::Test(t) => {
            indent(out, level);
            out.push_str(&format!("test '{}'\n", t.name));
            format_block(out, &t.body, level + 1, sink);
        }
        Decl::Actor(a) => {
            indent(out, level);
            out.push_str(&format!("actor {}\n", a.name));
            for h in &a.handlers {
                indent(out, level + 1);
                if h.is_loop {
                    out.push_str("*loop");
                    if let Some(ref sleep_ms) = h.loop_sleep_ms {
                        out.push(' ');
                        out.push_str(&format_expr(sleep_ms));
                    }
                } else {
                    out.push_str(&format!("*{}", h.name));
                    for p in &h.params {
                        out.push_str(&format!(" {}", p.name));
                        if let Some(ref ty) = p.ty {
                            out.push_str(&format!(" {}", format_type(ty)));
                        }
                    }
                }
                out.push('\n');
                format_block(out, &h.body, level + 2, sink);
            }
        }
        Decl::Store(s) => {
            indent(out, level);
            out.push_str(&format!("store {}\n", s.name));
            for field in &s.fields {
                indent(out, level + 1);
                out.push_str(&field.name.to_string());
                if let Some(ref ty) = field.ty {
                    out.push_str(&format!(" is {}", format_type(ty)));
                }
                out.push('\n');
            }
        }
        Decl::ErrDef(e) => {
            indent(out, level);
            out.push_str(&format!("err {}\n", e.name));
            for v in &e.variants {
                indent(out, level + 1);
                out.push_str(&v.name.to_string());
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
            format_stmt(out, stmt, level, sink);
        }
        Decl::Migration(_) => {}
        Decl::View(_) => {}
    }
}

fn format_fn(out: &mut String, f: &Fn, level: usize, sink: &mut CommentSink) {
    indent(out, level);
    out.push('*');
    out.push_str(&f.name.to_string());
    if !f.params.is_empty() {
        out.push('(');
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if let Some(ref lit) = p.literal {
                out.push_str(&format_expr(lit));
                continue;
            }
            out.push_str(&p.name.to_string());
            if p.ty.is_some() || p.access_mod.is_some() {
                out.push_str(" as ");
                if let Some(am) = p.access_mod {
                    out.push_str(match am {
                        crate::ast::AccessMod::Take => "take ",
                        crate::ast::AccessMod::Copy => "copy ",
                        crate::ast::AccessMod::Const => "const ",
                        _ => "",
                    });
                }
                if let Some(ref ty) = p.ty {
                    out.push_str(&format_type(ty));
                }
            }
            if let Some(ref d) = p.default {
                out.push_str(&format!(" is {}", format_expr(d)));
            }
        }
        out.push(')');
    }
    if let Some(ref ret) = f.ret {
        out.push_str(&format!(" returns {}", format_type(ret)));
    }
    out.push('\n');
    format_block(out, &f.body, level + 1, sink);
}

fn format_block(out: &mut String, stmts: &[Stmt], level: usize, sink: &mut CommentSink) {
    for stmt in stmts {
        sink.flush_before(out, stmt.span().start, level);
        format_stmt(out, stmt, level, sink);
        /* Trailing comments re-attach to single-line statements only; a
         * compound statement's interior comments flush inside its body. */
        let compound = matches!(
            stmt,
            Stmt::If(_)
                | Stmt::While(_)
                | Stmt::For(_)
                | Stmt::SimFor(_, _)
                | Stmt::Loop(_)
                | Stmt::Match(_)
                | Stmt::Defer(_, _)
                | Stmt::Transaction(_, _)
                | Stmt::SimBlock(_, _)
                | Stmt::Together(_, _, _, _)
        );
        if !compound {
            sink.attach_trailing(out, stmt.span().line);
        }
    }
}

fn format_stmt(out: &mut String, stmt: &Stmt, level: usize, sink: &mut CommentSink) {
    match stmt {
        Stmt::Bind(b) => {
            indent(out, level);
            out.push_str(&b.name.to_string());
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
        Stmt::If(i) => format_if(out, i, level, sink),
        Stmt::While(w) => {
            indent(out, level);
            out.push_str("while ");
            out.push_str(&format_expr(&w.cond));
            out.push('\n');
            format_block(out, &w.body, level + 1, sink);
        }
        Stmt::For(f) => {
            indent(out, level);
            if let Some(ref label) = f.label {
                out.push_str(&format!("{label} is "));
            }
            out.push_str("for ");
            out.push_str(&f.bind.to_string());
            out.push_str(" in ");
            out.push_str(&format_expr(&f.iter));
            out.push('\n');
            format_block(out, &f.body, level + 1, sink);
        }
        Stmt::SimFor(f, _) => {
            indent(out, level);
            out.push_str("sim for ");
            out.push_str(&f.bind.to_string());
            out.push_str(" in ");
            out.push_str(&format_expr(&f.iter));
            out.push('\n');
            format_block(out, &f.body, level + 1, sink);
        }
        Stmt::SimBlock(b, _) => {
            indent(out, level);
            out.push_str("sim\n");
            format_block(out, b, level + 1, sink);
        }
        Stmt::Loop(l) => {
            indent(out, level);
            out.push_str("loop\n");
            format_block(out, &l.body, level + 1, sink);
        }
        Stmt::Break(_, _) => {
            indent(out, level);
            out.push_str("break\n");
        }
        Stmt::Continue(_) => {
            indent(out, level);
            out.push_str("continue\n");
        }
        Stmt::Nop(_) => {
            indent(out, level);
            out.push_str("nop\n");
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
                out.push_str(" ?");
                // Single-expression arms print inline (`Pat ? expr`);
                // multi-statement arms indent underneath.
                if arm.body.len() == 1
                    && let Stmt::Expr(e) = &arm.body[0]
                {
                    out.push(' ');
                    out.push_str(&format_expr(e));
                    out.push('\n');
                } else {
                    out.push('\n');
                    format_block(out, &arm.body, level + 2, sink);
                }
            }
        }
        Stmt::TupleBind(bindings, expr, _) => {
            indent(out, level);
            out.push_str(&format!(
                "({}) is {}\n",
                Symbol::join_vec(bindings, ", "),
                format_expr(expr)
            ));
        }
        Stmt::StoreInsert(name, exprs, _) => {
            indent(out, level);
            out.push_str(&format!("insert into {name}"));
            for fi in exprs {
                out.push(' ');
                if let Some(fname) = &fi.name {
                    out.push_str(&format!("{fname} is "));
                }
                out.push_str(&format_expr(&fi.value));
            }
            out.push('\n');
        }
        Stmt::StoreDelete(name, _filter, _) => {
            indent(out, level);
            out.push_str(&format!("delete from {name}\n"));
        }
        Stmt::StoreDestroy(name, _filter, _) => {
            indent(out, level);
            out.push_str(&format!("destroy from {name}\n"));
        }
        Stmt::StoreRestore(name, _filter, _) => {
            indent(out, level);
            out.push_str(&format!("restore from {name}\n"));
        }
        Stmt::StoreSave(name, _) => {
            indent(out, level);
            out.push_str(&format!("save {name}\n"));
        }
        Stmt::StoreCompact(name, _) => {
            indent(out, level);
            out.push_str(&format!("compact {name}\n"));
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
            out.push_str(&format!("err {}\n", format_expr(e)));
        }
        Stmt::Defer(body, _) => {
            indent(out, level);
            out.push_str("defer\n");
            format_block(out, body, level + 1, sink);
        }
        Stmt::Transaction(body, _) => {
            indent(out, level);
            out.push_str("transaction\n");
            format_block(out, body, level + 1, sink);
        }
        Stmt::Together(name, body, _, _) => {
            indent(out, level);
            match name {
                Some(n) => out.push_str(&format!("together {n}\n")),
                None => out.push_str("together\n"),
            }
            format_block(out, body, level + 1, sink);
        }
        Stmt::ChannelClose(e, _) => {
            indent(out, level);
            out.push_str(&format!("close {}\n", format_expr(e)));
        }
        Stmt::Stop(e, _) => {
            indent(out, level);
            out.push_str(&format!("stop {}\n", format_expr(e)));
        }
        Stmt::Join(e, _) => {
            indent(out, level);
            out.push_str(&format!("join {}\n", format_expr(e)));
        }
        Stmt::UseLocal(u) => {
            indent(out, level);
            out.push_str("use ");
            out.push_str(&Symbol::join_vec(&u.path, "."));
            if let Some(ref imports) = u.imports {
                out.push_str(" import ");
                out.push_str(&Symbol::join_vec(imports, ", "));
            }
            out.push('\n');
        }
    }
}

fn format_if(out: &mut String, i: &If, level: usize, sink: &mut CommentSink) {
    indent(out, level);
    out.push_str("if ");
    out.push_str(&format_expr(&i.cond));
    out.push('\n');
    format_block(out, &i.then, level + 1, sink);
    for (cond, body) in &i.elifs {
        indent(out, level);
        out.push_str("else if ");
        out.push_str(&format_expr(cond));
        out.push('\n');
        format_block(out, body, level + 1, sink);
    }
    if let Some(ref els) = i.els {
        indent(out, level);
        out.push_str("else\n");
        format_block(out, els, level + 1, sink);
    }
}

fn format_expr(e: &Expr) -> String {
    match e {
        Expr::None(_) => "none".into(),
        Expr::Void(_) => "void".into(),
        Expr::Int(n, _) => n.to_string(),
        Expr::Float(f, _) => format!("{f}"),
        Expr::Str(s, _) => {
            // Single quotes interpolate `{ident}`; there are no escape
            // sequences. Quote choice must keep the reparse identical:
            //  - contains braces or a single quote, and no double quote →
            //    double-quote (raw) so nothing interpolates/terminates;
            //  - otherwise single-quote (a brace next to a double quote
            //    cannot interpolate as an identifier, so it stays literal).
            let braceish = s.contains('{') || s.contains('}') || s.contains('\'');
            if braceish && !s.contains('"') {
                format!("\"{s}\"")
            } else {
                format!("'{s}'")
            }
        }
        Expr::Bool(true, _) => "true".into(),
        Expr::Bool(false, _) => "false".into(),
        Expr::Ident(name, _) => name.to_string(),
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
                BinOp::Ushr => ">>>",
                BinOp::Exp => "pow",
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
            format!("{}({})", format_expr(callee), arg_strs.join(", "))
        }
        Expr::Method(obj, method, args, _) => {
            let arg_strs: Vec<String> = args.iter().map(format_expr).collect();
            format!("{}.{method}({})", format_expr(obj), arg_strs.join(", "))
        }
        Expr::Field(obj, field, _) => format!("{}.{field}", format_expr(obj)),
        Expr::Index(arr, idx, _) => format!("{}[{}]", format_expr(arr), format_expr(idx)),
        Expr::Ternary(c, t, f, _) => {
            format!(
                "{} ? {} ! {}",
                format_expr(c),
                format_expr(t),
                format_expr(f)
            )
        }
        Expr::Quaternary(subj, ok, nothing, err, _) => {
            let mut s = format_expr(subj);
            if let Some(ok) = ok {
                s.push_str(&format!(" ? {}", format_expr(ok)));
            }
            if let Some(nothing) = nothing {
                s.push_str(&format!(" ! {}", format_expr(nothing)));
            }
            if let Some(err) = err {
                s.push_str(&format!(" !! {}", format_expr(err)));
            }
            s
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
            let fs: Vec<String> = fields
                .iter()
                .map(|fi| {
                    if let Some(ref name) = fi.name {
                        format!("{name} is {}", format_expr(&fi.value))
                    } else {
                        format_expr(&fi.value)
                    }
                })
                .collect();
            format!("{name}({})", fs.join(", "))
        }
        Expr::IfExpr(i) => {
            format!(
                "{} ? {} ! {}",
                format_expr(&i.cond),
                if i.then.len() == 1 {
                    format_expr_from_stmt(&i.then[0])
                } else {
                    "...".into()
                },
                if let Some(ref els) = i.els {
                    if els.len() == 1 {
                        format_expr_from_stmt(&els[0])
                    } else {
                        "...".into()
                    }
                } else {
                    "none".into()
                }
            )
        }
        Expr::Pipe(l, r, rest, _) => {
            // The pipeline operator is `~` (the old printer emitted `|>`,
            // which does not lex as one token).
            let mut out = format!("{} ~ {}", format_expr(l), format_expr(r));
            for e in rest {
                out.push_str(&format!(", {}", format_expr(e)));
            }
            out
        }
        Expr::Block(_, _) => "do ... end".into(),
        Expr::Lambda(params, _, body, _) => {
            let ps: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
            // Lambdas print in the `|x| expr` form; a multi-statement body
            // reprints its final expression (the printer never emits the
            // old invalid `=> ...` placeholder).
            let body_txt = match body.last() {
                Some(Stmt::Expr(e)) => format_expr(e),
                Some(Stmt::Ret(Some(e), _)) => format_expr(e),
                _ => "0".to_string(),
            };
            format!("|{}| {}", ps.join(", "), body_txt)
        }
        Expr::Placeholder(_) => "$".into(),
        Expr::IndexPlaceholder(_) => "$$".into(),
        Expr::Ref(e, _) => format!("&{}", format_expr(e)),
        Expr::Deref(e, _) => format!("*{}", format_expr(e)),
        Expr::Embed(path, _) => format!("embed '{path}'"),
        Expr::ListComp(body, bind, iter, _, _, _) => {
            format!(
                "[{} for {bind} in {}]",
                format_expr(body),
                format_expr(iter)
            )
        }
        Expr::Unreachable(_) => "unreachable".into(),
        Expr::AsFormat(e, fmt, _) => format!("{} as {fmt}", format_expr(e)),
        Expr::StrictCast(e, ty, _) => format!("{} as strict {}", format_expr(e), format_type(ty)),
        Expr::Slice(obj, from, to, _) => {
            format!(
                "{} from {} to {}",
                format_expr(obj),
                format_expr(from),
                format_expr(to)
            )
        }
        Expr::NamedArg(name, value, _) => {
            format!("{name} is {}", format_expr(value))
        }
        Expr::Yield(v, _) => format!("yield {}", format_expr(v)),
        Expr::Spawn(name, inits, _) => {
            if inits.is_empty() {
                format!("spawn {name}")
            } else {
                let fs: Vec<String> = inits
                    .iter()
                    .map(|(n, e)| format!("{n} is {}", format_expr(e)))
                    .collect();
                format!("spawn {name}({})", fs.join(", "))
            }
        }
        Expr::ChannelCreate(ty, cap, _) => match ty {
            Some(t) => format!("channel of {}({})", format_type(t), format_expr(cap)),
            None => format!("channel({})", format_expr(cap)),
        },
        Expr::ChannelRecv(ch, _) => format!("receive {}", format_expr(ch)),
        Expr::ChannelSend(ch, v, _) => {
            format!("send {}, {}", format_expr(ch), format_expr(v))
        }
        Expr::OfCall(f, arg, _) => {
            format!("{} of {}", format_expr(f), format_expr(arg))
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
        Pat::Ident(name, _) => name.to_string(),
        Pat::Wild(_) => "_".into(),
        Pat::Tuple(pats, _) => {
            let ps: Vec<String> = pats.iter().map(format_pat).collect();
            format!("({})", ps.join(", "))
        }
        Pat::Ctor(name, pats, _) => {
            if pats.is_empty() {
                name.to_string()
            } else {
                let ps: Vec<String> = pats.iter().map(format_pat).collect();
                format!("{name}({})", ps.join(", "))
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
    use crate::types::Type;
    match ty {
        // Display prints `(a) -> r`, which the parser does not accept;
        // source syntax is `(a) returns r`.
        Type::Fn(params, ret) => {
            let ps: Vec<String> = params.iter().map(format_type).collect();
            format!("({}) returns {}", ps.join(", "), format_type(ret))
        }
        Type::Vec(inner) => format!("Vec of {}", format_type(inner)),
        Type::Map(k, v) => format!("Map of {}, {}", format_type(k), format_type(v)),
        _ => format!("{ty}"),
    }
}
