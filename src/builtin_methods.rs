use crate::types::Type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VecMethod {
    Push,
    Pop,
    Shift,
    First,
    Last,
    Get,
    Set,
    Remove,
    Clear,
    Len,
    Count,
    IsEmpty,
    Contains,
    Sum,
    Join,
    Take,
    Skip,
    Slice,
    Collect,
    Reverse,
    Sort,
    Flatten,
    Enumerate,
    Chain,
    Zip,
    Map,
    Filter,
    Fold,
    Find,
    Any,
    All,
}

impl VecMethod {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "push" => Self::Push,
            "pop" => Self::Pop,
            "shift" => Self::Shift,
            "first" => Self::First,
            "last" => Self::Last,
            "get" => Self::Get,
            "set" => Self::Set,
            "remove" => Self::Remove,
            "clear" => Self::Clear,
            "len" => Self::Len,
            "count" => Self::Count,
            "is_empty" => Self::IsEmpty,
            "contains" => Self::Contains,
            "sum" => Self::Sum,
            "join" => Self::Join,
            "take" => Self::Take,
            "skip" => Self::Skip,
            "slice" => Self::Slice,
            "collect" => Self::Collect,
            "reverse" => Self::Reverse,
            "sort" => Self::Sort,
            "flatten" => Self::Flatten,
            "enumerate" => Self::Enumerate,
            "chain" => Self::Chain,
            "zip" => Self::Zip,
            "map" => Self::Map,
            "filter" => Self::Filter,
            "fold" => Self::Fold,
            "find" => Self::Find,
            "any" => Self::Any,
            "all" => Self::All,
            _ => return None,
        })
    }

    pub fn ret_ty(self, elem_ty: &Type) -> Type {
        match self {
            Self::Push | Self::Clear | Self::Set => Type::Void,
            Self::Pop | Self::Get | Self::Remove | Self::Shift | Self::First | Self::Last
            | Self::Sum | Self::Find => elem_ty.clone(),
            Self::Len | Self::Count => Type::I64,
            Self::IsEmpty | Self::Contains | Self::Any | Self::All => Type::Bool,
            Self::Take
            | Self::Skip
            | Self::Slice
            | Self::Collect
            | Self::Reverse
            | Self::Sort
            | Self::Chain
            | Self::Filter => Type::Vec(Box::new(elem_ty.clone())),
            Self::Flatten => match elem_ty {
                Type::Vec(inner) => Type::Vec(inner.clone()),
                Type::Array(inner, _) => Type::Vec(inner.clone()),
                _ => Type::Vec(Box::new(elem_ty.clone())),
            },
            Self::Join => Type::String,
            Self::Enumerate => Type::Vec(Box::new(Type::Tuple(vec![Type::I64, elem_ty.clone()]))),
            Self::Zip | Self::Map | Self::Fold => Type::Void,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapMethod {
    Set,
    Get,
    Has,
    Contains,
    Remove,
    Clear,
    Len,
    Count,
    Keys,
    Values,
}

impl MapMethod {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "set" => Self::Set,
            "get" => Self::Get,
            "has" => Self::Has,
            "contains" => Self::Contains,
            "remove" => Self::Remove,
            "clear" => Self::Clear,
            "len" => Self::Len,
            "count" => Self::Count,
            "keys" => Self::Keys,
            "values" => Self::Values,
            _ => return None,
        })
    }

    pub fn ret_ty(self, key_ty: &Type, val_ty: &Type) -> Type {
        match self {
            Self::Set | Self::Remove | Self::Clear => Type::Void,
            Self::Get => val_ty.clone(),
            Self::Has | Self::Contains => Type::Bool,
            Self::Len | Self::Count => Type::I64,
            Self::Keys => Type::Vec(Box::new(key_ty.clone())),
            Self::Values => Type::Vec(Box::new(val_ty.clone())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrMethod {
    Len,
    Length,
    Contains,
    StartsWith,
    EndsWith,
    CharAt,
    Find,
    Slice,
    Trim,
    TrimLeft,
    TrimRight,
    Replace,
    ToUpper,
    ToLower,
    Repeat,
    Split,
    Lines,
    IsEmpty,
}

impl StrMethod {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "len" => Self::Len,
            "length" => Self::Length,
            "contains" => Self::Contains,
            "starts_with" => Self::StartsWith,
            "ends_with" => Self::EndsWith,
            "char_at" => Self::CharAt,
            "find" => Self::Find,
            "slice" => Self::Slice,
            "trim" => Self::Trim,
            "trim_left" => Self::TrimLeft,
            "trim_right" => Self::TrimRight,
            "replace" => Self::Replace,
            "to_upper" => Self::ToUpper,
            "to_lower" => Self::ToLower,
            "repeat" => Self::Repeat,
            "split" => Self::Split,
            "lines" => Self::Lines,
            "is_empty" => Self::IsEmpty,
            _ => return None,
        })
    }

    pub fn ret_ty(self) -> Type {
        match self {
            Self::Contains | Self::StartsWith | Self::EndsWith | Self::IsEmpty => Type::Bool,
            Self::Len | Self::Length | Self::CharAt | Self::Find => Type::I64,
            Self::Slice
            | Self::Trim
            | Self::TrimLeft
            | Self::TrimRight
            | Self::Replace
            | Self::ToUpper
            | Self::ToLower
            | Self::Repeat => Type::String,
            Self::Split | Self::Lines => Type::Vec(Box::new(Type::String)),
        }
    }

    pub fn is_exclusive(self) -> bool {
        !matches!(self, Self::Len | Self::Length | Self::IsEmpty)
    }
}
