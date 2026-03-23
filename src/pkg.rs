use std::path::Path;
use crate::lexer::{Lexer, Token};

#[derive(Debug, Clone)]
pub struct SemVer {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for SemVer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl SemVer {
    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("invalid semver: {s}"));
        }
        Ok(SemVer {
            major: parts[0].parse().map_err(|_| format!("invalid major: {}", parts[0]))?,
            minor: parts[1].parse().map_err(|_| format!("invalid minor: {}", parts[1]))?,
            patch: parts[2].parse().map_err(|_| format!("invalid patch: {}", parts[2]))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Dependency {
    pub name: String,
    pub url: String,
    pub version: SemVer,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub version: SemVer,
    pub author: Option<String>,
    pub requires: Vec<Dependency>,
}

impl Package {
    /// Parse a jade.pkg manifest. Uses the Jade lexer for tokenization.
    /// Format:
    ///   package <name>
    ///   version <X.Y.Z>
    ///   author <name>
    ///   require <name> <url> <version>
    pub fn parse(input: &str) -> Result<Self, String> {
        let mut name = None;
        let mut version = None;
        let mut author = None;
        let mut requires = Vec::new();

        for (line_num, line) in input.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let tokens = Lexer::new(trimmed)
                .tokenize()
                .map_err(|e| format!("jade.pkg line {}: {e}", line_num + 1))?;
            let toks: Vec<&Token> = tokens.iter().map(|s| &s.token).collect();
            match toks.first() {
                Some(Token::Ident(kw)) if kw == "package" => {
                    if let Some(Token::Ident(n)) = toks.get(1) {
                        name = Some(n.clone());
                    } else {
                        return Err(format!("jade.pkg line {}: expected package name", line_num + 1));
                    }
                }
                Some(Token::Ident(kw)) if kw == "version" => {
                    let rest = trimmed.strip_prefix("version").unwrap().trim();
                    version = Some(SemVer::parse(rest)
                        .map_err(|e| format!("jade.pkg line {}: {e}", line_num + 1))?);
                }
                Some(Token::Ident(kw)) if kw == "author" => {
                    let rest = trimmed.strip_prefix("author").unwrap().trim();
                    author = Some(rest.to_string());
                }
                Some(Token::Ident(kw)) if kw == "require" => {
                    // require <name> <url> <version>
                    let parts: Vec<&str> = trimmed.splitn(4, char::is_whitespace).collect();
                    if parts.len() < 4 {
                        return Err(format!("jade.pkg line {}: require needs name url version", line_num + 1));
                    }
                    requires.push(Dependency {
                        name: parts[1].to_string(),
                        url: parts[2].to_string(),
                        version: SemVer::parse(parts[3].trim())
                            .map_err(|e| format!("jade.pkg line {}: {e}", line_num + 1))?,
                    });
                }
                _ => {
                    return Err(format!("jade.pkg line {}: unknown directive: {trimmed}", line_num + 1));
                }
            }
        }

        Ok(Package {
            name: name.ok_or("jade.pkg: missing 'package' directive")?,
            version: version.ok_or("jade.pkg: missing 'version' directive")?,
            author,
            requires,
        })
    }

    pub fn from_file(path: &Path) -> Result<Self, String> {
        let input = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::parse(&input)
    }

    /// Generate a jade.pkg manifest string
    pub fn to_string_repr(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("package {}\n", self.name));
        out.push_str(&format!("version {}\n", self.version));
        if let Some(ref a) = self.author {
            out.push_str(&format!("author {a}\n"));
        }
        if !self.requires.is_empty() {
            out.push('\n');
            for dep in &self.requires {
                out.push_str(&format!("require {} {} {}\n", dep.name, dep.url, dep.version));
            }
        }
        out
    }
}
