use crate::ast::{FieldDecorator, StoreDecorator};
use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    None,
    Int,
    Ident,
}

#[derive(Debug, Clone, Copy)]
pub struct StoreDecoratorSpec {
    pub name: &'static str,
    pub arg: ArgKind,
    pub excludes: &'static [&'static str],
    pub summary: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldDecoratorSpec {
    pub name: &'static str,
    pub arg: ArgKind,
    pub excludes: &'static [&'static str],
    pub numeric_only: bool,
    pub summary: &'static str,
}

pub const STORE_DECORATORS: &[StoreDecoratorSpec] = &[
    StoreDecoratorSpec {
        name: "simple",
        arg: ArgKind::None,
        excludes: &["versioned", "kv", "graph", "vector", "timeseries"],
        summary: "Plain record store with no built-in sid/uuid/audit columns.",
    },
    StoreDecoratorSpec {
        name: "mem",
        arg: ArgKind::None,
        excludes: &["versioned", "transient", "compact"],
        summary: "In-memory store with no on-disk persistence.",
    },
    StoreDecoratorSpec {
        name: "transient",
        arg: ArgKind::None,
        excludes: &["mem"],
        summary: "Persistence relaxed; data not durably flushed.",
    },
    StoreDecoratorSpec {
        name: "versioned",
        arg: ArgKind::None,
        excludes: &["simple", "mem", "kv"],
        summary: "Maintains per-record version history.",
    },
    StoreDecoratorSpec {
        name: "graph",
        arg: ArgKind::None,
        excludes: &["simple", "kv", "vector", "timeseries"],
        summary: "Graph store supporting edges and traversal.",
    },
    StoreDecoratorSpec {
        name: "kv",
        arg: ArgKind::None,
        excludes: &[
            "simple",
            "versioned",
            "graph",
            "vector",
            "timeseries",
            "column",
        ],
        summary: "Key/value store with schema-driven key/value types.",
    },
    StoreDecoratorSpec {
        name: "vector",
        arg: ArgKind::Int,
        excludes: &["simple", "kv", "graph", "timeseries"],
        summary: "Vector store for n-dimensional nearest-neighbour search.",
    },
    StoreDecoratorSpec {
        name: "compact",
        arg: ArgKind::Int,
        excludes: &["mem"],
        summary: "Auto-compact tombstones once the threshold is exceeded.",
    },
    StoreDecoratorSpec {
        name: "timeseries",
        arg: ArgKind::Ident,
        excludes: &["simple", "kv", "graph", "vector"],
        summary: "Time-series store ordered by the named timestamp field.",
    },
    StoreDecoratorSpec {
        name: "before_insert",
        arg: ArgKind::Ident,
        excludes: &[],
        summary: "Hook invoked before each insert.",
    },
    StoreDecoratorSpec {
        name: "after_insert",
        arg: ArgKind::Ident,
        excludes: &[],
        summary: "Hook invoked after each insert.",
    },
    StoreDecoratorSpec {
        name: "before_delete",
        arg: ArgKind::Ident,
        excludes: &[],
        summary: "Hook invoked before each delete.",
    },
    StoreDecoratorSpec {
        name: "after_delete",
        arg: ArgKind::Ident,
        excludes: &[],
        summary: "Hook invoked after each delete.",
    },
    StoreDecoratorSpec {
        name: "column",
        arg: ArgKind::None,
        excludes: &["kv"],
        summary: "Columnar storage layout.",
    },
];

