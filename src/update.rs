//! Otomatik güncelleme: GitHub'daki son sürümü denetler. Windows'ta exe'yi indirip
//! doğrular ve yerine koyar; yeni sürüm bir sonraki açılışta çalışır.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Her sürümle yayınlanan bilgi dosyası. API'ye gitmediği için istek sınırına takılmaz.
const LATEST: &str =
    "https://github.com/some-rao/starfile-explorer/releases/latest/download/latest.json";

/// Güncellemeyi kuran gizli süreç bu bayrakla yeni sürümü başlatır; yeni süreç
/// eskisinin çıkmasını bekleyip arka planda bekler.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

pub enum Found {
    /// Yeni exe yerine kondu
    Installed(String),
    /// Kurulamıyor (exe yazılamayan bir yerde ya da Windows dışı); sürüm ve sayfası
    Available(String, String),
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    page: String,
    windows: Option<Binary>,
}

#[derive(Deserialize)]
struct Binary {
    url: String,
    sha256: String,
}

/// "v0.5.1" > "0.5.0"; sayı olmayan etiketler yeni sayılmaz
fn newer(tag: &str, current: &str) -> bool {
    let parse = |s: &str| -> Option<Vec<u64>> {
        s.trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().ok())
            .collect()
    };
    match (parse(tag), parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Geliştirme derlemesi (target/ altında) kendini güncellemesin
fn is_dev_build(exe: &Path) -> bool {
    cfg!(debug_assertions)
        || exe
            .ancestors()
            .any(|a| a.file_name().is_some_and(|n| n == "target"))
}

/// Scoop kurduğu klasöre bu iki dosyayı koyar; sürümü kendisi takip eder
fn scoop_install(exe: &Path) -> bool {
    exe.parent()
        .is_some_and(|d| d.join("install.json").exists() && d.join("manifest.json").exists())
}

/// Flatpak ve Scoop kendi güncellemesini yapar
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("FLATPAK_ID").is_none()
            && !std::env::current_exe().is_ok_and(|e| scoop_install(&e))
    })
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .build()
}

/// Son sürümü denetler, Windows'ta kurar. Yeni sürüm yoksa `None`.
pub fn check() -> Result<Option<Found>, String> {
    let ua = format!("StarFile/{}", env!("CARGO_PKG_VERSION"));
    let mut body = Vec::new();
    agent()
        .get(LATEST)
        .set("User-Agent", &ua)
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(64 << 10)
        .read_to_end(&mut body)
        .map_err(|e| e.to_string())?;
    let m: Manifest = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    if !newer(&m.version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let available = Ok(Some(Found::Available(m.version.clone(), m.page.clone())));
    if !cfg!(windows) {
        return available;
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let Some(bin) = &m.windows else {
        return available;
    };
    if is_dev_build(&exe) {
        return available;
    }
    // Bu sürüm açılışta çöktüğü için geri alındı; kullanıcı isterse kendisi indirir
    if std::fs::read_to_string(with_ext(&exe, ".skip")).is_ok_and(|v| v.trim() == m.version) {
        return available;
    }
    match install(&exe, &bin.url, &bin.sha256, &ua) {
        Ok(()) => Ok(Some(Found::Installed(m.version))),
        // Program Files gibi yazılamayan bir klasör: kullanıcı kendisi indirir
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => available,
        Err(e) => Err(e.to_string()),
    }
}

fn with_ext(exe: &Path, ext: &str) -> PathBuf {
    let mut s = exe.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

/// Önceki güncellemeden kalan eski exe; onu çalıştıran süreç kapandıysa silinebilir
pub fn remove_old() {
    if let Ok(exe) = std::env::current_exe() {
        // Yeni sürüm açıldığını doğrulamadıysa eski exe geri dönüş için kalsın
        if !with_ext(&exe, ".pending").exists() {
            let _ = std::fs::remove_file(with_ext(&exe, ".old"));
            let _ = std::fs::remove_file(with_ext(&exe, ".bad"));
        }
    }
}

/// Kurulan sürüm bu kadar başlangıçta ilk karesini çizemezse eskisine dönülür
const MAX_FAILED_STARTS: u32 = 2;

/// Yeni sürüm her başlangıçta `.pending` sayacını artırır, `confirm_started` onu siler.
/// Sayaç sınırdaysa yeni sürüm açılışta çöküyordur: eski exe geri konur ve başlatılır.
pub fn startup_guard() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    if roll_back_if_failing(&exe, env!("CARGO_PKG_VERSION")) {
        let _ = std::process::Command::new(&exe)
            .args(std::env::args_os().skip(1))
            .spawn();
        std::process::exit(0);
    }
}

fn roll_back_if_failing(exe: &Path, version: &str) -> bool {
    let pending = with_ext(exe, ".pending");
    let Ok(text) = std::fs::read_to_string(&pending) else {
        return false;
    };
    let starts: u32 = text.trim().parse().unwrap_or(0);
    let old = with_ext(exe, ".old");
    if !old.exists() {
        let _ = std::fs::remove_file(&pending);
        return false;
    }
    if starts >= MAX_FAILED_STARTS {
        let bad = with_ext(exe, ".bad");
        let _ = std::fs::remove_file(&bad);
        if std::fs::rename(exe, &bad).is_ok() {
            if std::fs::rename(&old, exe).is_ok() {
                let _ = std::fs::write(with_ext(exe, ".skip"), version);
                let _ = std::fs::remove_file(&pending);
                return true;
            }
            let _ = std::fs::rename(&bad, exe);
        }
    }
    let _ = std::fs::write(&pending, (starts + 1).to_string());
    false
}

/// Yeni sürüm ilk karesini çizdi, geri dönüş gerekmez
pub fn confirm_started() {
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        if let Ok(exe) = std::env::current_exe() {
            if std::fs::remove_file(with_ext(&exe, ".pending")).is_ok() {
                remove_old();
            }
        }
    });
}

