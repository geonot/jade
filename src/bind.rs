use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn bind_header(path: &Path) -> Result<String, String> {
    let src =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let mut seen: Vec<String> = Vec::new();
    let expanded = inline_local_includes(&src, path, &mut seen, 0);
    let cleaned = strip_extern_linkage(&strip_preprocessor(&strip_comments(&src)));
    let cleaned_expanded = strip_extern_linkage(&strip_preprocessor(&strip_comments(&expanded)));
    let mut out = String::new();
    out.push_str(&format!(
        "# Auto-generated Jinn bindings from {}\n\
         # Review before use: a C header carries information (ownership, \
         nullability, array lengths)\n# that does not survive the translation.\n\n",
        path.display()
    ));

    let decls = parse_declarations(&cleaned);
    let mut aliases: HashMap<String, CType> = HashMap::new();
    for decl in parse_declarations(&cleaned_expanded) {
        if let CDecl::Typedef(name, ty) = decl {
            aliases.insert(name, ty);
        }
    }

    for decl in decls {
        match decl {
            CDecl::Function(mut f) => {
                f.ret = resolve_alias(&f.ret, &aliases, 0);
                for p in &mut f.params {
                    p.ty = resolve_alias(&p.ty, &aliases, 0);
                }
                out.push_str(&emit_extern(&f));
                out.push('\n');
            }
            CDecl::Struct(name) => {
                out.push_str(&format!("# struct {name} (opaque)\n"));
            }
            CDecl::Typedef(..) => {}
        }
    }

    Ok(out)
}

