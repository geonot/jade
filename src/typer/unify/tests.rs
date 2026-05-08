use super::*;
use super::*;

#[test]
fn test_fresh_var() {
    let mut ctx = InferCtx::new();
    let v0 = ctx.fresh_var();
    let v1 = ctx.fresh_var();
    assert_eq!(v0, Type::TypeVar(0));
    assert_eq!(v1, Type::TypeVar(1));
}

#[test]
fn test_unify_var_concrete() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    ctx.unify(&v, &Type::I64).unwrap();
    assert_eq!(ctx.resolve(&v), Type::I64);
}

#[test]
fn test_unify_two_vars() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh_var();
    let b = ctx.fresh_var();
    ctx.unify(&a, &b).unwrap();
    ctx.unify(&b, &Type::String).unwrap();
    assert_eq!(ctx.resolve(&a), Type::String);
    assert_eq!(ctx.resolve(&b), Type::String);
}

#[test]
fn test_structural_unify() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let arr_a = Type::Vec(Box::new(v.clone()));
    let arr_b = Type::Vec(Box::new(Type::F64));
    ctx.unify(&arr_a, &arr_b).unwrap();
    assert_eq!(ctx.resolve(&v), Type::F64);
}

#[test]
fn test_occurs_check() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let circular = Type::Vec(Box::new(v.clone()));
    assert!(ctx.unify(&v, &circular).is_err());
}

#[test]
fn test_unsolved_defaults_to_i64() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    let v = ctx.fresh_var();
    assert_eq!(ctx.resolve(&v), Type::I64);
}

#[test]
fn test_fn_unify() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let fn_a = Type::Fn(vec![v.clone()], Box::new(Type::Bool));
    let fn_b = Type::Fn(vec![Type::String], Box::new(Type::Bool));
    ctx.unify(&fn_a, &fn_b).unwrap();
    assert_eq!(ctx.resolve(&v), Type::String);
}

#[test]
fn test_transitive_unification() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh_var();
    let b = ctx.fresh_var();
    let c = ctx.fresh_var();
    ctx.unify(&a, &b).unwrap();
    ctx.unify(&b, &c).unwrap();
    ctx.unify(&c, &Type::F64).unwrap();
    assert_eq!(ctx.resolve(&a), Type::F64);
}

#[test]
fn test_concrete_mismatch_errors() {
    let mut ctx = InferCtx::new();
    assert!(ctx.unify(&Type::I64, &Type::String).is_err());
    assert!(ctx.unify(&Type::Bool, &Type::F64).is_err());
    assert!(ctx.unify(&Type::I32, &Type::I64).is_err());
}

#[test]
fn test_concrete_same_ok() {
    let mut ctx = InferCtx::new();
    assert!(ctx.unify(&Type::I64, &Type::I64).is_ok());
    assert!(ctx.unify(&Type::String, &Type::String).is_ok());
    assert!(ctx.unify(&Type::Bool, &Type::Bool).is_ok());
}

#[test]
fn test_structural_vec_mismatch() {
    let mut ctx = InferCtx::new();
    let va = Type::Vec(Box::new(Type::I64));
    let vb = Type::Vec(Box::new(Type::String));
    assert!(ctx.unify(&va, &vb).is_err());
}

#[test]
fn test_tuple_arity_mismatch() {
    let mut ctx = InferCtx::new();
    let ta = Type::Tuple(vec![Type::I64]);
    let tb = Type::Tuple(vec![Type::I64, Type::Bool]);
    assert!(ctx.unify(&ta, &tb).is_err());
}

#[test]
fn test_tuple_unify_with_vars() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh_var();
    let b = ctx.fresh_var();
    let ta = Type::Tuple(vec![a.clone(), b.clone()]);
    let tb = Type::Tuple(vec![Type::String, Type::Bool]);
    ctx.unify(&ta, &tb).unwrap();
    assert_eq!(ctx.resolve(&a), Type::String);
    assert_eq!(ctx.resolve(&b), Type::Bool);
}

#[test]
fn test_map_unify() {
    let mut ctx = InferCtx::new();
    let k = ctx.fresh_var();
    let v = ctx.fresh_var();
    let ma = Type::Map(Box::new(k.clone()), Box::new(v.clone()));
    let mb = Type::Map(Box::new(Type::String), Box::new(Type::I64));
    ctx.unify(&ma, &mb).unwrap();
    assert_eq!(ctx.resolve(&k), Type::String);
    assert_eq!(ctx.resolve(&v), Type::I64);
}

