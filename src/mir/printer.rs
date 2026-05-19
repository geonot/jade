use super::*;
use std::fmt::Write;

fn fmt_args(args: &[impl std::fmt::Display]) -> String {
    args.iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn print_program(prog: &Program) -> String {
    let mut out = String::new();

    for td in &prog.types {
        let _ = writeln!(out, "type {} {{", td.name);
        for (fname, fty) in &td.fields {
            let _ = writeln!(out, "  {}: {:?}", fname, fty);
        }
        let _ = writeln!(out, "}}\n");
    }

    for ext in &prog.externs {
        let _ = write!(out, "extern fn {}(", ext.name);
        for (i, p) in ext.params.iter().enumerate() {
            if i > 0 {
                let _ = write!(out, ", ");
            }
            let _ = write!(out, "{:?}", p);
        }
        let _ = writeln!(out, ") -> {:?}\n", ext.ret);
    }

    for func in &prog.functions {
        out.push_str(&print_function(func));
        out.push('\n');
    }

    out
}

pub fn print_function(func: &Function) -> String {
    let mut out = String::new();

    let _ = write!(out, "fn {}(", func.name);
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 {
            let _ = write!(out, ", ");
        }
        let _ = write!(out, "{}: {:?} = {}", p.name, p.ty, p.value);
    }
    let _ = writeln!(out, ") -> {:?} {{", func.ret_ty);

    for block in &func.blocks {
        let _ = writeln!(out, "  {}:  // {}", block.id, block.label);

        for phi in &block.phis {
            let _ = write!(out, "    {} = phi {:?} ", phi.dest, phi.ty);
            for (i, (bb, val)) in phi.incoming.iter().enumerate() {
                if i > 0 {
                    let _ = write!(out, ", ");
                }
                let _ = write!(out, "[{}: {}]", bb, val);
            }
            let _ = writeln!(out);
        }

        for inst in &block.insts {
            if let Some(dest) = inst.dest {
                let _ = write!(out, "    {} = ", dest);
            } else {
                let _ = write!(out, "    ");
            }
            let _ = write!(out, "{}", format_inst_kind(&inst.kind));
            if !matches!(inst.ty, crate::types::Type::Void) {
                let _ = write!(out, "  // {:?}", inst.ty);
            }
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "    {}", format_terminator(&block.terminator));
    }

    let _ = writeln!(out, "}}");
    out
}

