//! Sözdizimi renklendirme.
//!
//! Önizleme bölmesi ve Hızlı Önizleme katmanı için metni renkli parçalara
//! ayırır. Tam bir ayrıştırıcı değil; yorum, dize, sayı, anahtar sözcük, tür
//! ve işlev adlarını ayırt eden tek geçişli bir tarayıcıdır. Renkler burada
//! değil `App::code_job` içinde temaya göre atanır.
//!
//! Amaç hız: 256 KiB'e kadar metin UI iş parçacığında taranır, bu yüzden
//! tarayıcı tahsis yapmaz (yalnızca sonuç vektörü büyür).

use std::ops::Range;
use std::path::Path;

/// Parçanın türü; renk eşlemesi çağıran tarafta yapılır.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Plain,
    Comment,
    Str,
    Num,
    Kw,
    Type,
    Func,
    Punct,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Rust,
    /// C, C++, C#, Java, JavaScript, TypeScript, Kotlin...
    CLike,
    Python,
    Go,
    Json,
    /// TOML, YAML, INI, .env
    Config,
    Shell,
    PowerShell,
    Sql,
}

impl Lang {
    /// Dosya uzantısına göre dil (küçük harf beklenir).
    pub fn from_ext(ext: &str) -> Option<Lang> {
        Some(match ext {
            "rs" => Lang::Rust,
            "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hxx" | "cs" | "java" | "kt" | "kts"
            | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" | "mts" | "cts" | "swift" | "dart"
            | "scala" | "groovy" | "gradle" | "vue" | "svelte" => Lang::CLike,
            "py" | "pyw" | "pyi" => Lang::Python,
            "go" => Lang::Go,
            "json" | "jsonc" | "json5" | "webmanifest" => Lang::Json,
            "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "properties" | "env" => Lang::Config,
            "sh" | "bash" | "zsh" | "fish" | "mk" => Lang::Shell,
            "ps1" | "psm1" | "psd1" => Lang::PowerShell,
            "sql" => Lang::Sql,
            _ => return None,
        })
    }

    /// Kod bloğu bilgisinden (` ```rust `) dil.
    pub fn from_name(name: &str) -> Option<Lang> {
        let n = name.trim().to_ascii_lowercase();
        let n = n.split_whitespace().next().unwrap_or("");
        Some(match n {
            "rust" | "rs" => Lang::Rust,
            "c" | "cpp" | "c++" | "cs" | "csharp" | "java" | "js" | "javascript" | "ts"
            | "typescript" | "jsx" | "tsx" | "kotlin" | "swift" | "dart" | "scala" | "css"
            | "scss" | "html" | "xml" => Lang::CLike,
            "python" | "py" | "python3" => Lang::Python,
            "go" | "golang" => Lang::Go,
            "json" | "jsonc" => Lang::Json,
            "toml" | "yaml" | "yml" | "ini" | "conf" | "config" | "dotenv" => Lang::Config,
            "sh" | "bash" | "shell" | "zsh" | "console" => Lang::Shell,
            "powershell" | "ps1" | "ps" => Lang::PowerShell,
            "sql" => Lang::Sql,
            _ => return None,
        })
    }

    /// Dosya yoluna göre dil; uzantı yoksa bilinen dosya adlarına bakar.
    pub fn from_path(path: &Path) -> Option<Lang> {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if let Some(l) = Self::from_ext(&ext.to_ascii_lowercase()) {
                return Some(l);
            }
        }
        let name = path.file_name()?.to_str()?;
        Some(match name {
            "Makefile" | "makefile" | "GNUmakefile" | ".bashrc" | ".zshrc" | ".profile" => {
                Lang::Shell
            }
            "Cargo.lock" | "Cargo.toml" | ".gitignore" | ".gitattributes" | ".editorconfig" => {
                Lang::Config
            }
            ".gitconfig" => Lang::Config,
            _ => return None,
        })
    }
}

struct Syntax {
    line_comments: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [char],
    /// Dize tek satırı aşabilir mi (Python'un üçlü tırnağı hariç hepsi hayır)
    multiline: bool,
    keywords: &'static [&'static str],
    types: &'static [&'static str],
    literals: &'static [&'static str],
}

const RUST_KW: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "trait",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "macro_rules",
];
const RUST_TY: &[&str] = &[
    "bool",
    "char",
    "str",
    "String",
    "Vec",
    "Option",
    "Result",
    "Box",
    "Rc",
    "Arc",
    "HashMap",
    "HashSet",
    "BTreeMap",
    "Some",
    "None",
    "Ok",
    "Err",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "isize",
    "usize",
    "f32",
    "f64",
    "Path",
    "PathBuf",
    "Cell",
    "RefCell",
    "Mutex",
    "Cow",
    "PhantomData",
    "Duration",
    "Instant",
];