#[test]
fn test_channel_unify() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let ca = Type::Channel(Box::new(v.clone()));
    let cb = Type::Channel(Box::new(Type::String));
    ctx.unify(&ca, &cb).unwrap();
    assert_eq!(ctx.resolve(&v), Type::String);
}

#[test]
fn test_fn_arity_mismatch() {
    let mut ctx = InferCtx::new();
    let fa = Type::Fn(vec![Type::I64], Box::new(Type::Void));
    let fb = Type::Fn(vec![Type::I64, Type::Bool], Box::new(Type::Void));
    assert!(ctx.unify(&fa, &fb).is_err());
}

#[test]
fn test_array_length_mismatch() {
    let mut ctx = InferCtx::new();
    let aa = Type::Array(Box::new(Type::I64), 3);
    let ab = Type::Array(Box::new(Type::I64), 5);
    assert!(ctx.unify(&aa, &ab).is_err());
}

#[test]
fn test_deeply_nested_unification() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let a = Type::Vec(Box::new(Type::Map(
        Box::new(Type::String),
        Box::new(v.clone()),
    )));
    let b = Type::Vec(Box::new(Type::Map(
        Box::new(Type::String),
        Box::new(Type::Bool),
    )));
    ctx.unify(&a, &b).unwrap();
    assert_eq!(ctx.resolve(&v), Type::Bool);
}

#[test]
fn test_unify_at_records_origin() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    let span = crate::ast::Span {
        start: 0,
        end: 1,
        line: 10,
        col: 5,
    };
    ctx.unify_at(&v, &Type::String, span, "test constraint")
        .unwrap();
    let origin = ctx.origin_of(&v).unwrap();
    assert_eq!(origin.span.line, 10);
    assert_eq!(origin.reason, "test constraint");
}

#[test]
fn test_try_resolve_unsolved() {
    let mut ctx = InferCtx::new();
    let v = ctx.fresh_var();
    assert!(ctx.try_resolve(&v).is_none());
    ctx.unify(&v, &Type::Bool).unwrap();
    assert_eq!(ctx.try_resolve(&v), Some(Type::Bool));
}

#[test]
fn test_default_warnings_disabled_by_default() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    let v = ctx.fresh_var();
    let _ = ctx.resolve(&v);
    let warnings = ctx.drain_default_warnings();
    assert!(warnings.is_empty());
}

#[test]
fn test_default_warnings_collected_when_enabled() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    ctx.enable_default_warnings();
    let span = Span {
        start: 0,
        end: 0,
        line: 5,
        col: 3,
    };
    let v = ctx.fresh_var();
    let v2 = ctx.fresh_var();
    let _ = ctx.unify_at(&v, &v2, span, "test param");
    let resolved = ctx.resolve(&v);
    assert_eq!(resolved, Type::I64);
    let warnings = ctx.drain_default_warnings();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unsolved type variable defaulted to i64"));
    assert!(warnings[0].contains("test param"));
}

#[test]
fn test_default_warnings_not_emitted_for_solved_vars() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    ctx.enable_default_warnings();
    let v = ctx.fresh_var();
    ctx.unify(&v, &Type::String).unwrap();
    let _ = ctx.resolve(&v);
    let warnings = ctx.drain_default_warnings();
    assert!(warnings.is_empty());
}

#[test]
fn test_default_warnings_not_emitted_for_constrained_numeric() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    ctx.enable_default_warnings();
    let v = ctx.fresh_integer_var();
    let resolved = ctx.resolve(&v);
    assert_eq!(resolved, Type::I64);
    let warnings = ctx.drain_default_warnings();
    assert!(warnings.is_empty());
}

#[test]
fn test_default_warnings_float_constraint_no_warning() {
    let mut ctx = InferCtx::new();
    ctx.disable_strict_types();
    ctx.enable_default_warnings();
    let v = ctx.fresh_float_var();
    let resolved = ctx.resolve(&v);
    assert_eq!(resolved, Type::F64);
    let warnings = ctx.drain_default_warnings();
    assert!(warnings.is_empty());
}
