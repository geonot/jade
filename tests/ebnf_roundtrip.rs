//! EBNF ↔ implementation drift detector (B.8 / P1-13).
//!
//! `jinn.ebnf` is the canonical surface grammar. It is paired with two
//! implementations: the Rust parser in `src/parser/` and the tree-sitter
//! grammar in `tree-sitter-jinn/grammar.js`. This test guards against
//! three classes of drift:
//!
//!   1. **EBNF internal consistency** — every non-terminal referenced on
//!      a right-hand side must be defined somewhere (or be a documented
//!      lexer-produced terminal). Catches dangling rule references.
//!
//!   2. **EBNF keyword ↔ lexer** — every alphabetic keyword terminal the
//!      grammar quotes (e.g. `"actor"`, `"returns"`) must actually be a
//!      reserved word in the lexer. Catches a grammar that documents a
//!      keyword the lexer never produces.
//!
//!   3. **Grammar ↔ parser corpus** — every snippet in `tests/ebnf_corpus/`
//!      exercises a production from the EBNF and must be accepted by the
//!      real Rust parser. The same corpus is fed to tree-sitter by the
//!      `ebnf-roundtrip` CI job (see `.github/workflows/ci.yml`), so a
//!      construct the EBNF claims to support but either implementation
//!      rejects is flagged.

use std::collections::HashSet;
use std::path::PathBuf;

use jinnc::lexer::{Lexer, Token};
use jinnc::parser::Parser;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_ebnf() -> String {
    std::fs::read_to_string(repo_root().join("jinn.ebnf")).expect("jinn.ebnf must exist")
}

/// Strip `(* ... *)` block comments and `? ... ?` special sequences
/// (prose terminals) from the EBNF so they don't pollute rule/terminal
/// extraction. Quote-aware: `?` and `(` inside `"..."` are preserved.
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
                // escaped char (e.g. \" or \\) — copy verbatim, do not
                // let the escaped quote close the string.
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
            // skip to closing *)
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b')') {
                i += 1;
            }
            i += 2;
        } else if bytes[i] == b'?' {
            // EBNF special sequence: ? prose ?
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

/// Lexer-produced or pseudo-terminals that legitimately appear on a
/// right-hand side without a `rule = ...` definition.
fn known_terminals() -> HashSet<&'static str> {
    [
        "NEWLINE", "INDENT", "DEDENT", // off-side rule tokens
        "identifier", "letter", "digit", "literal", "integer", "float", "string", "char", "bool",
        "string_char", // lexical terminals (defined or self-evident)
    ]
    .into_iter()
    .collect()
}

#[test]
fn ebnf_has_no_dangling_rule_references() {
    let src = strip_comments(&read_ebnf());

    // Defined rules: every `name = ...` at the start of a production.
    // Productions are separated by ` . ` (a period terminator). We scan
    // line-by-line for `lhs = ` openers.
    let mut defined: HashSet<String> = HashSet::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        if let Some(eq) = trimmed.find('=') {
            let lhs = trimmed[..eq].trim();
            // LHS must be a single lowercase identifier (rule name).
            if !lhs.is_empty()
                && lhs
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            {
                defined.insert(lhs.to_string());
            }
        }
    }

    // Referenced rules: lowercase identifiers appearing on RHS that are
    // not inside quotes. We tokenise crudely: split on whitespace and
    // grammar punctuation, then keep bare lowercase identifiers.
    let mut referenced: HashSet<String> = HashSet::new();
    for line in src.lines() {
        let trimmed = line.trim_start();
        // Skip past the `lhs =` opener so the LHS name isn't counted.
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

/// Pull every alphabetic, all-lowercase quoted terminal out of the EBNF.
/// These are keyword candidates: `"actor"`, `"returns"`, `"match"`, ...
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
            if !lit.is_empty()
                && lit.chars().all(|c| c.is_ascii_lowercase())
                && lit.len() > 1
            {
                out.insert(lit.to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// A word is a lexer keyword iff lexing it yields exactly one reserved
/// token (i.e. NOT `Token::Ident`).
fn is_lexer_keyword(word: &str) -> bool {
    let mut lx = Lexer::new(word);
    let toks = match lx.tokenize() {
        Ok(t) => t,
        Err(_) => return false,
    };
    // tokenize() appends a trailing Eof; the keyword is the first token.
    match toks.first() {
        Some(spanned) => !matches!(spanned.token, Token::Ident(_)),
        None => false,
    }
}

#[test]
fn ebnf_keywords_are_reserved_in_lexer() {
    let src = strip_comments(&read_ebnf());
    let candidates = ebnf_keyword_terminals(&src);

    // Words the grammar quotes that are intentionally NOT lexer keywords:
    // they are primitive type names handled as identifiers by the lexer
    // and resolved later, or contextual words.
    let non_keyword_allow: HashSet<&str> = [
        "i64", "i32", "u64", "u32", "f64", "f32", "bool", "string", "char", "void",
        "self", "block",
        // Contextual keywords: lexed as identifiers, recognised by the
        // parser only in specific positions.
        "on", "vec",
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