const C_KW: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "auto",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "extern",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "goto",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "internal",
    "let",
    "namespace",
    "new",
    "of",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "sealed",
    "sizeof",
    "static",
    "super",
    "switch",
    "template",
    "this",
    "throw",
    "throws",
    "try",
    "typedef",
    "typename",
    "typeof",
    "union",
    "unsafe",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "yield",
];
const C_TY: &[&str] = &[
    "bool", "byte", "char", "decimal", "double", "float", "int", "long", "object", "short",
    "signed", "size_t", "string", "String", "uint", "ulong", "unsigned", "ushort", "wchar_t",
    "int8_t", "int16_t", "int32_t", "int64_t", "uint8_t", "uint16_t", "uint32_t", "uint64_t",
    "BigInt", "Number", "Boolean", "any", "never", "unknown", "std", "vector", "map", "set",
];

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield",
];
const PY_TY: &[&str] = &[
    "bool",
    "bytes",
    "dict",
    "float",
    "frozenset",
    "int",
    "list",
    "object",
    "set",
    "str",
    "tuple",
    "type",
    "Self",
    "Optional",
    "Any",
    "Callable",
    "Iterable",
    "Sequence",
    "Mapping",
];

const GO_KW: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];
const GO_TY: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
    "nil",
];

const SH_KW: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function", "if", "in",
    "local", "readonly", "return", "select", "set", "shift", "then", "unset", "until", "while",
];
const SH_TY: &[&str] = &[];

const PS_KW: &[&str] = &[
    "begin",
    "break",
    "catch",
    "class",
    "continue",
    "data",
    "do",
    "dynamicparam",
    "else",
    "elseif",
    "end",
    "enum",
    "exit",
    "filter",
    "finally",
    "for",
    "foreach",
    "from",
    "function",
    "if",
    "in",
    "param",
    "process",
    "return",
    "switch",
    "throw",
    "trap",
    "try",
    "until",
    "using",
    "while",
    "workflow",
];
const PS_TY: &[&str] = &[
    "bool",
    "byte",
    "char",
    "datetime",
    "decimal",
    "double",
    "float",
    "guid",
    "hashtable",
    "int",
    "int16",
    "int32",
    "int64",
    "long",
    "object",
    "psobject",
    "regex",
    "string",
    "void",
];

const SQL_KW: &[&str] = &[
    "add",
    "all",
    "alter",
    "and",
    "any",
    "as",
    "asc",
    "begin",
    "between",
    "by",
    "case",
    "cast",
    "column",
    "commit",
    "constraint",
    "create",
    "cross",
    "database",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "except",
    "exists",
    "foreign",
    "from",
    "full",
    "group",
    "having",
    "if",
    "in",
    "index",
    "inner",
    "insert",
    "intersect",
    "into",
    "is",
    "join",
    "key",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "or",
    "order",
    "outer",
    "primary",
    "references",
    "right",
    "rollback",
    "select",
    "set",
    "table",
    "then",
    "top",
    "transaction",
    "union",
    "unique",
    "update",
    "values",
    "view",
    "when",
    "where",
    "with",
];

fn syntax(lang: Lang) -> Syntax {
    match lang {
        Lang::Rust => Syntax {
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\''],
            multiline: false,
            keywords: RUST_KW,
            types: RUST_TY,
            literals: &["true", "false"],
        },
        Lang::CLike => Syntax {
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            multiline: false,
            keywords: C_KW,
            types: C_TY,
            literals: &[
                "true",
                "false",
                "null",
                "nullptr",
                "NULL",
                "undefined",
                "NaN",
            ],
        },
        Lang::Python => Syntax {
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
            multiline: true,
            keywords: PY_KW,
            types: PY_TY,
            literals: &["True", "False", "None", "self", "cls"],
        },
        Lang::Go => Syntax {
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"', '\'', '`'],
            multiline: true,
            keywords: GO_KW,
            types: GO_TY,
            literals: &["true", "false", "iota"],
        },
        Lang::Json => Syntax {
            line_comments: &["//"],
            block: Some(("/*", "*/")),
            quotes: &['"'],
            multiline: false,
            keywords: &[],
            types: &[],
            literals: &["true", "false", "null"],
        },
        Lang::Config => Syntax {
            line_comments: &["#", ";"],
            block: None,
            quotes: &['"', '\''],
            multiline: true,
            keywords: &[],
            types: &[],
            literals: &["true", "false", "null", "yes", "no", "on", "off"],
        },
        Lang::Shell => Syntax {
            line_comments: &["#"],
            block: None,
            quotes: &['"', '\''],
            multiline: true,
            keywords: SH_KW,
            types: SH_TY,
            literals: &["true", "false"],
        },
        Lang::PowerShell => Syntax {
            line_comments: &["#"],
            block: Some(("<#", "#>")),
            quotes: &['"', '\''],
            multiline: true,
            keywords: PS_KW,
            types: PS_TY,
            literals: &["$true", "$false", "$null"],
        },
        Lang::Sql => Syntax {
            line_comments: &["--"],
            block: Some(("/*", "*/")),
            quotes: &['\'', '"'],
            multiline: true,
            keywords: SQL_KW,
            types: &[
                "bigint",
                "binary",
                "bit",
                "blob",
                "char",
                "date",
                "datetime",
                "decimal",
                "double",
                "float",
                "int",
                "integer",
                "json",
                "numeric",
                "real",
                "text",
                "time",
                "timestamp",
                "tinyint",
                "varchar",
            ],
            literals: &["true", "false", "null"],
        },
    }
}