fn inline_local_includes(src: &str, path: &Path, seen: &mut Vec<String>, depth: u32) -> String {
    if depth > 4 {
        return src.to_string();
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut out = String::with_capacity(src.len());
    for line in src.lines() {
        let t = line.trim();
        let target = t
            .strip_prefix('#')
            .map(str::trim_start)
            .and_then(|r| r.strip_prefix("include"))
            .map(str::trim)
            .and_then(|r| match r.chars().next() {
                Some('"') => r[1..].split('"').next(),
                Some('<') => r[1..].split('>').next(),
                _ => None,
            });
        if let Some(name) = target {
            let candidates = [dir.join(name), Path::new("/usr/include").join(name)];
            let mut inlined = false;
            for candidate in candidates {
                let key = candidate.display().to_string();
                if seen.contains(&key) {
                    inlined = true;
                    break;
                }
                if let Ok(text) = fs::read_to_string(&candidate) {
                    seen.push(key);
                    out.push_str(&inline_local_includes(&text, &candidate, seen, depth + 1));
                    out.push('\n');
                    inlined = true;
                    break;
                }
            }
            if inlined {
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            let mut spanned_lines = false;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    spanned_lines = true;
                }
                i += 1;
            }
            i += 2;
            out.push(if spanned_lines { '\n' } else { ' ' });
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn strip_preprocessor(src: &str) -> String {
    let mut out = String::new();
    let mut in_continuation = false;
    for line in src.lines() {
        let trimmed = line.trim();
        let continues = trimmed.ends_with('\\');
        if in_continuation {
            in_continuation = continues;
            continue;
        }
        if trimmed.starts_with('#') {
            in_continuation = continues;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn strip_extern_linkage(src: &str) -> String {
    let mut out = String::new();
    let mut linkage = 0usize;
    let mut depth = 0u32;
    for line in src.lines() {
        let t = line.trim();
        if depth == 0 {
            if t.starts_with("extern \"") && t.ends_with('{') {
                linkage += 1;
                continue;
            }
            if t == "}" && linkage > 0 {
                linkage -= 1;
                continue;
            }
        }
        for ch in t.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn strip_macro_tokens(s: &str) -> String {
    let tokens: Vec<&str> = s.split_whitespace().collect();
    let has_primitive = tokens.iter().any(|t| {
        let bare = t.trim_matches('*');
        matches!(
            bare,
            "void"
                | "char"
                | "short"
                | "int"
                | "long"
                | "float"
                | "double"
                | "signed"
                | "unsigned"
        ) || bare.ends_with("_t")
    });
    if !has_primitive {
        return s.to_string();
    }
    let kept: Vec<&str> = tokens
        .into_iter()
        .filter(|t| {
            let bare = t.trim_matches('*');
            if bare.is_empty() {
                return true;
            }
            !(bare
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                && bare.chars().any(|c| c.is_ascii_uppercase()))
        })
        .collect();
    if kept.is_empty() {
        return s.to_string();
    }
    kept.join(" ")
}

#[derive(Debug)]
#[allow(dead_code)]
enum CDecl {
    Function(CFn),
    Struct(String),
    Typedef(String, CType),
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

fn resolve_alias(ty: &CType, aliases: &HashMap<String, CType>, depth: u32) -> CType {
    if depth > 8 {
        return ty.clone();
    }
    match ty {
        CType::Named(n) => match aliases.get(n) {
            Some(target) => resolve_alias(target, aliases, depth + 1),
            None => ty.clone(),
        },
        CType::Ptr(inner) => CType::Ptr(Box::new(resolve_alias(inner, aliases, depth + 1))),
        _ => ty.clone(),
    }
}

fn parse_declarations(src: &str) -> Vec<CDecl> {
    let mut decls = Vec::new();

    let joined = join_declarations(src);

    for line in joined.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

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
            if let Some((name, ty)) = try_parse_typedef(trimmed) {
                decls.push(CDecl::Typedef(name, ty));
            }
            continue;
        }

        if let Some(f) = try_parse_function(trimmed) {
            decls.push(CDecl::Function(f));
        }
    }

    decls
}

fn join_declarations(src: &str) -> String {
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
            continue;
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

    let rest = line
        .strip_prefix("typedef")
        .map(|s| s.trim())
        .unwrap_or(line);
    let rest = rest.strip_prefix("struct")?.trim();
    let name = rest
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()?;
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

fn try_parse_typedef(line: &str) -> Option<(String, CType)> {
    let line = line
        .strip_prefix("typedef")?
        .trim()
        .strip_suffix(';')?
        .trim();
    if line.contains('(') || line.contains(',') {
        return None;
    }
    let split = line.rfind(|c: char| c.is_whitespace() || c == '*')?;
    let name = line[split + 1..].trim();
    let target = line[..split + 1].trim();
    if name.is_empty() || target.is_empty() || !is_jinn_ident(name) {
        return None;
    }
    Some((name.to_string(), parse_c_type(target)))
}

fn try_parse_function(line: &str) -> Option<CFn> {
    let line = line.strip_suffix(';')?.trim();

    let line = strip_qualifiers(line);

    let lparen = line.find('(')?;
    let rparen = line.rfind(')')?;
    if rparen <= lparen {
        return None;
    }

    let before_paren = line[..lparen].trim();
    let params_str = line[lparen + 1..rparen].trim();

    let (ret_str, name) = split_ret_and_name(before_paren)?;

    if name.contains(|c: char| !c.is_alphanumeric() && c != '_') {
        return None;
    }

    let ret = parse_c_type(ret_str.trim());
    let params = parse_params(params_str);

    Some(CFn {
        name: name.to_string(),
        ret,
        params,
    })
}

fn strip_qualifiers(line: &str) -> &str {
    let mut line = line;
    for q in &[
        "extern",
        "static",
        "inline",
        "__attribute__((visibility(\"default\")))",
        "const",
        "__restrict",
        "restrict",
        "__inline",
        "__extern_always_inline",
    ] {
        line = line.strip_prefix(q).map(|s| s.trim_start()).unwrap_or(line);
    }
    line
}

fn split_ret_and_name(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();

    let last_space = s.rfind(|c: char| c.is_whitespace() || c == '*')?;
    let name = s[last_space + 1..].trim();
    let ret = s[..last_space + 1].trim();
    if name.is_empty() {
        return None;
    }
    Some((ret, name))
}

fn parse_c_type(s: &str) -> CType {
    let s = s.trim();

    let s_no_const = s.replace("const ", "").replace(" const", "");
    let s_no_macros = strip_macro_tokens(s_no_const.trim());
    let s = s_no_macros.trim();

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
        return None;
    }

    let s_clean = s.replace("const ", "").replace(" const", "");
    let s_use = s_clean.trim();

    if let Some(last_space_pos) = s_use.rfind(|c: char| c.is_whitespace() || c == '*') {
        let name_part = s_use[last_space_pos + 1..].trim();
        let type_part = s_use[..last_space_pos + 1].trim();

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
    matches!(
        s,
        "int"
            | "char"
            | "void"
            | "short"
            | "long"
            | "float"
            | "double"
            | "signed"
            | "unsigned"
            | "struct"
            | "enum"
            | "union"
    )
}

fn sanitize_name(name: &str) -> String {
    let reserved = [
        "fn", "let", "if", "else", "for", "while", "loop", "match", "return", "break", "continue",
        "type", "enum", "use", "as", "true", "false", "none", "and", "or", "not", "in", "is",
    ];
    let name = name.trim_start_matches('*');
    if reserved.contains(&name) {
        format!("{name}_")
    } else {
        name.to_string()
    }
}

fn ctype_to_jinn(ty: &CType) -> String {
    match ty {
        CType::Void => "void".to_string(),
        CType::Char | CType::Int8 => "i8".to_string(),
        CType::UChar | CType::UInt8 => "i8".to_string(),
        CType::Short | CType::Int16 => "i16".to_string(),
        CType::UShort | CType::UInt16 => "i16".to_string(),
        CType::Int | CType::Int32 => "i32".to_string(),
        CType::UInt | CType::UInt32 => "i32".to_string(),
        CType::Long
        | CType::LongLong
        | CType::ULong
        | CType::ULongLong
        | CType::SizeT
        | CType::Int64
        | CType::UInt64 => "i64".to_string(),
        CType::Float => "f32".to_string(),
        CType::Double => "f64".to_string(),
        CType::Ptr(inner) => match inner.as_ref() {
            CType::Void | CType::Named(_) => "%i8".to_string(),
            _ => format!("%{}", ctype_to_jinn(inner)),
        },
        CType::Named(_) => "void".to_string(),
    }
}

fn collect_opaque(ty: &CType, out: &mut Vec<String>) {
    match ty {
        CType::Named(n) => out.push(n.clone()),
        CType::Ptr(inner) => collect_opaque(inner, out),
        _ => {}
    }
}

fn is_jinn_ident(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn emit_extern(f: &CFn) -> String {
    if !is_jinn_ident(&f.name) {
        return format!("# skipped `{}`: not a usable Jinn identifier\n", f.name);
    }
    let by_value_named = |ty: &CType| matches!(ty, CType::Named(_));
    if by_value_named(&f.ret) || f.params.iter().any(|p| by_value_named(&p.ty)) {
        return format!(
            "# skipped `{}`: it passes or returns a named C type by value, which has \
             no Jinn spelling\n",
            f.name
        );
    }

    let mut opaque: Vec<String> = Vec::new();
    collect_opaque(&f.ret, &mut opaque);
    for p in &f.params {
        collect_opaque(&p.ty, &mut opaque);
    }
    opaque.sort();
    opaque.dedup();

    let mut out = String::new();
    if !opaque.is_empty() {
        out.push_str(&format!(
            "# `{}`: {} came through as an opaque pointer; confirm the real layout \
             before relying on it\n",
            f.name,
            opaque.join("`, `")
        ));
    }
    out.push_str("extern *");
    out.push_str(&f.name);
    out.push('(');
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let pname = if is_jinn_ident(&p.name) {
            p.name.clone()
        } else {
            format!("arg{i}")
        };
        out.push_str(&pname);
        out.push_str(" as ");
        out.push_str(&ctype_to_jinn(&p.ty));
    }
    out.push(')');

    match &f.ret {
        CType::Void => {}
        ty => {
            out.push_str(" returns ");
            out.push_str(&ctype_to_jinn(ty));
        }
    }

    out
}