fn install(exe: &Path, url: &str, sha: &str, ua: &str) -> std::io::Result<()> {
    let new = with_ext(exe, ".new");
    let old = with_ext(exe, ".old");
    let resp = agent()
        .get(url)
        .set("User-Agent", ua)
        .call()
        .map_err(std::io::Error::other)?;
    let mut file = std::fs::File::create(&new)?;
    let mut hasher = Sha256::new();
    let mut reader = resp.into_reader().take(256 << 20);
    let mut buf = vec![0u8; 64 << 10];
    let res = (|| {
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
        }
        file.sync_all()?;
        drop(file);
        let got: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !got.eq_ignore_ascii_case(sha) {
            return Err(std::io::Error::other("sha256 eşleşmedi"));
        }
        // Çalışan exe silinemez ama yeniden adlandırılabilir
        let _ = std::fs::remove_file(&old);
        std::fs::rename(exe, &old)?;
        if let Err(e) = std::fs::rename(&new, exe) {
            let _ = std::fs::rename(&old, exe);
            return Err(e);
        }
        std::fs::write(with_ext(exe, ".pending"), "0")?;
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    res
}

/// Kurulan sürümü pencere göstermeden başlatır
pub fn launch_hidden() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe)
            .arg(AFTER_UPDATE_ARG)
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::{install, newer, roll_back_if_failing, with_ext};

    /// Ağ gerektirir: cargo test -- --ignored
    #[test]
    #[ignore]
    fn install_swaps_the_exe() {
        let url =
            "https://github.com/some-rao/starfile-explorer/releases/download/v0.5.0/StarFile.exe";
        let sha = "c04b3d39eb220c5958bfb0754977e5dd0891206f0808889b244fb3b01ddca3ee";
        let dir = std::env::temp_dir().join(format!("swift-update-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("StarFile.exe");
        std::fs::write(&exe, b"old").unwrap();

        assert!(install(&exe, url, &"0".repeat(64), "test").is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert!(!with_ext(&exe, ".new").exists());

        install(&exe, url, sha, "test").unwrap();
        assert_eq!(std::fs::read(with_ext(&exe, ".old")).unwrap(), b"old");
        assert_eq!(std::fs::metadata(&exe).unwrap().len(), 7398912);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rolls_back_after_two_failed_starts() {
        let dir = std::env::temp_dir().join(format!("swift-rollback-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("StarFile.exe");
        std::fs::write(&exe, b"new").unwrap();
        std::fs::write(with_ext(&exe, ".old"), b"old").unwrap();
        std::fs::write(with_ext(&exe, ".pending"), "0").unwrap();

        assert!(!roll_back_if_failing(&exe, "0.7.0"));
        assert!(!roll_back_if_failing(&exe, "0.7.0"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert!(roll_back_if_failing(&exe, "0.7.0"));
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        assert_eq!(std::fs::read(with_ext(&exe, ".bad")).unwrap(), b"new");
        assert_eq!(
            std::fs::read_to_string(with_ext(&exe, ".skip")).unwrap(),
            "0.7.0"
        );
        assert!(!with_ext(&exe, ".pending").exists());
        assert!(!roll_back_if_failing(&exe, "0.6.0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn version_order() {
        assert!(newer("v0.5.1", "0.5.0"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(!newer("v0.5.0", "0.5.0"));
        assert!(!newer("v0.4.9", "0.5.0"));
        assert!(!newer("nightly", "0.5.0"));
    }
}