/// Metni renkli parçalara ayırır. Dönen aralıklar sıralıdır ve metnin tamamını
/// (boşluklar dahil) kapsar; `Plain` parçalar da döner.
pub fn tokens(lang: Lang, src: &str) -> Vec<(Range<usize>, Tok)> {
    let s = syntax(lang);
    let b = src.as_bytes();
    let n = b.len();
    let mut out: Vec<(Range<usize>, Tok)> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let start = i;
        // Yorumlar
        if s.line_comments.iter().any(|p| src[start..].starts_with(*p)) {
            let mut j = start;
            while j < n && b[j] != b'\n' {
                j += 1;
            }
            out.push((start..j, Tok::Comment));
            i = j;
            continue;
        }
        if let Some((open, close)) = s.block {
            if src[start..].starts_with(open) {
                let mut j = start + open.len();
                while j < n && !src[j..].starts_with(close) {
                    j += 1;
                }
                let end = (j + close.len()).min(n);
                out.push((start..end, Tok::Comment));
                i = end;
                continue;
            }
        }
        let c = src[start..].chars().next().unwrap();
        // Dizeler (`\` kaçışı ve Python'un üçlü tırnağı desteklenir)
        if s.quotes.contains(&c) {
            let triple = s.multiline && src[start..].starts_with(&format!("{c}{c}{c}"));
            if triple {
                let pat = format!("{c}{c}{c}");
                let mut j = start + 3;
                while j < n && !src[j..].starts_with(&pat) {
                    j += src[j..].chars().next().unwrap().len_utf8();
                }
                let end = (j + 3).min(n);
                out.push((start..end, Tok::Str));
                i = end;
                continue;
            }
            let mut j = start + c.len_utf8();
            while j < n {
                let ch = src[j..].chars().next().unwrap();
                if ch == '\\' {
                    j += ch.len_utf8();
                    if j < n {
                        j += src[j..].chars().next().unwrap().len_utf8();
                    }
                    continue;
                }
                if ch == c {
                    j += ch.len_utf8();
                    break;
                }
                if ch == '\n' && !s.multiline {
                    break;
                }
                j += ch.len_utf8();
            }
            out.push((start..j, Tok::Str));
            i = j;
            continue;
        }
        // Sayılar (0xFF, 1_000, 1.5e-3)
        if c.is_ascii_digit() {
            let mut j = start;
            while j < n {
                let ch = b[j];
                if ch.is_ascii_alphanumeric() || ch == b'.' || ch == b'_' {
                    j += 1;
                } else if (ch == b'+' || ch == b'-') && j > start && matches!(b[j - 1], b'e' | b'E')
                {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push((start..j, Tok::Num));
            i = j;
            continue;
        }
        // Tanımlayıcılar
        if c.is_alphabetic() || c == '_' || c == '$' || c == '@' || c == '#' {
            let mut j = start;
            while j < n {
                let ch = src[j..].chars().next().unwrap();
                if ch.is_alphanumeric() || ch == '_' || ch == '$' || ch == '#' {
                    j += ch.len_utf8();
                } else {
                    break;
                }
            }
            let word = &src[start..j];
            let t = if s.types.contains(&word) {
                Tok::Type
            } else if s.keywords.contains(&word) || s.literals.contains(&word) {
                Tok::Kw
            } else if next_non_space(b, j) == Some(b'(') {
                Tok::Func
            } else {
                Tok::Plain
            };
            out.push((start..j, t));
            i = j;
            continue;
        }
        // Boşluk ve noktalama: tek karakter
        out.push((start..start + c.len_utf8(), Tok::Punct));
        i = start + c.len_utf8();
    }
    out
}

fn next_non_space(b: &[u8], mut i: usize) -> Option<u8> {
    while i < b.len() {
        if !b[i].is_ascii_whitespace() {
            return Some(b[i]);
        }
        i += 1;
    }
    None
}

/// Renklendirme için gereken renkler (tema paletinden türetilir).
#[derive(Clone, Copy)]
pub struct Colors {
    pub text: egui::Color32,
    pub weak: egui::Color32,
    pub accent: egui::Color32,
    pub dark: bool,
}

/// Renkli metni egui `LayoutJob`'una çevirir. `lang` yoksa düz eş aralıklı metin.
pub fn job(lang: Option<Lang>, src: &str, size: f32, c: Colors) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let font = egui::FontId::monospace(size);
    match lang {
        Some(l) => {
            for (r, t) in tokens(l, src) {
                job.append(
                    &src[r],
                    0.0,
                    egui::TextFormat {
                        font_id: font.clone(),
                        color: tok_color(t, c),
                        ..Default::default()
                    },
                );
            }
        }
        None => {
            job.append(
                src,
                0.0,
                egui::TextFormat {
                    font_id: font,
                    color: c.text,
                    ..Default::default()
                },
            );
        }
    }
    job
}