fn format_inst_kind(kind: &InstKind) -> String {
    match kind {
        InstKind::IntConst(n) => format!("int {n}"),
        InstKind::FloatConst(f) => format!("float {f}"),
        InstKind::BoolConst(b) => format!("bool {b}"),
        InstKind::StringConst(s) => format!("str {s:?}"),
        InstKind::Void => "void".into(),

        InstKind::BinOp(op, l, r) => format!("{op} {l} {r}"),
        InstKind::UnaryOp(op, v) => format!("{op:?} {v}"),
        InstKind::Cmp(op, l, r, _) => format!("{op} {l} {r}"),

        InstKind::Call(name, args) => format!("call {name}({})", fmt_args(args)),
        InstKind::MethodCall(obj, name, args, borrow) => {
            let suffix = if *borrow { " [borrow]" } else { "" };
            format!("method_call {obj}.{name}({}){suffix}", fmt_args(args))
        }
        InstKind::IndirectCall(f, args) => format!("indirect_call {f}({})", fmt_args(args)),

        InstKind::FnRef(name) => format!("fn_ref {name}"),
        InstKind::Load(name) => format!("load {name}"),
        InstKind::Store(name, val) => format!("store {name} {val}"),
        InstKind::GlobalLoad(name) => format!("global_load {name}"),
        InstKind::GlobalStore(name, val) => format!("global_store {name} {val}"),

        InstKind::FieldGet(obj, field) => format!("field_get {obj}.{field}"),
        InstKind::FieldSet(obj, field, val) => format!("field_set {obj}.{field} = {val}"),
        InstKind::FieldStore(var, field, val) => format!("field_store ${var}.{field} = {val}"),
        InstKind::FieldTombstone(var, field) => format!("field_tombstone ${var}.{field}"),

        InstKind::Index(arr, idx) => format!("index {arr}[{idx}]"),
        InstKind::IndexUnchecked(arr, idx) => format!("index_unchecked {arr}[{idx}]"),
        InstKind::IndexSet(arr, idx, val) => format!("index_set {arr}[{idx}] = {val}"),
        InstKind::IndexStore(var, idx, val) => format!("index_store ${var}[{idx}] = {val}"),

        InstKind::StructInit(name, fields) => {
            let fs = fields
                .iter()
                .map(|(n, v)| format!("{n}: {v}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("struct_init {name} {{ {fs} }}")
        }
        InstKind::VariantInit(enum_name, variant, tag, args) => {
            format!(
                "variant_init {enum_name}::{variant} (tag={tag}) ({})",
                fmt_args(args)
            )
        }
        InstKind::ArrayInit(vals) => format!("array [{}]", fmt_args(vals)),

        InstKind::Cast(v, ty) => format!("cast {v} as {ty:?}"),
        InstKind::StrictCast(v, ty) => format!("strict_cast {v} as {ty:?}"),
        InstKind::Ref(v) => format!("ref {v}"),
        InstKind::Deref(v) => format!("deref {v}"),
        InstKind::Alloc(v) => format!("alloc {v}"),
        InstKind::Drop(v, ty) => format!("drop {v} {ty:?}"),
        InstKind::DropMany(items) => {
            let parts: Vec<String> = items.iter().map(|(v, ty)| format!("{v}:{ty:?}")).collect();
            format!("drop_many [{}]", parts.join(", "))
        }
        InstKind::Copy(v) => format!("copy {v}"),
        InstKind::Clone(v, ty) => format!("clone {v} {ty:?}"),
        InstKind::Slice(a, s, e) => format!("slice {a}[{s}..{e}]"),

        InstKind::VecNew(vals) => format!("vec_new [{}]", fmt_args(vals)),
        InstKind::VecPush(vec, val) => format!("vec_push {vec} {val}"),
        InstKind::VecLen(v) => format!("vec_len {v}"),
        InstKind::MapInit => "map_init".into(),

        InstKind::ClosureCreate(name, captures) => {
            format!("closure_create {name}({})", fmt_args(captures))
        }
        InstKind::ClosureCall(f, args) => format!("closure_call {f}({})", fmt_args(args)),

        InstKind::SpawnActor(name, args) => format!(
            "spawn_actor {name}({})",
            args.iter()
                .map(|(n, v)| format!("{n} is %{v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        InstKind::ChanCreate(ty, _cap) => format!("chan_create {ty:?}"),
        InstKind::ChanSend(ch, val) => format!("chan_send {ch} {val}"),
        InstKind::ChanRecv(ch) => format!("chan_recv {ch}"),
        InstKind::SelectArm(arms, _) => format!("select [{}]", fmt_args(arms)),

        InstKind::Log(v) => format!("log {v}"),
        InstKind::Assert(v, msg) => format!("assert {v} {msg:?}"),

        InstKind::InlineAsm(template, args) => {
            format!("asm {:?} ({})", template, fmt_args(args))
        }
    }
}

fn format_terminator(term: &Terminator) -> String {
    match term {
        Terminator::Goto(bb) => format!("goto {bb}"),
        Terminator::Branch(cond, t, e) => format!("branch {cond} ? {t} : {e}"),
        Terminator::Return(Some(v)) => format!("return {v}"),
        Terminator::Return(None) => "return void".into(),
        Terminator::Switch(val, arms, default) => {
            let arms_str = arms
                .iter()
                .map(|(n, bb)| format!("{n} => {bb}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("switch {val} [{arms_str}] default {default}")
        }
        Terminator::Unreachable => "unreachable".into(),
    }
}
