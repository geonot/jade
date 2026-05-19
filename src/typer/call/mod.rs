use crate::intern::Symbol;
use std::collections::HashMap;

use crate::ast::{self, Expr, Span};
use crate::hir;
use crate::types::Type;

use super::{Typer, VarInfo};

mod args;
mod fn_call;
mod method_call;
mod mono;
mod pipe;
mod store_methods;
mod view_methods;
