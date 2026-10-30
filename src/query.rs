//! Arama/filtre sorgusu.
//!
//! Hem gezinti filtresi (yazdıkça) hem alt klasör araması aynı dilbilgisini
//! kullanır: serbest kelimeler, `ext:`, `type:`, `size:`, `modified:`, `kind:`
//! ve `-kelime` olumsuzlaması.
//!
//! Sorgu metni `Tab.filter` içinde tutulur; ayrıştırma ucuz olduğu için görünüm
//! her yeniden kuruluşunda tekrar yapılır (`Query::parse`). Böylece filtre metni
//! ile türetilmiş durum (çipler, koşullar) asla birbirinden ayrışmaz.

use std::time::SystemTime;

use chrono::{DateTime, Local};

use crate::fsutil::{self, Entry, Pattern};

/// Aramanın kapsamı: yalnızca bu klasör, alt klasörler dahil ya da tüm sürücüler.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum SearchScope {
    /// Yalnızca bu klasör — arama motoru çalışmaz, filtre bellekte uygulanır.
    Folder,
    /// Bu klasör ve alt klasörleri (Explorer'ın Enter davranışı).
    #[default]
    Subfolders,
    /// Bu bilgisayar: tüm sürücü kökleri.
    ThisPc,
}

#[derive(Clone)]
pub struct Chip {
    /// Sorgu metnindeki özgün yazım (`ext:pdf`, `-taslak`, ...)
    pub token: String,
}

#[derive(Clone, Copy)]
enum SizeCmp {
    Eq(u64),
    Gt(u64),
    Ge(u64),
    Lt(u64),
    Le(u64),
    Range(u64, u64),
}

#[derive(Clone)]
enum Pred {
    Ext(Vec<String>),
    /// `true` = yalnızca klasörler, `false` = yalnızca dosyalar
    Folder(bool),
    Size(SizeCmp),
    /// 0 = bugün, 1 = bu hafta, 2 = bu ay, 3 = bu yıl
    Modified(u8),
    Kind(String),
    Not(Pattern),
}

#[derive(Clone)]
pub struct Query {
    pub chips: Vec<Chip>,
    /// Serbest (anahtar:değer olmayan) kelimeler; `without_chip` yeniden kurmak için
    terms: Vec<String>,
    preds: Vec<Pred>,
    pattern: Pattern,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            chips: Vec::new(),
            terms: Vec::new(),
            preds: Vec::new(),
            pattern: Pattern::new(""),
        }
    }
}

