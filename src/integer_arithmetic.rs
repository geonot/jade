use crate::hir::BuiltinFn;
use crate::types::Type;

#[derive(Clone, Copy)]
pub enum IntegerOp {
    Add,
    Sub,
    Mul,
}

impl IntegerOp {
    pub fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum OverflowMode {
    Wrapping,
    Saturating,
    Checked,
}

pub struct IntegerArithmetic {
    pub source_name: &'static str,
    pub builtin: BuiltinFn,
    pub op: IntegerOp,
    pub mode: OverflowMode,
    mir_name: &'static str,
}

impl IntegerArithmetic {
    pub fn from_source(name: &str) -> Option<&'static Self> {
        OPERATIONS.iter().find(|op| op.source_name == name)
    }

    pub fn from_mir(name: &str) -> Option<&'static Self> {
        let name = name.strip_prefix("__builtin_")?;
        OPERATIONS.iter().find(|op| op.mir_name == name)
    }

    pub fn result_type(&self, operand: &Type) -> Type {
        if self.mode == OverflowMode::Checked {
            Type::Tuple(vec![operand.clone(), Type::Bool])
        } else {
            operand.clone()
        }
    }
}

macro_rules! operations {
    ($(($source:literal, $builtin:ident, $op:ident, $mode:ident)),* $(,)?) => {
        &[$(IntegerArithmetic {
            source_name: $source,
            builtin: BuiltinFn::$builtin,
            op: IntegerOp::$op,
            mode: OverflowMode::$mode,
            mir_name: stringify!($builtin),
        }),*]
    };
}

const OPERATIONS: &[IntegerArithmetic] = operations![
    ("wrapping_add", WrappingAdd, Add, Wrapping),
    ("wrapping_sub", WrappingSub, Sub, Wrapping),
    ("wrapping_mul", WrappingMul, Mul, Wrapping),
    ("saturating_add", SaturatingAdd, Add, Saturating),
    ("saturating_sub", SaturatingSub, Sub, Saturating),
    ("saturating_mul", SaturatingMul, Mul, Saturating),
    ("checked_add", CheckedAdd, Add, Checked),
    ("checked_sub", CheckedSub, Sub, Checked),
    ("checked_mul", CheckedMul, Mul, Checked),
];
