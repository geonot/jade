/// C header import tool — parses C function declarations and generates Jade `extern` declarations.
///
/// Usage: `jade bind header.h` → prints Jade extern declarations to stdout.
///
/// Handles: function declarations, typedefs (ignored), structs (comments), macros (ignored).
/// Does NOT handle: templates, C++ features, complex macros, inline functions.

use std::fs;
use std::path::Path;

pub fn bind_header(path: &Path) -> Result<String, String> {
    let src = fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let cleaned = strip_preprocessor(&strip_comments(&src));
    let mut out = String::new();
    out.push_str(&format!("// Auto-generated Jade bindings from {}\n\n", path.display()));

    for decl in parse_declarations(&cleaned) {
        match decl {
            CDecl::Function(f) => {
                out.push_str(&emit_extern(&f));
                out.push('\n');
            }
            CDecl::Struct(name) => {
                out.push_str(&format!("// struct {name} (opaque)\n"));
            }
            CDecl::Typedef(_) => {} // silently skip
        }
    }

    Ok(out)
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // line comment
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            // block comment
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i += 2; // skip */
            out.push(' ');
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn strip_preprocessor(src: &str) -> String {
    let mut out = String::new();
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[derive(Debug)]
#[allow(dead_code)]
enum CDecl {
    Function(CFn),
    Struct(String),
    Typedef(String),
}

#[derive(Debug)]
struct CFn {
    name: String,
    ret: CType,
    params: Vec<CParam>,
}

#[derive(Debug)]
struct CParam {
    name: String,
    ty: CType,
}

#[derive(Debug, Clone)]
enum CType {
    Void,
    Char,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    Float,
    Double,
    SizeT,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Ptr(Box<CType>),
    Named(String),
}

fn parse_declarations(src: &str) -> Vec<CDecl> {
    let mut decls = Vec::new();
    // Join lines that end with semicolons across multiple lines
    let joined = join_declarations(src);

    for line in joined.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip struct/union/enum bodies
        if trimmed.contains('{') {
            if let Some(name) = try_parse_struct_name(trimmed) {
                decls.push(CDecl::Struct(name));
            }
            continue;
        }
        if trimmed.starts_with('}') {
            continue;
        }

        if trimmed.starts_with("typedef") {
            if let Some(name) = try_parse_typedef_name(trimmed) {
                decls.push(CDecl::Typedef(name));
            }
            continue;
        }

        // Try to parse as function declaration
        if let Some(f) = try_parse_function(trimmed) {
            decls.push(CDecl::Function(f));
        }
    }

    decls
}

fn join_declarations(src: &str) -> String {
    // Simple: just return as-is, declarations should be on single lines after preprocessing
    // But handle multi-line declarations by joining until ';'
    let mut out = String::new();
    let mut current = String::new();
    let mut brace_depth: u32 = 0;

    for line in src.lines() {
        let trimmed = line.trim();
        for ch in trimmed.chars() {
            match ch {
                '{' => brace_depth += 1,
                '}' => brace_depth = brace_depth.saturating_sub(1),
                _ => {}
            }
        }

        if brace_depth > 0 {
            continue; // skip struct/union/enum bodies
        }

        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(trimmed);

        if trimmed.ends_with(';') || trimmed.ends_with('}') {
            out.push_str(&current);
            out.push('\n');
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push_str(&current);
        out.push('\n');
    }
    out
}

fn try_parse_struct_name(line: &str) -> Option<String> {
    let line = line.trim();
    // "struct Foo {" or "typedef struct Foo {"
    let rest = line.strip_prefix("typedef").map(|s| s.trim()).unwrap_or(line);
    let rest = rest.strip_prefix("struct")?.trim();
    let name = rest.split(|c: char| !c.is_alphanumeric() && c != '_').next()?;
    if name.is_empty() { return None; }
    Some(name.to_string())
}

fn try_parse_typedef_name(line: &str) -> Option<String> {
    // "typedef ... name;"
    let line = line.strip_prefix("typedef")?.trim().strip_suffix(';')?.trim();
    let name = line.rsplit_once(|c: char| c.is_whitespace() || c == '*')?.1;
    if name.is_empty() { return None; }
    Some(name.to_string())
}

fn try_parse_function(line: &str) -> Option<CFn> {
    let line = line.strip_suffix(';')?.trim();
    // Skip static/inline functions
    let line = strip_qualifiers(line);

    // Find the function name and params: "ret_type name(params)"
    let lparen = line.find('(')?;
    let rparen = line.rfind(')')?;
    if rparen <= lparen { return None; }

    let before_paren = line[..lparen].trim();
    let params_str = line[lparen + 1..rparen].trim();

    // Split before_paren into return type and name
    // Handle pointer returns: "int *foo" or "int* foo"
    let (ret_str, name) = split_ret_and_name(before_paren)?;

    // Skip if name looks weird (contains spaces, operators, etc.)
    if name.contains(|c: char| !c.is_alphanumeric() && c != '_') {
        return None;
    }

    let ret = parse_c_type(ret_str.trim());
    let params = parse_params(params_str);

    Some(CFn { name: name.to_string(), ret, params })
}

fn strip_qualifiers(line: &str) -> &str {
    let mut line = line;
    for q in &["extern", "static", "inline", "__attribute__((visibility(\"default\")))",
               "const", "__restrict", "restrict", "__inline", "__extern_always_inline"] {
        line = line.strip_prefix(q).map(|s| s.trim_start()).unwrap_or(line);
    }
    line
}

fn split_ret_and_name(s: &str) -> Option<(&str, &str)> {
    // Find the last word token — that's the function name
    // Handle: "int foo", "int *foo", "int * foo", "void* foo", "struct foo *bar"
    let s = s.trim();
    // Remove trailing pointer stars from name
    let last_space = s.rfind(|c: char| c.is_whitespace() || c == '*')?;
    let name = s[last_space + 1..].trim();
    let ret = s[..last_space + 1].trim();
    if name.is_empty() { return None; }
    Some((ret, name))
}

fn parse_c_type(s: &str) -> CType {
    let s = s.trim();
    // Count pointer indirections
    let s_no_const = s.replace("const ", "").replace(" const", "");
    let s = s_no_const.trim();

    let ptr_count = s.chars().filter(|&c| c == '*').count();
    let base = s.replace('*', "").trim().to_string();
    let base = base.trim();

    let mut ty = match base {
        "void" => CType::Void,
        "char" | "signed char" => CType::Char,
        "unsigned char" => CType::UChar,
        "short" | "signed short" | "short int" | "signed short int" => CType::Short,
        "unsigned short" | "unsigned short int" => CType::UShort,
        "int" | "signed" | "signed int" => CType::Int,
        "unsigned" | "unsigned int" => CType::UInt,
        "long" | "signed long" | "long int" | "signed long int" => CType::Long,
        "unsigned long" | "unsigned long int" => CType::ULong,
        "long long" | "signed long long" | "long long int" => CType::LongLong,
        "unsigned long long" | "unsigned long long int" => CType::ULongLong,
        "float" => CType::Float,
        "double" => CType::Double,
        "size_t" | "ssize_t" => CType::SizeT,
        "int8_t" | "__int8_t" => CType::Int8,
        "int16_t" | "__int16_t" => CType::Int16,
        "int32_t" | "__int32_t" => CType::Int32,
        "int64_t" | "__int64_t" => CType::Int64,
        "uint8_t" | "__uint8_t" => CType::UInt8,
        "uint16_t" | "__uint16_t" => CType::UInt16,
        "uint32_t" | "__uint32_t" => CType::UInt32,
        "uint64_t" | "__uint64_t" => CType::UInt64,
        other => CType::Named(other.to_string()),
    };

    for _ in 0..ptr_count {
        ty = CType::Ptr(Box::new(ty));
    }

    ty
}

fn parse_params(s: &str) -> Vec<CParam> {
    let s = s.trim();
    if s.is_empty() || s == "void" {
        return Vec::new();
    }

    let mut params = Vec::new();
    let mut depth = 0;
    let mut start = 0;

    // Split by commas, respecting parentheses (for function pointer params)
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                if let Some(p) = parse_single_param(&s[start..i]) {
                    params.push(p);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(p) = parse_single_param(&s[start..]) {
        params.push(p);
    }

    // If no names given, generate p0, p1, ...
    for (i, p) in params.iter_mut().enumerate() {
        if p.name.is_empty() {
            p.name = format!("p{i}");
        }
    }

    params
}

fn parse_single_param(s: &str) -> Option<CParam> {
    let s = s.trim();
    if s.is_empty() || s == "..." {
        return None; // skip varargs
    }

    // Try to split into type and name
    // Cases: "int x", "int *x", "const char *x", "int" (no name)
    let s_clean = s.replace("const ", "").replace(" const", "");
    let s_use = s_clean.trim();

    // Find last alphanumeric token
    if let Some(last_space_pos) = s_use.rfind(|c: char| c.is_whitespace() || c == '*') {
        let name_part = s_use[last_space_pos + 1..].trim();
        let type_part = s_use[..last_space_pos + 1].trim();
        // Check if name looks like a type keyword
        if is_type_keyword(name_part) {
            Some(CParam {
                name: String::new(),
                ty: parse_c_type(s),
            })
        } else {
            Some(CParam {
                name: sanitize_name(name_part),
                ty: parse_c_type(type_part),
            })
        }
    } else {
        Some(CParam {
            name: String::new(),
            ty: parse_c_type(s),
        })
    }
}

fn is_type_keyword(s: &str) -> bool {
    matches!(s, "int" | "char" | "void" | "short" | "long" | "float" | "double"
        | "signed" | "unsigned" | "struct" | "enum" | "union")
}

fn sanitize_name(name: &str) -> String {
    // Jade reserved words get suffixed with _
    let reserved = ["fn", "let", "if", "else", "for", "while", "loop", "match",
                     "return", "break", "continue", "type", "enum", "use", "as",
                     "true", "false", "none", "and", "or", "not", "in", "is"];
    let name = name.trim_start_matches('*');
    if reserved.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

fn ctype_to_jade(ty: &CType) -> String {
    match ty {
        CType::Void => "void".to_string(),
        CType::Char | CType::Int8 => "i8".to_string(),
        CType::UChar | CType::UInt8 => "i8".to_string(),
        CType::Short | CType::Int16 => "i16".to_string(),
        CType::UShort | CType::UInt16 => "i16".to_string(),
        CType::Int | CType::Int32 => "i32".to_string(),
        CType::UInt | CType::UInt32 => "i32".to_string(),
        CType::Long | CType::LongLong | CType::ULong | CType::ULongLong
        | CType::SizeT | CType::Int64 | CType::UInt64 => "i64".to_string(),
        CType::Float => "f32".to_string(),
        CType::Double => "f64".to_string(),
        CType::Ptr(inner) => format!("%{}", ctype_to_jade(inner)),
        CType::Named(n) => format!("%void /* {n} */"),
    }
}

fn emit_extern(f: &CFn) -> String {
    let mut out = String::from("extern *");
    out.push_str(&f.name);
    out.push('(');
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&p.name);
        out.push_str(" as ");
        out.push_str(&ctype_to_jade(&p.ty));
    }
    out.push(')');

    // Return type (skip if void)
    match &f.ret {
        CType::Void => {}
        ty => {
            out.push_str(" returns ");
            out.push_str(&ctype_to_jade(ty));
        }
    }

    out
}
