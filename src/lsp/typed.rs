use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::hir;
use crate::intern::Symbol;
use crate::lexer::Lexer;
use crate::parser::{ParseError, Parser};
use crate::typer::Typer;

use super::analysis::{DiagSeverity, LspDiag};

pub struct TypedAnalysis {
    pub typed: bool,
    pub diagnostics: Vec<LspDiag>,
    pub hovers: Vec<(usize, usize, String)>,
    pub occurrences: HashMap<u32, Vec<(usize, usize)>>,
    pub def_sites: HashMap<u32, (usize, usize)>,
    pub use_sites: Vec<(usize, usize, u32)>,
}

impl TypedAnalysis {
    fn empty() -> Self {
        TypedAnalysis {
            typed: false,
            diagnostics: Vec::new(),
            hovers: Vec::new(),
            occurrences: HashMap::new(),
            def_sites: HashMap::new(),
            use_sites: Vec::new(),
        }
    }

    pub fn hover_at(&self, offset: usize) -> Option<&str> {
        self.hovers
            .iter()
            .filter(|(s, e, _)| *s <= offset && offset < *e)
            .min_by_key(|(s, e, _)| e - s)
            .map(|(_, _, text)| text.as_str())
    }

    pub fn def_id_at(&self, offset: usize) -> Option<u32> {
        if let Some((_, _, id)) = self
            .use_sites
            .iter()
            .filter(|(s, e, _)| *s <= offset && offset < *e)
            .min_by_key(|(s, e, _)| e - s)
        {
            return Some(*id);
        }
        self.def_sites
            .iter()
            .find(|(_, (s, e))| *s <= offset && offset < *e)
            .map(|(id, _)| *id)
    }
}

pub fn position_to_offset(src: &str, line: u32, character: u32) -> Option<usize> {
    let line_start = line_start_offset(src, line)?;
    let line_text = &src[line_start..];
    let line_text = line_text.split(['\n']).next().unwrap_or("");
    let mut units = 0u32;
    if character == 0 {
        return Some(line_start);
    }
    for (byte_idx, ch) in line_text.char_indices() {
        units += ch.len_utf16() as u32;
        if units >= character {
            return Some(line_start + byte_idx + ch.len_utf8());
        }
    }
    Some(line_start + line_text.len())
}

pub fn offset_to_position(src: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(src.len());
    let before = &src[..offset];
    let line = before.bytes().filter(|b| *b == b'\n').count() as u32;
    let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col = src[line_start..offset]
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    (line, col)
}

fn line_start_offset(src: &str, line: u32) -> Option<usize> {
    if line == 0 {
        return Some(0);
    }
    let mut seen = 0u32;
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            seen += 1;
            if seen == line {
                return Some(i + 1);
            }
        }
    }
    None
}

fn uri_to_dir(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    let p = PathBuf::from(path);
    p.parent().map(|d| d.to_path_buf())
}

pub fn analyze_typed(uri: &str, src: &str) -> TypedAnalysis {
    let src_owned = src.to_string();
    let dir = uri_to_dir(uri);
    let spawned = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || analyze_typed_inner(&src_owned, dir));
    match spawned {
        Ok(handle) => match handle.join() {
            Ok(a) => a,
            Err(_) => ice_analysis(),
        },
        Err(_) => ice_analysis(),
    }
}

fn ice_analysis() -> TypedAnalysis {
    let mut a = TypedAnalysis::empty();
    a.diagnostics.push(LspDiag {
        line: 1,
        col: 1,
        end_col: 1,
        message: "internal error: the compiler frontend panicked while analyzing this file"
            .to_string(),
        severity: DiagSeverity::Error,
    });
    a
}

fn located_diag(line: u32, col: u32, message: String, severity: DiagSeverity) -> LspDiag {
    LspDiag {
        line: line.max(1),
        col: col.max(1),
        end_col: col.max(1) + 1,
        message,
        severity,
    }
}

fn parse_located_line(line: &str) -> Option<(u32, u32, String)> {
    let rest = line.strip_prefix("line ")?;
    let colon = rest.find(':')?;
    let l: u32 = rest[..colon].parse().ok()?;
    let rest2 = &rest[colon + 1..];
    let colon2 = rest2.find(':')?;
    let c: u32 = rest2[..colon2].parse().ok()?;
    let msg = rest2[colon2 + 1..].trim_start().to_string();
    Some((l, c, msg))
}