pub const FIELD_DECORATORS: &[FieldDecoratorSpec] = &[
    FieldDecoratorSpec {
        name: "index",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Maintain a secondary index on this field.",
    },
    FieldDecoratorSpec {
        name: "unique",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Enforce uniqueness; rejected on f64 fields.",
    },
    FieldDecoratorSpec {
        name: "sorted",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Keep an order-preserving index.",
    },
    FieldDecoratorSpec {
        name: "transient",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Field is not persisted.",
    },
    FieldDecoratorSpec {
        name: "increment",
        arg: ArgKind::None,
        excludes: &["unique"],
        numeric_only: true,
        summary: "Auto-incrementing integer field.",
    },
    FieldDecoratorSpec {
        name: "required",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Field must be provided on insert.",
    },
    FieldDecoratorSpec {
        name: "versioned",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Track per-field version history.",
    },
    FieldDecoratorSpec {
        name: "cascade",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Cascade deletes through this relation.",
    },
    FieldDecoratorSpec {
        name: "lazy",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Load this field lazily.",
    },
    FieldDecoratorSpec {
        name: "bloom",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Maintain a bloom filter for membership tests.",
    },
    FieldDecoratorSpec {
        name: "search",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Full-text search index on this field.",
    },
    FieldDecoratorSpec {
        name: "default",
        arg: ArgKind::None,
        excludes: &[],
        numeric_only: false,
        summary: "Default value when none is supplied.",
    },
];

pub fn render_docs() -> String {
    let mut out = String::new();
    out.push_str("# Store decorators\n\n");
    for s in STORE_DECORATORS {
        let arg = match s.arg {
            ArgKind::None => "",
            ArgKind::Int => "(n)",
            ArgKind::Ident => "(name)",
        };
        out.push_str(&format!("- `@{}{}` — {}", s.name, arg, s.summary));
        if !s.excludes.is_empty() {
            out.push_str(&format!(" (excludes: {})", s.excludes.join(", ")));
        }
        out.push('\n');
    }
    out.push_str("\n# Field decorators\n\n");
    for f in FIELD_DECORATORS {
        out.push_str(&format!("- `@{}` — {}", f.name, f.summary));
        if f.numeric_only {
            out.push_str(" (numeric fields only)");
        }
        if !f.excludes.is_empty() {
            out.push_str(&format!(" (excludes: {})", f.excludes.join(", ")));
        }
        out.push('\n');
    }
    out
}

pub fn store_spec(name: &str) -> Option<&'static StoreDecoratorSpec> {
    STORE_DECORATORS.iter().find(|s| s.name == name)
}

pub fn field_spec(name: &str) -> Option<&'static FieldDecoratorSpec> {
    FIELD_DECORATORS.iter().find(|s| s.name == name)
}

fn store_name(d: &StoreDecorator) -> &'static str {
    match d {
        StoreDecorator::Simple => "simple",
        StoreDecorator::Mem => "mem",
        StoreDecorator::Transient => "transient",
        StoreDecorator::Versioned => "versioned",
        StoreDecorator::Graph => "graph",
        StoreDecorator::Kv => "kv",
        StoreDecorator::Vector(_) => "vector",
        StoreDecorator::Compact(_) => "compact",
        StoreDecorator::TimeSeries(_) => "timeseries",
        StoreDecorator::BeforeInsert(_) => "before_insert",
        StoreDecorator::AfterInsert(_) => "after_insert",
        StoreDecorator::BeforeDelete(_) => "before_delete",
        StoreDecorator::AfterDelete(_) => "after_delete",
        StoreDecorator::Column => "column",
    }
}

fn field_name(d: &FieldDecorator) -> &'static str {
    match d {
        FieldDecorator::Index => "index",
        FieldDecorator::Unique => "unique",
        FieldDecorator::Sorted => "sorted",
        FieldDecorator::Transient => "transient",
        FieldDecorator::Increment => "increment",
        FieldDecorator::Required => "required",
        FieldDecorator::Versioned => "versioned",
        FieldDecorator::Default(_) => "default",
        FieldDecorator::Cascade => "cascade",
        FieldDecorator::Lazy => "lazy",
        FieldDecorator::Bloom => "bloom",
        FieldDecorator::Search => "search",
    }
}

