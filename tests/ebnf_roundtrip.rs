use std::collections::HashSet;
use std::path::PathBuf;

use jinnc::lexer::{Lexer, Token};
use jinnc::parser::Parser;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_ebnf() -> String {
    std::fs::read_to_string(repo_root().join("docs").join("jinn.ebnf"))
        .expect("docs/jinn.ebnf must exist")
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut quote = b' ';
    while i < bytes.len() {
        if in_string {
            out.push(bytes[i] as char);
            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if bytes[i] == quote {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if i + 1 < bytes.len() && bytes[i] == b'(' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b')') {
                i += 1;
            }
            i += 2;
        } else if bytes[i] == b'?' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'?' {
                i += 1;
            }
            i += 1;
            out.push(' ');
        } else if bytes[i] == b'"' {
            in_string = true;
            quote = bytes[i];
            out.push(bytes[i] as char);
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn known_terminals() -> HashSet<&'static str> {
    [
        "NEWLINE",
        "INDENT",
        "DEDENT",
        "identifier",
        "letter",
        "digit",
        "literal",
        "integer",
        "float",
        "string",
        "char",
        "bool",
        "string_char",
    ]
    .into_iter()
    .collect()
}

#[test]
fn ebnf_has_no_dangling_rule_references() {
    let src = strip_comments(&read_ebnf());

    let mut defined: HashSet<String> = HashSet::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(eq) = trimmed.find('=') {
            let lhs = trimmed[..eq].trim();

            if !lhs.is_empty() && lhs.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                defined.insert(lhs.to_string());
            }
        }
    }

    let mut referenced: HashSet<String> = HashSet::new();
    for line in src.lines() {
        let trimmed = line.trim_start();

        let rhs = match trimmed.find('=') {
            Some(eq)
                if trimmed[..eq]
                    .trim()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
                    && !trimmed[..eq].trim().is_empty() =>
            {
                &trimmed[eq + 1..]
            }
            _ => trimmed,
        };
        let mut in_string = false;
        let mut quote = ' ';
        let mut word = String::new();
        let flush = |w: &mut String, set: &mut HashSet<String>| {
            if !w.is_empty() {
                if w.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    set.insert(std::mem::take(w));
                } else {
                    w.clear();
                }
            }
        };
        for ch in rhs.chars() {
            if in_string {
                if ch == quote {
                    in_string = false;
                }
                continue;
            }
            match ch {
                '"' | '\'' => {
                    flush(&mut word, &mut referenced);
                    in_string = true;
                    quote = ch;
                }
                c if c.is_ascii_alphanumeric() || c == '_' => word.push(c),
                _ => flush(&mut word, &mut referenced),
            }
        }
        flush(&mut word, &mut referenced);
    }

    let known = known_terminals();
    let mut dangling: Vec<String> = referenced
        .into_iter()
        .filter(|r| !defined.contains(r) && !known.contains(r.as_str()))
        .collect();
    dangling.sort();

    assert!(
        dangling.is_empty(),
        "jinn.ebnf references undefined rules (drift between grammar and itself): {dangling:?}\n\
         Either define these productions or add them to `known_terminals()`."
    );
}

fn ebnf_keyword_terminals(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            let lit = &src[start..j];
            if !lit.is_empty() && lit.chars().all(|c| c.is_ascii_lowercase()) && lit.len() > 1 {
                out.insert(lit.to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

fn is_lexer_keyword(word: &str) -> bool {
    let mut lx = Lexer::new(word);
    let toks = match lx.tokenize() {
        Ok(t) => t,
        Err(_) => return false,
    };

    match toks.first() {
        Some(spanned) => !matches!(spanned.token, Token::Ident(_)),
        None => false,
    }
}

#[test]
fn ebnf_keywords_are_reserved_in_lexer() {
    let src = strip_comments(&read_ebnf());
    let candidates = ebnf_keyword_terminals(&src);

    let non_keyword_allow: HashSet<&str> = [
        "i64", "i32", "u64", "u32", "f64", "f32", "bool", "string", "char", "void", "self",
        "block", "on", "vec",
    ]
    .into_iter()
    .collect();

    let mut missing: Vec<String> = candidates
        .into_iter()
        .filter(|w| !non_keyword_allow.contains(w.as_str()) && !is_lexer_keyword(w))
        .collect();
    missing.sort();

    assert!(
        missing.is_empty(),
        "jinn.ebnf quotes keyword terminals the lexer does not reserve \
         (drift between grammar and lexer): {missing:?}\n\
         Either add them to the lexer KEYWORDS table or to `non_keyword_allow`."
    );
}

#[test]
fn ebnf_corpus_is_accepted_by_parser() {
    let dir = repo_root().join("tests").join("ebnf_corpus");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("ebnf_corpus dir must exist")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "jn").unwrap_or(false))
        .collect();
    files.sort();

    assert!(
        !files.is_empty(),
        "ebnf_corpus is empty; add at least one snippet per production"
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).expect("read corpus file");
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let toks = match Lexer::new(&src).tokenize() {
            Ok(t) => t,
            Err(e) => {
                failures.push(format!("{name}: lex error: {e}"));
                continue;
            }
        };
        if let Err(e) = Parser::new(toks).parse_program() {
            failures.push(format!("{name}: parse error: {e}"));
        }
    }

    assert!(
        failures.is_empty(),
        "ebnf_corpus snippets the parser rejects (grammar ↔ parser drift):\n{}",
        failures.join("\n")
    );
}
