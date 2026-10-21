//! Windows Subsystem for Linux dağıtımları: `\\wsl.localhost\<dağıtım>` kökleri.
//!
//! Windows 11 Dosya Gezgini'nin "Linux" düğümü gibi davranır. Liste `wsl.exe` ile
//! sorgulanır ve 10 saniye önbelleklenir; böylece kenar çubuğu her karede süreç
//! başlatmaz. `wsl.exe` yoksa veya hiç dağıtım kurulu değilse boş döner.

/// Tek bir WSL dağıtımı.
#[derive(Clone)]
pub struct Distro {
    pub name: String,
    /// `\\wsl.localhost\<ad>` biçiminde UNC kökü.
    pub root: std::path::PathBuf,
}

/// Kurulu dağıtımlar (10 sn önbellek). Windows dışında her zaman boş.
pub fn distros() -> Vec<Distro> {
    #[cfg(not(windows))]
    {
        Vec::new()
    }
    #[cfg(windows)]
    {
        use std::sync::Mutex;
        use std::time::{Duration, Instant};

        static CACHE: Mutex<Option<(Instant, Vec<Distro>)>> = Mutex::new(None);
        let mut cache = CACHE.lock().unwrap();
        if let Some((at, list)) = cache.as_ref() {
            if at.elapsed() < Duration::from_secs(10) {
                return list.clone();
            }
        }
        let list = query();
        *cache = Some((Instant::now(), list.clone()));
        list
    }
}

#[cfg(windows)]
fn query() -> Vec<Distro> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Ok(out) = Command::new("wsl.exe")
        .args(["--list", "--quiet"])
        // Yeni WSL sürümleri bu değişkenle UTF-8 yazar; eskiler UTF-16LE üretir.
        .env("WSL_UTF8", "1")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = decode(&out.stdout);
    text.lines()
        .map(|l| l.trim_matches(|c: char| c == '\u{feff}' || c == '\0' || c.is_whitespace()))
        .filter(|l| !l.is_empty())
        .map(|name| Distro {
            name: name.to_string(),
            root: std::path::PathBuf::from(format!(r"\\wsl.localhost\{name}")),
        })
        .collect()
}

/// `wsl.exe` çıktısını çözer: NUL varsa UTF-16LE, yoksa UTF-8.
#[cfg(windows)]
fn decode(bytes: &[u8]) -> String {
    if bytes.contains(&0) {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}