impl Query {
    /// Sorgu metnini ayrıştırır. Bilinmeyen `anahtar:değer` metinleri sıradan
    /// kelime sayılır (yazarken `ext:` yarım kaldığında liste boşalmasın).
    pub fn parse(text: &str) -> Query {
        let mut terms: Vec<String> = Vec::new();
        let mut chips: Vec<Chip> = Vec::new();
        let mut preds: Vec<Pred> = Vec::new();
        for token in text.split_whitespace() {
            if let Some(rest) = token.strip_prefix('-') {
                let p = Pattern::new(rest);
                if !p.is_empty() {
                    chips.push(Chip {
                        token: token.to_string(),
                    });
                    preds.push(Pred::Not(p));
                    continue;
                }
            }
            if let Some((key, value)) = token.split_once(':') {
                let key = key.to_ascii_lowercase();
                let value = value.trim();
                if !value.is_empty() {
                    if let Some(pred) = parse_pred(&key, value) {
                        chips.push(Chip {
                            token: token.to_string(),
                        });
                        preds.push(pred);
                        continue;
                    }
                }
            }
            terms.push(token.to_string());
        }
        Query {
            chips,
            pattern: Pattern::new(&terms.join(" ")),
            terms,
            preds,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_empty() && self.preds.is_empty()
    }

    /// Öge sorguyla eşleşiyor mu? `now`, arama sırasında bir kez alınır ki
    /// milyonlarca öge için saat yeniden okunmasın.
    pub fn matches(&self, e: &Entry, now: DateTime<Local>) -> bool {
        if !self.pattern.is_empty() && !self.pattern.matches(&e.name_lower) {
            return false;
        }
        self.preds.iter().all(|p| p.matches(e, now))
    }

    /// Bu klasör sayfasındaki sürücü kartları için yalnızca ad deseni.
    pub fn name_pattern(&self) -> &Pattern {
        &self.pattern
    }

    /// Serbest (anahtar:değer olmayan) kelimeler; skorlama için.
    pub fn free_terms(&self) -> &[String] {
        &self.terms
    }

    /// `i` numaralı çipi atarak sorgu metnini yeniden kurar (diğer çipler ve
    /// serbest kelimeler korunur).
    pub fn without_chip(&self, i: usize) -> String {
        let mut parts: Vec<String> = self.terms.clone();
        for (j, c) in self.chips.iter().enumerate() {
            if j != i {
                parts.push(c.token.clone());
            }
        }
        parts.join(" ")
    }
}

fn parse_pred(key: &str, value: &str) -> Option<Pred> {
    match key {
        "ext" | "uzanti" | "uzantı" => {
            let list: Vec<String> = value
                .split(',')
                .map(|x| x.trim().trim_start_matches('.').to_ascii_lowercase())
                .filter(|x| !x.is_empty())
                .collect();
            if list.is_empty() {
                None
            } else {
                Some(Pred::Ext(list))
            }
        }
        "type" | "tur" | "tür" => match value.to_ascii_lowercase().as_str() {
            "file" | "dosya" => Some(Pred::Folder(false)),
            "folder" | "klasor" | "klasör" => Some(Pred::Folder(true)),
            _ => None,
        },
        "size" | "boyut" => parse_size(value).map(Pred::Size),
        "modified" | "tarih" => {
            let w = match value.to_ascii_lowercase().as_str() {
                "today" | "bugun" | "bugün" => 0,
                "week" | "hafta" => 1,
                "month" | "ay" => 2,
                "year" | "yil" | "yıl" => 3,
                _ => return None,
            };
            Some(Pred::Modified(w))
        }
        "kind" | "cesit" | "çeşit" => {
            let k = value.to_ascii_lowercase();
            if is_known_kind(&k) {
                Some(Pred::Kind(k))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn parse_size(v: &str) -> Option<SizeCmp> {
    let v = v.trim().to_ascii_lowercase();
    if let Some((a, b)) = v.split_once("..") {
        let a = parse_bytes(a)?;
        let b = parse_bytes(b)?;
        return Some(if a <= b {
            SizeCmp::Range(a, b)
        } else {
            SizeCmp::Range(b, a)
        });
    }
    if let Some(r) = v.strip_prefix(">=") {
        return Some(SizeCmp::Ge(parse_bytes(r)?));
    }
    if let Some(r) = v.strip_prefix('>') {
        return Some(SizeCmp::Gt(parse_bytes(r)?));
    }
    if let Some(r) = v.strip_prefix("<=") {
        return Some(SizeCmp::Le(parse_bytes(r)?));
    }
    if let Some(r) = v.strip_prefix('<') {
        return Some(SizeCmp::Lt(parse_bytes(r)?));
    }
    Some(SizeCmp::Eq(parse_bytes(&v)?))
}

fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(v) = s.strip_suffix("tb") {
        (v, 1u64 << 40)
    } else if let Some(v) = s.strip_suffix("gb") {
        (v, 1u64 << 30)
    } else if let Some(v) = s.strip_suffix("mb") {
        (v, 1u64 << 20)
    } else if let Some(v) = s.strip_suffix("kb") {
        (v, 1u64 << 10)
    } else if let Some(v) = s.strip_suffix('b') {
        (v, 1)
    } else {
        (s.as_str(), 1)
    };
    let n: f64 = num.trim().replace(',', ".").parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    Some((n * mult as f64) as u64)
}

fn is_known_kind(k: &str) -> bool {
    matches!(
        k,
        "image"
            | "resim"
            | "video"
            | "audio"
            | "ses"
            | "doc"
            | "belge"
            | "archive"
            | "arsiv"
            | "arşiv"
            | "code"
            | "kod"
    )
}

fn class_matches(kind: &str, e: &Entry) -> bool {
    let ext = e.ext.as_str();
    match kind {
        "image" | "resim" => fsutil::is_image(ext),
        "video" => matches!(
            ext,
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "wmv" | "flv" | "m4v" | "mpg" | "mpeg"
        ),
        "audio" | "ses" => matches!(
            ext,
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "wma" | "opus" | "aiff"
        ),
        "doc" | "belge" => matches!(
            ext,
            "pdf"
                | "doc"
                | "docx"
                | "txt"
                | "rtf"
                | "odt"
                | "xls"
                | "xlsx"
                | "ppt"
                | "pptx"
                | "md"
                | "csv"
                | "epub"
        ),
        "archive" | "arsiv" | "arşiv" => matches!(
            ext,
            "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "iso" | "cab"
        ),
        "code" | "kod" => matches!(
            ext,
            "rs" | "js"
                | "mjs"
                | "ts"
                | "tsx"
                | "jsx"
                | "py"
                | "c"
                | "h"
                | "hpp"
                | "cpp"
                | "cs"
                | "java"
                | "go"
                | "rb"
                | "php"
                | "html"
                | "css"
                | "scss"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "sh"
                | "ps1"
                | "bat"
                | "sql"
                | "lua"
                | "kt"
        ),
        other => ext == other,
    }
}

fn within(t: Option<SystemTime>, window: u8, now: DateTime<Local>) -> bool {
    let Some(t) = t else { return false };
    let d: DateTime<Local> = t.into();
    let days = (now.date_naive() - d.date_naive()).num_days();
    match window {
        0 => days == 0,
        1 => (0..7).contains(&days),
        2 => (0..30).contains(&days),
        3 => (0..365).contains(&days),
        _ => false,
    }
}

impl Pred {
    fn matches(&self, e: &Entry, now: DateTime<Local>) -> bool {
        match self {
            Pred::Ext(list) => list.iter().any(|x| x == &e.ext),
            Pred::Folder(want) => e.is_dir == *want,
            Pred::Size(cmp) => {
                let n = e.size;
                match *cmp {
                    SizeCmp::Eq(v) => n == v,
                    SizeCmp::Gt(v) => n > v,
                    SizeCmp::Ge(v) => n >= v,
                    SizeCmp::Lt(v) => n < v,
                    SizeCmp::Le(v) => n <= v,
                    SizeCmp::Range(a, b) => (a..=b).contains(&n),
                }
            }
            Pred::Modified(w) => within(e.modified, *w, now),
            Pred::Kind(k) => class_matches(k, e),
            Pred::Not(p) => !p.matches(&e.name_lower),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool, size: u64, ext: &str) -> Entry {
        Entry {
            name: name.to_string(),
            name_lower: name.to_lowercase(),
            path: PathBuf::from(name),
            is_dir,
            size,
            modified: Some(SystemTime::now()),
            created: None,
            hidden: false,
            ext: ext.to_string(),
            plain_dir: is_dir,
            time_label: crate::fsutil::format_time(Some(SystemTime::now())),
        }
    }

    #[test]
    fn parses_chips_and_terms() {
        let q = Query::parse("rapor ext:pdf,docx -taslak size:>1mb");
        assert_eq!(q.chips.len(), 3);
        assert_eq!(q.terms, vec!["rapor".to_string()]);
        assert!(q.matches(
            &entry("yillik rapor.pdf", false, 2 << 20, "pdf"),
            Local::now()
        ));
        assert!(!q.matches(&entry("yillik rapor.pdf", false, 100, "pdf"), Local::now()));
        assert!(!q.matches(
            &entry("yillik rapor.txt", false, 2 << 20, "txt"),
            Local::now()
        ));
        assert!(!q.matches(
            &entry("taslak rapor.pdf", false, 2 << 20, "pdf"),
            Local::now()
        ));
    }

    #[test]
    fn type_and_size_ranges() {
        let q = Query::parse("type:folder");
        assert!(q.matches(&entry("src", true, 0, ""), Local::now()));
        assert!(!q.matches(&entry("a.txt", false, 1, "txt"), Local::now()));
        let q = Query::parse("size:1kb..1mb");
        assert!(q.matches(&entry("a.bin", false, 4096, "bin"), Local::now()));
        assert!(!q.matches(&entry("a.bin", false, 2 << 20, "bin"), Local::now()));
    }

    #[test]
    fn invalid_tokens_stay_text() {
        let q = Query::parse("ext: !!!");
        assert!(q.chips.is_empty());
        assert_eq!(q.terms.len(), 2);
        assert!(q.matches(&entry("ext: !!!", false, 0, ""), Local::now()));
    }

    #[test]
    fn removes_one_chip() {
        let q = Query::parse("a ext:pdf b size:>1mb");
        assert_eq!(q.without_chip(0), "a b size:>1mb");
        assert_eq!(q.without_chip(1), "a b ext:pdf");
    }
}
