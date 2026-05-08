//! Call-site typing: argument unification, generic instantiation, method resolution.

use crate::intern::Symbol;
use std::collections::HashMap;

use crate::ast::{self, Expr, Span};
use crate::hir;
use crate::types::Type;

use super::{Typer, VarInfo};

/// Resolve named arguments to positional order.
/// Given param names and call-site args (which may include NamedArg nodes),
/// reorder them to match param order. Positional args fill left-to-right,
/// named args fill by name. Returns the reordered arg list.
mod args;
mod fn_call;
mod method_call;
mod mono;
mod pipe;
mod store_methods;
mod view_methods;