pub fn validate_store_decorators(store: &str, decorators: &[StoreDecorator]) -> Result<(), String> {
    let names: Vec<&'static str> = decorators.iter().map(store_name).collect();
    for (i, &a) in names.iter().enumerate() {
        let spec = store_spec(a).expect("store decorator in table");
        for &b in &names[i + 1..] {
            if spec.excludes.contains(&b) {
                return Err(format!(
                    "store '{store}': decorators @{a} and @{b} cannot be combined"
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_field_decorators(
    store: &str,
    field: &str,
    ty: &Type,
    decorators: &[FieldDecorator],
) -> Result<(), String> {
    let names: Vec<&'static str> = decorators.iter().map(field_name).collect();
    for (i, &a) in names.iter().enumerate() {
        let spec = field_spec(a).expect("field decorator in table");
        if spec.numeric_only && !matches!(ty, Type::I64 | Type::F64) {
            return Err(format!(
                "store '{store}': @{a} on field '{field}' requires a numeric type, found `{ty}`"
            ));
        }
        if a == "unique" && matches!(ty, Type::F64) {
            return Err(format!(
                "store '{store}': @unique on field '{field}' is not allowed for `f64`"
            ));
        }
        for &b in &names[i + 1..] {
            if spec.excludes.contains(&b) {
                return Err(format!(
                    "store '{store}': field '{field}' decorators @{a} and @{b} cannot be combined"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FieldDecorator, StoreDecorator};
    use crate::intern::Symbol;

    fn s() -> Symbol {
        Symbol::intern("x")
    }

    #[test]
    fn every_store_variant_is_in_table() {
        let variants = [
            StoreDecorator::Simple,
            StoreDecorator::Mem,
            StoreDecorator::Transient,
            StoreDecorator::Versioned,
            StoreDecorator::Graph,
            StoreDecorator::Kv,
            StoreDecorator::Vector(1),
            StoreDecorator::Compact(1),
            StoreDecorator::TimeSeries(s()),
            StoreDecorator::BeforeInsert(s()),
            StoreDecorator::AfterInsert(s()),
            StoreDecorator::BeforeDelete(s()),
            StoreDecorator::AfterDelete(s()),
            StoreDecorator::Column,
        ];
        for v in &variants {
            assert!(store_spec(super::store_name(v)).is_some());
        }
    }

    #[test]
    fn every_field_variant_is_in_table() {
        let variants = [
            FieldDecorator::Index,
            FieldDecorator::Unique,
            FieldDecorator::Sorted,
            FieldDecorator::Transient,
            FieldDecorator::Increment,
            FieldDecorator::Required,
            FieldDecorator::Versioned,
            FieldDecorator::Default(String::new()),
            FieldDecorator::Cascade,
            FieldDecorator::Lazy,
            FieldDecorator::Bloom,
            FieldDecorator::Search,
        ];
        for v in &variants {
            assert!(field_spec(super::field_name(v)).is_some());
        }
    }

    #[test]
    fn excludes_are_symmetric() {
        for a in STORE_DECORATORS {
            for &b in a.excludes {
                let other = store_spec(b).expect("excluded name exists");
                assert!(
                    other.excludes.contains(&a.name),
                    "@{} excludes @{} but not vice versa",
                    a.name,
                    b
                );
            }
        }
    }

    #[test]
    fn rejects_kv_plus_vector() {
        let err = validate_store_decorators("x", &[StoreDecorator::Kv, StoreDecorator::Vector(8)])
            .unwrap_err();
        assert!(err.contains("@kv") && err.contains("@vector"));
    }

    #[test]
    fn rejects_mem_plus_versioned() {
        assert!(
            validate_store_decorators("x", &[StoreDecorator::Mem, StoreDecorator::Versioned])
                .is_err()
        );
    }

    #[test]
    fn rejects_unique_on_f64() {
        let err = validate_field_decorators("x", "score", &Type::F64, &[FieldDecorator::Unique])
            .unwrap_err();
        assert!(err.contains("@unique") && err.contains("f64"));
    }

    #[test]
    fn allows_unique_on_string() {
        assert!(
            validate_field_decorators("x", "name", &Type::String, &[FieldDecorator::Unique])
                .is_ok()
        );
    }

    #[test]
    fn rejects_increment_on_string() {
        assert!(
            validate_field_decorators("x", "id", &Type::String, &[FieldDecorator::Increment])
                .is_err()
        );
    }

    #[test]
    fn docs_mention_all_decorators() {
        let docs = render_docs();
        for s in STORE_DECORATORS {
            assert!(docs.contains(&format!("`@{}", s.name)));
        }
        for f in FIELD_DECORATORS {
            assert!(docs.contains(&format!("`@{}", f.name)));
        }
    }
}
