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

                let mut i = sp.start;
                let mut own_line = true;
                while i > 0 && bytes[i - 1] != b'\n' {
                    if !bytes[i - 1].is_ascii_whitespace() {
                        own_line = false;
                        break;
                    }
                    i -= 1;
                }

                let mut blank_before = false;
                if own_line && i > 0 {
                    let mut j = i - 1;
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
            out.push_str(&format!("type {}", t.name));
            if !t.type_params.is_empty() {
                out.push_str(&format!(" of {}", Symbol::join_vec(&t.type_params, ", ")));
            }
            if t.layout.packed {
                out.push_str(" @packed");
            }
            if t.layout.strict {
                out.push_str(" @strict");
            }
            if let Some(a) = t.layout.align {
                out.push_str(&format!(" @align({a})"));
            }
            match t.layout.category {
                Some(crate::ast::CategoryAssert::Value) => out.push_str(" @value"),
                Some(crate::ast::CategoryAssert::Aggregate) => out.push_str(" @aggregate"),
                None => {}
            }
            if t.layout.resource {
                out.push_str(" @resource");
            }
            out.push('\n');
            for field in &t.fields {
                indent(out, level + 1);
                out.push_str(&field.name.to_string());
                if let Some(ref ty) = field.ty {
                    out.push_str(&format!(" as {}", format_type(ty)));
                }
                if let Some(ref default) = field.default {
                    out.push_str(&format!(" is {}", format_expr(default)));
                }
                out.push('\n');
            }
            for m in &t.methods {
                out.push('\n');
                format_fn(out, m, level + 1, sink);
            }
        }
        Decl::Enum(e) => {
            indent(out, level);
            out.push_str(&format!("enum {}", e.name));
            if !e.type_params.is_empty() {
                out.push_str(&format!(" of {}", Symbol::join_vec(&e.type_params, ", ")));
            }
            out.push('\n');
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
            out.push_str(&format!("extern *{}(", e.name));
            let mut params: Vec<String> = e
                .params
                .iter()
                .map(|(name, ty)| format!("{name} as {}", format_type(ty)))
                .collect();
            if e.variadic {
                params.push("...".into());
            }
            out.push_str(&params.join(", "));
            out.push(')');
            if e.ret != crate::types::Type::Void {
                out.push_str(&format!(" returns {}", format_type(&e.ret)));
            }
            out.push('\n');
        }
        Decl::Use(u) => {
            indent(out, level);
            out.push_str("use ");
            out.push_str(&Symbol::join_vec(&u.path, "/"));
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
                for et in &m.error_types {
                    out.push_str(&format!(" ! {}", format_type(et)));
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
            for f in &a.fields {
                indent(out, level + 1);
                out.push_str(&f.name.to_string());
                if let Some(ref ty) = f.ty {
                    out.push_str(&format!(" as {}", format_type(ty)));
                }
                if let Some(ref default) = f.default {
                    out.push_str(&format!(" is {}", format_expr(default)));
                }
                out.push('\n');
            }
            for h in &a.handlers {
                indent(out, level + 1);
                if h.is_loop {
                    out.push_str("*loop");
                    if let Some(ref sleep_ms) = h.loop_sleep_ms {
                        out.push(' ');
                        out.push_str(&format_expr(sleep_ms));
                    }
                } else {
                    out.push_str(&format!("@{}", h.name));
                    let ps: Vec<String> = h
                        .params
                        .iter()
                        .map(|p| match &p.ty {
                            Some(ty) => format!("{} as {}", p.name, format_type(ty)),
                            None => p.name.to_string(),
                        })
                        .collect();
                    if !ps.is_empty() {
                        out.push_str(&format!(" {}", ps.join(", ")));
                    }
                }
                out.push('\n');
                format_block(out, &h.body, level + 2, sink);
            }
        }
        Decl::Store(s) => {
            indent(out, level);
            out.push_str(&format!("store {}", s.name));
            for d in &s.decorators {
                out.push(' ');
                out.push_str(&format_store_decorator(d));
            }
            out.push('\n');
            for field in &s.fields {
                indent(out, level + 1);
                if field.is_relation {
                    out.push('&');
                }
                out.push_str(&field.name.to_string());
                if let Some(ref ty) = field.ty {
                    if field.is_has_many {
                        out.push_str(&format!(" as [{}]", format_type(ty)));
                    } else {
                        out.push_str(&format!(" as {}", format_type(ty)));
                    }
                }
                for d in &field.decorators {
                    out.push(' ');
                    out.push_str(&format_field_decorator(d));
                }
                out.push('\n');
            }
            for m in &s.methods {
                format_fn(out, m, level + 1, sink);
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
                    out.push_str(&format!("({})", ts.join(", ")));
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
    if !f.type_params.is_empty() {
        out.push_str(&format!(" of {}", Symbol::join_vec(&f.type_params, ", ")));
    }
    let visible_params: Vec<&Param> = f
        .params
        .iter()
        .filter(|p| p.name.as_str() != "self")
        .collect();
    if !visible_params.is_empty() {
        out.push('(');
        for (i, p) in visible_params.iter().enumerate() {
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
    for et in &f.error_types {
        out.push_str(&format!(" ! {}", format_type(et)));
    }
    out.push('\n');
    format_block(out, &f.body, level + 1, sink);
}

fn format_block(out: &mut String, stmts: &[Stmt], level: usize, sink: &mut CommentSink) {
    for stmt in stmts {
        sink.flush_before(out, stmt.span().start, level);
        format_stmt(out, stmt, level, sink);

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

fn format_postfix_recv(e: &Expr) -> String {
    match e {
        Expr::BinOp(..) | Expr::Ternary(..) | Expr::As(..) | Expr::UnaryOp(..) => {
            format!("({})", format_expr(e))
        }
        _ => format_expr(e),
    }
}

fn format_query_block(out: &mut String, src: &Expr, clauses: &[QueryClause], level: usize) {
    out.push_str(&format!("{} query\n", format_expr(src)));
    for c in clauses {
        indent(out, level + 1);
        match c {
            QueryClause::Where(e, _) => out.push_str(&format!("where {}", format_expr(e))),
            QueryClause::Limit(e, _) => out.push_str(&format!("limit {}", format_expr(e))),
            QueryClause::Sort(f, asc, _) => {
                if *asc {
                    out.push_str(&format!("sort {f}"));
                } else {
                    out.push_str(&format!("sort {f} desc"));
                }
            }
            QueryClause::Take(e, _) => out.push_str(&format!("take {}", format_expr(e))),
            QueryClause::Skip(e, _) => out.push_str(&format!("skip {}", format_expr(e))),
            QueryClause::Set(f, e, _) => out.push_str(&format!("set {f} is {}", format_expr(e))),
            QueryClause::Delete(_) => out.push_str("delete"),
        }
        out.push('\n');
    }
}

fn format_filter_cond(field: &Symbol, op: &BinOp, pred: &FilterPred, value: &Expr) -> String {
    match pred {
        FilterPred::Contains => format!("{field} contains {}", format_expr(value)),
        FilterPred::StartsWith => format!("{field} starts_with {}", format_expr(value)),
        FilterPred::EndsWith => format!("{field} ends_with {}", format_expr(value)),
        FilterPred::IEq => format!("{field} iequals {}", format_expr(value)),
        FilterPred::IContains => format!("{field} icontains {}", format_expr(value)),
        FilterPred::IStartsWith => format!("{field} istarts_with {}", format_expr(value)),
        FilterPred::IEndsWith => format!("{field} iends_with {}", format_expr(value)),
        FilterPred::Cmp => {
            let op_s = match op {
                BinOp::Eq => "equals",
                BinOp::Ne => "neq",
                BinOp::Lt => "<",
                BinOp::Gt => ">",
                BinOp::Le => "<=",
                BinOp::Ge => ">=",
                _ => "equals",
            };
            format!("{field} {op_s} {}", format_expr(value))
        }
    }
}

fn format_store_filter(f: &StoreFilter) -> String {
    let mut out = format!(
        " where {}",
        format_filter_cond(&f.field, &f.op, &f.pred, &f.value)
    );
    for (lop, c) in &f.extra {
        let l = match lop {
            LogicalOp::And => "and",
            LogicalOp::Or => "or",
        };
        out.push_str(&format!(
            " {l} {}",
            format_filter_cond(&c.field, &c.op, &c.pred, &c.value)
        ));
    }
    out
}

fn format_store_decorator(d: &StoreDecorator) -> String {
    match d {
        StoreDecorator::Simple => "@simple".into(),
        StoreDecorator::Mem => "@mem".into(),
        StoreDecorator::Transient => "@transient".into(),
        StoreDecorator::Versioned => "@versioned".into(),
        StoreDecorator::Vector(n) => format!("@vector({n})"),
        StoreDecorator::Compact(n) => format!("@compact({n})"),
        StoreDecorator::Graph => "@graph".into(),
        StoreDecorator::TimeSeries(f) => format!("@timeseries({f})"),
        StoreDecorator::Kv => "@kv".into(),
        StoreDecorator::BeforeInsert(f) => format!("@before_insert({f})"),
        StoreDecorator::AfterInsert(f) => format!("@after_insert({f})"),
        StoreDecorator::BeforeDelete(f) => format!("@before_delete({f})"),
        StoreDecorator::AfterDelete(f) => format!("@after_delete({f})"),
        StoreDecorator::Column => "@column".into(),
    }
}

fn format_field_decorator(d: &FieldDecorator) -> String {
    match d {
        FieldDecorator::Index => "@index".into(),
        FieldDecorator::Unique => "@unique".into(),
        FieldDecorator::Sorted => "@sorted".into(),
        FieldDecorator::Transient => "@transient".into(),
        FieldDecorator::Increment => "@increment".into(),
        FieldDecorator::Required => "@required".into(),
        FieldDecorator::Versioned => "@versioned".into(),
        FieldDecorator::Default(v) => {
            if v.chars()
                .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
                && !v.is_empty()
            {
                format!("@default({v})")
            } else {
                format!("@default('{v}')")
            }
        }
        FieldDecorator::Cascade => "@cascade".into(),
        FieldDecorator::Lazy => "@lazy".into(),
        FieldDecorator::Bloom => "@bloom".into(),
        FieldDecorator::Search => "@search".into(),
    }
}

fn format_stmt(out: &mut String, stmt: &Stmt, level: usize, sink: &mut CommentSink) {
    match stmt {
        Stmt::Bind(b) => {
            indent(out, level);
            out.push_str(&b.name.to_string());
            if let Some(ref ty) = b.ty {
                out.push_str(&format!(" as {}", format_type(ty)));
            }
            out.push_str(" is ");
            match b.access_mod {
                Some(AccessMod::Take) => out.push_str("take "),
                Some(AccessMod::Copy) => out.push_str("copy "),
                Some(AccessMod::Const) => out.push_str("const "),
                None => {}
            }
            if let Expr::DispatchBlock(_, body, _) = &b.value {
                out.push_str("dispatch\n");
                format_block(out, body, level + 1, sink);
            } else if let Expr::Query(src, clauses, _) = &b.value {
                format_query_block(out, src, clauses, level);
            } else {
                out.push_str(&format_expr(&b.value));
                out.push('\n');
            }
        }
        Stmt::Assign(lhs, rhs, _) => {
            indent(out, level);
            out.push_str(&format_expr(lhs));
            out.push_str(" is ");
            out.push_str(&format_expr(rhs));
            out.push('\n');
        }
        Stmt::Expr(Expr::IfExpr(i)) => format_if(out, i, level, sink),
        Stmt::Expr(Expr::Query(src, clauses, _)) => {
            indent(out, level);
            format_query_block(out, src, clauses, level);
        }
        Stmt::Expr(Expr::Select(arms, default_body, _)) => {
            indent(out, level);
            out.push_str("select\n");
            for arm in arms {
                indent(out, level + 1);
                if arm.is_send {
                    out.push_str(&format!("send {}", format_expr(&arm.chan)));
                    if let Some(ref v) = arm.value {
                        out.push_str(&format!(", {}", format_expr(v)));
                    }
                } else {
                    out.push_str(&format!("receive {}", format_expr(&arm.chan)));
                    if let Some(ref b) = arm.binding {
                        out.push_str(&format!(" as {b}"));
                    }
                }
                out.push('\n');
                format_block(out, &arm.body, level + 2, sink);
            }
            if let Some(body) = default_body {
                indent(out, level + 1);
                out.push_str("default\n");
                format_block(out, body, level + 2, sink);
            }
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
            if let Some(ref b2) = f.bind2 {
                out.push_str(&format!(", {b2}"));
            }
            out.push_str(" in ");
            out.push_str(&format_expr(&f.iter));
            if let Some(ref end) = f.end {
                out.push_str(&format!(" to {}", format_expr(end)));
            }
            if let Some(ref step) = f.step {
                out.push_str(&format!(" by {}", format_expr(step)));
            }
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

                if arm.body.is_empty() {
                    out.push_str(" nop\n");
                } else if arm.body.len() == 1
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
            out.push_str(&format!("insert {name} "));
            let vals: Vec<String> = exprs
                .iter()
                .map(|fi| match &fi.name {
                    Some(fname) => format!("{fname} is {}", format_expr(&fi.value)),
                    None => format_expr(&fi.value),
                })
                .collect();
            out.push_str(&vals.join(", "));
            out.push('\n');
        }
        Stmt::StoreDelete(name, filter, _) => {
            indent(out, level);
            out.push_str(&format!("delete {name}{}\n", format_store_filter(filter)));
        }
        Stmt::StoreDestroy(name, filter, _) => {
            indent(out, level);
            out.push_str(&format!("destroy {name}{}\n", format_store_filter(filter)));
        }
        Stmt::StoreRestore(name, filter, _) => {
            indent(out, level);
            out.push_str(&format!("restore {name}{}\n", format_store_filter(filter)));
        }
        Stmt::StoreSave(name, _) => {
            indent(out, level);
            out.push_str(&format!("save {name}\n"));
        }
        Stmt::StoreCompact(name, _) => {
            indent(out, level);
            out.push_str(&format!("compact {name}\n"));
        }
        Stmt::StoreSet(name, assignments, filter, _) => {
            indent(out, level);
            out.push_str(&format!("set {name}{}", format_store_filter(filter)));
            let assigns: Vec<String> = assignments
                .iter()
                .map(|(k, v)| format!("{k} {}", format_expr(v)))
                .collect();
            out.push_str(&format!(" {}\n", assigns.join(", ")));
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
            out.push_str(&Symbol::join_vec(&u.path, "/"));
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

fn escape_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '{' => out.push_str("\\{"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn format_expr(e: &Expr) -> String {
    match e {
        Expr::None(_) => "none".into(),
        Expr::Void(_) => "void".into(),
        Expr::Int(n, _) => n.to_string(),
        Expr::Float(f, _) => {
            let t = format!("{f}");
            if t.chars().all(|c| c.is_ascii_digit() || c == '-') {
                format!("{t}.0")
            } else {
                t
            }
        }
        Expr::Str(s, _) => format!("'{}'", escape_string_literal(s)),
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
                BinOp::Ne => "neq",
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
            let wrap = |e: &Expr| -> String {
                match e {
                    Expr::BinOp(_, sub_op, _, _) if sub_op != op => {
                        format!("({})", format_expr(e))
                    }
                    Expr::Ternary(..) => format!("({})", format_expr(e)),
                    _ => format_expr(e),
                }
            };
            format!("{} {} {}", wrap(l), ops, wrap(r))
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
            format!(
                "{}.{method}({})",
                format_postfix_recv(obj),
                arg_strs.join(", ")
            )
        }
        Expr::Field(obj, field, _) => format!("{}.{field}", format_postfix_recv(obj)),
        Expr::Index(arr, idx, _) => {
            format!("{}[{}]", format_postfix_recv(arr), format_expr(idx))
        }
        Expr::Ternary(c, t, f, _) => {
            if matches!(**f, Expr::Void(_)) {
                format!("{} ? {}", format_expr(c), format_expr(t))
            } else if matches!(**t, Expr::Void(_)) {
                format!("{} ?! {}", format_expr(c), format_expr(f))
            } else {
                format!(
                    "{} ? {} ! {}",
                    format_expr(c),
                    format_expr(t),
                    format_expr(f)
                )
            }
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
        Expr::As(e, ty, _) => match **e {
            Expr::BinOp(..) | Expr::Ternary(..) => {
                format!("({}) as {}", format_expr(e), format_type(ty))
            }
            _ => format!("{} as {}", format_expr(e), format_type(ty)),
        },
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
            let then_txt = if i.then.len() == 1 {
                format_expr_from_stmt(&i.then[0])
            } else {
                "...".into()
            };
            match &i.els {
                Some(els) => {
                    let els_txt = if els.len() == 1 {
                        format_expr_from_stmt(&els[0])
                    } else {
                        "...".into()
                    };
                    format!("{} ? {} ! {}", format_expr(&i.cond), then_txt, els_txt)
                }
                None => format!("{} ? {}", format_expr(&i.cond), then_txt),
            }
        }
        Expr::Pipe(l, r, rest, _) => {
            let mut out = format!("{} ~ {}", format_expr(l), format_expr(r));
            for e in rest {
                out.push_str(&format!(", {}", format_expr(e)));
            }
            out
        }
        Expr::Block(_, _) => "do ... end".into(),
        Expr::Lambda(params, _, body, _) => {
            let ps: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();

            let body_txt = match body.last() {
                Some(Stmt::Expr(e)) => format_expr(e),
                Some(Stmt::Ret(Some(e), _)) => format_expr(e),
                _ => "0".to_string(),
            };
            format!("|{}| {}", ps.join(", "), body_txt)
        }
        Expr::Placeholder(_) => "$".into(),
        Expr::IndexPlaceholder(_) => "$$".into(),
        Expr::Ref(e, _) => format!("%{}", format_expr(e)),
        Expr::Deref(e, _) => format!("@{}", format_expr(e)),
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
        Expr::StoreQuery(store, filter, _) => {
            format!("{store}{}", format_store_filter(filter))
        }
        Expr::StoreCount(store, filter, _) => match filter {
            Some(f) => format!("count {store}{}", format_store_filter(f)),
            None => format!("count {store}"),
        },
        Expr::StoreAll(store, _) => format!("all {store}"),
        Expr::StoreGet(store, key, _) => format!("get {store} {}", format_expr(key)),
        Expr::StoreFirst(store, filter, _) => {
            format!("first {store}{}", format_store_filter(filter))
        }
        Expr::StoreExists(store, filter, _) => {
            format!("exists {store}{}", format_store_filter(filter))
        }
        Expr::StoreDistinct(store, field, _) => format!("distinct {store} {field}"),
        Expr::StoreInsert(store, exprs, _) => {
            let vals: Vec<String> = exprs
                .iter()
                .map(|fi| match &fi.name {
                    Some(fname) => format!("{fname} is {}", format_expr(&fi.value)),
                    None => format_expr(&fi.value),
                })
                .collect();
            format!("insert {store} {}", vals.join(", "))
        }
        Expr::StoreUpdate(store, assignments, filter, _) => {
            let assigns: Vec<String> = assignments
                .iter()
                .map(|(k, v)| format!("{k} {}", format_expr(v)))
                .collect();
            format!(
                "set {store}{} {}",
                format_store_filter(filter),
                assigns.join(", ")
            )
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
        Type::Fn(params, ret) => {
            let ps: Vec<String> = params.iter().map(format_type).collect();
            format!("({}) returns {}", ps.join(", "), format_type(ret))
        }
        Type::Vec(inner) => format!("Vec of {}", format_type(inner)),
        Type::Map(k, v) if matches!(**k, Type::String) => {
            format!("Map of {}", format_type(v))
        }
        Type::Map(k, v) => format!("Map of {}, {}", format_type(k), format_type(v)),
        Type::Ptr(inner) => format!("%{}", format_type(inner)),
        Type::Struct(n, args) if !args.is_empty() => {
            let ts: Vec<String> = args.iter().map(format_type).collect();
            format!("{n} of {}", ts.join(", "))
        }
        _ => format!("{ty}"),
    }
}
