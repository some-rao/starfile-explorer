//! Git deposu bilgisi: dal adı, değişiklik işareti, ileri/geri sayıları ve
//! dosya durumları.
//!
//! Dal adı her zaman `.git/HEAD` dosyasından okunur (hızlı, `git` gerekmez).
//! Dosya harfleri (`M`, `A`, `?`, ...) yalnızca `git` kuruluysa
//! `git status --porcelain=v2` ile alınır; yoksa sütun yalnızca dal gösterir.
//! Tüm çağrılar engelleyicidir; UI iş parçacığından **çağrılmamalıdır**.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Default)]
pub struct GitInfo {
    pub root: PathBuf,
    pub branch: String,
    pub dirty: bool,
    pub ahead: u32,
    pub behind: u32,
    /// Mutlak dosya yolu -> durum harfi
    pub files: HashMap<PathBuf, char>,
}

impl GitInfo {
    pub fn file_state(&self, p: &Path) -> Option<char> {
        self.files.get(p).copied()
    }
}

/// `dir`den yukarı yürüyerek `.git` (klasör ya da worktree dosyası) arar.
pub fn repo_root(dir: &Path) -> Option<PathBuf> {
    for a in dir.ancestors() {
        if a.join(".git").exists() {
            return Some(a.to_path_buf());
        }
    }
    None
}

/// Engellemeli. `dir` bir depo içindeyse bilgi döner.
pub fn read(dir: &Path) -> Option<GitInfo> {
    let root = repo_root(dir)?;
    let gd = git_dir(&root);
    let mut info = GitInfo {
        root: root.clone(),
        ..Default::default()
    };
    info.branch = read_branch(&gd).unwrap_or_default();
    if let Some(st) = run_status(&root) {
        info.dirty = st.dirty;
        info.files = st.files;
        if !st.branch.is_empty() {
            info.branch = st.branch;
        }
        info.ahead = st.ahead;
        info.behind = st.behind;
    }
    if info.branch.is_empty() {
        info.branch = "?".to_string();
    }
    Some(info)
}

/// Worktree'lerde `.git` bir dosyadır: `gitdir: <yol>` satırını çözer.
fn git_dir(root: &Path) -> PathBuf {
    let dot = root.join(".git");
    if dot.is_file() {
        if let Ok(text) = std::fs::read_to_string(&dot) {
            if let Some(rest) = text.trim().strip_prefix("gitdir:") {
                let p = PathBuf::from(rest.trim());
                return if p.is_absolute() { p } else { root.join(p) };
            }
        }
    }
    dot
}

/// `.git/HEAD` -> dal adı (ya da kopuk HEAD'de kısa SHA).
fn read_branch(gd: &Path) -> Option<String> {
    let head = std::fs::read_to_string(gd.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        let name = r.strip_prefix("refs/heads/").unwrap_or(r);
        Some(name.to_string())
    } else if head.len() >= 7 {
        Some(head[..7].to_string())
    } else {
        None
    }
}

#[derive(Default)]
struct Status {
    branch: String,
    dirty: bool,
    ahead: u32,
    behind: u32,
    files: HashMap<PathBuf, char>,
}

fn run_status(root: &Path) -> Option<Status> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=normal",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(parse_status(&text, root))
}

fn parse_status(out: &str, root: &Path) -> Status {
    let mut st = Status::default();
    for rec in out.split('\0') {
        if rec.is_empty() {
            continue;
        }
        if let Some(rest) = rec.strip_prefix("# branch.head ") {
            st.branch = rest.trim().to_string();
            continue;
        }
        if let Some(rest) = rec.strip_prefix("# branch.ab ") {
            for part in rest.split_whitespace() {
                if let Some(a) = part.strip_prefix('+') {
                    st.ahead = a.parse().unwrap_or(0);
                } else if let Some(b) = part.strip_prefix('-') {
                    st.behind = b.parse().unwrap_or(0);
                }
            }
            continue;
        }
        let bytes = rec.as_bytes();
        if bytes.len() < 3 {
            continue;
        }
        // `? <yol>` — izlenmeyen
        if bytes[0] == b'?' && bytes[1] == b' ' {
            st.dirty = true;
            st.files.insert(root.join(&rec[2..]), '?');
            continue;
        }
        // `1 XY ...`, `2 XY ...` (yeniden adlandırma), `u XY ...` (çakışma)
        let tokens = match bytes[0] {
            b'1' => 9,
            b'2' => 10,
            b'u' => 11,
            _ => 0,
        };
        if tokens == 0 || bytes[1] != b' ' {
            continue;
        }
        let mut it = rec.splitn(tokens, ' ');
        it.next(); // kayıt türü
        let xy = it.next().unwrap_or("..").to_string();
        let mut path = String::new();
        for _ in 3..=tokens {
            if let Some(p) = it.next() {
                path = p.to_string();
            }
        }
        if path.is_empty() {
            continue;
        }
        st.dirty = true;
        st.files.insert(root.join(path), status_letter(&xy));
    }
    st
}

/// `XY` çiftinden gösterilecek tek harf: çalışma ağacı harfi varsa o, yoksa indeks harfi.
fn status_letter(xy: &str) -> char {
    let mut cs = xy.chars();
    let x = cs.next().unwrap_or('.');
    let y = cs.next().unwrap_or('.');
    if y != '.' {
        y
    } else if x != '.' {
        x
    } else {
        ' '
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_ab_and_files() {
        let text = "# branch.oid 1234\0# branch.head main\0# branch.ab +2 -3\0\
                    1 .M N... 100644 100644 100644 aaa bbb src/app.rs\0\
                    1 A. N... 100644 100644 100644 aaa bbb yeni dosya.txt\0\
                    ? taslak.md\0";
        let root = Path::new("C:\\repo");
        let st = parse_status(text, root);
        assert_eq!(st.branch, "main");
        assert_eq!((st.ahead, st.behind), (2, 3));
        assert!(st.dirty);
        assert_eq!(st.files.get(&root.join("src/app.rs")), Some(&'M'));
        assert_eq!(st.files.get(&root.join("yeni dosya.txt")), Some(&'A'));
        assert_eq!(st.files.get(&root.join("taslak.md")), Some(&'?'));
    }

    #[test]
    fn status_letter_prefers_worktree() {
        assert_eq!(status_letter("M."), 'M');
        assert_eq!(status_letter(".M"), 'M');
        assert_eq!(status_letter("MM"), 'M');
        assert_eq!(status_letter(".."), ' ');
    }
}
