//! Pretty-printer for MIR.

use std::fmt::Write;
use super::*;

/// Format a comma-separated list of displayable values.
fn fmt_args(args: &[impl std::fmt::Display]) -> String {
    args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")
}

/// Print the entire MIR program.
pub fn print_program(prog: &Program) -> String {
    let mut out = String::new();

    for td in &prog.types {
        writeln!(out, "type {} {{", td.name).unwrap();
        for (fname, fty) in &td.fields {
            writeln!(out, "  {}: {:?}", fname, fty).unwrap();
        }
        writeln!(out, "}}\n").unwrap();
    }

    for ext in &prog.externs {
        write!(out, "extern fn {}(", ext.name).unwrap();
        for (i, p) in ext.params.iter().enumerate() {
            if i > 0 { write!(out, ", ").unwrap(); }
            write!(out, "{:?}", p).unwrap();
        }
        writeln!(out, ") -> {:?}\n", ext.ret).unwrap();
    }

    for func in &prog.functions {
        out.push_str(&print_function(func));
        out.push('\n');
    }

    out
}

/// Print a single MIR function.
pub fn print_function(func: &Function) -> String {
    let mut out = String::new();

    write!(out, "fn {}(", func.name).unwrap();
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 { write!(out, ", ").unwrap(); }
        write!(out, "{}: {:?} = {}", p.name, p.ty, p.value).unwrap();
    }
    writeln!(out, ") -> {:?} {{", func.ret_ty).unwrap();

    for block in &func.blocks {
        writeln!(out, "  {}:  // {}", block.id, block.label).unwrap();

        for phi in &block.phis {
            write!(out, "    {} = phi {:?} ", phi.dest, phi.ty).unwrap();
            for (i, (bb, val)) in phi.incoming.iter().enumerate() {
                if i > 0 { write!(out, ", ").unwrap(); }
                write!(out, "[{}: {}]", bb, val).unwrap();
            }
            writeln!(out).unwrap();
        }

        for inst in &block.insts {
            if let Some(dest) = inst.dest {
                write!(out, "    {} = ", dest).unwrap();
            } else {
                write!(out, "    ").unwrap();
            }
            write!(out, "{}", format_inst_kind(&inst.kind)).unwrap();
            if !matches!(inst.ty, crate::types::Type::Void) {
                write!(out, "  // {:?}", inst.ty).unwrap();
            }
            writeln!(out).unwrap();
        }

        writeln!(out, "    {}", format_terminator(&block.terminator)).unwrap();
    }

    writeln!(out, "}}").unwrap();
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
        InstKind::MethodCall(obj, name, args) => format!("method_call {obj}.{name}({})", fmt_args(args)),
        InstKind::IndirectCall(f, args) => format!("indirect_call {f}({})", fmt_args(args)),

        InstKind::FnRef(name) => format!("fn_ref {name}"),
        InstKind::Load(name) => format!("load {name}"),
        InstKind::Store(name, val) => format!("store {name} {val}"),

        InstKind::FieldGet(obj, field) => format!("field_get {obj}.{field}"),
        InstKind::FieldSet(obj, field, val) => format!("field_set {obj}.{field} = {val}"),
        InstKind::FieldStore(var, field, val) => format!("field_store ${var}.{field} = {val}"),

        InstKind::Index(arr, idx) => format!("index {arr}[{idx}]"),
        InstKind::IndexSet(arr, idx, val) => format!("index_set {arr}[{idx}] = {val}"),
        InstKind::IndexStore(var, idx, val) => format!("index_store ${var}[{idx}] = {val}"),

        InstKind::StructInit(name, fields) => {
            let fs = fields.iter().map(|(n, v)| format!("{n}: {v}")).collect::<Vec<_>>().join(", ");
            format!("struct_init {name} {{ {fs} }}")
        }
        InstKind::VariantInit(enum_name, variant, tag, args) => {
            format!("variant_init {enum_name}::{variant} (tag={tag}) ({})", fmt_args(args))
        }
        InstKind::ArrayInit(vals) => format!("array [{}]", fmt_args(vals)),

        InstKind::Cast(v, ty) => format!("cast {v} as {ty:?}"),
        InstKind::StrictCast(v, ty) => format!("strict_cast {v} as {ty:?}"),
        InstKind::Ref(v) => format!("ref {v}"),
        InstKind::Deref(v) => format!("deref {v}"),
        InstKind::Alloc(v) => format!("alloc {v}"),
        InstKind::Drop(v, ty) => format!("drop {v} {ty:?}"),
        InstKind::RcInc(v) => format!("rc_inc {v}"),
        InstKind::RcDec(v) => format!("rc_dec {v}"),
        InstKind::Copy(v) => format!("copy {v}"),
        InstKind::Slice(a, s, e) => format!("slice {a}[{s}..{e}]"),

        // Collections
        InstKind::VecNew(vals) => format!("vec_new [{}]", fmt_args(vals)),
        InstKind::VecPush(vec, val) => format!("vec_push {vec} {val}"),
        InstKind::VecLen(v) => format!("vec_len {v}"),
        InstKind::MapInit => "map_init".into(),
        InstKind::SetInit => "set_init".into(),
        InstKind::PQInit => "pq_init".into(),
        InstKind::DequeInit => "deque_init".into(),

        // Closures
        InstKind::ClosureCreate(name, captures) => format!("closure_create {name}({})", fmt_args(captures)),
        InstKind::ClosureCall(f, args) => format!("closure_call {f}({})", fmt_args(args)),

        // RC
        InstKind::RcNew(v, ty) => format!("rc_new {v} {ty:?}"),
        InstKind::RcClone(v) => format!("rc_clone {v}"),
        InstKind::WeakUpgrade(v) => format!("weak_upgrade {v}"),

        // Actors/channels
        InstKind::SpawnActor(name, args) => format!("spawn_actor {name}({})", fmt_args(args)),
        InstKind::ChanCreate(ty, _cap) => format!("chan_create {ty:?}"),
        InstKind::ChanSend(ch, val) => format!("chan_send {ch} {val}"),
        InstKind::ChanRecv(ch) => format!("chan_recv {ch}"),
        InstKind::SelectArm(arms, _) => format!("select [{}]", fmt_args(arms)),

        // Builtins
        InstKind::Log(v) => format!("log {v}"),
        InstKind::Assert(v, msg) => format!("assert {v} {msg:?}"),

        // Dynamic dispatch
        InstKind::DynDispatch(obj, trait_name, method, args) => {
            format!("dyn_dispatch {obj}.{trait_name}::{method}({})", fmt_args(args))
        }
        InstKind::DynCoerce(v, type_name, trait_name) => {
            format!("dyn_coerce {v} as {type_name}:{trait_name}")
        }

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
            let arms_str = arms.iter().map(|(n, bb)| format!("{n} => {bb}")).collect::<Vec<_>>().join(", ");
            format!("switch {val} [{arms_str}] default {default}")
        }
        Terminator::Unreachable => "unreachable".into(),
    }
}