/// Editör renkleri. Açık temada VS Code'un açık paletine, koyu temada One Dark'a
/// yakın; anahtar sözcükler temanın vurgu rengini kullanır.
fn tok_color(t: Tok, c: Colors) -> egui::Color32 {
    let rgb = |r: u8, g: u8, b: u8| egui::Color32::from_rgb(r, g, b);
    match t {
        Tok::Plain | Tok::Punct => c.text,
        Tok::Comment => c.weak,
        Tok::Kw => c.accent,
        Tok::Str => {
            if c.dark {
                rgb(0x9e, 0xd1, 0x7a)
            } else {
                rgb(0x0a, 0x7a, 0x3d)
            }
        }
        Tok::Num => {
            if c.dark {
                rgb(0xb5, 0xce, 0xa8)
            } else {
                rgb(0x09, 0x86, 0x58)
            }
        }
        Tok::Type => {
            if c.dark {
                rgb(0x4e, 0xc9, 0xb0)
            } else {
                rgb(0x1a, 0x6f, 0x8a)
            }
        }
        Tok::Func => {
            if c.dark {
                rgb(0xdc, 0xdc, 0xaa)
            } else {
                rgb(0x79, 0x5e, 0x26)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Lang, src: &str) -> Vec<(String, Tok)> {
        tokens(lang, src)
            .into_iter()
            .map(|(r, t)| (src[r].to_string(), t))
            .collect()
    }

    #[test]
    fn rust_comment_string_and_keyword() {
        let src = "fn main() { let s = \"hi\"; } // done\n";
        let out = kinds(Lang::Rust, src);
        assert!(out.contains(&("fn".to_string(), Tok::Kw)));
        assert!(out.contains(&("\"hi\"".to_string(), Tok::Str)));
        assert!(out.contains(&("// done".to_string(), Tok::Comment)));
        // Tüm metin kapsanır
        let total: usize = tokens(Lang::Rust, src).iter().map(|(r, _)| r.len()).sum();
        assert_eq!(total, src.len());
    }

    #[test]
    fn block_comment_spans_lines() {
        let src = "/* a\n b */ x";
        let out = kinds(Lang::CLike, src);
        assert!(out.contains(&("/* a\n b */".to_string(), Tok::Comment)));
    }

    #[test]
    fn python_triple_quote() {
        let src = "x = \"\"\"a\nb\"\"\"\n";
        let out = kinds(Lang::Python, src);
        assert!(out.contains(&("\"\"\"a\nb\"\"\"".to_string(), Tok::Str)));
    }

    #[test]
    fn numbers_and_calls() {
        let out = kinds(Lang::Rust, "foo(0xFF, 1.5e-3)");
        assert!(out.contains(&("foo".to_string(), Tok::Func)));
        assert!(out.contains(&("0xFF".to_string(), Tok::Num)));
        assert!(out.contains(&("1.5e-3".to_string(), Tok::Num)));
    }

    #[test]
    fn unknown_extension_has_no_language() {
        assert!(Lang::from_ext("zzz").is_none());
        assert_eq!(Lang::from_name("rust"), Some(Lang::Rust));
    }
}