fn push_message_lines(out: &mut Vec<LspDiag>, text: &str, severity: DiagSeverity) {
    for line in text.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some((l, c, msg)) = parse_located_line(line) {
            out.push(located_diag(
                l,
                c,
                msg,
                match severity {
                    DiagSeverity::Error => DiagSeverity::Error,
                    DiagSeverity::Warning => DiagSeverity::Warning,
                },
            ));
        } else if let Some(last) = out.last_mut() {
            last.message.push('\n');
            last.message.push_str(line.trim_start());
        } else {
            out.push(located_diag(
                1,
                1,
                line.trim_start().to_string(),
                match severity {
                    DiagSeverity::Error => DiagSeverity::Error,
                    DiagSeverity::Warning => DiagSeverity::Warning,
                },
            ));
        }
    }
}

fn analyze_typed_inner(src: &str, dir: Option<PathBuf>) -> TypedAnalysis {
    let mut a = TypedAnalysis::empty();

    let tokens = match Lexer::new(src).tokenize() {
        Ok(t) => t,
        Err(e) => {
            push_message_lines(&mut a.diagnostics, &e.to_string(), DiagSeverity::Error);
            return a;
        }
    };
    let mut prog = match Parser::new(tokens).parse_program() {
        Ok(p) => p,
        Err(ParseError::Error { line, col, msg }) => {
            let mut lines = msg.lines();
            if let Some(first) = lines.next() {
                a.diagnostics.push(located_diag(
                    line,
                    col,
                    first.to_string(),
                    DiagSeverity::Error,
                ));
            }
            let rest: String = lines.collect::<Vec<_>>().join("\n");
            push_message_lines(&mut a.diagnostics, &rest, DiagSeverity::Error);
            return a;
        }
        Err(e) => {
            push_message_lines(&mut a.diagnostics, &e.to_string(), DiagSeverity::Error);
            return a;
        }
    };

    let mut std_files: HashSet<Symbol> = HashSet::new();
    let has_uses = prog
        .decls
        .iter()
        .any(|d| matches!(d, crate::ast::Decl::Use(_)));
    if has_uses {
        let base_dir = dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let mut loaded: HashSet<Symbol> = HashSet::new();
        let packages: HashMap<Symbol, PathBuf> = HashMap::new();
        if let Err(e) = crate::driver::resolve_modules(
            &mut prog,
            &base_dir,
            &mut loaded,
            &packages,
            &mut std_files,
        ) {
            a.diagnostics.push(located_diag(
                1,
                1,
                format!("{e}; type analysis skipped for this file"),
                DiagSeverity::Warning,
            ));
            return a;
        }
    }

    let mut typer = Typer::new();
    if let Some(d) = dir {
        typer.set_source_dir(d);
    }
    typer.set_std_files(std_files);

    let result = typer.lower_program(&prog);
    for w in &typer.warnings {
        push_message_lines(&mut a.diagnostics, w, DiagSeverity::Warning);
    }
    match result {
        Ok(hprog) => {
            a.typed = true;
            let sigs = fn_signatures(&typer);
            let mut walk = Walk {
                src,
                a: &mut a,
                sigs: &sigs,
            };
            walk.program(&hprog);
        }
        Err(e) => {
            let text = e.strip_prefix("type checking failed:\n").unwrap_or(&e);
            push_message_lines(&mut a.diagnostics, text, DiagSeverity::Error);
        }
    }
    a
}

fn fn_signatures(typer: &Typer) -> HashMap<Symbol, String> {
    let mut sigs = HashMap::new();
    for (name, (_, params, ret)) in typer.fns.iter() {
        let pnames = typer.fn_param_names.get(name);
        let rendered: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(i, ty)| match pnames.and_then(|ns| ns.get(i)) {
                Some(n) => format!("{n} as {ty}"),
                None => format!("{ty}"),
            })
            .collect();
        let ret_str = if matches!(ret, crate::types::Type::Void) {
            String::new()
        } else {
            format!(" returns {ret}")
        };
        sigs.insert(*name, format!("*{name}({}){ret_str}", rendered.join(", ")));
    }
    sigs
}

struct Walk<'a> {
    src: &'a str,
    a: &'a mut TypedAnalysis,
    sigs: &'a HashMap<Symbol, String>,
}

