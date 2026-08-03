use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/parallel.rs"]
mod parallel;

type Unit = (usize, String, Vec<(String, String)>);

fn jinnc() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jinnc"))
}

fn doc_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("jinn.md")
}

#[derive(Debug, Default, Clone)]
struct Markers {
    skip: Option<String>,
    prelude: Vec<String>,
    file: Option<String>,
}

#[derive(Debug)]
struct Block {
    line: usize,
    code: String,
    markers: Markers,
}

fn extract_blocks(doc: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut pending = Markers::default();
    let mut lines = doc.lines().enumerate().peekable();

    while let Some((idx, line)) = lines.next() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("<!-- doctest:") {
            if let Some(reason) = rest.strip_prefix("skip") {
                pending.skip = Some(reason.trim_end_matches("-->").trim().to_string());
            } else if let Some(name) = rest.strip_prefix("file") {
                pending.file = Some(name.trim_end_matches("-->").trim().to_string());
            } else if rest.starts_with("prelude") {
                for (_, pl) in lines.by_ref() {
                    if pl.trim() == "-->" {
                        break;
                    }
                    pending.prelude.push(pl.to_string());
                }
            } else {
                panic!(
                    "jinn.md line {}: unknown doctest marker: {trimmed}",
                    idx + 1
                );
            }
            continue;
        }
        if trimmed == "```jinn" {
            let start = idx + 1;
            let mut code = String::new();
            for (_, cl) in lines.by_ref() {
                if cl.trim_start().starts_with("```") {
                    break;
                }
                code.push_str(cl);
                code.push('\n');
            }
            blocks.push(Block {
                line: start,
                code,
                markers: std::mem::take(&mut pending),
            });
        } else if trimmed.starts_with("```") && trimmed != "```" {
            pending = Markers::default();
        }
    }
    blocks
}

const DECL_STARTERS: &[&str] = &[
    "*",
    "type ",
    "enum ",
    "err ",
    "actor ",
    "store ",
    "use ",
    "extern ",
    "alias ",
    "trait ",
    "impl ",
    "global ",
    "migration ",
    "view ",
];

fn is_decl_start(line: &str) -> bool {
    DECL_STARTERS.iter().any(|k| line.starts_with(k))
}

fn chunks(src: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut cur_is_comment_only = true;
    for line in src.lines() {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                cur.push(String::new());
            }
            continue;
        }
        if !line.starts_with(' ') && !cur.is_empty() && !cur_is_comment_only {
            out.push(std::mem::take(&mut cur));
            cur_is_comment_only = true;
        }
        let is_comment = line.trim_start().starts_with('#');
        if !is_comment {
            cur_is_comment_only = false;
        }
        cur.push(line.to_string());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn wrap(code: &str) -> String {
    let all = chunks(code);
    let has_main = all.iter().any(|c| {
        c.iter().any(|l| {
            let t = l.trim_start();
            t.starts_with("*main")
                && !t
                    .trim_start_matches("*main")
                    .starts_with(char::is_alphanumeric)
        })
    });
    if has_main {
        return code.to_string();
    }
    let mut decls: Vec<String> = Vec::new();
    let mut stmts: Vec<String> = Vec::new();
    for chunk in all {
        let first = chunk
            .iter()
            .map(|l| l.trim_start())
            .find(|t| !t.is_empty() && !t.starts_with('#'))
            .unwrap_or("");
        if is_decl_start(first) {
            for l in &chunk {
                decls.push(l.clone());
            }
            decls.push(String::new());
        } else {
            for l in &chunk {
                if l.is_empty() {
                    stmts.push(String::new());
                } else {
                    stmts.push(format!("    {l}"));
                }
            }
        }
    }
    let mut out = decls.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str("*main\n");
    let has_real_stmt = stmts
        .iter()
        .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    if !has_real_stmt {
        stmts.push("    nop".to_string());
    }
    out.push_str(&stmts.join("\n"));
    out.push('\n');
    out
}

fn assemble(block: &Block) -> String {
    if block.markers.prelude.is_empty() {
        wrap(&block.code)
    } else {
        let mut src = block.markers.prelude.join("\n");
        src.push('\n');
        src.push_str(&block.code);
        wrap(&src)
    }
}

fn compile_in(dir: &Path, source: &str) -> Result<(), String> {
    let main = dir.join("example.jn");
    std::fs::write(&main, source).map_err(|e| e.to_string())?;
    let out = Command::new(jinnc())
        .arg("example.jn")
        .arg("-o")
        .arg(dir.join("example.bin"))
        .current_dir(dir)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "exit {:?}\n--- stderr ---\n{}\n--- stdout ---\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout),
        ))
    }
}

#[test]
fn every_jinn_block_in_docs_compiles() {
    let doc = std::fs::read_to_string(doc_path()).expect("read docs/jinn.md");
    let blocks = extract_blocks(&doc);
    assert!(
        blocks.len() > 40,
        "expected to find the doc's examples; got {}",
        blocks.len()
    );

    let mut pending_files: Vec<(String, String)> = Vec::new();
    let mut skipped = 0usize;
    let mut units: Vec<Unit> = Vec::new();

    for block in &blocks {
        if let Some(reason) = &block.markers.skip {
            assert!(
                !reason.is_empty(),
                "jinn.md line {}: doctest:skip requires a reason",
                block.line
            );
            skipped += 1;
            continue;
        }
        if let Some(name) = &block.markers.file {
            pending_files.push((name.clone(), block.code.clone()));
            continue;
        }
        units.push((
            block.line,
            assemble(block),
            std::mem::take(&mut pending_files),
        ));
    }

    let compiled = units.len();
    let failures: Vec<String> = parallel::par_map(units, |(line, source, files)| {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
        compile_in(dir.path(), source).err().map(|e| {
            format!(
                "docs/jinn.md line {line}: block failed to compile: {e}\n--- assembled source ---\n{source}"
            )
        })
    })
    .into_iter()
    .flatten()
    .collect();

    eprintln!("doc_examples: {compiled} blocks compiled, {skipped} skipped");
    assert!(
        failures.is_empty(),
        "{} documented example(s) do not compile:\n\n{}",
        failures.len(),
        failures.join("\n=====\n")
    );
}