impl Walk<'_> {
    fn name_range(&self, span: &crate::ast::Span, name: &str) -> Option<(usize, usize)> {
        if span.file.is_some() || name.is_empty() {
            return None;
        }
        let start = span.start;
        if start >= self.src.len() {
            return None;
        }
        let window_end = (start + 160).min(self.src.len());
        let window = &self.src[start..window_end];
        let mut search = 0;
        while let Some(pos) = window[search..].find(name) {
            let abs = search + pos;
            let before_ok = abs == 0
                || !window.as_bytes()[abs - 1].is_ascii_alphanumeric()
                    && window.as_bytes()[abs - 1] != b'_';
            let after = abs + name.len();
            let after_ok = after >= window.len()
                || !window.as_bytes()[after].is_ascii_alphanumeric()
                    && window.as_bytes()[after] != b'_';
            if before_ok && after_ok {
                return Some((start + abs, start + abs + name.len()));
            }
            search = abs + 1;
        }
        None
    }

    fn record_def(
        &mut self,
        id: hir::DefId,
        name: Symbol,
        ty_text: String,
        span: &crate::ast::Span,
    ) {
        let Some((s, e)) = self.name_range(span, &name.as_str()) else {
            return;
        };
        self.a.def_sites.insert(id.0, (s, e));
        self.a.occurrences.entry(id.0).or_default().push((s, e));
        self.a.hovers.push((s, e, ty_text));
    }

    fn record_use(
        &mut self,
        id: hir::DefId,
        name: Symbol,
        ty_text: String,
        span: &crate::ast::Span,
    ) {
        let Some((s, e)) = self.name_range(span, &name.as_str()) else {
            return;
        };
        self.a.use_sites.push((s, e, id.0));
        self.a.occurrences.entry(id.0).or_default().push((s, e));
        self.a.hovers.push((s, e, ty_text));
    }

    fn program(&mut self, p: &hir::Program) {
        for f in &p.fns {
            let sig = self
                .sigs
                .get(&f.name)
                .cloned()
                .unwrap_or_else(|| format!("*{}", f.name));
            self.record_def(f.def_id, f.name, sig, &f.span);
            for param in &f.params {
                self.record_def(
                    param.def_id,
                    param.name,
                    format!("{} as {}", param.name, param.ty),
                    &param.span,
                );
            }
            self.block(&f.body);
        }
    }

    fn block(&mut self, b: &hir::Block) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &hir::Stmt) {
        match s {
            hir::Stmt::Bind(b) => {
                self.record_def(b.def_id, b.name, format!("{}: {}", b.name, b.ty), &b.span);
                self.expr(&b.value);
            }
            hir::Stmt::TupleBind(binds, value, span) => {
                for (id, name, ty) in binds {
                    self.record_def(*id, *name, format!("{name}: {ty}"), span);
                }
                self.expr(value);
            }
            hir::Stmt::Assign(t, v, _) => {
                self.expr(t);
                self.expr(v);
            }
            hir::Stmt::Expr(e) => self.expr(e),
            hir::Stmt::If(i) => self.if_stmt(i),
            hir::Stmt::While(w) => {
                self.expr(&w.cond);
                self.block(&w.body);
            }
            hir::Stmt::For(f) | hir::Stmt::SimFor(f, _) => self.for_stmt(f),
            hir::Stmt::Loop(l) => self.block(&l.body),
            hir::Stmt::Ret(Some(e), _, _) => self.expr(e),
            hir::Stmt::Break(Some(e), _) => self.expr(e),
            hir::Stmt::Match(m) => {
                self.expr(&m.subject);
                for arm in &m.arms {
                    self.pat(&arm.pat);
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    self.block(&arm.body);
                }
            }
            hir::Stmt::ErrReturn(e, _, _) => self.expr(e),
            hir::Stmt::Defer(b, _) | hir::Stmt::SimBlock(b, _) | hir::Stmt::Transaction(b, _) => {
                self.block(b)
            }
            hir::Stmt::Together(_, b, _, handler, _) => {
                self.block(b);
                if let Some(h) = handler {
                    if let Some(arm) = &h.err_arm {
                        self.block(arm);
                    }
                    if let Some(arm) = &h.ok_arm {
                        self.block(arm);
                    }
                }
            }
            hir::Stmt::StoreInsert(_, exprs, _) => {
                for e in exprs {
                    self.expr(e);
                }
            }
            hir::Stmt::StoreSet(_, sets, filter, _) => {
                for (_, e) in sets {
                    self.expr(e);
                }
                self.filter(filter);
            }
            hir::Stmt::StoreDelete(_, filter, _)
            | hir::Stmt::StoreDestroy(_, filter, _)
            | hir::Stmt::StoreRestore(_, filter, _) => self.filter(filter),
            hir::Stmt::ChannelClose(e, _)
            | hir::Stmt::Stop(e, _)
            | hir::Stmt::Join(e, _)
            | hir::Stmt::GlobalStore(_, e, _) => self.expr(e),
            _ => {}
        }
    }

    fn if_stmt(&mut self, i: &hir::If) {
        self.expr(&i.cond);
        self.block(&i.then);
        for (c, b) in &i.elifs {
            self.expr(c);
            self.block(b);
        }
        if let Some(e) = &i.els {
            self.block(e);
        }
    }

    fn for_stmt(&mut self, f: &hir::For) {
        self.record_def(
            f.bind_id,
            f.bind,
            format!("{}: {}", f.bind, f.bind_ty),
            &f.span,
        );
        if let (Some(id2), Some(b2), Some(t2)) = (f.bind2_id, f.bind2, &f.bind2_ty) {
            self.record_def(id2, b2, format!("{b2}: {t2}"), &f.span);
        }
        self.expr(&f.iter);
        if let Some(e) = &f.end {
            self.expr(e);
        }
        if let Some(e) = &f.step {
            self.expr(e);
        }
        self.block(&f.body);
    }

    fn pat(&mut self, p: &hir::Pat) {
        match p {
            hir::Pat::Bind(id, name, ty, span) => {
                self.record_def(*id, *name, format!("{name}: {ty}"), span);
            }
            hir::Pat::Ctor(_, _, pats, _) => {
                for sub in pats {
                    self.pat(sub);
                }
            }
            hir::Pat::Or(pats, _) | hir::Pat::Tuple(pats, _) | hir::Pat::Array(pats, _) => {
                for sub in pats {
                    self.pat(sub);
                }
            }
            _ => {}
        }
    }

    fn filter(&mut self, f: &hir::StoreFilter) {
        self.expr(&f.value);
        for (_, cond) in &f.extra {
            self.expr(&cond.value);
        }
    }

    fn expr(&mut self, e: &hir::Expr) {
        match &e.kind {
            hir::ExprKind::Var(id, name) => {
                self.record_use(*id, *name, format!("{}: {}", name, e.ty), &e.span);
            }
            hir::ExprKind::FnRef(id, name) => {
                let sig = self
                    .sigs
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| format!("{}: {}", name, e.ty));
                self.record_use(*id, *name, sig, &e.span);
            }
            hir::ExprKind::Call(id, name, args) | hir::ExprKind::Pipe(_, id, name, args) => {
                if let Some(sig) = self.sigs.get(name) {
                    self.record_use(*id, *name, sig.clone(), &e.span);
                }
                if let hir::ExprKind::Pipe(recv, _, _, _) = &e.kind {
                    self.expr(recv);
                }
                for arg in args {
                    self.expr(arg);
                }
            }
            hir::ExprKind::BinOp(l, _, r) => {
                self.expr(l);
                self.expr(r);
            }
            hir::ExprKind::UnaryOp(_, inner)
            | hir::ExprKind::Coerce(inner, _)
            | hir::ExprKind::Cast(inner, _)
            | hir::ExprKind::Ref(inner)
            | hir::ExprKind::Deref(inner) => self.expr(inner),
            hir::ExprKind::Field(obj, fname, _) => {
                self.expr(obj);
                if let Some((s, end)) = {
                    let fspan = crate::ast::Span {
                        start: obj.span.end,
                        end: e.span.end.max(obj.span.end),
                        line: e.span.line,
                        col: e.span.col,
                        file: e.span.file,
                    };
                    self.name_range(&fspan, &fname.as_str())
                } {
                    self.a.hovers.push((s, end, format!("{}: {}", fname, e.ty)));
                }
            }
            hir::ExprKind::Index(obj, idx) => {
                self.expr(obj);
                self.expr(idx);
            }
            hir::ExprKind::Ternary(c, t, f) => {
                self.expr(c);
                self.expr(t);
                self.expr(f);
            }
            hir::ExprKind::IndirectCall(callee, args) => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            hir::ExprKind::Builtin(_, args)
            | hir::ExprKind::VecNew(args)
            | hir::ExprKind::Array(args)
            | hir::ExprKind::Tuple(args) => {
                for a in args {
                    self.expr(a);
                }
            }
            hir::ExprKind::Method(recv, _, _, args)
            | hir::ExprKind::StringMethod(recv, _, args)
            | hir::ExprKind::DeferredMethod(recv, _, args)
            | hir::ExprKind::VecMethod(recv, _, args)
            | hir::ExprKind::MapMethod(recv, _, args) => {
                self.expr(recv);
                for a in args {
                    self.expr(a);
                }
            }
            hir::ExprKind::Struct(_, inits) | hir::ExprKind::VariantCtor(_, _, _, inits) => {
                for init in inits {
                    self.expr(&init.value);
                }
            }
            hir::ExprKind::IfExpr(i) => self.if_stmt(i),
            hir::ExprKind::Block(b) => self.block(b),
            hir::ExprKind::Lambda(params, body) => {
                for p in params {
                    self.record_def(p.def_id, p.name, format!("{} as {}", p.name, p.ty), &p.span);
                }
                self.block(body);
            }
            _ => {}
        }
    }
}
