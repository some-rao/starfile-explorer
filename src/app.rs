//! Arayüz: Windows 11 Dosya Gezgini düzeni — sekmeler, gezinme çubuğu, komut çubuğu,
//! gezinti bölmesi (ağaç), Giriş sayfası, ayrıntılar/simge görünümleri, ayrıntılar bölmesi.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, DragAndDrop, Key, Layout, Modifiers, Pos2, Rect,
    RichText, Sense, Stroke, Vec2,
};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::fsutil::{self, Conflict, Entry, Job, JobKind, PreviewData, SortKey};
use crate::menu::{self, Cmd, Menu, Node, Quick};
use crate::newfile;
use crate::scheme;
use crate::shell::{self, ImageCache, Img, MenuResult};
use crate::tab::{normalize, Tab, THIS_PC};
use crate::win::{self, DriveInfo};

/// Görünüm seviyeleri: 0 = Ayrıntılar, diğerleri simge boyutu (mantıksal piksel)
const VIEW_LEVELS: [(u32, &str); 5] = [
    (0, "Ayrıntılar"),
    (32, "Küçük simgeler"),
    (48, "Orta simgeler"),
    (96, "Büyük simgeler"),
    (256, "Çok büyük simgeler"),
];

const ICONS: &str = "icons";
const FLUENT: &str = "fluent";

// ---------------------------------------------------------------- ayarlar

/// Bir klasörün kendi görünümü. `None` = "bu klasör bu ayarı değiştirmedi",
/// böylece yalnızca sıralamayı değiştiren bir klasör görünüm düzeyini dondurmaz.
#[derive(Serialize, Deserialize, Clone, Default)]
struct FolderView {
    view_level: Option<usize>,
    sort: Option<SortKey>,
    asc: Option<bool>,
    grouped: Option<bool>,
    show_hidden: Option<bool>,
    show_ext: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Config {
    bookmarks: Vec<PathBuf>,
    tabs: Vec<PathBuf>,
    active: usize,
    show_hidden: bool,
    /// Sağ yan bölme: 0 kapalı, 1 önizleme, 2 ayrıntılar (aynı anda yalnızca biri)
    pane: u8,
    show_ext: bool,
    sort: SortKey,
    asc: bool,
    view_level: usize,
    theme: u8,
    /// 0 = Windows diline göre, 1 = Türkçe, 2 = İngilizce
    lang: u8,
    recent: Vec<PathBuf>,
    tips_dismissed: bool,
    /// "Değiştirme tarihi"ne göre gruplanan klasörler (Explorer'da İndirilenler varsayılan olarak böyle)
    grouped: Vec<PathBuf>,
    /// Pencere kapanınca süreç gizlenip bekler, sonraki açılış anında olur (Windows)
    background: bool,
    auto_update: bool,
    /// Son güncelleme denetimi, Unix saniyesi. Uygulamayı sık açıp kapamak istek yağdırmasın.
    last_update_check: u64,
    /// Varsayılan dosya yöneticisi yapılmadan önceki uygulama (Linux'ta geri yüklemek için)
    previous_folder_app: Option<String>,
    /// Her klasör kendi görünümünü (düzen, sıralama, gruplama, sütun genişlikleri) hatırlasın
    remember_views: bool,
    /// Küçük harfli yol -> klasör görünümü
    folder_views: HashMap<String, FolderView>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            bookmarks: vec![],
            tabs: vec![],
            active: 0,
            show_hidden: false,
            pane: 1,
            show_ext: true,
            sort: SortKey::Name,
            asc: true,
            view_level: 0,
            theme: 0,
            lang: 0,
            recent: vec![],
            tips_dismissed: false,
            grouped: dirs::download_dir().into_iter().collect(),
            background: true,
            auto_update: true,
            last_update_check: 0,
            previous_folder_app: None,
            remember_views: true,
            folder_views: HashMap::new(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("SwiftExplorer").join("config.json"))
}

fn load_config() -> Config {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| legacy_config_path().and_then(|p| std::fs::read_to_string(p).ok()))
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

const UPDATE_INTERVAL: u64 = 3600;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Son denetimden bu yana bir saat geçtiyse yeni sürümü denetler: açılıştan biraz sonra,
/// saatte bir ve dönen kanala sinyal gelince, yani pencere yeniden açılınca. Pencere gizliyken
/// de çalışır: yeni sürüm kurulunca gizli süreç onu arka planda başlatıp çıkar. `enabled` yoksa denetlemez.
fn spawn_update_checker(
    enabled: Option<Arc<AtomicBool>>,
    last_check: Arc<AtomicU64>,
    ctx: egui::Context,
) -> (
    std::sync::mpsc::Receiver<crate::update::Found>,
    std::sync::mpsc::Sender<()>,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel::<()>();
    let Some(enabled) = enabled.filter(|_| crate::update::enabled()) else {
        return (rx, wake_tx);
    };
    std::thread::spawn(move || {
        crate::update::remove_old();
        // Açılışla yarışmasın
        std::thread::sleep(Duration::from_secs(15));
        let mut told = None;
        loop {
            let now = unix_now();
            let last = last_check.load(Relaxed);
            // Saat geri alındıysa da denetle
            let due = now < last || now - last >= UPDATE_INTERVAL;
            if enabled.load(Relaxed) && due {
                last_check.store(now, Relaxed);
                match crate::update::check() {
                    Ok(Some(crate::update::Found::Installed(v))) => {
                        if !fsutil::jobs_running() && win::take_parked() {
                            crate::update::launch_hidden();
                            std::process::exit(0);
                        }
                        let _ = tx.send(crate::update::Found::Installed(v));
                        ctx.request_repaint();
                        return;
                    }
                    Ok(Some(crate::update::Found::Available(v, url))) => {
                        if told.as_ref() != Some(&v) {
                            told = Some(v.clone());
                            let _ = tx.send(crate::update::Found::Available(v, url));
                            ctx.request_repaint();
                        }
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("güncelleme denetlenemedi: {e}"),
                }
            }
            // Bütün kopyalar aynı dakikada sormasın diye 10 dakikaya kadar rastgele kaydır
            use std::hash::BuildHasher;
            let jitter = std::collections::hash_map::RandomState::new().hash_one(now) % 600;
            let next = if enabled.load(Relaxed) {
                (last_check.load(Relaxed) + UPDATE_INTERVAL)
                    .saturating_sub(unix_now())
                    .min(UPDATE_INTERVAL)
            } else {
                UPDATE_INTERVAL
            };
            if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) =
                wake_rx.recv_timeout(Duration::from_secs(next + jitter))
            {
                return;
            }
        }
    });
    (rx, wake_tx)
}

/// Açılacak sekmeler ve etkin olanın sırası
fn startup_tabs(cfg: &Config, start: Option<PathBuf>) -> (Vec<PathBuf>, usize) {
    // Bir klasörle başlatıldıysa (ör. "Birlikte aç", Ctrl+N) yalnızca onu aç
    if let Some(s) = start.filter(|p| crate::tab::is_virtual(p) || p.is_dir()) {
        return (vec![s], 0);
    }
    let paths: Vec<PathBuf> = cfg
        .tabs
        .iter()
        .filter(|p| crate::tab::is_virtual(p) || p.is_dir())
        .cloned()
        .collect();
    if paths.is_empty() {
        return (vec![PathBuf::new()], 0); // Giriş
    }
    let active = cfg.active.min(paths.len() - 1);
    (paths, active)
}

/// Pencere ve GPU sürücüsü hazırlanırken (~200 ms) ilk ekranı hazırlar: açılış klasörünü
/// okur, ikon iş parçacıklarını başlatır ve ilk görünecek öğelerin ikon/küçük resimlerini
/// arayüzün isteyeceği boyut ve türde önceden üretir.
pub fn prefetch_startup(start: Option<PathBuf>) {
    crate::images::start();
    let cfg = load_config();
    let (paths, active) = startup_tabs(&cfg, start);
    let dir = paths
        .get(active)
        .filter(|p| !crate::tab::is_virtual(p))
        .map(|p| normalize(p));
    if let Some(d) = &dir {
        fsutil::prefetch(d.clone());
    }
    let mut level = cfg.view_level.min(VIEW_LEVELS.len() - 1);
    let mut grouped = dir.as_ref().is_some_and(|d| cfg.grouped.contains(d));
    let (mut sort, mut asc, mut show_hidden) = (cfg.sort, cfg.asc, cfg.show_hidden);
    // Klasörün kendi görünümü varsa ilk kare de onu kullanır (App::sync_folder_view
    // ile aynı kurallar); yoksa ilk karede yanlış düzen parlaması olurdu.
    if cfg.remember_views {
        if let Some(fv) = dir
            .as_ref()
            .and_then(|d| App::folder_key(d))
            .and_then(|k| cfg.folder_views.get(&k))
        {
            if let Some(v) = fv.view_level {
                level = v.min(VIEW_LEVELS.len() - 1);
            }
            if let Some(v) = fv.sort {
                sort = v;
            }
            if let Some(v) = fv.asc {
                asc = v;
            }
            if let Some(v) = fv.show_hidden {
                show_hidden = v;
            }
            if let Some(v) = fv.grouped {
                grouped = v;
            }
        }
    }
    std::thread::spawn(move || {
        let scale = shell::system_scale();
        let px = |logical: f32| px_bucket(logical * scale);
        let mut reqs = Vec::new();
        // İlk ekrandaki öğeler, arayüzdeki sırayla (rebuild_view ile aynı kurallar)
        if let Some(dir) = &dir {
            let mut entries: Vec<Entry> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|de| {
                    de.metadata()
                        .ok()
                        .map(|md| Entry::from_metadata(de.path(), &md))
                })
                .filter(|e| show_hidden || !e.hidden)
                .collect();
            entries.sort_by(|a, b| fsutil::compare(a, b, sort, asc));
            if grouped {
                let now = chrono::Local::now();
                entries.sort_by_key(|e| fsutil::date_group(e.modified, now));
            }
            let mut seen = std::collections::HashSet::new();
            for e in entries.iter().take(60) {
                let (p, icon_only) = if level == 0 {
                    (px(18.0), true)
                } else {
                    (
                        px(VIEW_LEVELS[level].0 as f32),
                        e.is_dir || !fsutil::has_thumbnail(&e.ext),
                    )
                };
                let key = if icon_only {
                    icon_key(&e.path, e.is_dir, &e.ext, p, e.plain_dir)
                } else {
                    format!("t{p}:{}", e.path.display())
                };
                if seen.insert(key) {
                    reqs.push((e.path.clone(), p, icon_only));
                }
            }
        }
        // Gezinti bölmesi ve sekme şeridi
        let small = px(16.0);
        for p in known_folders()
            .into_iter()
            .map(|(_, _, p)| p)
            .chain(paths.iter().map(|p| App::loc_icon(p)))
        {
            reqs.push((p, small, true));
        }
        crate::images::prefetch(reqs);
    });
}

/// Uygulamanın eski adıyla (Hızlı Gezgin) kaydedilmiş ayarlar; yeni dosya yoksa bunlar okunur
fn legacy_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("HizliGezgin").join("config.json"))
}

// ---------------------------------------------------------------- eylemler/diyaloglar

#[derive(Clone)]
struct DragPaths(Vec<PathBuf>);

enum Action {
    Navigate(PathBuf),
    OpenEntry(PathBuf, bool),
    OpenSelected,
    Up,
    Back,
    Forward,
    Refresh,
    NewTab(PathBuf),
    NewWindow,
    CloseTab(usize),
    Copy,
    Cut,
    Paste,
    PasteInto(PathBuf),
    /// "Şuraya kopyala…"/"Şuraya taşı…": hedef klasör seçildikten sonra transfer başlar
    PickDest {
        paths: Vec<PathBuf>,
        cut: bool,
    },
    Delete {
        permanent: bool,
    },
    Rename,
    NewFolder,
    /// newfile::KINDS dizininden öğe oluştur (0 klasör, 1 metin belgesi, diğerleri belgeler)
    New(usize),
    SelectAll,
    SelectNone,
    InvertSelection,
    CopyPath,
    Terminal(PathBuf),
    RunTerminal {
        name: String,
        args: String,
        dir: PathBuf,
    },
    Properties(PathBuf),
    OpenWith(PathBuf),
    ShellMenu {
        extended: bool,
    },
    /// Modern (Win11 tarzı) sağ tık menüsünü aç
    OpenMenu {
        target: MenuTarget,
        pos: Pos2,
    },
    /// Gerçek yerel menüyü aç (Shift+F10 / "Daha fazla seçenek")
    NativeMenu {
        paths: Vec<PathBuf>,
        background: Option<PathBuf>,
        extended: bool,
    },
    CalcSize,
    AddBookmark(PathBuf),
    RemoveBookmark(PathBuf),
    StartSearch,
    ExitSearch,
    Sort(SortKey),
    SetView(usize),
    DropPaths {
        paths: Vec<PathBuf>,
        target: PathBuf,
        ctrl: bool,
        shift: bool,
    },
    Undo,
    // Komut çubuğu Görünüm menüsü (Node menü makinesiyle çalışır)
    TogglePreview,
    ToggleDetails,
    // Boşluk ile PowerToys Peek önizleme
    Peek,
    ToggleHidden,
    ToggleExt,
    ToggleRememberViews,
    ForgetFolderView,
    ToggleDefaultFm,
    ToggleBackground,
    ToggleAutostart,
    ToggleAutoUpdate,
    SetTheme(u8),
    SetLanguage(u8),
}

enum Dialog {
    None,
    ConfirmDelete {
        paths: Vec<PathBuf>,
    },
    Conflict {
        files: Vec<PathBuf>,
        dest: PathBuf,
        cut: bool,
        names: Vec<String>,
    },
    Help,
    #[cfg(not(windows))]
    Properties(Box<crate::properties::Properties>),
}

enum UndoOp {
    Rename { from: PathBuf, to: PathBuf },
    Created(PathBuf),
    Moved(Vec<(PathBuf, PathBuf)>),
    Copied(Vec<PathBuf>),
    Trashed(Vec<PathBuf>),
}

/// Sağ tık menüsünün hedefi: seçili öğeler ya da klasörün boş alanı.
#[derive(Clone)]
enum MenuTarget {
    Items {
        paths: Vec<PathBuf>,
        is_dir: bool,
        in_search: bool,
        primary: PathBuf,
    },
    Background {
        folder: PathBuf,
        in_search: bool,
    },
}

/// Açık modern sağ tık menüsü ve ürettiği uygulama komutları.
struct MenuState {
    menu: Menu,
    actions: Vec<Action>,
    target: MenuTarget,
}

struct Renaming {
    path: PathBuf,
    text: String,
    first: bool,
}

struct Preview {
    path: Option<PathBuf>,
    data: Arc<Mutex<Option<(PathBuf, PreviewData)>>>,
}

pub struct App {
    tabs: Vec<Tab>,
    active: usize,
    sort: SortKey,
    asc: bool,
    show_hidden: bool,
    pane: u8,
    show_ext: bool,
    view_level: usize,
    theme: u8,
    lang: u8,
    bookmarks: Vec<PathBuf>,
    recent: Vec<PathBuf>,
    tips_dismissed: bool,
    grouped: Vec<PathBuf>,
    /// Her klasörün kendi görünümü (bkz. `FolderView`)
    remember_views: bool,
    folder_views: HashMap<String, FolderView>,
    /// Görünümü en son uygulanan klasörün anahtarı; sekme/klasör değişimini yakalar
    view_key: Option<String>,
    background: bool,
    /// Sonraki açılışları bu süreç karşılıyor; kapanınca gizlenip bekleyebilir
    resident: bool,
    autostart: bool,
    previous_folder_app: Option<String>,
    /// Varsayılan dosya yöneticisi mi; menü ilk açılınca sorulur (Linux'ta bir süreç çalıştırır)
    default_fm: Option<bool>,
    /// Arka plandaki denetleyici okur; kapatılınca bir sonraki denetim atlanır
    auto_update: Arc<AtomicBool>,
    update_rx: std::sync::mpsc::Receiver<crate::update::Found>,
    /// Güncelleme denetleyicisini uyandırır
    update_wake: std::sync::mpsc::Sender<()>,
    last_update_check: Arc<AtomicU64>,
    /// Yeni sürüm kuruldu: pencere kapanınca gizlenmek yerine yeni sürüm başlatılır
    update_installed: bool,
    /// Oturum açılışında başladı: pencere hiç gösterilmeden beklemeye geçer
    #[cfg_attr(not(windows), allow(dead_code))]
    start_hidden: bool,
    /// Bilinen klasörler (Masaüstü, İndirilenler…) — açılışta bir kez
    known: Vec<(&'static str, &'static str, PathBuf)>,
    /// Gezinti bölmesinde açık düğümler
    tree_open: std::collections::HashSet<PathBuf>,
    drives: Arc<Mutex<Vec<DriveInfo>>>,
    drives_refreshed: Instant,
    clipboard: Option<(Vec<PathBuf>, bool)>,
    jobs: Vec<Arc<Job>>,
    dir_sizes: Arc<Mutex<HashMap<PathBuf, u64>>>,
    dialog: Dialog,
    address_edit: Option<String>,
    address_focus: bool,
    focus_search: bool,
    type_buf: String,
    type_time: Instant,
    toasts: Vec<(String, Instant, bool)>,
    preview: Preview,
    ctrl_v_prev: bool,
    last_title: String,
    images: ImageCache,
    type_names: HashMap<String, String>,
    hwnd: Option<isize>,
    renaming: Option<Renaming>,
    undo: Vec<UndoOp>,
    /// Alt klasör listeleri (gezinti ağacı, adres çubuğu). Arka planda okunur; paylaşımlı
    /// tutulur ki her karede kopyalanmasın.
    subdirs: HashMap<PathBuf, Arc<[PathBuf]>>,
    subdirs_pending: std::collections::HashSet<PathBuf>,
    subdirs_tx: std::sync::mpsc::Sender<(PathBuf, bool, Vec<PathBuf>)>,
    subdirs_rx: std::sync::mpsc::Receiver<(PathBuf, bool, Vec<PathBuf>)>,
    grid_cols: usize,
    inject_release: Option<Pos2>,
    accent: Color32,
    zoom_acc: f32,
    marquee: Option<Pos2>,
    item_rects: Vec<(usize, Rect)>,
    grid_scroll: (f32, f32),
    silent_jobs: Vec<usize>,
    /// Açık modern sağ tık menüsü
    menu: Option<MenuState>,
    saved_config: String,
    last_autosave: Instant,
    /// Açılış: pencere ilk içerik (liste + ikonlar) hazır olana dek gizli
    #[cfg_attr(not(windows), allow(dead_code))]
    started: Instant,
    frames: u32,
    revealed: bool,
    /// Açılışta eframe'in geri yüklediği pencere geometrisi denetlendi mi (bkz. `repair_restored_window`)
    window_checked: bool,
    /// Pencere simge durumunda mı (bkz. `App::save`)
    minimized: bool,
    /// Simge durumunda kaydedilmesin diye saklanan son geçerli eframe pencere durumu
    good_window: Option<String>,
    ctx: egui::Context,
}

// ---------------------------------------------------------------- tema

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // Explorer'ın kullandığı arayüz yazı tipi: Windows 11'de Segoe UI Variable (varsayılan
    // örnek "Regular Text"), Windows 10'da Segoe UI. egui'nin kendi Ubuntu-Light'ı aile
    // listesinden çıkarılır; aksi halde Segoe'da olmayan glifler daha ince bir yazı tipiyle
    // çizilip aynı satırda farklı kalınlıklar oluşur.
    let mut text_fonts: Vec<String> = Vec::new();
    let mut symbol_fonts: Vec<String> = Vec::new();
    #[cfg(not(windows))]
    if let Some(bytes) = linux_ui_font() {
        fonts.font_data.insert(
            "system_ui".into(),
            Arc::new(egui::FontData::from_owned(bytes)),
        );
        text_fonts.push("system_ui".into());
    }
    #[cfg(windows)]
    {
        let ui = ["SegUIVar.ttf", "segoeui.ttf"]
            .into_iter()
            .find_map(|file| std::fs::read(format!("C:\\Windows\\Fonts\\{file}")).ok());
        if let Some(bytes) = ui {
            fonts.font_data.insert(
                "system_ui".into(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            text_fonts.push("system_ui".into());
        }
        // Simge ve sembol glifleri için yedek
        if let Ok(bytes) = std::fs::read("C:\\Windows\\Fonts\\seguisym.ttf") {
            fonts.font_data.insert(
                "segoe_sym".into(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            symbol_fonts.push("segoe_sym".into());
        }
    }
    if text_fonts.is_empty() {
        // Sistem yazı tipi okunamadıysa egui'nin varsayılanına düş
        text_fonts.push("Ubuntu-Light".into());
    }
    let mut proportional = text_fonts;
    proportional.push("NotoEmoji-Regular".into());
    proportional.push("emoji-icon-font".into());
    proportional.extend(symbol_fonts);
    fonts
        .families
        .insert(egui::FontFamily::Proportional, proportional);
    // İkonlar: önce egui'nin emoji/ikon fontları, böylece Segoe UI'ın tek renkli glifleri araya girmez
    let mut icons: Vec<String> = vec!["emoji-icon-font".into(), "NotoEmoji-Regular".into()];
    icons.extend(
        fonts.families[&egui::FontFamily::Proportional]
            .iter()
            .cloned(),
    );
    // Explorer'ın kendi glifleri: Windows 11'de Segoe Fluent Icons, Windows 10'da Segoe MDL2 Assets
    let mut fluent = icons.clone();
    #[cfg(not(windows))]
    {
        fonts.font_data.insert(
            FLUENT.into(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/fluent-icons.ttf"
            ))),
        );
        fluent.insert(0, FLUENT.into());
    }
    #[cfg(windows)]
    for file in ["SegoeIcons.ttf", "segmdl2.ttf"] {
        if let Ok(bytes) = std::fs::read(format!("C:\\Windows\\Fonts\\{file}")) {
            fonts
                .font_data
                .insert(FLUENT.into(), Arc::new(egui::FontData::from_owned(bytes)));
            fluent.insert(0, FLUENT.into());
            break;
        }
    }
    fonts
        .families
        .insert(egui::FontFamily::Name(ICONS.into()), icons);
    fonts
        .families
        .insert(egui::FontFamily::Name(FLUENT.into()), fluent);
    ctx.set_fonts(fonts);
}

/// Çince, Japonca ve Korece glifler: dosya adları ve arayüz kutucuk yerine yazıyla görünsün diye
/// sistemin CJK fontları yedek olarak eklenir. Dosyalar 5-20 MB olduğundan arka planda okunur.
fn load_cjk_fonts(ctx: &egui::Context) {
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for (name, data) in cjk_fonts() {
            let families = [
                egui::FontFamily::Proportional,
                egui::FontFamily::Name(ICONS.into()),
                egui::FontFamily::Name(FLUENT.into()),
            ]
            .into_iter()
            .map(|family| egui::epaint::text::InsertFontFamily {
                family,
                priority: egui::epaint::text::FontPriority::Lowest,
            })
            .collect();
            ctx.add_font(egui::epaint::text::FontInsert::new(name, data, families));
        }
        ctx.request_repaint();
    });
}

/// Microsoft YaHei (Çince ve kana), Hangul için Malgun Gothic. Japonca arayüzde Japon glif
/// biçimleri için önce Yu Gothic; YaHei yine de kalır, çünkü Yu Gothic'te Basitleştirilmiş Çince yok.
#[cfg(windows)]
fn cjk_fonts() -> Vec<(&'static str, egui::FontData)> {
    let read = |file: &str| {
        std::fs::read(format!("C:\\Windows\\Fonts\\{file}"))
            .ok()
            .map(egui::FontData::from_owned)
    };
    // Yu Gothic'in glifleri Segoe UI'ın taban çizgisinin üstünde kalıyor; biraz aşağı alınır
    let ja = (crate::i18n::lang_code() == "ja")
        .then(|| read("YuGothM.ttc"))
        .flatten()
        .map(|d| {
            d.tweak(egui::FontTweak {
                y_offset_factor: 0.3,
                ..Default::default()
            })
        });
    [
        ("cjk_ja", ja),
        ("cjk", read("msyh.ttc")),
        ("hangul", read("malgun.ttf")),
    ]
    .into_iter()
    .filter_map(|(n, d)| Some((n, d?)))
    .collect()
}

/// fontconfig'in Çince (Japonca arayüzde Japonca) için seçtiği font; Noto Sans CJK Korece'yi de kapsar
#[cfg(not(windows))]
fn cjk_fonts() -> Vec<(&'static str, egui::FontData)> {
    let lang = if crate::i18n::lang_code() == "ja" {
        "ja"
    } else {
        "zh-cn"
    };
    let font = || {
        let out = std::process::Command::new("fc-match")
            .args([
                "-f",
                "%{file}\n%{index}",
                &format!("sans-serif:lang={lang}"),
            ])
            .output()
            .ok()?;
        let out = String::from_utf8(out.stdout).ok()?;
        let (path, index) = out.split_once('\n')?;
        let mut data = egui::FontData::from_owned(std::fs::read(path.trim()).ok()?);
        data.index = index.trim().parse().unwrap_or(0);
        Some(data)
    };
    font().map(|d| vec![("cjk", d)]).unwrap_or_default()
}

/// Masaüstünün arayüz fontu: GNOME'un ayarı (ör. Adwaita Sans), yoksa fontconfig'in "sans-serif" seçimi
#[cfg(not(windows))]
fn linux_ui_font() -> Option<Vec<u8>> {
    gnome_font_family()
        .and_then(|f| font_file(&f))
        .or_else(|| font_file("sans-serif"))
}

/// `gsettings` çıktısı "'Adwaita Sans 11'" gibidir: tırnaklar ve sondaki boyut atılır
#[cfg(not(windows))]
fn gnome_font_family() -> Option<String> {
    let out = crate::win::host_command("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim().trim_matches('\'');
    let family = s
        .rsplit_once(' ')
        .filter(|(_, size)| size.parse::<f32>().is_ok())
        .map_or(s, |(f, _)| f);
    (!family.is_empty()).then(|| family.to_string())
}

#[cfg(not(windows))]
fn font_file(pattern: &str) -> Option<Vec<u8>> {
    let out = std::process::Command::new("fc-match")
        .args(["-f", "%{file}", pattern])
        .output()
        .ok()?;
    let path = String::from_utf8(out.stdout).ok()?;
    let path = path.trim();
    (path.ends_with(".ttf") || path.ends_with(".otf"))
        .then(|| std::fs::read(path).ok())
        .flatten()
}

/// Explorer'dan ölçülen renkler (Windows 11, koyu/açık)
#[derive(Clone, Copy)]
struct Pal {
    /// Sekme şeridi (başlık çubuğu)
    strip: Color32,
    /// Etkin sekme + gezinme satırı
    nav: Color32,
    /// Adres ve arama kutusu
    field: Color32,
    field_stroke: Stroke,
    /// Satırları ayıran yatay çizgiler
    line: Color32,
    /// Komut çubuğu, gezinti bölmesi, içerik
    content: Color32,
    status: Color32,
    hover: Color32,
    /// Gezinti bölmesinde seçili satır
    nav_sel: Color32,
    /// İçerikte seçili öğe
    sel: Color32,
    sel_hover: Color32,
    text: Color32,
    weak: Color32,
    disabled: Color32,
}

fn pal(dark: bool) -> Pal {
    if let Some(c) = scheme::active().filter(|c| c.dark == dark) {
        let sel = if dark { 0.28 } else { 0.22 };
        return Pal {
            strip: c.crust,
            nav: c.mantle,
            field: c.surface,
            field_stroke: if dark {
                Stroke::NONE
            } else {
                Stroke::new(1.0_f32, c.border)
            },
            line: c.border,
            content: c.base,
            status: c.base,
            hover: scheme::mix(c.base, c.surface, 0.7),
            nav_sel: scheme::mix(c.surface, c.border, 0.5),
            sel: scheme::mix(c.base, c.accent, sel),
            sel_hover: scheme::mix(c.base, c.accent, sel + 0.08),
            text: c.text,
            weak: c.weak,
            disabled: c.disabled,
        };
    }
    if dark {
        Pal {
            strip: Color32::from_rgb(10, 10, 10),
            nav: Color32::from_rgb(31, 31, 31),
            field: Color32::from_rgb(44, 44, 44),
            field_stroke: Stroke::NONE,
            line: Color32::from_rgb(58, 58, 58),
            content: Color32::from_rgb(25, 25, 25),
            status: Color32::from_rgb(28, 28, 28),
            hover: Color32::from_rgb(45, 45, 45),
            nav_sel: Color32::from_rgb(51, 51, 51),
            sel: Color32::from_rgb(65, 65, 65),
            sel_hover: Color32::from_rgb(77, 77, 77),
            text: Color32::from_rgb(255, 255, 255),
            weak: Color32::from_rgb(200, 200, 200),
            disabled: Color32::from_rgb(109, 109, 109),
        }
    } else {
        Pal {
            strip: Color32::from_rgb(232, 232, 232),
            nav: Color32::from_rgb(249, 249, 249),
            field: Color32::from_rgb(255, 255, 255),
            field_stroke: Stroke::new(1.0_f32, Color32::from_rgb(229, 229, 229)),
            line: Color32::from_rgb(229, 229, 229),
            content: Color32::from_rgb(255, 255, 255),
            status: Color32::from_rgb(255, 255, 255),
            hover: Color32::from_rgb(240, 240, 240),
            nav_sel: Color32::from_rgb(234, 234, 234),
            sel: Color32::from_rgb(204, 232, 255),
            sel_hover: Color32::from_rgb(190, 225, 255),
            // Explorer dosya adlarını tam siyah çizer; egui ClearType kullanmadığı için
            // 26,26,26 gri tonu ince metinde soluk kalıyordu
            text: Color32::BLACK,
            weak: Color32::from_rgb(96, 96, 96),
            disabled: Color32::from_rgb(160, 160, 160),
        }
    }
}

fn pal_of(ui: &egui::Ui) -> Pal {
    pal(ui.visuals().dark_mode)
}

/// Dil ayarını uygular (0 = Windows diline göre, n = `i18n::LANGS` içindeki n-1. dil)
fn apply_lang(lang: u8) {
    // SWIFT_EXPLORER_LANG=en|tr|de|… ayarı geçersiz kılar (test ve kısayol için)
    let forced = std::env::var("SWIFT_EXPLORER_LANG")
        .ok()
        .and_then(|c| crate::i18n::lang_index(&c));
    crate::i18n::set_lang(match (forced, lang) {
        (Some(i), _) => i,
        (None, 0) => crate::i18n::system_lang(),
        (None, n) => n as usize - 1,
    });
}

/// Şemanın vurgu rengi, yoksa Windows'unki
fn theme_accent(theme: u8) -> Color32 {
    scheme::get(theme)
        .map(|c| c.accent)
        .or_else(shell::accent_color)
        .unwrap_or(Color32::from_rgb(0, 120, 212))
}

/// Geçici ölçüm: SWIFT_EXPLORER_WHEELLOG=1 verildiğinde gelen kaydırma
/// olaylarını (birim, değer, zaman) %TEMP%\swift-wheel-log.txt dosyasına
/// yazar. Dokunmatik hareketin hangi birimle ve ne boyutta geldiğini
/// görmek için; temas sonunda kaldırılacak.
fn wheel_log(ctx: &egui::Context) {
    use std::io::Write as _;
    use std::sync::OnceLock;
    use std::time::Instant;
    static T0: OnceLock<Instant> = OnceLock::new();

    let events: Vec<(egui::MouseWheelUnit, egui::Vec2, bool)> = ctx.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => {
                    Some((*unit, *delta, delta.y.fract() != 0.0))
                }
                _ => None,
            })
            .collect()
    });
    if events.is_empty() {
        return;
    }
    let t0 = T0.get_or_init(Instant::now);
    let path = std::env::temp_dir().join("swift-wheel-log.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        for (unit, delta, fract) in events {
            let _ = writeln!(
                f,
                "{:>6} ms {:?} dx={:>8.3} dy={:>8.3} fract={}",
                t0.elapsed().as_millis(),
                unit,
                delta.x,
                delta.y,
                fract
            );
        }
    }
}

/// Geçici ölçüm: kare aralığı, arayüz kurulum süresi ve ikon kuyruğu
/// (SWIFT_EXPLORER_WHEELLOG=1). Kaydırma takıntısının çerçeve maliyetinden
/// mi (dosya/ikon yükleme, disk yazımı) yoksa giriş kararkolundan mı
/// geldiğini ayırt etmek için; tema sonunda kaldırılacak.
fn perf_log(
    ctx: &egui::Context,
    images: &ImageCache,
    pre_ms: f32,
    panels_ms: f32,
    center_ms: f32,
    post_ms: f32,
) {
    use std::sync::Mutex;
    use std::time::Instant;
    static PREV: Mutex<Option<Instant>> = Mutex::new(None);
    static NEXT: Mutex<Option<Instant>> = Mutex::new(None);

    let now = Instant::now();
    let gap_ms = PREV
        .lock()
        .unwrap()
        .replace(now)
        .map(|p| now.duration_since(p).as_secs_f64() * 1000.0);
    let wheel = ctx.input(|i| i.raw_scroll_delta != egui::Vec2::ZERO);
    let icon_q = images.pending();
    let odd = gap_ms.map(|g| g > 24.0).unwrap_or(false);
    let write = {
        let mut next = NEXT.lock().unwrap();
        let due = next
            .map(|t| now.duration_since(t).as_millis() >= 100)
            .unwrap_or(true);
        if due {
            *next = Some(now);
        }
        due && (wheel || odd || icon_q > 0)
    };
    if !write {
        return;
    }
    let path = std::env::temp_dir().join("swift-wheel-log.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write as _;
        let _ = match gap_ms {
            Some(g) => writeln!(
                f,
                "PERF gap={g:>6.1} pre={pre_ms:>5.1} pan={panels_ms:>5.1} ctr={center_ms:>5.1} post={post_ms:>5.1} wheel={wheel} iconq={icon_q}"
            ),
            None => writeln!(
                f,
                "PERF gap=  --- pre={pre_ms:>5.1} pan={panels_ms:>5.1} ctr={center_ms:>5.1} post={post_ms:>5.1} wheel={wheel} iconq={icon_q}"
            ),
        };
    }
}

fn apply_theme(ctx: &egui::Context, accent: Color32, theme: u8) {
    scheme::set_active(theme);
    ctx.options_mut(|o| {
        o.theme_preference = match (scheme::get(theme), theme) {
            (Some(c), _) if c.dark => egui::ThemePreference::Dark,
            (Some(_), _) | (None, 1) => egui::ThemePreference::Light,
            (None, 2) => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        };
        // Fare tekerleği bir diş döndüğünde Windows'un "kaç satır kaydırılsın"
        // ayarı kadar ilerlesin (Explorer'daki gibi). egui'nin varsayılanı çark
        // dişi başına yalnızca ~1 satırdır (40 px); 28 pt bir liste satırıdır.
        o.line_scroll_speed = 28.0 * text_factor() * crate::system::wheel_scroll_lines() as f32;
        // Dokunmatik kaydırma (kaydırma çeteceli): sürücü kesirli wheel-line
        // bildirir; 1 "line" ≈ bir satır. Parmağın hızına birebir izler,
        // kalkınca momentum ile süzülür (vendored egui'deki momentum yaması).
        o.touchpad_line_scroll_speed = 28.0 * text_factor();
    });
    for t in [egui::Theme::Dark, egui::Theme::Light] {
        let dark = t == egui::Theme::Dark;
        ctx.style_mut_of(t, |s| {
            s.interaction.selectable_labels = false;
            s.interaction.tooltip_delay = 0.45;
            // Windows'ta "Animasyonlar" kapalıysa anında tepki ver; açıksa yumuşak geçiş
            let anim = crate::system::animations();
            s.animation_time = if anim { 0.12 } else { 0.0 };
            s.scroll_animation = if anim {
                egui::style::ScrollAnimation::duration(0.18)
            } else {
                egui::style::ScrollAnimation::none()
            };
            s.spacing.item_spacing = egui::vec2(6.0, 4.0);
            s.spacing.button_padding = egui::vec2(8.0, 4.0);
            s.spacing.interact_size.y = (26.0 * text_factor()).round();
            s.spacing.menu_margin = egui::Margin::same(6);
            for (style, size) in [
                (egui::TextStyle::Body, 14.0),
                (egui::TextStyle::Button, 14.0),
                (egui::TextStyle::Small, 12.0),
                (egui::TextStyle::Heading, 20.0),
                (egui::TextStyle::Monospace, 13.0),
            ] {
                if let Some(f) = s.text_styles.get_mut(&style) {
                    f.size = text_size(size);
                }
            }
            let v = &mut s.visuals;
            let c = Chrome::of(dark);
            v.panel_fill = c.panel;
            v.window_fill = c.popup;
            v.extreme_bg_color = c.extreme;
            v.faint_bg_color = c.faint;
            v.code_bg_color = c.window;
            v.override_text_color = None;
            v.hyperlink_color = accent;
            let p = pal(dark);
            v.selection.bg_fill = p.sel;
            v.selection.stroke = Stroke::new(1.0_f32, c.text);
            v.window_corner_radius = CornerRadius::same(8);
            v.menu_corner_radius = CornerRadius::same(8);
            v.window_stroke = Stroke::new(1.0_f32, c.popup_stroke);
            v.striped = false;
            let w = &mut v.widgets;
            for wv in [
                &mut w.noninteractive,
                &mut w.inactive,
                &mut w.hovered,
                &mut w.active,
                &mut w.open,
            ] {
                wv.corner_radius = CornerRadius::same(5);
            }
            // Gezinti bölmesinin sağındaki ayraç ve menü ayraçları
            w.noninteractive.bg_stroke = Stroke::new(1.0_f32, c.divider);
            w.inactive.weak_bg_fill = c.button;
            w.inactive.bg_fill = w.inactive.weak_bg_fill;
            w.inactive.bg_stroke = Stroke::new(1.0_f32, c.button_stroke);
            w.hovered.weak_bg_fill = p.hover;
            w.hovered.bg_fill = w.hovered.weak_bg_fill;
            w.hovered.bg_stroke = Stroke::NONE;
            w.active.weak_bg_fill = c.pressed;
            w.active.bg_fill = w.active.weak_bg_fill;
            w.active.bg_stroke = Stroke::NONE;
            w.inactive.fg_stroke.color = c.text;
            w.hovered.fg_stroke.color = c.text;
            w.active.fg_stroke.color = c.text;
            w.noninteractive.fg_stroke.color = c.label;
        });
    }
    // Pencere çerçevesi: yuvarlatılmış köşeler ve koyu başlık
    let dark_frame = match (scheme::get(theme), theme) {
        (Some(c), _) => c.dark,
        (None, 1) => false,
        (None, 2) => true,
        _ => ctx.style().visuals.dark_mode,
    };
    crate::system::apply_backdrop(dark_frame);
}

/// Ölçek: yerleşim (pencere, kenar boşlukları, simgeler) yalnızca monitör DPI'sına uyar;
/// Windows "Metin boyutu" ayarı (Ayarlar > Erişilebilirlik) yalnızca **yazı boyutuna**
/// uygulanır — Explorer'ın yaptığı da budur. Uygulama içinden yakınlaştırma
/// (Ctrl +, Ctrl -, Ctrl 0) kapalıdır.
fn apply_system_scale(ctx: &egui::Context) {
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    ctx.set_zoom_factor(1.0);
    set_ui_metrics(ctx, crate::system::text_scale());
}

/// Sistem metin ölçeği ve o anki cihaz piksel oranı (bkz. `ui_font`).
static UI_TEXT_SCALE: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0f32
static UI_PPP: AtomicU32 = AtomicU32::new(0x3F80_0000); // 1.0f32

fn set_ui_metrics(ctx: &egui::Context, scale: f32) {
    UI_TEXT_SCALE.store(scale.to_bits(), Relaxed);
    UI_PPP.store(ctx.pixels_per_point().to_bits(), Relaxed);
}

fn text_scale() -> f32 {
    f32::from_bits(UI_TEXT_SCALE.load(Relaxed))
}

/// Yazı boyutu (nokta): metin ölçeği uygulanır **ve tam cihaz pikseline yuvarlanır**.
///
/// Taban, Windows'un ileti yazı tipidir (`system::message_font_px`): DPI ve
/// "Metin boyutu" ayarı bu metriğe zaten yansımıştır. Explorer ekranda tam bu
/// boyda çizer. Tasarım tabanı 14 px olduğu için fark oransal uygulanır.
///
/// Yuvarlama kritik: 14 punto × 1,39 = 19,4 piksel em boyu yarım piksele düşerse glif
/// gövdeleri iki piksel sütununa yayılır ve bir kelimede yalnızca rastgele bir iki harf
/// keskin kalır. Tam piksel em boyunda gövdeler piksel sütunlarına oturur.
pub(crate) fn text_size(size: f32) -> f32 {
    let ppp = f32::from_bits(UI_PPP.load(Relaxed)).max(0.25);
    let scaled = (size * text_factor()).max(11.0);
    // Yuvarlarken küçültme: en yakın tam piksele oturt
    let px = (scaled * ppp).round().max(1.0);
    px / ppp
}

/// Metnin tasarım tabanına (14 px) göre oranı; kaynak: sistem yazı tipi boyu.
pub(crate) fn text_factor() -> f32 {
    let system_px = crate::system::message_font_px().unwrap_or(12.0 * text_scale());
    (system_px / 14.0).max(0.75)
}

/// Ölçeklenmiş, piksele oturan yazı tipi
pub(crate) fn ui_font(size: f32) -> egui::FontId {
    egui::FontId::proportional(text_size(size))
}

/// Kaldırma öncesi: otomatik başlatmayı ve varsayılan dosya gezgini kayıtlarını geri al.
/// Inno Setup `--uninstall-cleanup` ile dosyalar silinmeden önce çağırır.
pub fn uninstall_cleanup() {
    let cfg = load_config();
    if win::is_default_file_manager() {
        win::set_default_file_manager(false, cfg.previous_folder_app.as_deref());
    }
    win::set_autostart(false);
}

/// egui'nin kendi çizdiği yüzeyler: menüler, düğmeler, metin kutuları
struct Chrome {
    panel: Color32,
    window: Color32,
    popup: Color32,
    popup_stroke: Color32,
    extreme: Color32,
    faint: Color32,
    divider: Color32,
    button: Color32,
    button_stroke: Color32,
    pressed: Color32,
    text: Color32,
    label: Color32,
}

impl Chrome {
    fn of(dark: bool) -> Chrome {
        if let Some(c) = scheme::active().filter(|c| c.dark == dark) {
            let field = if dark { c.surface } else { c.base };
            return Chrome {
                panel: c.mantle,
                window: c.base,
                popup: field,
                popup_stroke: c.border,
                extreme: field,
                faint: scheme::mix(c.base, c.mantle, 0.5),
                divider: scheme::mix(c.mantle, c.border, 0.6),
                button: if dark {
                    c.surface
                } else {
                    scheme::mix(c.base, c.surface, 0.3)
                },
                button_stroke: c.border,
                pressed: c.border,
                text: c.text,
                label: c.text,
            };
        }
        if dark {
            Chrome {
                panel: Color32::from_rgb(32, 32, 32),
                window: Color32::from_rgb(25, 25, 25),
                popup: Color32::from_rgb(44, 44, 44),
                popup_stroke: Color32::from_gray(60),
                extreme: Color32::from_rgb(44, 44, 44),
                faint: Color32::from_rgb(29, 29, 29),
                divider: Color32::from_gray(48),
                button: Color32::from_gray(45),
                button_stroke: Color32::from_gray(58),
                pressed: Color32::from_gray(66),
                text: Color32::from_rgb(240, 240, 240),
                label: Color32::from_gray(225),
            }
        } else {
            Chrome {
                panel: Color32::from_rgb(243, 243, 243),
                window: Color32::WHITE,
                popup: Color32::WHITE,
                popup_stroke: Color32::from_gray(215),
                extreme: Color32::WHITE,
                faint: Color32::from_rgb(250, 250, 250),
                divider: Color32::from_gray(229),
                button: Color32::from_gray(251),
                button_stroke: Color32::from_gray(222),
                pressed: Color32::from_gray(222),
                text: Color32::BLACK,
                // egui varsayılan metin rengi (ui.label) buradan gelir
                label: Color32::BLACK,
            }
        }
    }
}

/// Sekmelerin durduğu, pencere başlık çubuğu yerine geçen şeridin yüksekliği.
/// Metin büyüdükçe şerit de büyür; aksi hâlde sekme yazısı kırpılır.
fn title_h() -> f32 {
    (34.0 * text_factor()).round()
}

#[derive(Clone, Copy, PartialEq)]
enum Caption {
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// Windows 11 pencere düğmesi (46 px genişlik, kapatta kırmızı vurgu)
fn caption_button(ui: &mut egui::Ui, kind: Caption) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(46.0, title_h() - 4.0), Sense::click());
    let dark = ui.visuals().dark_mode;
    let mut fg = pal_of(ui).text;
    if resp.hovered() || resp.is_pointer_button_down_on() {
        let bg = if kind == Caption::Close {
            fg = Color32::WHITE;
            if resp.is_pointer_button_down_on() {
                Color32::from_rgb(148, 22, 16)
            } else {
                Color32::from_rgb(196, 43, 28)
            }
        } else if resp.is_pointer_button_down_on() {
            if dark {
                Color32::from_white_alpha(14)
            } else {
                Color32::from_black_alpha(10)
            }
        } else if dark {
            Color32::from_white_alpha(20)
        } else {
            Color32::from_black_alpha(15)
        };
        ui.painter().rect_filled(rect, 0.0, bg);
    }
    let glyph = match kind {
        Caption::Minimize => g::MIN,
        Caption::Maximize => g::MAX,
        Caption::Restore => g::RESTORE,
        Caption::Close => g::CLOSE,
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        fluent_font(10.0),
        fg,
    );
    resp
}

/// Segoe Fluent Icons glif kodları (Explorer'ın kullandıkları)
#[cfg(windows)]
pub(crate) mod g {
    pub const BACK: &str = "\u{E72B}";
    pub const FORWARD: &str = "\u{E72A}";
    pub const UP: &str = "\u{E74A}";
    pub const REFRESH: &str = "\u{E72C}";
    pub const CHEVRON_RIGHT: &str = "\u{E76C}";
    pub const CHEVRON_DOWN: &str = "\u{E70D}";
    pub const PC: &str = "\u{E7F4}";
    pub const SEARCH: &str = "\u{E721}";
    pub const NEW: &str = "\u{ECC8}";
    pub const CUT: &str = "\u{E8C6}";
    pub const COPY: &str = "\u{E8C8}";
    pub const PASTE: &str = "\u{E77F}";
    pub const RENAME: &str = "\u{E8AC}";
    pub const LINK: &str = "\u{E71B}";
    pub const DELETE: &str = "\u{E74D}";
    pub const SORT: &str = "\u{E8CB}";
    pub const LIST: &str = "\u{E8FD}";
    pub const TILES: &str = "\u{ECA5}";
    pub const MORE: &str = "\u{E712}";
    pub const PANE: &str = "\u{E89F}";
    pub const PIN: &str = "\u{E718}";
    pub const ADD: &str = "\u{E710}";
    pub const CANCEL: &str = "\u{E711}";
    pub const MIN: &str = "\u{E921}";
    pub const MAX: &str = "\u{E922}";
    pub const RESTORE: &str = "\u{E923}";
    pub const CLOSE: &str = "\u{E8BB}";
    pub const ICONS_VIEW: &str = "\u{E739}";
    pub const OPEN_WITH: &str = "\u{E7AC}";
    pub const NEW_TAB: &str = "\u{E8A7}";
    pub const SHOW: &str = "\u{E8B7}";
    pub const COPY_PATH: &str = "\u{E71B}";
    pub const PROPERTIES: &str = "\u{E713}";
    pub const VIEW: &str = "\u{E890}";
    pub const UNDO: &str = "\u{E7A7}";
    pub const FOLDER: &str = "\u{E8F4}";
    pub const DOCUMENT: &str = "\u{E8A5}";
    pub const TERMINAL: &str = "\u{E756}";
    pub const CALC_SIZE: &str = "\u{E8EF}";
}

/// Fluent UI System Icons glif kodları (assets/fluent-icons.ttf, MIT)
#[cfg(not(windows))]
pub(crate) mod g {
    pub const BACK: &str = "\u{F15B}";
    pub const FORWARD: &str = "\u{F181}";
    pub const UP: &str = "\u{F19B}";
    pub const REFRESH: &str = "\u{F13D}";
    pub const CHEVRON_RIGHT: &str = "\u{F2AF}";
    pub const CHEVRON_DOWN: &str = "\u{F2A2}";
    pub const PC: &str = "\u{F359}";
    pub const SEARCH: &str = "\u{F68F}";
    pub const NEW: &str = "\u{F10C}";
    pub const CUT: &str = "\u{F33A}";
    pub const COPY: &str = "\u{F32B}";
    pub const PASTE: &str = "\u{F2D5}";
    pub const RENAME: &str = "\u{F669}";
    pub const LINK: &str = "\u{F4E4}";
    pub const DELETE: &str = "\u{F34C}";
    pub const SORT: &str = "\u{F18A}";
    pub const LIST: &str = "\u{ECD9}";
    pub const TILES: &str = "\u{F8CB}";
    pub const MORE: &str = "\u{E824}";
    pub const PANE: &str = "\u{E8BD}";
    pub const PIN: &str = "\u{F601}";
    pub const ADD: &str = "\u{F109}";
    pub const CANCEL: &str = "\u{F369}";
    pub const MIN: &str = "\u{EBD0}";
    pub const MAX: &str = "\u{E7EB}";
    pub const RESTORE: &str = "\u{EB96}";
    pub const CLOSE: &str = "\u{F369}";
    pub const ICONS_VIEW: &str = "\u{F462}";
    pub const OPEN_WITH: &str = "\u{F8CB}";
    pub const NEW_TAB: &str = "\u{F109}";
    pub const SHOW: &str = "\u{F68F}";
    pub const COPY_PATH: &str = "\u{F4E4}";
    pub const PROPERTIES: &str = "\u{E824}";
    pub const VIEW: &str = "\u{F462}";
    pub const UNDO: &str = "\u{F15B}";
    pub const FOLDER: &str = "\u{F10C}";
    pub const DOCUMENT: &str = "\u{ECD9}";
    pub const TERMINAL: &str = "\u{E8BD}";
    pub const CALC_SIZE: &str = "\u{F18A}";
}

/// Windows'un kendi sağ tık menüsü ve OLE sürükle-bırak yalnızca Windows'ta
const NATIVE_MENU: bool = cfg!(windows);

/// Shell ayrıştırma adları: ikonları doğrudan Windows'tan alınır
const HOME_SHELL: &str = "::{F874310E-B6B7-47DC-BC84-B9E6B38F5903}";
const NETWORK_SHELL: &str = "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}";

pub(crate) fn fluent_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(FLUENT.into()))
}

fn fl(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).font(fluent_font(size))
}

/// Explorer tarzı düz ikon düğmesi: yalnızca üzerine gelince zemin
fn flat_icon(
    ui: &mut egui::Ui,
    glyph: &str,
    size: Vec2,
    glyph_size: f32,
    enabled: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let p = pal_of(ui);
    if enabled && (resp.hovered() || resp.is_pointer_button_down_on()) {
        let bg = if resp.is_pointer_button_down_on() {
            p.nav_sel
        } else {
            p.hover
        };
        ui.painter().rect_filled(rect, 4.0, bg);
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        fluent_font(glyph_size),
        if enabled { p.text } else { p.disabled },
    );
    resp
}

/// İkon glifi + metin (+ isteğe bağlı aşağı ok) tek satırda
fn glyph_job(
    ui: &egui::Ui,
    glyph: &str,
    glyph_color: Option<Color32>,
    text: &str,
    chevron: bool,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let style = ui.style();
    let mut i = fl(glyph, 16.0);
    if let Some(c) = glyph_color {
        i = i.color(c);
    }
    i.append_to(&mut job, style, egui::FontSelection::Default, Align::Center);
    RichText::new(format!("  {text}")).append_to(
        &mut job,
        style,
        egui::FontSelection::Default,
        Align::Center,
    );
    if chevron {
        RichText::new("  ").append_to(&mut job, style, egui::FontSelection::Default, Align::Center);
        fl(g::CHEVRON_DOWN, 8.0).append_to(
            &mut job,
            style,
            egui::FontSelection::Default,
            Align::Center,
        );
    }
    job
}

/// Sistem ikonunu verilen dikdörtgene çizer (alan ayırmadan)
fn draw_icon(
    painter: &egui::Painter,
    images: &mut ImageCache,
    path: &Path,
    is_dir: bool,
    ext: &str,
    rect: Rect,
) {
    let px = icon_px(painter.ctx(), rect.width());
    let key = icon_key(path, is_dir, ext, px, false);
    match images.get(&key, path, px, true) {
        Img::Ready(tex) => {
            painter.image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        Img::Failed => paint_fallback(painter, rect, is_dir),
        Img::Pending => {}
    }
}

/// Explorer grup başlığı: "⌄ Dün". Tıklanınca açılır/kapanır.
fn group_header(
    ui: &mut egui::Ui,
    rect: Rect,
    label: &str,
    open: bool,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, Sense::click());
    let p = pal_of(ui);
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect.shrink2(egui::vec2(2.0, 2.0)), 4.0, p.hover);
    }
    let c = egui::pos2(rect.left() + 13.0, rect.center().y);
    ui.painter().text(
        c,
        Align2::CENTER_CENTER,
        if open {
            g::CHEVRON_DOWN
        } else {
            g::CHEVRON_RIGHT
        },
        fluent_font(8.0),
        p.text,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 26.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        ui_font(14.0),
        p.text,
    );
    resp
}

/// Türkçe binlik ayracı: 1172 → "1.172"
fn fmt_count(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    out
}

fn icon_rt(s: &str) -> RichText {
    RichText::new(s).family(egui::FontFamily::Name(ICONS.into()))
}

/// İkon + metin, ikonu ikon fontuyla
fn icon_label(
    ui: &egui::Ui,
    icon: &str,
    color: Option<Color32>,
    text: &str,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let style = ui.style();
    let mut i = icon_rt(&format!("{icon}  "));
    if let Some(c) = color {
        i = i.color(c);
    }
    i.append_to(&mut job, style, egui::FontSelection::Default, Align::Center);
    RichText::new(text).append_to(&mut job, style, egui::FontSelection::Default, Align::Center);
    job
}

fn is_digit_key(k: Key) -> bool {
    matches!(
        k,
        Key::Num0
            | Key::Num1
            | Key::Num2
            | Key::Num3
            | Key::Num4
            | Key::Num5
            | Key::Num6
            | Key::Num7
            | Key::Num8
            | Key::Num9
    )
}

/// Değiştirici tuşları birebir eşleyen kısayol. Rakamlar fiziksel tuşa göre eşlenir (Türkçe Q'da Shift+2 = ').
fn shortcut(i: &mut egui::InputState, mods: Modifiers, key: Key) -> bool {
    let mut hit = false;
    i.events.retain(|ev| match ev {
        egui::Event::Key {
            key: k,
            physical_key,
            pressed: true,
            modifiers,
            ..
        } if (*k == key || (is_digit_key(key) && *physical_key == Some(key)))
            && modifiers.ctrl == mods.ctrl
            && modifiers.shift == mods.shift
            && modifiers.alt == mods.alt =>
        {
            hit = true;
            false
        }
        _ => true,
    });
    hit
}

fn per_path_icon(ext: &str) -> bool {
    matches!(
        ext,
        "exe"
            | "lnk"
            | "ico"
            | "url"
            | "cur"
            | "ani"
            | "msc"
            | "cpl"
            | "scr"
            | "appref-ms"
            | "website"
    )
}

/// Küçük ikon için önbellek anahtarı + kaynak yol
fn icon_key(path: &Path, is_dir: bool, ext: &str, px: i32, plain_dir: bool) -> String {
    if plain_dir {
        format!("i{px}:<klasör>")
    } else if is_dir || ext.is_empty() || per_path_icon(ext) {
        format!("i{px}:{}", path.display())
    } else {
        format!("i{px}:.{ext}")
    }
}

fn icon_px(ctx: &egui::Context, logical: f32) -> i32 {
    px_bucket(logical * ctx.pixels_per_point())
}

/// İstenen fiziksel boyutu karşılayan en küçük standart ikon boyutu
fn px_bucket(physical: f32) -> i32 {
    let want = physical.ceil() as i32;
    [16, 24, 32, 48, 64, 96, 128, 256, 512]
        .into_iter()
        .find(|&b| b >= want)
        .unwrap_or(512)
}

/// Shell ikonu alınamazsa çizilen sade klasör/dosya simgesi
fn paint_fallback(painter: &egui::Painter, rect: Rect, is_dir: bool) {
    let r = rect.shrink(rect.width() * 0.08);
    if is_dir {
        let tab = Rect::from_min_size(
            r.left_top() + egui::vec2(0.0, r.height() * 0.12),
            egui::vec2(r.width() * 0.42, r.height() * 0.2),
        );
        painter.rect_filled(tab, 2.0, Color32::from_rgb(214, 160, 40));
        let body = Rect::from_min_max(
            r.left_top() + egui::vec2(0.0, r.height() * 0.24),
            r.right_bottom() - egui::vec2(0.0, r.height() * 0.08),
        );
        painter.rect_filled(body, 2.5, Color32::from_rgb(240, 190, 70));
    } else {
        let page = Rect::from_center_size(r.center(), egui::vec2(r.width() * 0.72, r.height()));
        painter.rect(
            page,
            2.0,
            Color32::from_gray(245),
            Stroke::new(1.0_f32, Color32::from_gray(150)),
            egui::StrokeKind::Inside,
        );
        for k in 1..4 {
            let y = page.top() + page.height() * (0.25 + 0.18 * k as f32);
            painter.line_segment(
                [
                    egui::pos2(page.left() + page.width() * 0.2, y),
                    egui::pos2(page.right() - page.width() * 0.2, y),
                ],
                Stroke::new(1.0_f32, Color32::from_gray(170)),
            );
        }
    }
}

/// Ekrana sistem ikonunu ya da (yüklenemezse) sade bir yedek simge çizer.
fn paint_icon(
    ui: &mut egui::Ui,
    images: &mut ImageCache,
    path: &Path,
    is_dir: bool,
    ext: &str,
    plain_dir: bool,
    size: f32,
    fallback: &str,
) -> egui::Response {
    let px = icon_px(ui.ctx(), size);
    let key = icon_key(path, is_dir, ext, px, plain_dir);
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    match images.get(&key, path, px, true) {
        Img::Ready(tex) => {
            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        Img::Failed => {
            let _ = fallback;
            paint_fallback(ui.painter(), rect, is_dir);
        }
        Img::Pending => {}
    }
    resp
}

fn fit_rect(tex: Vec2, b: Rect) -> Rect {
    let s = (b.width() / tex.x.max(1.0)).min(b.height() / tex.y.max(1.0));
    Rect::from_center_size(b.center(), tex * s)
}

fn display_name(e: &Entry, show_ext: bool) -> String {
    if show_ext || e.is_dir || e.ext.is_empty() {
        e.name.clone()
    } else {
        e.name[..e.name.len() - e.ext.len() - 1].to_string()
    }
}

pub(crate) fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.chars().any(|c| "\\/:*?\"<>|".contains(c)) && n != "." && n != ".."
}

#[cfg(not(windows))]
fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let dev = |p: &Path| {
        p.ancestors()
            .find_map(|x| std::fs::metadata(x).ok())
            .map(|m| m.dev())
    };
    dev(a) == dev(b)
}

#[cfg(windows)]
fn same_volume(a: &Path, b: &Path) -> bool {
    let root = |p: &Path| {
        p.components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_uppercase())
    };
    root(a) == root(b)
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max - 1).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

/// Bir yanıtı sürükle-bırak hedefi yapar: üzerine gelince vurgular, bırakınca eylem üretir.
fn drop_target(
    ui: &egui::Ui,
    resp: &egui::Response,
    target: &Path,
    accent: Color32,
    actions: &mut Vec<Action>,
) {
    if let Some(p) = resp.dnd_hover_payload::<DragPaths>() {
        if !p.0.iter().any(|s| s == target || target.starts_with(s)) {
            ui.painter().rect_stroke(
                resp.rect.expand(1.0),
                4.0,
                Stroke::new(2.0_f32, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    if let Some(p) = resp.dnd_release_payload::<DragPaths>() {
        let (ctrl, shift) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        actions.push(Action::DropPaths {
            paths: p.0.clone(),
            target: target.to_path_buf(),
            ctrl,
            shift,
        });
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start: Option<PathBuf>, hidden: bool) -> App {
        let ctx = cc.egui_ctx.clone();
        setup_fonts(&ctx);

        let cfg = load_config();
        let accent = theme_accent(cfg.theme);
        apply_lang(cfg.lang);
        load_cjk_fonts(&ctx);
        apply_system_scale(&ctx);
        apply_theme(&ctx, accent, cfg.theme);
        crate::system::set_ctx(ctx.clone());

        let (paths, active) = startup_tabs(&cfg, start);
        let (subdirs_tx, subdirs_rx) = std::sync::mpsc::channel();
        let resident = win::serve(ctx.clone());
        let auto_update = Arc::new(AtomicBool::new(cfg.auto_update));
        // Aynı anda tek süreç güncellesin: sonraki açılışları karşılayan süreç
        let last_update_check = Arc::new(AtomicU64::new(cfg.last_update_check));
        let (update_rx, update_wake) = spawn_update_checker(
            (resident || !cfg!(windows)).then(|| auto_update.clone()),
            last_update_check.clone(),
            ctx.clone(),
        );
        // Yalnızca etkin sekme hemen yüklenir; diğerleri ilk açıldıklarında
        let tabs = paths
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                if i == active {
                    Tab::new(p, &ctx)
                } else {
                    Tab::new_lazy(p)
                }
            })
            .collect();

        let app = App {
            tabs,
            active,
            sort: cfg.sort,
            asc: cfg.asc,
            show_hidden: cfg.show_hidden,
            pane: cfg.pane.min(2),
            show_ext: cfg.show_ext,
            view_level: cfg.view_level.min(VIEW_LEVELS.len() - 1),
            theme: cfg.theme,
            lang: cfg.lang,
            bookmarks: cfg.bookmarks,
            recent: cfg.recent,
            tips_dismissed: cfg.tips_dismissed,
            grouped: cfg.grouped,
            remember_views: cfg.remember_views,
            folder_views: cfg.folder_views,
            view_key: None,
            background: cfg.background,
            resident,
            autostart: win::autostart(),
            previous_folder_app: cfg.previous_folder_app,
            default_fm: None,
            auto_update,
            update_rx,
            update_wake,
            last_update_check,
            update_installed: false,
            start_hidden: hidden,
            known: known_folders(),
            tree_open: Default::default(),
            drives: Arc::new(Mutex::new(vec![])),
            drives_refreshed: Instant::now(),
            clipboard: None,
            jobs: vec![],
            dir_sizes: Arc::new(Mutex::new(HashMap::new())),
            dialog: Dialog::None,
            address_edit: None,
            address_focus: false,
            focus_search: false,
            type_buf: String::new(),
            type_time: Instant::now(),
            toasts: vec![],
            preview: Preview {
                path: None,
                data: Arc::new(Mutex::new(None)),
            },
            ctrl_v_prev: false,
            last_title: String::new(),
            images: ImageCache::new(&ctx),
            type_names: HashMap::new(),
            hwnd: None,
            renaming: None,
            undo: vec![],
            subdirs: HashMap::new(),
            subdirs_pending: Default::default(),
            subdirs_tx,
            subdirs_rx,
            grid_cols: 1,
            inject_release: None,
            accent,
            zoom_acc: 0.0,
            marquee: None,
            item_rects: vec![],
            grid_scroll: (0.0, 0.0),
            silent_jobs: vec![],
            menu: None,
            saved_config: String::new(),
            last_autosave: Instant::now(),
            started: Instant::now(),
            frames: 0,
            revealed: false,
            window_checked: false,
            minimized: false,
            good_window: cc
                .storage
                .and_then(|s| s.get_string("window"))
                .filter(|w| !w.contains("-32000")),
            ctx,
        };
        app.refresh_drives();
        app.watch_drives();
        app
    }

    fn poll_update(&mut self) {
        while let Ok(found) = self.update_rx.try_recv() {
            match found {
                crate::update::Found::Installed(v) => {
                    self.update_installed = true;
                    self.toast(
                        crate::i18n::tf(
                            "Swift Explorer {} yüklendi. Pencereyi kapatınca yeni sürüme geçilir.",
                            &[&v],
                        ),
                        false,
                    );
                }
                crate::update::Found::Available(v, url) => {
                    self.toast(
                        crate::i18n::tf("Yeni sürüm var: {}. İndirmek için: {}", &[&v, &url]),
                        false,
                    );
                }
            }
        }
    }

    fn save_config(&mut self) {
        let cfg = Config {
            bookmarks: self.bookmarks.clone(),
            tabs: self.tabs.iter().map(|t| t.path.clone()).collect(),
            active: self.active,
            show_hidden: self.show_hidden,
            pane: self.pane,
            show_ext: self.show_ext,
            sort: self.sort,
            asc: self.asc,
            view_level: self.view_level,
            theme: self.theme,
            lang: self.lang,
            recent: self.recent.clone(),
            tips_dismissed: self.tips_dismissed,
            grouped: self.grouped.clone(),
            remember_views: self.remember_views,
            folder_views: self.folder_views.clone(),
            background: self.background,
            auto_update: self.auto_update.load(Relaxed),
            last_update_check: self.last_update_check.load(Relaxed),
            previous_folder_app: self.previous_folder_app.clone(),
        };
        if let Some(p) = config_path() {
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Ok(s) = serde_json::to_string_pretty(&cfg) {
                if s != self.saved_config {
                    let _ = std::fs::write(p, &s);
                    self.saved_config = s;
                }
            }
        }
    }

    fn refresh_drives(&self) {
        let store = self.drives.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let d = win::drives();
            *store.lock().unwrap() = d;
            ctx.request_repaint();
        });
    }

    /// USB bellek takılınca listeyi hemen yeniler. 20 saniyelik yenilemeyi beklemesin diye
    /// sürücü maskesine saniyede bir bakar; maske değişmedikçe arayüzü uyandırmaz.
    fn watch_drives(&self) {
        let store = self.drives.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let mut last = win::drive_mask();
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let mask = win::drive_mask();
                if mask != last {
                    last = mask;
                    *store.lock().unwrap() = win::drives();
                    ctx.request_repaint();
                }
            }
        });
    }

    /// Linux'ta pencere eframe'in ilk karesiyle görünür; yalnızca açılış ön
    /// sonuçlarından kullanılmayanları bırakır.
    #[cfg(not(windows))]
    fn reveal_when_ready(&mut self, _ctx: &egui::Context) {
        if !self.revealed {
            self.frames += 1;
            if self.frames >= 2 {
                crate::images::end_prefetch();
                self.revealed = true;
            }
        }
    }

    /// eframe, pencere simge durumundayken (küçültülüp kapatıldığında) bozuk geometri
    /// kaydeder: konum (-32000, -32000), istemci boyutu ~160×33 piksel. Sonraki açılışta
    /// pencere bu yüzden ekranın sol üst köşesinde küçük bir kutu olarak geri gelir.
    /// İlk karede, pencere hâlâ gizliyken makul boyut ve konuma döndür.
    fn repair_restored_window(&mut self, ctx: &egui::Context) {
        if self.window_checked {
            return;
        }
        let (inner, outer, monitor, minimized) = ctx.input(|i| {
            let v = i.viewport();
            (
                v.inner_rect,
                v.outer_rect,
                v.monitor_size,
                v.minimized == Some(true),
            )
        });
        // İlk karede pencere bilgisi henüz gelmemiş olabilir; sonraki karede tekrar dene
        let Some(inner) = inner else { return };
        self.window_checked = true;
        if minimized {
            return;
        }
        let too_small = inner.width() < 600.0 || inner.height() < 380.0;
        let off_screen = outer.is_some_and(|r| r.min.x < -10_000.0 || r.min.y < -10_000.0);
        if !too_small && !off_screen {
            return;
        }
        let Some(monitor) = monitor.filter(|m| m.x >= 640.0 && m.y >= 400.0) else {
            return;
        };
        let size = egui::vec2(1060.0, 640.0).min(monitor * 0.9);
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        let pos = ((monitor - size) * 0.5).max(egui::Vec2::ZERO);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos.to_pos2()));
    }

    /// Pencereyi içerik hazır olunca tek seferde gösterir: önce boş liste,
    /// sonra tek tek beliren ikonlar yerine. En fazla 350 ms bekler.
    #[cfg(windows)]
    fn reveal_when_ready(&mut self, ctx: &egui::Context) {
        if self.revealed {
            return;
        }
        let Some(hwnd) = self.hwnd else { return };
        self.frames += 1;
        let ready = !self.tabs[self.active].is_loading() && !self.images.busy();
        if self.frames >= 2 && (ready || self.started.elapsed() >= Duration::from_millis(350)) {
            if std::mem::take(&mut self.start_hidden) {
                shell::end_startup_cloak();
                crate::images::end_prefetch();
                self.revealed = true;
                if self.resident {
                    self.park(hwnd);
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            shell::set_cloak(hwnd, false);
            shell::end_startup_cloak();
            crate::images::end_prefetch();
            self.revealed = true;
        } else {
            ctx.request_repaint();
        }
    }

    fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    /// Kapatma: pencere gizlenir, süreç sonraki açılışı bekler
    fn park(&mut self, hwnd: isize) {
        self.renaming = None;
        self.save_config();
        win::park(hwnd);
    }

    /// Gizliyken gelen açılış. Klasörle açıldıysa o sekmeye geçer, yoksa yeni sekmede açar.
    fn reopen(&mut self, start: Option<PathBuf>) {
        // Pencere yeni içerik hazır olana dek gizli kalsın, açılıştaki gibi
        self.revealed = false;
        self.frames = 0;
        self.started = Instant::now();
        if let Some(p) = start
            .map(|p| normalize(&p))
            .filter(|p| crate::tab::is_virtual(p) || p.is_dir())
        {
            match self.tabs.iter().position(|t| t.path == p) {
                Some(i) => self.active = i,
                None => self.apply(Action::NewTab(p)),
            }
        }
        self.refresh_drives();
        let _ = self.update_wake.send(());
    }

    fn toast(&mut self, msg: impl Into<String>, error: bool) {
        self.toasts.push((msg.into(), Instant::now(), error));
    }

    fn mark_views_dirty(&mut self) {
        for t in &mut self.tabs {
            t.view_dirty = true;
        }
    }

    /// Klasör anahtarı: Windows'ta büyük/küçük harf duyarsız, sanal konumlar hatırlanmaz.
    fn folder_key(p: &Path) -> Option<String> {
        if crate::tab::is_virtual(p) {
            return None;
        }
        Some(p.to_string_lossy().to_lowercase())
    }

    /// Geçerli klasörün görünümünü kaydet (yalnızca kullanıcı bir ayarı değiştirdiğinde
    /// çağrılır; böylece yalnızca gezilen her klasör için kayıt oluşmaz).
    fn remember_view(&mut self) {
        if !self.remember_views {
            return;
        }
        let path = self.tabs[self.active].path.clone();
        let Some(key) = Self::folder_key(&path) else {
            return;
        };
        let fv = FolderView {
            view_level: Some(self.view_level),
            sort: Some(self.sort),
            asc: Some(self.asc),
            grouped: Some(self.grouped.iter().any(|g| g == &path)),
            show_hidden: Some(self.show_hidden),
            show_ext: Some(self.show_ext),
        };
        self.folder_views.insert(key, fv);
    }

    /// Etkin sekmenin klasörü değiştiyse o klasörün kayıtlı görünümünü uygula.
    fn sync_folder_view(&mut self) {
        let key = Self::folder_key(&self.tabs[self.active].path);
        if key == self.view_key {
            return;
        }
        self.view_key = key;
        if !self.remember_views {
            return;
        }
        let path = self.tabs[self.active].path.clone();
        let Some(key) = Self::folder_key(&path) else {
            return;
        };
        let Some(fv) = self.folder_views.get(&key).cloned() else {
            return;
        };
        if let Some(v) = fv.view_level {
            self.view_level = v.min(VIEW_LEVELS.len() - 1);
        }
        if let Some(v) = fv.sort {
            self.sort = v;
        }
        if let Some(v) = fv.asc {
            self.asc = v;
        }
        if let Some(v) = fv.show_hidden {
            if v != self.show_hidden {
                self.subdirs.clear();
            }
            self.show_hidden = v;
        }
        if let Some(v) = fv.show_ext {
            self.show_ext = v;
        }
        if let Some(v) = fv.grouped {
            let has = self.grouped.iter().any(|g| g == &path);
            if v && !has {
                self.grouped.push(path);
            } else if !v && has {
                self.grouped.retain(|g| g != &path);
            }
        }
        self.mark_views_dirty();
    }

    /// Bu klasörün kayıtlı görünümünü unut ve Explorer varsayılanlarına dön.
    fn forget_folder_view(&mut self) {
        let path = self.tabs[self.active].path.clone();
        let Some(key) = Self::folder_key(&path) else {
            return;
        };
        self.folder_views.remove(&key);
        self.grouped.retain(|g| g != &path);
        self.view_level = 0;
        self.sort = SortKey::Name;
        self.asc = true;
        self.mark_views_dirty();
        self.save_config();
        self.toast(crate::i18n::t("Bu klasörün görünümü sıfırlandı"), false);
    }

    fn reload_active(&mut self, select: Option<PathBuf>) {
        let ctx = self.ctx.clone();
        let t = self.tab();
        if select.is_some() {
            t.select_after_load = select;
        }
        t.load(false, &ctx);
    }

    /// Yapıştırma/bırakma: çakışma varsa sor, yoksa başlat.
    fn start_transfer(&mut self, files: Vec<PathBuf>, dest: PathBuf, cut: bool) {
        if dest.as_os_str().is_empty() {
            return;
        }
        let names = fsutil::conflicts(&files, &dest);
        if names.is_empty() {
            let ctx = self.ctx.clone();
            self.push_job(
                fsutil::spawn_transfer(files, dest, cut, Conflict::KeepBoth, ctx),
                true,
            );
        } else {
            self.dialog = Dialog::Conflict {
                files,
                dest,
                cut,
                names,
            };
        }
    }

    fn create_new(&mut self, kind: usize) {
        let t = &self.tabs[self.active];
        if t.is_home() || t.search.is_some() {
            return;
        }
        let dir = t.path.clone();
        let spec = &newfile::KINDS[kind];
        let folder = kind == 0;
        let target = fsutil::unique_destination(&dir, crate::i18n::t(spec.name_key), folder);
        let r = if folder {
            std::fs::create_dir(&target)
        } else {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .and_then(|mut f| f.write_all(&(spec.template)()))
                .map(|_| ())
        };
        match r {
            Ok(()) => {
                self.undo.push(UndoOp::Created(target.clone()));
                self.tab().rename_after_load = Some(target.clone());
                self.reload_active(Some(target));
            }
            Err(e) => self.toast(crate::i18n::tf("Oluşturulamadı: {}", &[&e]), true),
        }
    }

    fn commit_rename(&mut self, path: PathBuf, text: String) {
        let text = text.trim().to_string();
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            return;
        };
        if text == name || text.is_empty() {
            return;
        }
        if !valid_name(&text) {
            self.toast(
                crate::i18n::t("Ad şu karakterleri içeremez: \\ / : * ? \" < > |"),
                true,
            );
            return;
        }
        let target = path.with_file_name(&text);
        // Yalnızca büyük/küçük harf farkı Windows'ta da çalışsın diye ayrı kontrol
        if target.exists() && !text.eq_ignore_ascii_case(&name) {
            self.toast(
                crate::i18n::tf("\"{}\" adında bir öğe zaten var", &[&text]),
                true,
            );
            return;
        }
        match std::fs::rename(&path, &target) {
            Ok(()) => {
                self.undo.push(UndoOp::Rename {
                    from: path,
                    to: target.clone(),
                });
                self.reload_active(Some(target));
            }
            Err(e) => self.toast(crate::i18n::tf("Yeniden adlandırılamadı: {}", &[&e]), true),
        }
    }

    fn do_undo(&mut self) {
        let Some(op) = self.undo.pop() else {
            self.toast(crate::i18n::t("Geri alınacak bir işlem yok"), false);
            return;
        };
        let ctx = self.ctx.clone();
        match op {
            UndoOp::Rename { from, to } => match std::fs::rename(&to, &from) {
                Ok(()) => {
                    self.toast(crate::i18n::t("Yeniden adlandırma geri alındı"), false);
                    self.reload_active(Some(from));
                }
                Err(e) => self.toast(crate::i18n::tf("Geri alınamadı: {}", &[&e]), true),
            },
            UndoOp::Created(p) => {
                self.push_job(fsutil::spawn_delete(vec![p], false, ctx), false);
                self.toast(crate::i18n::t("Oluşturma geri alındı"), false);
            }
            UndoOp::Moved(pairs) => {
                let mut failed: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
                for (src, dst) in pairs {
                    if std::fs::rename(&dst, &src).is_err() {
                        if let Some(parent) = src.parent() {
                            failed.entry(parent.to_path_buf()).or_default().push(dst);
                        }
                    }
                }
                for (parent, files) in failed {
                    self.push_job(
                        fsutil::spawn_transfer(
                            files,
                            parent,
                            true,
                            Conflict::KeepBoth,
                            ctx.clone(),
                        ),
                        false,
                    );
                }
                self.toast(crate::i18n::t("Taşıma geri alındı"), false);
            }
            UndoOp::Copied(dsts) => {
                self.push_job(fsutil::spawn_delete(dsts, false, ctx), false);
                self.toast(
                    crate::i18n::t("Kopyalama geri alındı (kopyalar geri dönüşüm kutusuna)"),
                    false,
                );
            }
            UndoOp::Trashed(paths) => {
                self.push_job(fsutil::spawn_restore(paths, ctx), false);
            }
        }
    }

    /// `undoable`: bittiğinde geri-al yığınına kaydedilsin mi (geri alma işlerinin kendisi kaydedilmez)
    fn push_job(&mut self, job: Arc<Job>, undoable: bool) {
        if !undoable {
            self.silent_jobs.push(Arc::as_ptr(&job) as usize);
        }
        self.jobs.push(job);
    }
}

// ================================================================ klavye + eylemler

impl App {
    fn handle_keys(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let typing = ctx.wants_keyboard_input();
        let focused = ctx.input(|i| i.focused);
        let home = !self.tabs[self.active].lists_files();

        // Ctrl+V: panoda metin yoksa egui olay üretmez, doğrudan klavyeyi soruyoruz.
        let v = focused && win::ctrl_v_down();
        if v && !self.ctrl_v_prev && !typing && matches!(self.dialog, Dialog::None) {
            actions.push(Action::Paste);
        }
        self.ctrl_v_prev = v;
        // Linux: Ctrl+V pano metniyle birlikte yapıştırma olayı olarak gelir
        #[cfg(not(windows))]
        if !typing && matches!(self.dialog, Dialog::None) {
            let pasted = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Paste(s) = e {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(text) = pasted {
                win::note_paste_text(&text);
                actions.push(Action::Paste);
            }
        }

        if !matches!(self.dialog, Dialog::None) {
            return;
        }

        // Ctrl+tekerlek: görünüm boyutu (Explorer gibi)
        let zoom = ctx.input(|i| i.zoom_delta());
        if zoom != 1.0 {
            self.zoom_acc += zoom.ln();
            if self.zoom_acc.abs() > 0.12 {
                let up = self.zoom_acc > 0.0;
                self.zoom_acc = 0.0;
                let lvl = if up {
                    (self.view_level + 1).min(VIEW_LEVELS.len() - 1)
                } else {
                    self.view_level.saturating_sub(1)
                };
                actions.push(Action::SetView(lvl));
            }
        }

        // Yazarken bile çalışan kısayollar
        ctx.input_mut(|i| {
            if shortcut(i, Modifiers::CTRL, Key::T) {
                actions.push(Action::NewTab(PathBuf::new()));
            }
            if shortcut(i, Modifiers::CTRL, Key::W) {
                actions.push(Action::CloseTab(self.active));
            }
            if shortcut(i, Modifiers::CTRL, Key::N) {
                actions.push(Action::NewWindow);
            }
            if shortcut(i, Modifiers::CTRL, Key::L)
                || shortcut(i, Modifiers::ALT, Key::D)
                || shortcut(i, Modifiers::NONE, Key::F4)
            {
                self.address_edit =
                    Some(self.tabs[self.active].path.to_string_lossy().into_owned());
                self.address_focus = true;
            }
            if shortcut(i, Modifiers::CTRL, Key::F)
                || shortcut(i, Modifiers::CTRL, Key::E)
                || shortcut(i, Modifiers::NONE, Key::F3)
            {
                self.focus_search = true;
            }
            if shortcut(i, Modifiers::CTRL | Modifiers::SHIFT, Key::Tab) {
                self.active = (self.active + self.tabs.len() - 1) % self.tabs.len();
            } else if shortcut(i, Modifiers::CTRL, Key::Tab) {
                self.active = (self.active + 1) % self.tabs.len();
            }
            if shortcut(i, Modifiers::NONE, Key::F5) || shortcut(i, Modifiers::CTRL, Key::R) {
                actions.push(Action::Refresh);
            }
            if shortcut(i, Modifiers::NONE, Key::F1) {
                self.dialog = Dialog::Help;
            }
            if shortcut(i, Modifiers::NONE, Key::F11) {
                let fs = i.viewport().fullscreen.unwrap_or(false);
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
            }
            if shortcut(i, Modifiers::ALT, Key::ArrowLeft)
                || i.pointer.button_pressed(egui::PointerButton::Extra1)
            {
                actions.push(Action::Back);
            }
            if shortcut(i, Modifiers::ALT, Key::ArrowRight)
                || i.pointer.button_pressed(egui::PointerButton::Extra2)
            {
                actions.push(Action::Forward);
            }
            if shortcut(i, Modifiers::ALT, Key::ArrowUp) {
                actions.push(Action::Up);
            }
            if shortcut(i, Modifiers::ALT, Key::P) || shortcut(i, Modifiers::CTRL, Key::P) {
                self.pane = if self.pane == 1 { 0 } else { 1 };
            }
            if shortcut(i, Modifiers::ALT | Modifiers::SHIFT, Key::P) {
                self.pane = if self.pane == 2 { 0 } else { 2 };
            }
            if shortcut(i, Modifiers::CTRL, Key::H) {
                self.show_hidden = !self.show_hidden;
                self.remember_view();
                for t in &mut self.tabs {
                    t.view_dirty = true;
                }
            }
            // Ctrl+Shift+1..6: Explorer'daki görünüm kısayolları
            for (k, lvl) in [
                (Key::Num1, 4),
                (Key::Num2, 3),
                (Key::Num3, 2),
                (Key::Num4, 1),
                (Key::Num6, 0),
            ] {
                if shortcut(i, Modifiers::CTRL | Modifiers::SHIFT, k) {
                    actions.push(Action::SetView(lvl));
                }
            }
            for (n, k) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                if shortcut(i, Modifiers::CTRL, k) && n < self.tabs.len() {
                    self.active = n;
                }
            }
        });

        if typing || home {
            return;
        }

        let mut text_typed = String::new();
        ctx.input_mut(|i| {
            if shortcut(i, Modifiers::CTRL | Modifiers::SHIFT, Key::N) {
                actions.push(Action::NewFolder);
            }
            if shortcut(i, Modifiers::CTRL, Key::Z) {
                actions.push(Action::Undo);
            }
            if shortcut(i, Modifiers::CTRL, Key::A) {
                actions.push(Action::SelectAll);
            }
            if shortcut(i, Modifiers::CTRL | Modifiers::SHIFT, Key::C) {
                actions.push(Action::CopyPath);
            }
            if shortcut(i, Modifiers::CTRL, Key::D) {
                let p = self.tabs[self.active].path.clone();
                actions.push(Action::AddBookmark(p));
            }
            if shortcut(i, Modifiers::SHIFT, Key::Delete) {
                actions.push(Action::Delete { permanent: true });
            }
            if shortcut(i, Modifiers::NONE, Key::Delete) {
                actions.push(Action::Delete { permanent: false });
            }
            if shortcut(i, Modifiers::NONE, Key::F2) {
                actions.push(Action::Rename);
            }
            if NATIVE_MENU && shortcut(i, Modifiers::SHIFT, Key::F10) {
                actions.push(Action::ShellMenu { extended: false });
            }
            if shortcut(i, Modifiers::NONE, Key::Backspace) {
                actions.push(Action::Up);
            }
            if shortcut(i, Modifiers::ALT, Key::Enter) {
                if let Some(e) = self.tabs[self.active].selected_entries().first() {
                    actions.push(Action::Properties(e.path.clone()));
                }
            }
            if shortcut(i, Modifiers::NONE, Key::Enter) {
                actions.push(Action::OpenSelected);
            }
            if shortcut(i, Modifiers::NONE, Key::Escape) {
                let t = &mut self.tabs[self.active];
                if t.search.is_some() {
                    actions.push(Action::ExitSearch);
                } else if !t.filter.is_empty() {
                    t.filter.clear();
                    t.view_dirty = true;
                } else {
                    t.selection.clear();
                }
            }
            for ev in &i.events {
                match ev {
                    egui::Event::Copy => actions.push(Action::Copy),
                    egui::Event::Cut => actions.push(Action::Cut),
                    egui::Event::Text(t) if !i.modifiers.ctrl && !i.modifiers.alt => {
                        text_typed.push_str(t)
                    }
                    _ => {}
                }
            }
        });

        // Ok tuşları: ayrıntılarda satır, simgelerde ızgara
        let grid = self.view_level > 0;
        let cols = if grid {
            self.grid_cols.max(1) as isize
        } else {
            1
        };
        let (n, shift, ctrl, step) = ctx.input(|i| {
            let mut step: Option<isize> = None;
            let page = if grid { cols * 4 } else { 20 };
            if i.key_pressed(Key::ArrowDown) {
                step = Some(cols);
            } else if i.key_pressed(Key::ArrowUp) {
                step = Some(-cols);
            } else if grid && i.key_pressed(Key::ArrowRight) {
                step = Some(1);
            } else if grid && i.key_pressed(Key::ArrowLeft) {
                step = Some(-1);
            } else if i.key_pressed(Key::PageDown) {
                step = Some(page);
            } else if i.key_pressed(Key::PageUp) {
                step = Some(-page);
            } else if i.key_pressed(Key::Home) {
                step = Some(isize::MIN / 2);
            } else if i.key_pressed(Key::End) {
                step = Some(isize::MAX / 2);
            }
            (
                self.tabs[self.active].view.len(),
                i.modifiers.shift,
                i.modifiers.ctrl,
                step,
            )
        });
        if let (Some(step), true) = (step, n > 0) {
            let t = self.tab();
            let cur = t.cursor_row().map(|r| r as isize).unwrap_or(-1);
            let row = if cur < 0 && step > 0 && step < 100_000 {
                0
            } else {
                cur.saturating_add(step).clamp(0, n as isize - 1) as usize
            };
            let idx = t.view[row];
            if shift {
                t.select_range_to(row);
            } else if ctrl {
                t.cursor = Some(idx);
            } else {
                t.select_only(idx);
            }
            t.scroll_to_cursor = true;
        }
        if ctrl {
            ctx.input(|i| {
                if i.key_pressed(Key::Space) {
                    let t = &mut self.tabs[self.active];
                    if let Some(c) = t.cursor {
                        if !t.selection.remove(&c) {
                            t.selection.insert(c);
                        }
                    }
                }
            });
        }
        // Boşluk: seçili dosyayı PowerToys Peek ile önizle (Ctrl+Boşluk çoklu seçimdir)
        if !ctrl
            && !shift
            && matches!(self.dialog, Dialog::None)
            && self.tabs[self.active].lists_files()
        {
            ctx.input(|i| {
                if i.key_pressed(Key::Space) {
                    actions.push(Action::Peek);
                }
            });
        }

        // Yazarak atla: ada göre ilk eşleşmeye git (aynı harfe tekrar basınca sıradakine)
        let text_typed: String = text_typed.chars().filter(|c| !c.is_control()).collect();
        if !text_typed.is_empty() && text_typed != " " {
            if self.type_time.elapsed() > Duration::from_millis(900) {
                self.type_buf.clear();
            }
            self.type_time = Instant::now();
            self.type_buf.push_str(&text_typed.to_lowercase());
            let buf = self.type_buf.clone();
            let t = self.tab();
            let start = t.cursor_row().unwrap_or(0);
            let n = t.view.len();
            let first = buf.chars().next().unwrap();
            let single_repeat = buf.chars().all(|c| c == first);
            let (needle, from) = if single_repeat {
                (first.to_string(), start + 1)
            } else {
                (buf, start)
            };
            if let Some(row) = (0..n)
                .map(|k| (from + k) % n)
                .find(|&r| t.entries[t.view[r]].name_lower.starts_with(&needle))
            {
                let idx = t.view[row];
                t.select_only(idx);
                t.scroll_to_cursor = true;
            }
        }
    }

    fn apply(&mut self, action: Action) {
        let ctx = self.ctx.clone();
        let busy_here = {
            let t = &self.tabs[self.active];
            t.is_home() || t.search.is_some()
        };
        match action {
            Action::Navigate(p) => {
                self.address_edit = None;
                self.renaming = None;
                self.images.drop_pending();
                if crate::tab::is_virtual(&p) || p.is_dir() {
                    if p.as_os_str() == crate::tab::GALLERY {
                        self.view_level = 3;
                    }
                    self.tab().navigate(p, true, &ctx);
                } else if p.is_file() {
                    let _ = open::that_detached(&p);
                } else {
                    self.toast(
                        crate::i18n::tf(
                            "Windows \"{}\" öğesini bulamıyor. Adı doğru yazdığınızdan emin olun.",
                            &[&p.display()],
                        ),
                        true,
                    );
                }
            }
            Action::OpenEntry(p, is_dir) => {
                if is_dir {
                    self.renaming = None;
                    self.images.drop_pending();
                    self.tab().navigate(p, true, &ctx);
                } else if let Err(e) = open::that_detached(&p) {
                    self.toast(crate::i18n::tf("Açılamadı: {}", &[&e]), true);
                }
            }
            Action::OpenSelected => {
                let sel: Vec<(PathBuf, bool)> = self
                    .tab()
                    .selected_entries()
                    .iter()
                    .map(|e| (e.path.clone(), e.is_dir))
                    .collect();
                if let [(p, true)] = sel.as_slice() {
                    self.apply(Action::OpenEntry(p.clone(), true));
                } else {
                    for (p, is_dir) in sel.into_iter().take(32) {
                        if is_dir {
                            self.tabs.push(Tab::new(p, &ctx));
                        } else {
                            let _ = open::that_detached(&p);
                        }
                    }
                }
            }
            Action::Up => {
                self.renaming = None;
                self.tab().go_up(&ctx);
            }
            Action::Back => self.tab().go_back(&ctx),
            Action::Forward => self.tab().go_forward(&ctx),
            Action::Refresh => {
                self.subdirs.clear();
                let t = self.tab();
                if t.search.is_none() {
                    t.refresh_free_space(&ctx);
                    t.load(false, &ctx);
                }
                self.refresh_drives();
            }
            Action::NewTab(p) => {
                self.tabs.push(Tab::new(p, &ctx));
                self.active = self.tabs.len() - 1;
            }
            Action::NewWindow => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe)
                        .arg(&self.tabs[self.active].path)
                        .spawn();
                }
            }
            Action::CloseTab(i) => {
                if self.tabs.len() > 1 {
                    self.tabs[i].cancel_search();
                    self.tabs.remove(i);
                    if self.active >= self.tabs.len() || self.active > i {
                        self.active = self.active.saturating_sub(1).min(self.tabs.len() - 1);
                    }
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            Action::Copy | Action::Cut => {
                let cut = matches!(action, Action::Cut);
                let paths = self.tab().selected_paths();
                if paths.is_empty() {
                    return;
                }
                win::clipboard_set_files(&paths, cut);
                #[cfg(not(windows))]
                self.ctx.copy_text(win::files_as_text(&paths));
                let n = paths.len();
                self.clipboard = Some((paths, cut));
                self.toast(
                    crate::i18n::tf(
                        "{} öğe {} — yapıştırmak için Ctrl+V",
                        &[
                            &n,
                            &if cut {
                                crate::i18n::t("kesildi")
                            } else {
                                crate::i18n::t("kopyalandı")
                            },
                        ],
                    ),
                    false,
                );
            }
            Action::Paste => {
                if busy_here {
                    return;
                }
                let dest = self.tab().path.clone();
                self.apply(Action::PasteInto(dest));
            }
            Action::PickDest { paths, cut } => {
                if !paths.is_empty() {
                    let start = self.tab().path.clone();
                    self.renaming = None;
                    if let Some(dest) = shell::pick_folder(self.hwnd.unwrap_or(0), Some(&start)) {
                        self.start_transfer(paths, dest, cut);
                    }
                }
            }
            Action::PasteInto(dest) => {
                let (files, cut) = match (win::clipboard_get_files(), &self.clipboard) {
                    (Some((os, os_cut)), Some((mine, my_cut))) if &os == mine => {
                        (os, *my_cut || os_cut)
                    }
                    (Some((os, os_cut)), _) => (os, os_cut),
                    (None, Some((mine, my_cut))) if !cfg!(windows) => (mine.clone(), *my_cut),
                    (None, _) => {
                        self.toast(crate::i18n::t("Panoda yapıştırılacak dosya yok"), false);
                        return;
                    }
                };
                self.start_transfer(files, dest, cut);
                if cut {
                    self.clipboard = None;
                }
            }
            Action::DropPaths {
                paths,
                target,
                ctrl,
                shift,
            } => {
                let paths: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| {
                        p.parent() != Some(target.as_path())
                            && *p != target
                            && !target.starts_with(p)
                    })
                    .collect();
                if paths.is_empty() || target.as_os_str().is_empty() {
                    return;
                }
                // Explorer kuralı: aynı sürücüde taşı, farklı sürücüde kopyala; Ctrl kopyala, Shift taşı
                let cut = if ctrl {
                    false
                } else if shift {
                    true
                } else {
                    same_volume(&paths[0], &target)
                };
                self.start_transfer(paths, target, cut);
            }
            Action::Delete { permanent } => {
                if self.tab().is_home() {
                    return;
                }
                let paths = self.tab().selected_paths();
                if paths.is_empty() {
                    return;
                }
                if permanent {
                    self.dialog = Dialog::ConfirmDelete { paths };
                } else {
                    self.push_job(fsutil::spawn_delete(paths, false, ctx), true);
                }
            }
            Action::Rename => {
                if let Some(e) = self.tab().selected_entries().first() {
                    self.renaming = Some(Renaming {
                        path: e.path.clone(),
                        text: e.name.clone(),
                        first: true,
                    });
                }
            }
            Action::NewFolder => self.create_new(0),
            Action::New(i) => self.create_new(i),
            Action::SelectAll => {
                let t = self.tab();
                t.selection = t.view.iter().copied().collect();
            }
            Action::SelectNone => self.tab().selection.clear(),
            Action::InvertSelection => {
                let t = self.tab();
                t.selection = t
                    .view
                    .iter()
                    .copied()
                    .filter(|i| !t.selection.contains(i))
                    .collect();
            }
            Action::CopyPath => {
                let paths = self.tab().selected_paths();
                let text = if paths.is_empty() {
                    self.tab().path.to_string_lossy().into_owned()
                } else {
                    paths
                        .iter()
                        .map(|p| format!("\"{}\"", p.display()))
                        .collect::<Vec<_>>()
                        .join("\r\n")
                };
                ctx.copy_text(text);
                self.toast(crate::i18n::t("Yol panoya kopyalandı"), false);
            }
            Action::Terminal(p) => {
                win::open_terminal(&p);
            }
            Action::RunTerminal { name, args, dir } => {
                if !win::run_terminal(&name, &args, &dir) {
                    self.toast(crate::i18n::tf("{} başlatılamadı", &[&name]), true);
                }
            }
            // Windows'un kendi penceresi: Güvenlik, Uyumluluk, Önceki Sürümler gibi sekmeler orada
            #[cfg(windows)]
            Action::Properties(p) => win::show_properties(&p),
            #[cfg(not(windows))]
            Action::Properties(p) => {
                // Seçili öğelerden biri için istendiyse hepsinin toplamı gösterilir
                let mut paths = self.tab().selected_paths();
                if !paths.contains(&p) {
                    paths = vec![p];
                }
                let entry = self
                    .tab()
                    .entries
                    .iter()
                    .find(|e| paths.len() == 1 && e.path == paths[0])
                    .cloned();
                let tn = match entry {
                    Some(e) => type_name_cached(&mut self.type_names, &e),
                    None => crate::i18n::t(if paths.first().is_some_and(|p| p.is_dir()) {
                        "Dosya klasörü"
                    } else {
                        "Dosya"
                    })
                    .into(),
                };
                match crate::properties::Properties::new(paths, tn, &ctx) {
                    Some(pr) => self.dialog = Dialog::Properties(Box::new(pr)),
                    None => self.toast(crate::i18n::t("Özellikler gösterilemedi"), true),
                }
            }
            Action::OpenWith(p) => win::open_with(&p),
            Action::ShellMenu { extended } => {
                let mut paths = self.tab().selected_paths();
                if paths.is_empty() && !self.tab().is_home() {
                    paths = vec![self.tab().path.clone()];
                }
                if let (Some(hwnd), false) = (self.hwnd, paths.is_empty()) {
                    if let MenuResult::Rename = shell::context_menu(hwnd, &paths, extended) {
                        self.apply(Action::Rename);
                    }
                    // Menü kendi mesaj döngüsünü çalıştırdı; egui'deki basılı fare durumunu sıfırla
                    self.inject_release = ctx.input(|i| i.pointer.latest_pos());
                }
            }
            Action::OpenMenu { target, pos } => self.open_context_menu(&ctx, target, pos),
            Action::NativeMenu {
                paths,
                background,
                extended,
            } => {
                if let Some(hwnd) = self.hwnd {
                    let res = if let Some(folder) = background {
                        shell::background_context_menu(hwnd, &folder, extended)
                    } else {
                        shell::context_menu(hwnd, &paths, extended)
                    };
                    if let MenuResult::Rename = res {
                        self.apply(Action::Rename);
                    }
                    self.inject_release = ctx.input(|i| i.pointer.latest_pos());
                }
            }
            Action::CalcSize => {
                let mut dirs: Vec<PathBuf> = self
                    .tab()
                    .selected_entries()
                    .iter()
                    .filter(|e| e.is_dir)
                    .map(|e| e.path.clone())
                    .collect();
                if dirs.is_empty() {
                    let t = self.tab();
                    dirs = t
                        .view
                        .iter()
                        .map(|&i| &t.entries[i])
                        .filter(|e| e.is_dir)
                        .map(|e| e.path.clone())
                        .collect();
                }
                if !dirs.is_empty() {
                    let sizes = self.dir_sizes.clone();
                    self.push_job(fsutil::spawn_sizes(dirs, sizes, ctx), false);
                }
            }
            Action::AddBookmark(p) => {
                if !p.as_os_str().is_empty() && !self.bookmarks.contains(&p) {
                    self.bookmarks.push(p);
                    self.save_config();
                    self.toast(crate::i18n::t("Hızlı erişime sabitlendi"), false);
                }
            }
            Action::RemoveBookmark(p) => {
                self.bookmarks.retain(|b| b != &p);
                self.save_config();
            }
            Action::StartSearch => {
                self.renaming = None;
                let roots = if self.tab().is_pc() {
                    self.drives
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|d| d.root.clone())
                        .collect()
                } else if self.tab().is_gallery() {
                    // Galeri sanal bir konum; arama Pictures ağacında yapılır
                    dirs::picture_dir().into_iter().collect()
                } else {
                    vec![self.tab().path.clone()]
                };
                self.tab().start_search(roots, &ctx);
            }
            Action::ExitSearch => self.tab().exit_search(&ctx),
            Action::Sort(k) => {
                if self.sort == k {
                    self.asc = !self.asc;
                } else {
                    self.sort = k;
                    self.asc = k == SortKey::Name || k == SortKey::Type;
                }
                self.remember_view();
                self.mark_views_dirty();
            }
            Action::SetView(l) => {
                self.view_level = l.min(VIEW_LEVELS.len() - 1);
                self.tab().scroll_to_cursor = true;
                self.remember_view();
            }
            Action::TogglePreview => self.pane = if self.pane == 1 { 0 } else { 1 },
            Action::ToggleDetails => self.pane = if self.pane == 2 { 0 } else { 2 },
            Action::ToggleHidden => {
                self.show_hidden = !self.show_hidden;
                self.subdirs.clear();
                self.remember_view();
                self.mark_views_dirty();
            }
            Action::ToggleExt => {
                self.show_ext = !self.show_ext;
                self.remember_view();
            }
            Action::ToggleRememberViews => {
                self.remember_views = !self.remember_views;
                if self.remember_views {
                    self.remember_view();
                }
                self.save_config();
            }
            Action::ForgetFolderView => self.forget_folder_view(),
            Action::ToggleDefaultFm => {
                let target = !self.default_fm.unwrap_or_else(win::is_default_file_manager);
                let before =
                    win::set_default_file_manager(target, self.previous_folder_app.as_deref());
                self.previous_folder_app = before;
                self.default_fm = Some(win::is_default_file_manager());
            }
            Action::ToggleBackground => {
                self.background = !self.background;
                // Arka plan bekleme kapanırsa otomatik başlatma da anlamsızlaşır
                if !self.background && self.autostart {
                    win::set_autostart(false);
                    self.autostart = win::autostart();
                }
            }
            Action::ToggleAutostart => {
                win::set_autostart(!self.autostart);
                self.autostart = win::autostart();
            }
            Action::ToggleAutoUpdate => {
                let on = !self.auto_update.load(Relaxed);
                self.auto_update.store(on, Relaxed);
            }
            Action::SetTheme(v) => {
                if self.theme != v {
                    self.theme = v;
                    self.accent = theme_accent(v);
                    apply_theme(&ctx, self.accent, v);
                }
            }
            Action::SetLanguage(v) => {
                if self.lang != v {
                    self.lang = v;
                    apply_lang(v);
                    // Dile bağlı önbellekler: bilinen klasör adları, sürücü etiketleri, tür adları
                    self.known = known_folders();
                    self.type_names.clear();
                    self.refresh_drives();
                    self.mark_views_dirty();
                }
            }
            Action::Undo => self.do_undo(),
            Action::Peek => {
                let Some(entry) = self.tab().selected_entries().into_iter().next() else {
                    return;
                };
                if !win::peek_file(&entry.path) {
                    self.toast(
                        crate::i18n::t("Peek açılamadı — PowerToys kurulu mu?"),
                        true,
                    );
                }
            }
        }
    }
}

// ================================================================ üst çubuklar, kenar çubuğu, Giriş

fn list_subdirs(dir: &Path, show_hidden: bool) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|de| {
            let md = de.metadata().ok()?;
            let a = fsutil::attributes(&de.path(), &md);
            let is_dir = a & fsutil::ATTR_DIR != 0;
            let hidden = fsutil::is_hidden_attr(a);
            (is_dir && (show_hidden || !hidden))
                .then(|| (de.file_name().to_string_lossy().to_lowercase(), de.path()))
        })
        .collect();
    v.sort_by(|a, b| fsutil::natural_cmp(&a.0, &b.0));
    v.into_iter().map(|(_, p)| p).collect()
}

fn known_folders() -> Vec<(&'static str, &'static str, PathBuf)> {
    [
        ("🖥", crate::i18n::t("Masaüstü"), dirs::desktop_dir()),
        ("⬇", crate::i18n::t("İndirilenler"), dirs::download_dir()),
        ("📄", crate::i18n::t("Belgeler"), dirs::document_dir()),
        ("🖼", crate::i18n::t("Resimler"), dirs::picture_dir()),
        ("🎵", crate::i18n::t("Müzikler"), dirs::audio_dir()),
        ("🎞", crate::i18n::t("Videolar"), dirs::video_dir()),
    ]
    .into_iter()
    .filter_map(|(i, n, p)| p.map(|p| (i, n, p)))
    .collect()
}

fn path_label(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().trim_end_matches('\\').to_string())
}

impl App {
    /// Alt klasörler; henüz okunmadıysa okumayı başlatır ve şimdilik boş döner
    fn subdirs_cached(&mut self, dir: &Path) -> Arc<[PathBuf]> {
        if let Some(v) = self.subdirs.get(dir) {
            return v.clone();
        }
        if self.subdirs_pending.insert(dir.to_path_buf()) {
            let (dir, hidden, tx, ctx) = (
                dir.to_path_buf(),
                self.show_hidden,
                self.subdirs_tx.clone(),
                self.ctx.clone(),
            );
            std::thread::spawn(move || {
                let v = list_subdirs(&dir, hidden);
                let _ = tx.send((dir, hidden, v));
                ctx.request_repaint();
            });
        }
        Arc::from([])
    }

    fn poll_subdirs(&mut self) {
        while let Ok((dir, hidden, v)) = self.subdirs_rx.try_recv() {
            self.subdirs_pending.remove(&dir);
            // Gizli öğe ayarı bu arada değiştiyse sonuç eskidir; bir sonraki istekte yeniden okunur
            if hidden == self.show_hidden {
                self.subdirs.insert(dir, v.into());
            }
        }
    }

    /// Explorer'ın gösterdiği ad: Giriş, Bu bilgisayar, sürücü etiketi, bilinen klasör adı ya da klasör adı
    fn loc_name(&self, p: &Path) -> String {
        if p.as_os_str().is_empty() {
            return crate::i18n::t("Giriş").into();
        }
        if p.as_os_str() == THIS_PC {
            return crate::i18n::t("Bu bilgisayar").into();
        }
        if p.as_os_str() == crate::tab::GALLERY {
            return crate::i18n::t("Galeri").into();
        }
        if p.parent().is_none() {
            if let Some(d) = self.drives.lock().unwrap().iter().find(|d| d.root == p) {
                return d.label.clone();
            }
        }
        if let Some((_, n, _)) = self.known.iter().find(|(_, _, k)| k == p) {
            return n.to_string();
        }
        path_label(p)
    }

    /// Konumun ikonu için Shell yolu (Giriş/Bu bilgisayar dahil)
    fn loc_icon(p: &Path) -> PathBuf {
        if p.as_os_str().is_empty() {
            PathBuf::from(HOME_SHELL)
        } else {
            p.to_path_buf()
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let p = pal_of(ui);
        // Başlık çubuğu: boş yerden sürükleyince pencere taşınır, çift tıklayınca büyür/küçülür.
        // Önce kaydedilir ki üstüne gelen sekmeler ve düğmeler tıklamayı alsın.
        let bar = Rect::from_min_size(
            ui.max_rect().min,
            egui::vec2(ui.available_width(), title_h()),
        );
        let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        let drag = ui.interact(bar, egui::Id::new("title_drag"), Sense::click_and_drag());
        if drag.drag_started_by(egui::PointerButton::Primary) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
        }
        let n = self.tabs.len();
        // Explorer'da sekme en fazla ~236 px; sığmazsa daralır
        let tab_w = ((bar.width() - 46.0 * 3.0 - 60.0) / n as f32).clamp(60.0, 236.0);
        let tab_h = title_h() - 2.0;
        ui.with_layout(Layout::left_to_right(Align::Max), |ui| {
            ui.set_min_height(title_h());
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..n {
                let active = i == self.active;
                let (title, path) = {
                    let t = &self.tabs[i];
                    let title = if t.search.is_some() {
                        t.title()
                    } else {
                        self.loc_name(&t.path)
                    };
                    (title, t.path.clone())
                };
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(tab_w, tab_h), Sense::click());
                let painter = ui.painter().clone();
                let radius = CornerRadius {
                    nw: 6,
                    ne: 6,
                    sw: 0,
                    se: 0,
                };
                if active {
                    painter.rect_filled(rect, radius, p.nav);
                } else if resp.hovered() {
                    painter.rect_filled(
                        rect.shrink2(egui::vec2(2.0, 0.0))
                            .with_max_y(rect.bottom() - 4.0),
                        6.0,
                        p.hover,
                    );
                } else if i + 1 < n && i + 1 != self.active {
                    // Etkin olmayan sekmeler arasında ince ayraç
                    let x = rect.right() - 0.5;
                    painter.vline(
                        x,
                        rect.center().y - 8.0..=rect.center().y + 6.0,
                        Stroke::new(1.0_f32, p.line),
                    );
                }
                let icon = Rect::from_center_size(
                    egui::pos2(rect.left() + 20.0, rect.center().y),
                    Vec2::splat(16.0),
                );
                if path.as_os_str() == THIS_PC || path.as_os_str().is_empty() || path.is_absolute()
                {
                    draw_icon(
                        &painter,
                        &mut self.images,
                        &Self::loc_icon(&path),
                        true,
                        "",
                        icon,
                    );
                }
                // Kapat düğmesi
                let close_rect = Rect::from_center_size(
                    egui::pos2(rect.right() - 20.0, rect.center().y),
                    egui::vec2(28.0, 24.0),
                );
                let close = ui
                    .interact(close_rect, egui::Id::new(("tab_close", i)), Sense::click())
                    .on_hover_text(crate::i18n::t("Sekmeyi kapat (Ctrl+W)"));
                if close.hovered() {
                    painter.rect_filled(close_rect, 4.0, p.nav_sel);
                }
                painter.text(
                    close_rect.center(),
                    Align2::CENTER_CENTER,
                    g::CLOSE,
                    fluent_font(10.0),
                    p.text,
                );
                if close.clicked() {
                    actions.push(Action::CloseTab(i));
                }
                let text_w = (close_rect.left() - 4.0) - (icon.right() + 10.0);
                let mut job =
                    egui::text::LayoutJob::simple_singleline(title, ui_font(14.0), p.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(text_w.max(10.0));
                let galley = painter.layout_job(job);
                painter.galley(
                    egui::pos2(icon.right() + 10.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    p.text,
                );

                if resp.clicked() {
                    self.active = i;
                }
                if resp.middle_clicked() {
                    actions.push(Action::CloseTab(i));
                }
                // Sürüklerken sekmenin üzerine gelince o sekmeye geç (Explorer gibi)
                if resp.dnd_hover_payload::<DragPaths>().is_some() && !active {
                    self.active = i;
                }
                if !crate::tab::is_virtual(&path) {
                    drop_target(ui, &resp, &path, accent, actions);
                }
                let tip = if path.as_os_str().is_empty() || path.as_os_str() == THIS_PC {
                    self.loc_name(&path)
                } else {
                    path.display().to_string()
                };
                resp.on_hover_text(tip).context_menu(|ui| {
                    if ui.button(crate::i18n::t("Sekmeyi çoğalt")).clicked() {
                        actions.push(Action::NewTab(path.clone()));
                        ui.close_menu();
                    }
                    if ui.button(crate::i18n::t("Yeni pencerede aç")).clicked() {
                        let _ = std::env::current_exe()
                            .map(|exe| std::process::Command::new(exe).arg(&path).spawn());
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button(crate::i18n::t("Sekmeyi kapat")).clicked() {
                        actions.push(Action::CloseTab(i));
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            n > 1,
                            egui::Button::new(crate::i18n::t("Diğer sekmeleri kapat")),
                        )
                        .clicked()
                    {
                        for k in (0..n).rev().filter(|&k| k != i) {
                            actions.push(Action::CloseTab(k));
                        }
                        ui.close_menu();
                    }
                });
            }
            ui.add_space(6.0);
            let plus = ui
                .allocate_ui_with_layout(
                    egui::vec2(40.0, tab_h),
                    Layout::left_to_right(Align::Center),
                    |ui| flat_icon(ui, g::ADD, egui::vec2(40.0, 30.0), 12.0, true),
                )
                .inner;
            if plus
                .on_hover_text(crate::i18n::t("Yeni sekme (Ctrl+T)"))
                .clicked()
            {
                actions.push(Action::NewTab(PathBuf::new()));
            }
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if caption_button(ui, Caption::Close).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                let (kind, tip) = if maximized {
                    (Caption::Restore, crate::i18n::t("Geri getir"))
                } else {
                    (Caption::Maximize, crate::i18n::t("Ekranı kapla"))
                };
                if caption_button(ui, kind).on_hover_text(tip).clicked() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                }
                if caption_button(ui, Caption::Minimize)
                    .on_hover_text(crate::i18n::t("Simge durumuna küçült"))
                    .clicked()
                {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            });
        });
    }

    fn nav_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = pal_of(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let t = &self.tabs[self.active];
            let (can_back, can_fwd, can_up) = (!t.back.is_empty(), !t.forward.is_empty(), !t.path.as_os_str().is_empty());
            let size = egui::vec2(40.0, 32.0);
            if flat_icon(ui, g::BACK, size, 16.0, can_back).on_hover_text(crate::i18n::t("Geri (Alt+Sol ok)")).clicked() {
                actions.push(Action::Back);
            }
            if flat_icon(ui, g::FORWARD, size, 16.0, can_fwd).on_hover_text(crate::i18n::t("İleri (Alt+Sağ ok)")).clicked() {
                actions.push(Action::Forward);
            }
            if flat_icon(ui, g::UP, size, 16.0, can_up).on_hover_text(crate::i18n::t("Üst klasör (Alt+Yukarı ok)")).clicked() {
                actions.push(Action::Up);
            }
            if flat_icon(ui, g::REFRESH, size, 16.0, true).on_hover_text(crate::i18n::t("Yenile (F5)")).clicked() {
                actions.push(Action::Refresh);
            }
            ui.add_space(2.0);

            // Explorer: arama kutusu pencere genişliğinin dörtte biri
            let search_w = (ui.ctx().screen_rect().width() * 0.25).clamp(180.0, 480.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Arama kutusu (Explorer'daki gibi sağda)
                let place = {
                    let t = &self.tabs[self.active];
                    self.loc_name(&t.path)
                };
                let t = &mut self.tabs[self.active];
                let hint = if t.search.is_some() { crate::i18n::t("Sonuçlarda filtrele").to_string() } else { crate::i18n::tf("{} klasöründe ara", &[&place]) };
                let frame = egui::Frame::new().fill(p.field).stroke(p.field_stroke).corner_radius(4.0).inner_margin(egui::Margin { left: 11, right: 10, top: 0, bottom: 0 });
                let mut submit = false;
                let mut clear = false;
                frame.show(ui, |ui| {
                    ui.set_width(search_w - 21.0);
                    ui.set_height(32.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(fl(g::SEARCH, 14.0).color(p.text));
                        if !t.filter.is_empty() && flat_icon(ui, g::CANCEL, egui::vec2(24.0, 24.0), 10.0, true).on_hover_text(crate::i18n::t("Temizle (Esc)")).clicked() {
                            clear = true;
                        }
                        let hint = RichText::new(hint).color(p.weak);
                        let resp = ui.add(egui::TextEdit::singleline(&mut t.filter).hint_text(hint).frame(false).desired_width(ui.available_width()).id(egui::Id::new("search")));
                        if self.focus_search {
                            resp.request_focus();
                            self.focus_search = false;
                        }
                        if resp.changed() {
                            t.view_dirty = true;
                        }
                        if resp.has_focus() && ui.input(|i| i.key_pressed(Key::ArrowDown)) {
                            resp.surrender_focus();
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !t.filter.trim().is_empty() {
                            submit = true;
                        }
                        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                            clear = true;
                        }
                        resp.on_hover_text(crate::i18n::t("Yazdıkça bu klasörü anında filtreler.\nEnter: tüm alt klasörlerde ara.\n* ve ? joker karakterleri desteklenir (ör. *.pdf)."));
                    });
                });
                if clear {
                    t.filter.clear();
                    t.view_dirty = true;
                }
                if submit {
                    let p = expand_path(t.filter.trim());
                    if t.is_home() && (p.exists() || !t.is_pc()) {
                        t.filter.clear();
                        if p.exists() {
                            actions.push(Action::Navigate(p));
                        }
                    } else if t.search.is_none() {
                        actions.push(Action::StartSearch);
                    }
                }
                ui.allocate_ui_with_layout(ui.available_size(), Layout::left_to_right(Align::Center), |ui| {
                    self.address_bar(ui, actions);
                });
            });
        });
    }

    /// Adres çubuğundaki parçalar: (gidilecek yer, görünen ad). Explorer gibi bilinen klasörler
    /// ("İndirilenler") doğrudan, diğerleri "Bu bilgisayar > C: > …" olarak gösterilir.
    fn crumbs(&self, path: &Path) -> Vec<(PathBuf, String)> {
        if crate::tab::is_virtual(path) {
            return vec![(path.to_path_buf(), self.loc_name(path))];
        }
        let home = dirs::home_dir();
        let base = self
            .known
            .iter()
            .filter(|(_, _, k)| Some(k) != home.as_ref() && path.starts_with(k))
            .max_by_key(|(_, _, k)| k.as_os_str().len())
            .map(|(_, _, k)| k.clone());
        let mut out = Vec::new();
        let mut cur = Some(path);
        while let Some(c) = cur {
            out.push((c.to_path_buf(), self.loc_name(c)));
            if Some(c) == base.as_deref() {
                break;
            }
            cur = c.parent();
        }
        if base.is_none() {
            out.push((
                PathBuf::from(THIS_PC),
                crate::i18n::t("Bu bilgisayar").into(),
            ));
        }
        out.reverse();
        out
    }

    /// Bir parçanın yanındaki ok menüsünde gösterilecek alt öğeler
    fn crumb_children(&mut self, p: &Path) -> Vec<(PathBuf, String)> {
        if p.as_os_str().is_empty() {
            let mut v = vec![(
                PathBuf::from(THIS_PC),
                crate::i18n::t("Bu bilgisayar").to_string(),
            )];
            v.extend(
                self.known
                    .iter()
                    .map(|(_, n, k)| (k.clone(), n.to_string())),
            );
            return v;
        }
        if p.as_os_str() == THIS_PC {
            return self
                .drives
                .lock()
                .unwrap()
                .iter()
                .map(|d| (d.root.clone(), d.label.clone()))
                .collect();
        }
        self.subdirs_cached(p)
            .iter()
            .map(|s| (s.clone(), path_label(s)))
            .collect()
    }

    fn address_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let p = pal_of(ui);
        let frame = egui::Frame::new()
            .fill(p.field)
            .stroke(p.field_stroke)
            .corner_radius(4.0)
            .inner_margin(egui::Margin::symmetric(6, 0));

        if self.address_edit.is_some() {
            let mut navigate_to: Option<PathBuf> = None;
            let mut entered: Option<String> = None;
            let mut close = false;
            let out = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.set_height(32.0);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let text = self.address_edit.as_mut().unwrap();
                    let out = egui::TextEdit::singleline(text)
                        .frame(false)
                        .desired_width(f32::INFINITY)
                        .id(egui::Id::new("address"))
                        .show(ui);
                    if self.address_focus {
                        out.response.request_focus();
                        let mut st = out.state.clone();
                        let n = text.chars().count();
                        st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(n),
                        )));
                        st.store(ui.ctx(), out.response.id);
                        self.address_focus = false;
                    }
                    if out.response.lost_focus() {
                        if ui.input(|i| i.key_pressed(Key::Enter)) {
                            entered = Some(text.trim().trim_matches('"').to_string());
                        }
                        close = true;
                    }
                    out.response
                })
                .inner
            });
            let field = out.inner;
            if let Some(s) = entered {
                // "cmd", "powershell", "terminal"…: aynı adda bir alt klasör yoksa o klasörde terminal aç
                let dir = if self.tab().is_home() {
                    dirs::home_dir().unwrap_or_default()
                } else {
                    self.tab().path.clone()
                };
                let (name, args) = s
                    .split_once(char::is_whitespace)
                    .map_or((s.as_str(), ""), |(n, a)| (n, a.trim()));
                if win::is_terminal(name) && !dir.join(&s).is_dir() {
                    actions.push(Action::RunTerminal {
                        name: name.to_string(),
                        args: args.to_string(),
                        dir,
                    });
                } else {
                    navigate_to = Some(expand_path(&s));
                }
            }

            // Otomatik tamamlama: yazılan yolun alt klasörleri
            let text = self.address_edit.clone().unwrap_or_default();
            let (dir, prefix) = if text.ends_with('\\') || text.ends_with('/') {
                (PathBuf::from(&text), String::new())
            } else {
                let p = PathBuf::from(&text);
                (
                    p.parent().map(Path::to_path_buf).unwrap_or_default(),
                    p.file_name()
                        .map(|n| n.to_string_lossy().to_lowercase())
                        .unwrap_or_default(),
                )
            };
            // Klasör değilse arka plandaki okuma boş liste döner; burada diske sorulmaz, ağ yolunda takılmasın
            if field.has_focus() && !dir.as_os_str().is_empty() {
                let subs = self.subdirs_cached(&dir);
                let matches: Vec<PathBuf> = subs
                    .iter()
                    .filter(|p| path_label(p).to_lowercase().starts_with(&prefix))
                    .take(12)
                    .cloned()
                    .collect();
                if !matches.is_empty()
                    && !(matches.len() == 1 && path_label(&matches[0]).to_lowercase() == prefix)
                {
                    if ui.input_mut(|i| shortcut(i, Modifiers::NONE, Key::Tab)) {
                        let full = format!("{}{}", matches[0].display(), std::path::MAIN_SEPARATOR);
                        self.address_edit = Some(full.clone());
                        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), field.id) {
                            let n = full.chars().count();
                            st.cursor.set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(n),
                            )));
                            st.store(ui.ctx(), field.id);
                        }
                        field.request_focus();
                    }
                    egui::Area::new(egui::Id::new("addr_suggest"))
                        .fixed_pos(field.rect.left_bottom() + egui::vec2(-6.0, 6.0))
                        .order(egui::Order::Foreground)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.set_min_width(field.rect.width().min(500.0));
                                for m in &matches {
                                    let r = ui.add(
                                        egui::Button::new(icon_label(
                                            ui,
                                            "📁",
                                            Some(Color32::from_rgb(232, 184, 74)),
                                            &m.display().to_string(),
                                        ))
                                        .frame(false),
                                    );
                                    if r.is_pointer_button_down_on() || r.clicked() {
                                        navigate_to = Some(m.clone());
                                        close = true;
                                    }
                                }
                                ui.label(
                                    RichText::new(crate::i18n::t("Tab: tamamla · Enter: git"))
                                        .small()
                                        .weak(),
                                );
                            });
                        });
                }
            }
            if let Some(p) = navigate_to {
                actions.push(Action::Navigate(p));
            }
            if close {
                self.address_edit = None;
            }
            return;
        }

        // Kırıntılar (breadcrumb): her parçanın sağındaki ok, alt klasör menüsünü açar; sığmayanlar « menüsüne
        let path = self.tabs[self.active].path.clone();
        let crumbs = self.crumbs(&path);
        let mut open_menu: Option<(PathBuf, Pos2)> = None;
        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(32.0);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(4.0);
                if flat_icon(ui, g::PC, egui::vec2(30.0, 26.0), 16.0, true)
                    .on_hover_text(crate::i18n::t("Bu bilgisayar"))
                    .clicked()
                {
                    actions.push(Action::Navigate(PathBuf::from(THIS_PC)));
                }
                let chev = |ui: &mut egui::Ui, id: &Path| -> egui::Response {
                    let r = flat_icon(ui, g::CHEVRON_RIGHT, egui::vec2(24.0, 26.0), 10.0, true);
                    let _ = id;
                    r
                };
                let r = chev(ui, Path::new(""));
                if r.clicked() {
                    open_menu = Some((PathBuf::new(), r.rect.left_bottom()));
                }
                // Sığanları sondan say
                let font = ui_font(14.0);
                let avail = ui.available_width() - 40.0;
                let mut used = 0.0;
                let mut first_visible = crumbs.len();
                for (k, (_, label)) in crumbs.iter().enumerate().rev() {
                    let w = ui.fonts(|f| {
                        f.layout_no_wrap(label.clone(), font.clone(), Color32::WHITE)
                            .size()
                            .x
                    }) + 16.0
                        + 24.0;
                    if used + w > avail && k + 1 < crumbs.len() {
                        break;
                    }
                    used += w;
                    first_visible = k;
                }
                if first_visible > 0 {
                    ui.menu_button(RichText::new(" « "), |ui| {
                        for (c, label) in crumbs[..first_visible].iter().rev() {
                            if ui.button(label).clicked() {
                                actions.push(Action::Navigate(c.clone()));
                                ui.close_menu();
                            }
                        }
                    });
                }
                for (c, label) in crumbs.iter().skip(first_visible) {
                    let galley =
                        ui.fonts(|f| f.layout_no_wrap(label.clone(), font.clone(), p.text));
                    let (rect, r) = ui.allocate_exact_size(
                        egui::vec2(galley.size().x + 16.0, 26.0),
                        Sense::click(),
                    );
                    if r.hovered() {
                        ui.painter().rect_filled(rect, 4.0, p.hover);
                    }
                    ui.painter()
                        .galley(rect.center() - galley.size() / 2.0, galley, p.text);
                    if r.clicked() {
                        actions.push(Action::Navigate(c.clone()));
                    }
                    if r.middle_clicked() {
                        actions.push(Action::NewTab(c.clone()));
                    }
                    if !crate::tab::is_virtual(c) {
                        drop_target(ui, &r, c, accent, actions);
                    }
                    let cr = chev(ui, c);
                    if cr.clicked() {
                        open_menu = Some((c.clone(), cr.rect.left_bottom()));
                    }
                }
                let rest =
                    ui.allocate_response(egui::vec2(ui.available_width(), 26.0), Sense::click());
                if rest.clicked() {
                    self.address_edit = Some(if crate::tab::is_virtual(&path) {
                        String::new()
                    } else {
                        path.to_string_lossy().into_owned()
                    });
                    self.address_focus = true;
                }
                rest.on_hover_text(crate::i18n::t("Yolu yazmak için tıklayın (Ctrl+L)"));
            });
        });

        // Ok menüsü: alt klasörler
        let menu_id = egui::Id::new("crumb_menu");
        let menu_just_opened = open_menu.is_some();
        if let Some((target, pos)) = open_menu {
            ui.ctx().data_mut(|d| d.insert_temp(menu_id, (target, pos)));
        }
        if let Some((target, pos)) = ui.ctx().data(|d| d.get_temp::<(PathBuf, Pos2)>(menu_id)) {
            let items = self.crumb_children(&target);
            let mut close = false;
            let area = egui::Area::new(menu_id)
                .fixed_pos(pos + egui::vec2(0.0, 4.0))
                .order(egui::Order::Foreground)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(200.0);
                        egui::ScrollArea::vertical()
                            .max_height(420.0)
                            .show(ui, |ui| {
                                for (s, label) in items.iter().take(500) {
                                    let r = ui
                                        .horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 8.0;
                                            paint_icon(
                                                ui,
                                                &mut self.images,
                                                &Self::loc_icon(s),
                                                true,
                                                "",
                                                false,
                                                16.0,
                                                "📁",
                                            );
                                            ui.add(egui::Button::new(label.as_str()).frame(false))
                                        })
                                        .inner;
                                    if r.clicked() {
                                        actions.push(Action::Navigate(s.clone()));
                                        close = true;
                                    }
                                }
                                if items.is_empty() {
                                    ui.label(
                                        RichText::new(crate::i18n::t("Alt klasör yok")).weak(),
                                    );
                                }
                            });
                    });
                });
            let clicked_outside = ui.input(|i| i.pointer.any_pressed())
                && !area.response.contains_pointer()
                && !menu_just_opened;
            if close || clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
                ui.ctx().data_mut(|d| d.remove::<(PathBuf, Pos2)>(menu_id));
            }
        }
    }

    fn command_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let (home, in_search, has_sel, cur) = {
            let t = &self.tabs[self.active];
            (
                t.is_home(),
                t.search.is_some(),
                !t.selected_entries().is_empty(),
                t.path.clone(),
            )
        };
        let can_paste = !home && !in_search;
        let p = pal_of(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(32.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
            ui.spacing_mut().interact_size.y = (32.0 * text_factor()).round();
            let sep = |ui: &mut egui::Ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 24.0), Sense::hover());
                ui.painter()
                    .vline(r.center().x, r.y_range(), Stroke::new(1.0_f32, p.line));
            };
            let tool = |ui: &mut egui::Ui, glyph: &str, enabled: bool, tip: &str| {
                flat_icon(ui, glyph, egui::vec2(40.0, 32.0), 16.0, enabled)
                    .on_hover_text(tip)
                    .clicked()
            };
            // Metinli menü düğmesi (Yeni ⌄, Sırala ⌄, Görünüm ⌄)
            let menu_btn =
                |ui: &mut egui::Ui, glyph: &str, text: &str, chevron: bool, enabled: bool| {
                    let job = glyph_job(ui, glyph, None, text, chevron);
                    egui::Button::new(job)
                        .frame(false)
                        .min_size(egui::vec2(0.0, 32.0))
                        .sense(if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        })
                };

            ui.add_enabled_ui(can_paste, |ui| {
                let b = menu_btn(ui, g::NEW, crate::i18n::t("Yeni"), true, can_paste);
                egui::menu::menu_custom_button(ui, b, |ui| {
                    for (i, k) in newfile::KINDS.iter().enumerate() {
                        let label = if i == 0 {
                            crate::i18n::t("Klasör          Ctrl+Shift+N")
                        } else {
                            crate::i18n::t(k.label_key)
                        };
                        if ui.button(icon_label(ui, k.emoji, k.tint, &label)).clicked() {
                            actions.push(Action::New(i));
                            ui.close_menu();
                        }
                    }
                });
            });
            sep(ui);
            if tool(ui, g::CUT, has_sel, crate::i18n::t("Kes (Ctrl+X)")) {
                actions.push(Action::Cut);
            }
            if tool(ui, g::COPY, has_sel, crate::i18n::t("Kopyala (Ctrl+C)")) {
                actions.push(Action::Copy);
            }
            if tool(ui, g::PASTE, can_paste, crate::i18n::t("Yapıştır (Ctrl+V)")) {
                actions.push(Action::Paste);
            }
            if tool(
                ui,
                g::RENAME,
                has_sel,
                crate::i18n::t("Yeniden adlandır (F2)"),
            ) {
                actions.push(Action::Rename);
            }
            if tool(
                ui,
                g::LINK,
                !home,
                crate::i18n::t("Yolu kopyala (Ctrl+Shift+C)"),
            ) {
                actions.push(Action::CopyPath);
            }
            if tool(
                ui,
                g::DELETE,
                has_sel,
                crate::i18n::t("Sil (Del)  ·  Kalıcı sil: Shift+Del"),
            ) {
                actions.push(Action::Delete { permanent: false });
            }
            sep(ui);

            let sort_names = [
                (SortKey::Name, crate::i18n::t("Ad")),
                (SortKey::Modified, crate::i18n::t("Değiştirme tarihi")),
                (SortKey::Type, crate::i18n::t("Tür")),
                (SortKey::Size, crate::i18n::t("Boyut")),
            ];
            let b = menu_btn(ui, g::SORT, crate::i18n::t("Sırala"), true, true);
            egui::menu::menu_custom_button(ui, b, |ui| {
                for (k, n) in sort_names {
                    if ui.radio(self.sort == k, n).clicked() {
                        if self.sort != k {
                            actions.push(Action::Sort(k));
                        }
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.radio(self.asc, crate::i18n::t("Artan")).clicked() {
                    self.asc = true;
                    self.remember_view();
                    self.mark_views_dirty();
                    ui.close_menu();
                }
                if ui.radio(!self.asc, crate::i18n::t("Azalan")).clicked() {
                    self.asc = false;
                    self.remember_view();
                    self.mark_views_dirty();
                    ui.close_menu();
                }
                ui.separator();
                ui.add_enabled_ui(!home, |ui| {
                    ui.menu_button(crate::i18n::t("Grupla"), |ui| {
                        let on = self.grouped.contains(&cur);
                        if ui.radio(on, crate::i18n::t("Değiştirme tarihi")).clicked() {
                            if !on {
                                self.grouped.push(cur.clone());
                            }
                            self.remember_view();
                            self.mark_views_dirty();
                            ui.close_menu();
                        }
                        if ui.radio(!on, crate::i18n::t("(Yok)")).clicked() {
                            self.grouped.retain(|g| g != &cur);
                            self.remember_view();
                            self.mark_views_dirty();
                            ui.close_menu();
                        }
                    });
                });
            });
            let view_glyph = if self.view_level == 0 {
                g::LIST
            } else {
                g::TILES
            };
            let b = menu_btn(ui, view_glyph, crate::i18n::t("Görünüm"), true, true);
            let view_resp = ui.add(b);
            if view_resp.clicked() {
                if self.menu.is_some() {
                    // Açık menüyü kapat (tekrar tıklama, sağ tık menü dahil)
                    self.menu = None;
                } else {
                    let mut acts = Vec::new();
                    let m = self.build_view_menu(view_resp.rect.left_bottom(), &mut acts);
                    self.menu = Some(MenuState {
                        menu: m,
                        actions: acts,
                        target: MenuTarget::Background {
                            folder: cur.clone(),
                            in_search: false,
                        },
                    });
                }
            }
            sep(ui);
            let more = egui::Button::new(fl(g::MORE, 16.0))
                .frame(false)
                .min_size(egui::vec2(40.0, 32.0));
            egui::menu::menu_custom_button(ui, more, |ui| {
                ui.add_enabled_ui(!home, |ui| {
                    if ui
                        .button(crate::i18n::t("Tümünü seç            Ctrl+A"))
                        .clicked()
                    {
                        actions.push(Action::SelectAll);
                        ui.close_menu();
                    }
                    if ui
                        .button(crate::i18n::t("Hiçbirini seçme        Esc"))
                        .clicked()
                    {
                        actions.push(Action::SelectNone);
                        ui.close_menu();
                    }
                    if ui.button(crate::i18n::t("Seçimi tersine çevir")).clicked() {
                        actions.push(Action::InvertSelection);
                        ui.close_menu();
                    }
                });
                ui.separator();
                if ui
                    .add_enabled(
                        !self.undo.is_empty(),
                        egui::Button::new(crate::i18n::t("Geri al                   Ctrl+Z")),
                    )
                    .clicked()
                {
                    actions.push(Action::Undo);
                    ui.close_menu();
                }
                ui.add_enabled_ui(!home, |ui| {
                    if ui.button(crate::i18n::t("Terminalde aç")).clicked() {
                        actions.push(Action::Terminal(cur.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    ui.add_enabled_ui(has_sel, |ui| {
                        let paths = self.tabs[self.active].selected_paths();
                        if ui.button(crate::i18n::t("Şuraya kopyala…")).clicked() {
                            actions.push(Action::PickDest {
                                paths: paths.clone(),
                                cut: false,
                            });
                            ui.close_menu();
                        }
                        if ui.button(crate::i18n::t("Şuraya taşı…")).clicked() {
                            actions.push(Action::PickDest { paths, cut: true });
                            ui.close_menu();
                        }
                    });
                    if ui
                        .button(crate::i18n::t("Klasör boyutlarını hesapla"))
                        .clicked()
                    {
                        actions.push(Action::CalcSize);
                        ui.close_menu();
                    }
                    if ui
                        .button(crate::i18n::t("Hızlı erişime sabitle   Ctrl+D"))
                        .clicked()
                    {
                        actions.push(Action::AddBookmark(cur.clone()));
                        ui.close_menu();
                    }
                    let open_label = if cfg!(windows) {
                        "Explorer'da aç"
                    } else {
                        "Dosya yöneticisinde aç"
                    };
                    if ui.button(crate::i18n::t(open_label)).clicked() {
                        #[cfg(windows)]
                        let _ = std::process::Command::new("explorer.exe").arg(&cur).spawn();
                        #[cfg(not(windows))]
                        let _ = open::that_detached(&cur);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .button(crate::i18n::t("Özellikler              Alt+Enter"))
                        .clicked()
                    {
                        let target = self.tabs[self.active]
                            .selected_paths()
                            .into_iter()
                            .next()
                            .unwrap_or(cur.clone());
                        actions.push(Action::Properties(target));
                        ui.close_menu();
                    }
                });
                if ui
                    .button(crate::i18n::t("Klavye kısayolları      F1"))
                    .clicked()
                {
                    self.dialog = Dialog::Help;
                    ui.close_menu();
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // Ayrıntılar bölmesi (sağ yan bölme)
                let mut b = menu_btn(ui, g::PANE, crate::i18n::t("Ayrıntılar"), false, true);
                if self.pane == 2 {
                    b = b.fill(p.nav_sel);
                }
                let details = ui.add(b);
                if details
                    .on_hover_text(crate::i18n::t("Ayrıntılar bölmesi"))
                    .clicked()
                {
                    self.pane = if self.pane == 2 { 0 } else { 2 };
                }
                // Önizleme bölmesi
                let mut b = menu_btn(ui, g::VIEW, crate::i18n::t("Önizleme"), false, true);
                if self.pane == 1 {
                    b = b.fill(p.nav_sel);
                }
                let prev = ui.add(b);
                if prev
                    .on_hover_text(crate::i18n::t("Önizleme bölmesi"))
                    .clicked()
                {
                    self.pane = if self.pane == 1 { 0 } else { 1 };
                }
                if in_search {
                    sep(ui);
                    let b = menu_btn(ui, g::CANCEL, crate::i18n::t("Aramayı kapat"), false, true);
                    if ui.add(b).clicked() {
                        actions.push(Action::ExitSearch);
                    }
                }
            });
        });
    }

    /// Gezinti bölmesi satırı (Explorer ölçüleri: 24 px yükseklik; açma oku, ikon, ad, sağda iğne).
    /// Dönüş: (satır yanıtı, ok tıklandı mı)
    #[allow(clippy::too_many_arguments)]
    fn nav_row(
        &mut self,
        ui: &mut egui::Ui,
        depth: usize,
        expand: Option<bool>,
        icon: &Path,
        label: &str,
        selected: bool,
        pinned: bool,
    ) -> (egui::Response, bool) {
        let p = pal_of(ui);
        // Satır yüksekliği yazı boyutundan türer: 19 px metinde 34 px (Explorer'la aynı
        // ferahlık), 12 px metinde 26 px. Sabit 24 px metni sıkıştırıyordu.
        let row_h = text_size(14.0) + 15.0;
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
        let painter = ui.painter().clone();
        let bg = rect.shrink2(egui::vec2(3.0, 0.0));
        if selected {
            painter.rect_filled(bg, 4.0, p.nav_sel);
        } else if resp.hovered() {
            painter.rect_filled(bg, 4.0, p.hover);
        }
        let x0 = rect.left() + 16.0 * depth as f32;
        let mut toggled = false;
        if let Some(open) = expand {
            let cr = Rect::from_center_size(
                egui::pos2(x0 + 13.0, rect.center().y),
                egui::vec2(18.0, 24.0),
            );
            let r = ui.interact(cr, ui.id().with(("nav_tog", icon, depth)), Sense::click());
            toggled = r.clicked();
            let c = if r.hovered() { p.text } else { p.weak };
            painter.text(
                cr.center(),
                Align2::CENTER_CENTER,
                if open {
                    g::CHEVRON_DOWN
                } else {
                    g::CHEVRON_RIGHT
                },
                fluent_font(9.0),
                c,
            );
        }
        let icon_rect =
            Rect::from_center_size(egui::pos2(x0 + 34.0, rect.center().y), Vec2::splat(16.0));
        draw_icon(&painter, &mut self.images, icon, true, "", icon_rect);
        let right = if pinned {
            rect.right() - 24.0
        } else {
            rect.right() - 6.0
        };
        let mut job =
            egui::text::LayoutJob::simple_singleline(label.to_string(), ui_font(14.0), p.text);
        job.wrap = egui::text::TextWrapping::truncate_at_width((right - (x0 + 46.0)).max(10.0));
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(x0 + 46.0, rect.center().y - galley.size().y / 2.0),
            galley,
            p.text,
        );
        if pinned {
            painter.text(
                egui::pos2(rect.right() - 14.0, rect.center().y),
                Align2::CENTER_CENTER,
                g::PIN,
                fluent_font(12.0),
                p.weak,
            );
        }
        (resp, toggled)
    }

    fn tree_node(
        &mut self,
        ui: &mut egui::Ui,
        depth: usize,
        path: &Path,
        label: &str,
        actions: &mut Vec<Action>,
    ) {
        let current = self.tabs[self.active].path.clone();
        let accent = self.accent;
        let open = self.tree_open.contains(path);
        let (r, toggled) = self.nav_row(ui, depth, Some(open), path, label, current == path, false);
        if toggled || r.double_clicked() {
            if open {
                self.tree_open.remove(path);
            } else {
                self.tree_open.insert(path.to_path_buf());
            }
        }
        if r.clicked() {
            actions.push(Action::Navigate(path.to_path_buf()));
        }
        if r.middle_clicked() {
            actions.push(Action::NewTab(path.to_path_buf()));
        }
        drop_target(ui, &r, path, accent, actions);
        let p = path.to_path_buf();
        r.on_hover_text(path.display().to_string())
            .context_menu(|ui| {
                if ui.button(crate::i18n::t("Aç")).clicked() {
                    actions.push(Action::Navigate(p.clone()));
                    ui.close_menu();
                }
                if ui.button(crate::i18n::t("Yeni sekmede aç")).clicked() {
                    actions.push(Action::NewTab(p.clone()));
                    ui.close_menu();
                }
                if ui.button(crate::i18n::t("Hızlı erişime sabitle")).clicked() {
                    actions.push(Action::AddBookmark(p.clone()));
                    ui.close_menu();
                }
                if ui.button(crate::i18n::t("Özellikler")).clicked() {
                    actions.push(Action::Properties(p.clone()));
                    ui.close_menu();
                }
            });
        if open {
            let children = self.subdirs_cached(path);
            for c in children.iter().take(2000) {
                let l = path_label(c);
                self.tree_node(ui, depth + 1, c, &l, actions);
            }
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let current = self.tabs[self.active].path.clone();
        let line = pal_of(ui).line;
        let separator = |ui: &mut egui::Ui| {
            let (r, _) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), (18.0 * text_factor()).round()),
                Sense::hover(),
            );
            ui.painter().hline(
                r.left() + 12.0..=r.right() - 12.0,
                r.center().y,
                Stroke::new(1.0_f32, line),
            );
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = (2.0 * text_factor()).round();
                ui.add_space(6.0 * text_factor());
                let (r, _) = self.nav_row(
                    ui,
                    0,
                    None,
                    Path::new(HOME_SHELL),
                    crate::i18n::t("Giriş"),
                    current.as_os_str().is_empty(),
                    false,
                );
                if r.clicked() {
                    actions.push(Action::Navigate(PathBuf::new()));
                }
                let gallery = PathBuf::from(crate::tab::GALLERY);
                let (r, _) = self.nav_row(
                    ui,
                    0,
                    None,
                    &gallery,
                    crate::i18n::t("Galeri"),
                    current == gallery,
                    false,
                );
                if r.clicked() {
                    actions.push(Action::Navigate(gallery.clone()));
                }
                if r.middle_clicked() {
                    actions.push(Action::NewTab(gallery.clone()));
                }
                separator(ui);

                // Hızlı erişim: sabitlenmiş klasörler (iğneli) + sık kullanılanlar
                let mut items: Vec<(PathBuf, String, bool)> = self
                    .known
                    .iter()
                    .map(|(_, n, p)| (p.clone(), n.to_string(), true))
                    .collect();
                for b in &self.bookmarks {
                    items.push((b.clone(), path_label(b), true));
                }
                let frequent: Vec<PathBuf> = self
                    .recent
                    .iter()
                    .filter(|p| {
                        !crate::tab::is_virtual(p)
                            && p.parent().is_some()
                            && !items.iter().any(|(q, _, _)| q == *p)
                            && Some(*p) != dirs::home_dir().as_ref()
                    })
                    .take(4)
                    .cloned()
                    .collect();
                items.extend(frequent.into_iter().map(|p| {
                    let n = path_label(&p);
                    (p, n, false)
                }));
                let pinned_bookmarks = self.bookmarks.clone();
                for (p, name, pinned) in items {
                    let (r, _) = self.nav_row(ui, 0, None, &p, &name, current == p, pinned);
                    if r.clicked() {
                        actions.push(Action::Navigate(p.clone()));
                    }
                    if r.middle_clicked() {
                        actions.push(Action::NewTab(p.clone()));
                    }
                    drop_target(ui, &r, &p, accent, actions);
                    let is_bookmark = pinned_bookmarks.contains(&p);
                    r.on_hover_text(p.display().to_string()).context_menu(|ui| {
                        if ui.button(crate::i18n::t("Yeni sekmede aç")).clicked() {
                            actions.push(Action::NewTab(p.clone()));
                            ui.close_menu();
                        }
                        if is_bookmark {
                            if ui
                                .button(crate::i18n::t("Hızlı erişimden kaldır"))
                                .clicked()
                            {
                                actions.push(Action::RemoveBookmark(p.clone()));
                                ui.close_menu();
                            }
                        } else if !pinned {
                            if ui.button(crate::i18n::t("Hızlı erişime sabitle")).clicked() {
                                actions.push(Action::AddBookmark(p.clone()));
                                ui.close_menu();
                            }
                        }
                        if ui.button(crate::i18n::t("Özellikler")).clicked() {
                            actions.push(Action::Properties(p.clone()));
                            ui.close_menu();
                        }
                    });
                }
                separator(ui);

                let pc = PathBuf::from(THIS_PC);
                let open = self.tree_open.contains(&pc);
                let (r, toggled) = self.nav_row(
                    ui,
                    0,
                    Some(open),
                    &pc,
                    crate::i18n::t("Bu bilgisayar"),
                    current == pc,
                    false,
                );
                if toggled || r.double_clicked() {
                    if open {
                        self.tree_open.remove(&pc);
                    } else {
                        self.tree_open.insert(pc.clone());
                    }
                }
                if r.clicked() {
                    actions.push(Action::Navigate(pc.clone()));
                }
                if open {
                    let drives = self.drives.lock().unwrap().clone();
                    for d in drives {
                        self.tree_node(ui, 1, &d.root, &d.label, actions);
                    }
                }
                if cfg!(windows) {
                    let (r, toggled) = self.nav_row(
                        ui,
                        0,
                        Some(false),
                        Path::new(NETWORK_SHELL),
                        crate::i18n::t("Ağ"),
                        false,
                        false,
                    );
                    if r.clicked() || toggled {
                        let _ = std::process::Command::new("explorer.exe")
                            .arg(format!("shell:{NETWORK_SHELL}"))
                            .spawn();
                    }
                    r.on_hover_text(crate::i18n::t("Ağ, Windows Dosya Gezgini'nde açılır"));
                }
                // Windows 11'deki "Linux" düğümü: kurulu WSL dağıtımları
                if cfg!(windows) {
                    let linux = crate::wsl::distros();
                    if !linux.is_empty() {
                        separator(ui);
                        let key = Path::new(r"\\wsl.localhost");
                        let open = self.tree_open.contains(key);
                        let (r, toggled) = self.nav_row(
                            ui,
                            0,
                            Some(open),
                            &linux[0].root,
                            crate::i18n::t("Linux"),
                            false,
                            false,
                        );
                        if toggled || r.clicked() {
                            if open {
                                self.tree_open.remove(key);
                            } else {
                                self.tree_open.insert(key.to_path_buf());
                            }
                        }
                        r.on_hover_text(key.display().to_string());
                        if open {
                            for d in &linux {
                                let (r, _) = self.nav_row(
                                    ui,
                                    1,
                                    None,
                                    &d.root,
                                    &d.name,
                                    current == d.root,
                                    false,
                                );
                                if r.clicked() {
                                    actions.push(Action::Navigate(d.root.clone()));
                                }
                                if r.middle_clicked() {
                                    actions.push(Action::NewTab(d.root.clone()));
                                }
                                r.on_hover_text(d.root.display().to_string());
                            }
                        }
                    }
                }
                ui.add_space(8.0);
            });
    }

    /// Açılıp kapanabilen grup başlığı (Giriş/Bu bilgisayar sayfaları için)
    fn page_group(&mut self, ui: &mut egui::Ui, key: &str, label: &str) -> bool {
        let id = egui::Id::new(("page_group", key));
        let open = !ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
        if group_header(
            ui,
            rect.with_max_x(rect.left() + 260.0),
            label,
            open,
            id.with("h"),
        )
        .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(id, open));
        }
        open
    }

    fn home_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let p = pal_of(ui);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                if self.page_group(ui, "quick", crate::i18n::t("Hızlı erişim")) {
                    let mut tiles: Vec<(PathBuf, String, bool)> = self
                        .known
                        .iter()
                        .map(|(_, n, p)| (p.clone(), n.to_string(), false))
                        .collect();
                    tiles.extend(
                        self.bookmarks
                            .iter()
                            .map(|b| (b.clone(), path_label(b), true)),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        ui.add_space(10.0);
                        for (path, name, pinned) in tiles {
                            let (rect, resp) =
                                ui.allocate_exact_size(egui::vec2(250.0, 62.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(rect, 4.0, p.hover);
                            }
                            let icon = Rect::from_min_size(
                                rect.left_top() + egui::vec2(8.0, 7.0),
                                Vec2::splat(48.0),
                            );
                            draw_icon(ui.painter(), &mut self.images, &path, true, "", icon);
                            let x = icon.right() + 10.0;
                            ui.painter().text(
                                egui::pos2(x, rect.center().y - 9.0),
                                Align2::LEFT_CENTER,
                                short(&name, 26),
                                ui_font(14.0),
                                p.text,
                            );
                            let sub = if pinned {
                                crate::i18n::t("Sabitlendi").to_string()
                            } else {
                                crate::i18n::t("Bu bilgisayar").to_string()
                            };
                            ui.painter().text(
                                egui::pos2(x, rect.center().y + 10.0),
                                Align2::LEFT_CENTER,
                                sub,
                                ui_font(12.0),
                                p.weak,
                            );
                            if resp.clicked() || resp.double_clicked() {
                                actions.push(Action::Navigate(path.clone()));
                            }
                            if resp.middle_clicked() {
                                actions.push(Action::NewTab(path.clone()));
                            }
                            drop_target(ui, &resp, &path, accent, actions);
                            resp.on_hover_text(path.display().to_string())
                                .context_menu(|ui| {
                                    if ui.button(crate::i18n::t("Yeni sekmede aç")).clicked() {
                                        actions.push(Action::NewTab(path.clone()));
                                        ui.close_menu();
                                    }
                                    if pinned
                                        && ui
                                            .button(crate::i18n::t("Hızlı erişimden kaldır"))
                                            .clicked()
                                    {
                                        actions.push(Action::RemoveBookmark(path.clone()));
                                        ui.close_menu();
                                    }
                                });
                        }
                    });
                    ui.add_space(10.0);
                }

                let recent: Vec<PathBuf> = self
                    .recent
                    .iter()
                    .filter(|p| !crate::tab::is_virtual(p))
                    .take(20)
                    .cloned()
                    .collect();
                if !recent.is_empty() && self.page_group(ui, "recent", crate::i18n::t("Son")) {
                    for path in &recent {
                        let (rect, r) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 30.0),
                            Sense::click(),
                        );
                        if r.hovered() {
                            ui.painter().rect_filled(
                                rect.shrink2(egui::vec2(4.0, 0.0)),
                                4.0,
                                p.hover,
                            );
                        }
                        let icon = Rect::from_center_size(
                            egui::pos2(rect.left() + 30.0, rect.center().y),
                            Vec2::splat(20.0),
                        );
                        draw_icon(ui.painter(), &mut self.images, path, true, "", icon);
                        ui.painter().text(
                            egui::pos2(rect.left() + 50.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            self.loc_name(path),
                            ui_font(14.0),
                            p.text,
                        );
                        let parent = path
                            .parent()
                            .map(|x| x.display().to_string())
                            .unwrap_or_default();
                        ui.painter().text(
                            egui::pos2(
                                rect.left() + 50.0 + (rect.width() * 0.4).max(260.0),
                                rect.center().y,
                            ),
                            Align2::LEFT_CENTER,
                            parent,
                            ui_font(14.0),
                            p.weak,
                        );
                        if r.clicked() || r.double_clicked() {
                            actions.push(Action::Navigate(path.clone()));
                        }
                        if r.middle_clicked() {
                            actions.push(Action::NewTab(path.clone()));
                        }
                        let pp = path.clone();
                        r.context_menu(|ui| {
                            if ui.button(crate::i18n::t("Yeni sekmede aç")).clicked() {
                                actions.push(Action::NewTab(pp.clone()));
                                ui.close_menu();
                            }
                            if ui.button(crate::i18n::t("Son listesini temizle")).clicked() {
                                self.recent.clear();
                                ui.close_menu();
                            }
                        });
                    }
                }
                ui.add_space(20.0);
            });
    }

    /// "Bu bilgisayar": Cihazlar ve sürücüler
    fn pc_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let p = pal_of(ui);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(6.0);
                if self.page_group(ui, "drives", crate::i18n::t("Cihazlar ve sürücüler")) {
                    // Yazarken sürücüleri süz; Enter tüm sürücülerde arar
                    let pat = fsutil::Pattern::new(&self.tabs[self.active].filter);
                    let drives: Vec<DriveInfo> = self
                        .drives
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|d| pat.matches(&d.label.to_lowercase()))
                        .cloned()
                        .collect();
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                        ui.add_space(10.0);
                        for d in drives {
                            let (rect, resp) =
                                ui.allocate_exact_size(egui::vec2(262.0, 54.0), Sense::click());
                            let sel = self.tabs[self.active].cursor_path.as_ref() == Some(&d.root);
                            if sel {
                                ui.painter().rect_filled(rect, 4.0, p.sel);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, 4.0, p.hover);
                            }
                            let icon = Rect::from_min_size(
                                rect.left_top() + egui::vec2(4.0, 3.0),
                                Vec2::splat(48.0),
                            );
                            draw_icon(ui.painter(), &mut self.images, &d.root, true, "", icon);
                            let x = icon.right() + 8.0;
                            ui.painter().text(
                                egui::pos2(x, rect.top() + 11.0),
                                Align2::LEFT_CENTER,
                                &d.label,
                                ui_font(13.0),
                                p.text,
                            );
                            if d.total > 0 {
                                let used = 1.0 - d.free as f32 / d.total as f32;
                                let bar = Rect::from_min_size(
                                    egui::pos2(x, rect.top() + 21.0),
                                    egui::vec2(190.0, 14.0),
                                );
                                let track = if ui.visuals().dark_mode {
                                    Color32::from_rgb(230, 230, 230)
                                } else {
                                    Color32::from_rgb(230, 230, 230)
                                };
                                ui.painter().rect_filled(bar, 0.0, track);
                                // Explorer: %90 dolu olunca kırmızı
                                let fill = if used > 0.9 {
                                    Color32::from_rgb(196, 43, 28)
                                } else {
                                    accent
                                };
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        bar.min,
                                        egui::vec2(
                                            bar.width() * used.clamp(0.0, 1.0),
                                            bar.height(),
                                        ),
                                    ),
                                    0.0,
                                    fill,
                                );
                                ui.painter().rect_stroke(
                                    bar,
                                    0.0,
                                    Stroke::new(1.0_f32, Color32::from_rgb(188, 188, 188)),
                                    egui::StrokeKind::Inside,
                                );
                                let info = crate::i18n::tf(
                                    "{} boş, {} toplam",
                                    &[&fsutil::format_size(d.free), &fsutil::format_size(d.total)],
                                );
                                ui.painter().text(
                                    egui::pos2(x, rect.top() + 45.0),
                                    Align2::LEFT_CENTER,
                                    info,
                                    ui_font(13.0),
                                    p.weak,
                                );
                            }
                            if resp.clicked() {
                                self.tabs[self.active].cursor_path = Some(d.root.clone());
                            }
                            if resp.double_clicked() {
                                actions.push(Action::Navigate(d.root.clone()));
                            }
                            if resp.middle_clicked() {
                                actions.push(Action::NewTab(d.root.clone()));
                            }
                            drop_target(ui, &resp, &d.root, accent, actions);
                            let root = d.root.clone();
                            resp.context_menu(|ui| {
                                if ui.button(crate::i18n::t("Aç")).clicked() {
                                    actions.push(Action::Navigate(root.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(crate::i18n::t("Yeni sekmede aç")).clicked() {
                                    actions.push(Action::NewTab(root.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(crate::i18n::t("Özellikler")).clicked() {
                                    actions.push(Action::Properties(root.clone()));
                                    ui.close_menu();
                                }
                            });
                        }
                    });
                }
                ui.add_space(20.0);
            });
    }

    /// Modern (Windows 11 tarzı) sağ tık menüsünü kurar: uygulama komutları + gerçek
    /// kabuk komutları (terminal, arşiv, birlikte aç…) + "Daha fazla seçenek".
    fn open_context_menu(&mut self, ctx: &egui::Context, target: MenuTarget, pos: Pos2) {
        let mut m = Menu::new(pos);
        let mut acts: Vec<Action> = Vec::new();
        let mut shell_paths: Vec<PathBuf> = Vec::new();
        match target.clone() {
            MenuTarget::Items {
                paths,
                is_dir,
                in_search,
                primary,
            } => {
                shell_paths = paths;
                let open_icon = if is_dir { g::FOLDER } else { g::DOCUMENT };
                app_node(
                    &mut acts,
                    &mut m,
                    "Aç",
                    open_icon,
                    Action::OpenSelected,
                    |n| n.shortcut("Enter"),
                );
                if is_dir {
                    app_node(
                        &mut acts,
                        &mut m,
                        "Yeni sekmede aç",
                        g::NEW_TAB,
                        Action::NewTab(primary.clone()),
                        |n| n,
                    );
                    app_node(
                        &mut acts,
                        &mut m,
                        "Terminalde aç",
                        g::TERMINAL,
                        Action::Terminal(primary.clone()),
                        |n| n,
                    );
                } else {
                    app_node(
                        &mut acts,
                        &mut m,
                        "Birlikte aç…",
                        g::OPEN_WITH,
                        Action::OpenWith(primary.clone()),
                        |n| n,
                    );
                }
                if in_search {
                    if let Some(parent) = primary.parent() {
                        app_node(
                            &mut acts,
                            &mut m,
                            "Dosya konumunu aç",
                            g::SHOW,
                            Action::NewTab(parent.to_path_buf()),
                            |n| n,
                        );
                    }
                }
                if is_dir {
                    app_node(
                        &mut acts,
                        &mut m,
                        "Boyutu hesapla",
                        g::CALC_SIZE,
                        Action::CalcSize,
                        |n| n.sep_before(),
                    );
                    app_node(
                        &mut acts,
                        &mut m,
                        "Hızlı erişime sabitle",
                        g::PIN,
                        Action::AddBookmark(primary.clone()),
                        |n| n,
                    );
                }
                let sel_paths = self.tabs[self.active].selected_paths();
                app_node(
                    &mut acts,
                    &mut m,
                    "Şuraya kopyala…",
                    g::COPY,
                    Action::PickDest {
                        paths: sel_paths.clone(),
                        cut: false,
                    },
                    |n| n.sep_before(),
                );
                app_node(
                    &mut acts,
                    &mut m,
                    "Şuraya taşı…",
                    g::CUT,
                    Action::PickDest {
                        paths: sel_paths,
                        cut: true,
                    },
                    |n| n,
                );
                m.quick = vec![
                    quick(&mut acts, Action::Cut, g::CUT, "Kes (Ctrl+X)", true),
                    quick(&mut acts, Action::Copy, g::COPY, "Kopyala (Ctrl+C)", true),
                    quick(
                        &mut acts,
                        Action::Rename,
                        g::RENAME,
                        "Yeniden adlandır (F2)",
                        true,
                    ),
                    quick(
                        &mut acts,
                        Action::Delete { permanent: false },
                        g::DELETE,
                        "Sil                               Del",
                        true,
                    ),
                ];
            }
            MenuTarget::Background { folder, in_search } => {
                let mut view = Node::new(crate::i18n::t("Görünüm")).with_icon(g::VIEW);
                for (i, (_, n)) in VIEW_LEVELS.iter().enumerate() {
                    let id = acts.len() as u32;
                    acts.push(Action::SetView(i));
                    view.children.push(
                        Node::new(crate::i18n::t(n))
                            .with_cmd(Cmd::App(id))
                            .checked(i == self.view_level),
                    );
                }
                m.nodes.push(view);
                let cur_sort = self.sort;
                let mut sort = Node::new(crate::i18n::t("Sırala")).with_icon(g::SORT);
                for (k, n) in [
                    (SortKey::Name, crate::i18n::t("Ad")),
                    (SortKey::Modified, crate::i18n::t("Değiştirme tarihi")),
                    (SortKey::Type, crate::i18n::t("Tür")),
                    (SortKey::Size, crate::i18n::t("Boyut")),
                ] {
                    let id = acts.len() as u32;
                    acts.push(Action::Sort(k));
                    sort.children
                        .push(Node::new(n).with_cmd(Cmd::App(id)).checked(cur_sort == k));
                }
                m.nodes.push(sort);
                app_node(
                    &mut acts,
                    &mut m,
                    "Yenile (F5)",
                    g::REFRESH,
                    Action::Refresh,
                    |n| n,
                );
                if !in_search {
                    let has_clip = self.clipboard.is_some();
                    app_node(
                        &mut acts,
                        &mut m,
                        "Yapıştır (Ctrl+V)",
                        g::PASTE,
                        Action::Paste,
                        move |n| {
                            let n = n.sep_before();
                            if has_clip {
                                n
                            } else {
                                n.disabled()
                            }
                        },
                    );
                    if !self.undo.is_empty() {
                        app_node(
                            &mut acts,
                            &mut m,
                            "Geri al                   Ctrl+Z",
                            g::UNDO,
                            Action::Undo,
                            |n| n,
                        );
                    }
                    let mut new = Node::new(crate::i18n::t("Yeni"))
                        .with_icon(g::ADD)
                        .sep_before();
                    for (i, k) in newfile::KINDS.iter().enumerate() {
                        let id = acts.len() as u32;
                        acts.push(Action::New(i));
                        let mut row = Node::new(crate::i18n::t(k.label_key))
                            .with_colored_icon(k.emoji, k.tint)
                            .with_cmd(Cmd::App(id));
                        if i == 0 {
                            row = row.shortcut("Ctrl+Shift+N");
                        }
                        new.children.push(row);
                    }
                    m.nodes.push(new);
                    app_node(
                        &mut acts,
                        &mut m,
                        "Terminalde aç",
                        g::TERMINAL,
                        Action::Terminal(folder.clone()),
                        |n| n,
                    );
                }
            }
        }
        if cfg!(windows) && !shell_paths.is_empty() {
            // Kabuk komutları (arşiv, üçüncü taraf, birlikte aç…) uygulama komutlarından sonra
            self.push_shell_nodes(ctx, &mut m, &shell_paths, None);
        }
        if let MenuTarget::Items { primary, .. } = &target {
            app_node(
                &mut acts,
                &mut m,
                "Yol olarak kopyala   Ctrl+Shift+C",
                g::COPY_PATH,
                Action::CopyPath,
                |n| n.sep_before(),
            );
            app_node(
                &mut acts,
                &mut m,
                "Özellikler",
                g::PROPERTIES,
                Action::Properties(primary.clone()),
                |n| n.shortcut("Alt+Enter"),
            );
        }
        if cfg!(windows) {
            // Son satır: Explorer'ın klasik menüsünü açar
            let (label, _) = menu::parts("Daha fazla seçenek göster  Shift+F10");
            m.nodes.push(
                Node::new(label)
                    .with_icon(g::MORE)
                    .with_cmd(Cmd::More)
                    .sep_before(),
            );
        }
        self.menu = Some(MenuState {
            menu: m,
            actions: acts,
            target,
        });
    }

    /// Komut çubuğu Görünüm düğmesinin menüsü: Explorer düzeninde satırlar, çizilmiş
    /// şema ikonları ve Göster/Tema/Dil alt menüleri. Komutlar `acts` içinde
    /// Cmd::App kimliğiyle saklanır; menü kapanınca apply ile yürütülür.
    fn build_view_menu(&self, pos: Pos2, acts: &mut Vec<Action>) -> Menu {
        let mut m = Menu::new(pos);
        // (görünüm düzeyi, şema ikonu, kısayol) — Explorer sırası
        for (lvl, mock, sc) in [
            (4usize, 0usize, "Ctrl+Shift+1"),
            (3, 1, "Ctrl+Shift+2"),
            (2, 2, "Ctrl+Shift+3"),
            (1, 3, "Ctrl+Shift+4"),
            (0, 5, "Ctrl+Shift+6"),
        ] {
            let (_, n) = VIEW_LEVELS[lvl];
            let id = acts.len() as u32;
            acts.push(Action::SetView(lvl));
            m.nodes.push(
                Node::new(crate::i18n::t(n))
                    .with_mock(mock)
                    .with_cmd(Cmd::App(id))
                    .shortcut(sc)
                    .checked(self.view_level == lvl),
            );
        }
        // Önizleme bölmesi
        let (label, sc) = menu::parts("Önizleme bölmesi (Alt+P)");
        let id = acts.len() as u32;
        acts.push(Action::TogglePreview);
        let mut prow = Node::new(label).with_cmd(Cmd::App(id));
        if let Some(s) = sc {
            prow = prow.shortcut(&s);
        }
        prow.checked = self.pane == 1;
        m.nodes.push(prow);
        // Ayrıntılar bölmesi
        let (label, sc) = menu::parts("Ayrıntılar bölmesi (Alt+Shift+P)");
        let id = acts.len() as u32;
        acts.push(Action::ToggleDetails);
        let mut drow = Node::new(label).with_cmd(Cmd::App(id));
        if let Some(s) = sc {
            drow = drow.shortcut(&s);
        }
        drow.checked = self.pane == 2;
        drow.sep = true;
        m.nodes.push(drow);
        // Göster alt menüsü: gizlilik ve uygulama ayarları
        let mut show = Node::new(crate::i18n::t("Göster"));
        let (label, sc) = menu::parts("Gizli öğeler (Ctrl+H)");
        let id = acts.len() as u32;
        acts.push(Action::ToggleHidden);
        let mut row = Node::new(label).with_cmd(Cmd::App(id));
        if let Some(s) = sc {
            row = row.shortcut(&s);
        }
        row.checked = self.show_hidden;
        show.children.push(row);
        let id = acts.len() as u32;
        acts.push(Action::ToggleExt);
        show.children.push(
            Node::new(crate::i18n::t("Dosya adı uzantıları"))
                .with_cmd(Cmd::App(id))
                .checked(self.show_ext),
        );
        let id = acts.len() as u32;
        acts.push(Action::ToggleRememberViews);
        let mut row = Node::new(crate::i18n::t("Her klasörün görünümünü hatırla"))
            .with_cmd(Cmd::App(id))
            .checked(self.remember_views);
        row.sep = true;
        show.children.push(row);
        let id = acts.len() as u32;
        acts.push(Action::ForgetFolderView);
        let mut row = Node::new(crate::i18n::t("Klasör görünümünü sıfırla")).with_cmd(Cmd::App(id));
        row.enabled = !crate::tab::is_virtual(&self.tabs[self.active].path);
        show.children.push(row);
        let id = acts.len() as u32;
        acts.push(Action::ToggleDefaultFm);
        let mut row = Node::new(crate::i18n::t("Varsayılan dosya gezgini olarak kullan"))
            .with_cmd(Cmd::App(id));
        row.checked = self.default_fm.unwrap_or_else(win::is_default_file_manager);
        row.sep = true;
        show.children.push(row);
        if cfg!(windows) {
            let id = acts.len() as u32;
            acts.push(Action::ToggleBackground);
            show.children.push(
                Node::new(crate::i18n::t("Anında açılış için arka planda bekle"))
                    .with_cmd(Cmd::App(id))
                    .checked(self.background),
            );
            let id = acts.len() as u32;
            acts.push(Action::ToggleAutostart);
            let mut row = Node::new(crate::i18n::t("Oturum açılınca arka planda başlat"))
                .with_cmd(Cmd::App(id));
            row.checked = self.autostart;
            row.enabled = self.background;
            show.children.push(row);
        }
        if crate::update::enabled() {
            let id = acts.len() as u32;
            acts.push(Action::ToggleAutoUpdate);
            let mut row =
                Node::new(crate::i18n::t("Güncellemeleri otomatik yükle")).with_cmd(Cmd::App(id));
            row.checked = self.auto_update.load(Relaxed);
            show.children.push(row);
        }
        m.nodes.push(show);
        // Tema alt menüsü: Sistem/Açık/Koyu + renk şemaları (radyo noktalı)
        let mut theme = Node::new(crate::i18n::t("Tema")).sep_before();
        for (v, key) in [(0u8, "Sistem"), (1, "Açık"), (2, "Koyu")] {
            let id = acts.len() as u32;
            acts.push(Action::SetTheme(v));
            theme.children.push(
                Node::new(crate::i18n::t(key))
                    .with_cmd(Cmd::App(id))
                    .checked(self.theme == v),
            );
        }
        for (i, c) in scheme::SCHEMES.iter().enumerate() {
            let v = i as u8 + scheme::FIRST;
            let id = acts.len() as u32;
            acts.push(Action::SetTheme(v));
            let mut row = Node::new(c.name).with_cmd(Cmd::App(id));
            row.checked = self.theme == v;
            row.sep = i == 0;
            theme.children.push(row);
        }
        m.nodes.push(theme);
        // Dil alt menüsü: Sistem + tanımlı diller (radyo noktalı)
        let mut lang = Node::new(crate::i18n::t("Dil"));
        let id = acts.len() as u32;
        acts.push(Action::SetLanguage(0));
        lang.children.push(
            Node::new(crate::i18n::t("Sistem"))
                .with_cmd(Cmd::App(id))
                .checked(self.lang == 0),
        );
        for (i, (_, name)) in crate::i18n::LANGS.iter().enumerate() {
            let v = i as u8 + 1;
            let id = acts.len() as u32;
            acts.push(Action::SetLanguage(v));
            lang.children.push(
                Node::new(*name)
                    .with_cmd(Cmd::App(id))
                    .checked(self.lang == v),
            );
        }
        m.nodes.push(lang);
        m
    }

    /// Gerçek kabuk menüsünü okuyup modern menüye ekler.
    fn push_shell_nodes(
        &mut self,
        ctx: &egui::Context,
        m: &mut Menu,
        paths: &[PathBuf],
        background: Option<&Path>,
    ) {
        let Some(hwnd) = self.hwnd else { return };
        let Some(sm) = crate::shell::ShellMenu::build(hwnd, paths, background, false) else {
            return;
        };
        if sm.is_empty() {
            return;
        }
        let mut pending_sep = true;
        for item in sm.items.iter() {
            if item.separator {
                pending_sep = true;
                continue;
            }
            // Klasik kabuk komutları (7-Zip, "Open with …", sürücü araçları) Windows 11'in
            // modern menüsünde görünmez; yalnızca "Daha fazla seçenek"te listelenir.
            if !modern_shell_item(&item.label) {
                continue;
            }
            let mut n = menu::shell_node(item, ctx, &item.id.to_string());
            n.cmd = Some(Cmd::Shell(item.id));
            if pending_sep {
                n.sep = true;
                pending_sep = false;
            }
            m.nodes.push(n);
        }
        m.shell = Some(sm);
    }

    /// Açık menüyü çizer ve seçilen komutu çalıştırır.
    fn draw_context_menu(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !matches!(self.dialog, Dialog::None) {
            self.menu = None;
            return;
        }
        let Some(state) = self.menu.as_mut() else {
            return;
        };
        let outcome = menu::show(ctx, &mut state.menu);
        match outcome {
            None => {}
            Some(menu::Outcome::Close) => self.menu = None,
            Some(menu::Outcome::Command(Cmd::App(i))) => {
                if let Some(mut st) = self.menu.take() {
                    if (i as usize) < st.actions.len() {
                        let a = st.actions.remove(i as usize);
                        actions.push(a);
                    }
                }
            }
            Some(menu::Outcome::Command(Cmd::Shell(id))) => {
                if let Some(st) = self.menu.take() {
                    if let Some(sm) = st.menu.shell {
                        if let MenuResult::Rename = sm.invoke(id) {
                            actions.push(Action::Rename);
                        }
                        self.inject_release = ctx.input(|i| i.pointer.latest_pos());
                    }
                }
            }
            Some(menu::Outcome::Command(Cmd::More)) => {
                if let Some(st) = self.menu.take() {
                    let (paths, background) = match st.target {
                        MenuTarget::Items { paths, .. } => (paths, None),
                        MenuTarget::Background { folder, .. } => (vec![], Some(folder)),
                    };
                    actions.push(Action::NativeMenu {
                        paths,
                        background,
                        extended: false,
                    });
                }
            }
        }
    }
}

/// Windows 11'in modern sağ tık menüsünde görünen kabuk komutları. Klasik menüde
/// kalanlar (arşiv araçları, "Open with …" vb.) yalnızca "Daha fazla seçenek"te çıkar.
fn modern_shell_item(label: &str) -> bool {
    const MODERN: &[&str] = &[
        "set as desktop background",
        "rotate right",
        "rotate left",
        "cast to device",
        "give access to",
        "edit",
        "compress to zip file",
        "always keep on this device",
        "free up space",
        "scan with microsoft defender",
        "add to favorites",
        "masaüstü arka planı olarak ayarla",
        "sağa döndür",
        "sola döndür",
        "cihaza yayınla",
        "erişim ver",
        "düzenle",
        "sıkıştırılmış (zip) klasör oluştur",
        "her zaman bu cihazda tut",
        "yer aç",
        "microsoft defender ile tara",
        "sık kullanılanlara ekle",
    ];
    let l = label.trim().trim_end_matches(['.', '…']).to_lowercase();
    MODERN.iter().any(|m| l == *m)
}

/// Menüye uygulama komutu ekler; komut `MenuState.actions` içinde saklanır.
/// Etiket ve varsa kısayol i18n anahtarından ayrılır.
fn app_node(
    acts: &mut Vec<Action>,
    m: &mut Menu,
    key: &str,
    glyph: &'static str,
    action: Action,
    build: impl FnOnce(Node) -> Node,
) {
    let (label, shortcut) = menu::parts(key);
    let id = acts.len() as u32;
    acts.push(action);
    let mut node = Node::new(label).with_icon(glyph).with_cmd(Cmd::App(id));
    if let Some(sc) = shortcut {
        node.shortcut = Some(sc);
    }
    m.nodes.push(build(node));
}

/// Üstteki simge düğmesi (komutu `actions` içinde saklar).
fn quick(
    acts: &mut Vec<Action>,
    action: Action,
    glyph: &'static str,
    key: &str,
    enabled: bool,
) -> Quick {
    let (label, _) = menu::parts(key);
    let id = acts.len() as u32;
    acts.push(action);
    Quick {
        cmd: Cmd::App(id),
        glyph,
        label,
        enabled,
    }
}

/// Öğe menüsünün hedefi: tıklanan öğe artı o anki seçim.
fn menu_target_items(t: &Tab, path: &Path, is_dir: bool, in_search: bool) -> MenuTarget {
    let mut paths = t.selected_paths();
    if paths.is_empty() {
        paths.push(path.to_path_buf());
    }
    MenuTarget::Items {
        paths,
        is_dir,
        in_search,
        primary: path.to_path_buf(),
    }
}

/// %ORTAM% değişkenleri, ~ ve kabuk kısayolları (shell:downloads vb.)
fn expand_path(s: &str) -> PathBuf {
    let lower = s.to_lowercase();
    let shortcut = match lower.as_str() {
        "shell:desktop" | "masaüstü" | "desktop" => dirs::desktop_dir(),
        "shell:downloads" | "indirilenler" | "downloads" => dirs::download_dir(),
        "shell:personal" | "belgeler" | "documents" => dirs::document_dir(),
        "shell:my pictures" | "resimler" | "pictures" => dirs::picture_dir(),
        "shell:appdata" => dirs::config_dir(),
        "shell:local appdata" => dirs::data_local_dir(),
        "shell:profile" => dirs::home_dir(),
        "giriş" | "home" | "" => Some(PathBuf::new()),
        _ => None,
    };
    if let Some(p) = shortcut {
        return p;
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('%') {
            Some(b) => {
                let var = &after[..b];
                out.push_str(&std::env::var(var).unwrap_or_else(|_| format!("%{var}%")));
                rest = &after[b + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if let Some(r) = out.strip_prefix('~') {
        if let Some(h) = dirs::home_dir() {
            return normalize(&PathBuf::from(format!("{}{}", h.display(), r)));
        }
    }
    normalize(Path::new(&out))
}

// ================================================================ dosya görünümleri

fn type_name_cached(map: &mut HashMap<String, String>, e: &Entry) -> String {
    if e.is_dir {
        return crate::i18n::t("Dosya klasörü").into();
    }
    if e.ext.is_empty() {
        return crate::i18n::t("Dosya").into();
    }
    map.entry(e.ext.clone())
        .or_insert_with(|| {
            let n = shell::type_name(&e.ext);
            if n.is_empty() {
                fsutil::kind_label(e)
            } else {
                n
            }
        })
        .clone()
}

fn drop_target_p(
    painter: &egui::Painter,
    resp: &egui::Response,
    target: &Path,
    accent: Color32,
    actions: &mut Vec<Action>,
) -> bool {
    let mut hovering = false;
    if let Some(p) = resp.dnd_hover_payload::<DragPaths>() {
        if !p.0.iter().any(|s| s == target || target.starts_with(s)) {
            painter.rect(
                resp.rect,
                4.0,
                accent.gamma_multiply(0.18),
                Stroke::new(1.5_f32, accent),
                egui::StrokeKind::Inside,
            );
            hovering = true;
        }
    }
    if let Some(p) = resp.dnd_release_payload::<DragPaths>() {
        let (ctrl, shift) = resp.ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
        actions.push(Action::DropPaths {
            paths: p.0.clone(),
            target: target.to_path_buf(),
            ctrl,
            shift,
        });
    }
    hovering
}

/// Ayrıntılar görünümünde bir satır: grup başlığı ya da `view` içindeki öğe
enum Line {
    Head(usize),
    Item(usize),
}

/// Simge görünümünde bir satır: grup başlığı ya da `view[a..b]`
enum GLine {
    Head(usize),
    Row(usize, usize),
}

/// Satır/kutu tıklama mantığı (Explorer ile aynı)
fn handle_item_click(
    t: &mut Tab,
    resp: &egui::Response,
    row: usize,
    idx: usize,
    selected: bool,
    e: &Entry,
    actions: &mut Vec<Action>,
) {
    let (ctrl, shift) = resp.ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
    if resp.clicked() {
        if ctrl {
            if !t.selection.remove(&idx) {
                t.selection.insert(idx);
            }
            t.cursor = Some(idx);
            t.anchor = Some(idx);
        } else if shift {
            t.select_range_to(row);
        } else {
            t.select_only(idx);
        }
    }
    if resp.double_clicked() {
        actions.push(Action::OpenEntry(e.path.clone(), e.is_dir));
    }
    if resp.middle_clicked() && e.is_dir {
        actions.push(Action::NewTab(e.path.clone()));
    }
    if (resp.secondary_clicked() || resp.drag_started()) && !selected {
        t.select_only(idx);
    }
    if resp.drag_started() {
        let paths = t.selected_paths();
        if !paths.is_empty() {
            DragAndDrop::set_payload(&resp.ctx, DragPaths(paths));
        }
    }
    if NATIVE_MENU && resp.secondary_clicked() && shift {
        actions.push(Action::ShellMenu { extended: true });
    }
}

impl App {
    fn file_area(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.accent;
        let (in_search, current) = {
            let t = &self.tabs[self.active];
            (t.search.is_some(), t.path.clone())
        };

        // Boş alan: tık seçimi temizler, sürükleme seçim dikdörtgeni çizer, sağ tık klasör menüsü
        let bg = ui.interact(
            ui.max_rect(),
            egui::Id::new("list_bg"),
            Sense::click_and_drag(),
        );
        if bg.clicked() {
            self.tab().selection.clear();
        }
        if bg.drag_started() {
            self.marquee = bg.interact_pointer_pos();
            if !ui.input(|i| i.modifiers.ctrl) {
                self.tab().selection.clear();
            }
        }
        if bg.secondary_clicked() {
            let pos = bg.interact_pointer_pos().unwrap_or_default();
            if NATIVE_MENU && ui.input(|i| i.modifiers.shift) {
                actions.push(Action::NativeMenu {
                    paths: vec![],
                    background: Some(current.clone()),
                    extended: true,
                });
            } else {
                actions.push(Action::OpenMenu {
                    target: MenuTarget::Background {
                        folder: current.clone(),
                        in_search,
                    },
                    pos,
                });
            }
        }

        {
            let t = &self.tabs[self.active];
            if let Some(err) = &t.error {
                ui.add_space(8.0);
                ui.colored_label(
                    Color32::from_rgb(209, 52, 56),
                    crate::i18n::tf("⚠  Bu klasöre erişilemiyor: {}", &[&err]),
                );
            }
            if t.view.is_empty() && !t.is_loading() && t.error.is_none() {
                let msg = if t.search.as_ref().is_some_and(|s| !s.done) {
                    crate::i18n::t("Aranıyor…")
                } else if t.search.is_some() {
                    crate::i18n::t("Aramanızla eşleşen öğe yok.")
                } else if !t.filter.is_empty() {
                    crate::i18n::t("Filtreyle eşleşen öğe yok — tüm alt klasörlerde aramak için Enter'a basın.")
                } else {
                    crate::i18n::t("Bu klasör boş.")
                };
                ui.add_space(40.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(msg).weak()));
            }
        }

        self.item_rects.clear();
        if self.view_level == 0 {
            self.details_view(ui, actions);
        } else {
            self.grid_view(ui, actions);
        }

        // Boş alana bırakma: bu klasöre
        if !in_search {
            if let Some(p) = bg.dnd_release_payload::<DragPaths>() {
                let (ctrl, shift) = ui.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
                actions.push(Action::DropPaths {
                    paths: p.0.clone(),
                    target: current.clone(),
                    ctrl,
                    shift,
                });
            }
        }

        // Seçim dikdörtgeni
        if let Some(start) = self.marquee {
            if bg.dragged() {
                if let Some(now) = ui.input(|i| i.pointer.latest_pos()) {
                    let r = Rect::from_two_pos(start, now);
                    ui.painter().rect(
                        r,
                        2.0,
                        accent.gamma_multiply(0.15),
                        Stroke::new(1.0_f32, accent),
                        egui::StrokeKind::Inside,
                    );
                    let hits: Vec<usize> = self
                        .item_rects
                        .iter()
                        .filter(|(_, rr)| rr.intersects(r))
                        .map(|(i, _)| *i)
                        .collect();
                    let t = self.tab();
                    if !ui.input(|i| i.modifiers.ctrl) {
                        t.selection.clear();
                    }
                    t.selection.extend(hits.iter().copied());
                    if let Some(&last) = hits.last() {
                        t.cursor = Some(last);
                    }
                }
            } else {
                self.marquee = None;
            }
        }
    }

    fn details_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let App {
            tabs,
            active,
            images,
            renaming,
            type_names,
            show_ext,
            accent,
            clipboard,
            dir_sizes,
            sort,
            asc,
            item_rects,
            remember_views,
            ..
        } = self;
        let (sort, asc, show_ext, accent) = (*sort, *asc, *show_ext, *accent);
        let t = &mut tabs[*active];
        let in_search = t.search.is_some();
        // Klon yerine kilit hizasını kullan: harita her karede yeniden kopyalanmaz.
        let dir_sizes = dir_sizes.lock().unwrap();
        let cut_set: std::collections::HashSet<PathBuf> = match clipboard {
            Some((v, true)) => v.iter().cloned().collect(),
            _ => Default::default(),
        };
        let mut commit: Option<(PathBuf, String)> = None;
        let mut cancel_rename = false;
        let focus_line = pal_of(ui).line;
        let _ = accent;

        let row_h = (28.0 * text_factor()).round();
        // Sağ kenarda metin pencere kenarına yapışmasın: Explorer'da Boyut sütunu
        // ile çerçeve arasında bir boşluk var.
        let avail = ui.available_width();
        let show_type = avail > 620.0;
        let table_w = (avail - 12.0).max(240.0);
        ui.set_max_width(table_w);

        // Başlangıç genişlikleri **içerikten** ölçülür: Explorer'da sütun, çoğu
        // değeri kırpmadan gösterecek kadar geniştir. Kullanıcı ayırıcıyı sürükleyince
        // değer egui belleğinde kalır; ayırıcıya çift tıklamak içeriğe sığdırır.
        let gap = 8.0;
        let pad = 18.0;
        let px = |s: &str| {
            ui.fonts(|f| {
                f.layout_no_wrap(s.to_owned(), ui_font(14.0), egui::Color32::BLACK)
                    .size()
                    .x
            })
        };
        // Başlık düğmesi kendi dolgusunu ve (sıralıysa) oku da taşır: sütun en az
        // başlık etiketini kırpmadan gösterecek kadar geniş olmalı.
        let head = |label: &str| px(label) + px("  \u{2B07}") + 16.0;
        let w_loc = (px("Some Folder Name") + pad).clamp(120.0, 240.0);
        let w_date = (px("00.00.0000 00:00").max(head(crate::i18n::t("Değiştirme tarihi"))) + pad)
            .clamp(110.0, (table_w * 0.24).max(130.0));
        let w_size = (px("000,00 MB").max(head(crate::i18n::t("Boyut"))) + pad).clamp(80.0, 170.0);
        let w_type_max = px("Microsoft Edge PDF Document").max(head(crate::i18n::t("Tür"))) + pad;
        let gaps = gap * (2.0 + show_type as i32 as f32 + in_search as i32 as f32);
        let loc_w = if in_search { w_loc } else { 0.0 };
        // Ad sütunu önceliklidir (Explorer'da da öyle): tablonun ~%42'si. Tür sütunu
        // içeriğine sığdığı kadar yer alır, artan yer ada geri döner.
        let mut w_name = (table_w * 0.42).clamp(220.0, 900.0);
        let w_type =
            (table_w - w_name - w_date - w_size - loc_w - gaps).clamp(110.0, w_type_max.max(110.0));
        w_name = (table_w - w_type - w_date - w_size - loc_w - gaps).clamp(180.0, 900.0);

        // İlk karede panel genişliği henüz oturmamış olur (yan paneller çizilmemiş);
        // sütun genişlikleri o karede oluşursa çarpık değerler kalıcılaşır. Bu yüzden
        // ilk iki kare ayrı bir anahtar kullanır, gerçek durum oturmuş genişlikle doğar.
        let per_folder;
        let salt: &str = if ui.ctx().cumulative_pass_nr() < 3 {
            "details_cols_boot"
        } else if *remember_views {
            // Her klasör kendi sütun genişliklerini hatırlar: egui_extras durumu bu
            // anahtarın altında tutar (bkz. Config.folder_views).
            match Self::folder_key(&t.path) {
                Some(k) => {
                    per_folder = format!("details_cols_v3:{k}");
                    &per_folder
                }
                None => "details_cols_v3",
            }
        } else {
            "details_cols_v3"
        };
        // egui_extras, her sütun ayırıcısını tablo boyunca ince bir çizgiyle gösteriyor;
        // Explorer sütun çizgisi çizmez (yalnızca üzerine gelince imleç değişir).
        // Çizgileri tablo süresince kapatıyoruz.
        let saved_style = ui.style().clone();
        {
            let mut style = (*saved_style).clone();
            style.visuals.widgets.noninteractive.bg_stroke = Stroke::NONE;
            style.visuals.widgets.hovered.bg_stroke = Stroke::NONE;
            style.visuals.widgets.active.bg_stroke = Stroke::NONE;
            ui.set_style(style);
        }
        let mut table = TableBuilder::new(ui)
            .id_salt(salt)
            .striped(false)
            // Bütün ayırıcılar sürüklenebilir (Explorer gibi), genişlikler kalıcı.
            .resizable(true)
            .sense(Sense::click_and_drag())
            .drag_to_scroll(false)
            .auto_shrink(false)
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(
                Column::initial(w_name)
                    .at_least(150.0)
                    .at_most((table_w * 0.80).max(200.0))
                    .resizable(true)
                    .clip(true),
            );
        if in_search {
            table = table.column(
                Column::initial(w_loc)
                    .at_least(90.0)
                    .at_most((table_w * 0.35).max(140.0))
                    .resizable(true)
                    .clip(true),
            );
        }
        table = table.column(
            Column::initial(w_date)
                .at_least(100.0)
                .at_most((table_w * 0.40).max(140.0))
                .resizable(true)
                .clip(true),
        );
        if show_type {
            table = table.column(
                Column::initial(w_type)
                    .at_least(110.0)
                    .at_most((table_w * 0.50).max(160.0))
                    .resizable(true)
                    .clip(true),
            );
        }
        table = table.column(
            Column::initial(w_size)
                .at_least(70.0)
                .at_most((table_w * 0.35).max(120.0))
                .resizable(true)
                .clip(true),
        );
        // Sağdaki boşluğu yutan görünmez sütun: satır vurgusu pencere kenarına kadar
        // sürsün ve sütunlar daraldığında fazla yer oraya gitsin.
        table = table.column(Column::remainder().resizable(false));
        // Satırlar: grup başlıkları + öğeler (gruplama yoksa yalnızca öğeler)
        let lines: Vec<Line> = if t.groups.is_empty() {
            (0..t.view.len()).map(Line::Item).collect()
        } else {
            let mut v = Vec::with_capacity(t.view.len() + t.groups.len());
            for (gi, g) in t.groups.iter().enumerate() {
                v.push(Line::Head(gi));
                v.extend((g.start..g.start + g.len).map(Line::Item));
            }
            v
        };
        if t.scroll_to_cursor {
            if let Some(r) = t.cursor_row() {
                if let Some(li) = lines
                    .iter()
                    .position(|l| matches!(l, Line::Item(x) if *x == r))
                {
                    table = table.scroll_to_row(li, None);
                }
            }
            t.scroll_to_cursor = false;
        }
        let mut toggle_group: Option<String> = None;

        let arrow = |k: SortKey| {
            if sort == k {
                if asc {
                    "  ⏶"
                } else {
                    "  ⏷"
                }
            } else {
                ""
            }
        };
        let mut head_cells: Vec<Rect> = Vec::new();
        table
            .header(28.0, |mut h| {
                let mut head = |h: &mut egui_extras::TableRow,
                                label: &str,
                                key: Option<SortKey>,
                                right: bool| {
                    let (rect, _) = h.col(|ui| {
                        let text = format!("{label}{}", key.map(arrow).unwrap_or(""));
                        let layout = if right {
                            Layout::right_to_left(Align::Center)
                        } else {
                            Layout::left_to_right(Align::Center)
                        };
                        ui.with_layout(layout, |ui| {
                            let r = ui.add(
                                egui::Button::new(
                                    RichText::new(text).color(ui.visuals().weak_text_color()),
                                )
                                .frame(false),
                            );
                            if r.clicked() {
                                if let Some(k) = key {
                                    actions.push(Action::Sort(k));
                                }
                            }
                        });
                    });
                    head_cells.push(rect);
                };
                head(&mut h, crate::i18n::t("Ad"), Some(SortKey::Name), false);
                if in_search {
                    head(&mut h, crate::i18n::t("Klasör"), None, false);
                }
                head(
                    &mut h,
                    crate::i18n::t("Değiştirme tarihi"),
                    Some(SortKey::Modified),
                    true,
                );
                if show_type {
                    head(&mut h, crate::i18n::t("Tür"), Some(SortKey::Type), true);
                }
                head(&mut h, crate::i18n::t("Boyut"), Some(SortKey::Size), true);
                // Sağdaki boşluk sütunu
                h.col(|_ui| {});
            })
            .body(|mut body| {
                let weak = body.ui_mut().visuals().weak_text_color();
                let painter = body.ui_mut().painter().clone();
                let heights: Vec<f32> = lines
                    .iter()
                    .map(|l| {
                        if matches!(l, Line::Head(_)) {
                            34.0
                        } else {
                            row_h
                        }
                    })
                    .collect();
                body.heterogeneous_rows(heights.into_iter(), |mut row| {
                    let r = match lines[row.index()] {
                        Line::Head(gi) => {
                            let g = &t.groups[gi];
                            row.col(|ui| {
                                let rect = ui.max_rect().with_max_x(ui.max_rect().left() + 260.0);
                                if group_header(
                                    ui,
                                    rect,
                                    crate::i18n::t(&g.label),
                                    !g.collapsed,
                                    egui::Id::new(("ghead", &g.label)),
                                )
                                .clicked()
                                {
                                    toggle_group = Some(g.label.clone());
                                }
                            });
                            return;
                        }
                        Line::Item(r) => r,
                    };
                    let idx = t.view[r];
                    let e = t.entries[idx].clone();
                    let selected = t.selection.contains(&idx);
                    row.set_selected(selected);
                    let dim = e.hidden || cut_set.contains(&e.path);
                    let txt = |s: String| {
                        let rt = RichText::new(s);
                        if dim {
                            rt.color(weak)
                        } else {
                            rt
                        }
                    };
                    row.col(|ui| {
                        ui.add_space(4.0);
                        let resp = paint_icon(
                            ui,
                            images,
                            &e.path,
                            e.is_dir,
                            &e.ext,
                            e.plain_dir,
                            18.0,
                            fsutil::icon(&e),
                        );
                        if dim {
                            ui.painter().rect_filled(
                                resp.rect,
                                0.0,
                                ui.visuals().extreme_bg_color.gamma_multiply(0.5),
                            );
                        }
                        ui.add_space(4.0);
                        match renaming.as_mut().filter(|rn| rn.path == e.path) {
                            Some(rn) => {
                                let out = egui::TextEdit::singleline(&mut rn.text)
                                    .id(egui::Id::new("inline_rename"))
                                    .desired_width(f32::INFINITY)
                                    .show(ui);
                                if rn.first {
                                    out.response.request_focus();
                                    let stem = if e.is_dir {
                                        rn.text.chars().count()
                                    } else {
                                        rn.text
                                            .rfind('.')
                                            .filter(|&d| d > 0)
                                            .map(|d| rn.text[..d].chars().count())
                                            .unwrap_or(rn.text.chars().count())
                                    };
                                    let mut st = out.state.clone();
                                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(stem),
                                    )));
                                    st.store(ui.ctx(), out.response.id);
                                    rn.first = false;
                                } else if out.response.lost_focus() {
                                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                                        cancel_rename = true;
                                    } else {
                                        commit = Some((rn.path.clone(), rn.text.clone()));
                                    }
                                }
                            }
                            None => {
                                ui.add(
                                    egui::Label::new(txt(display_name(&e, show_ext))).truncate(),
                                );
                            }
                        }
                    });
                    if in_search {
                        row.col(|ui| {
                            let loc = e
                                .path
                                .parent()
                                .and_then(|p| p.strip_prefix(&t.path).ok())
                                .map(|p| {
                                    if p.as_os_str().is_empty() {
                                        ".".into()
                                    } else {
                                        p.to_string_lossy().into_owned()
                                    }
                                })
                                .unwrap_or_default();
                            ui.add(egui::Label::new(RichText::new(loc).color(weak)).truncate());
                        });
                    }
                    row.col(|ui| {
                        // Başlık sağa yaslı; değer de aynı sağ kenara oturmalı.
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(fsutil::format_time(e.modified)).color(weak));
                        });
                    });
                    if show_type {
                        row.col(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(type_name_cached(type_names, &e)).color(weak),
                                    )
                                    .truncate(),
                                );
                            });
                        });
                    }
                    row.col(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let s = if e.is_dir {
                                dir_sizes
                                    .get(&e.path)
                                    .map(|s| fsutil::format_size(*s))
                                    .unwrap_or_default()
                            } else {
                                fsutil::format_size(e.size)
                            };
                            ui.label(RichText::new(s).color(weak));
                        });
                    });
                    // Sağdaki boşluk sütunu
                    row.col(|_ui| {});

                    let resp = row.response();
                    item_rects.push((idx, resp.rect));
                    if t.cursor == Some(idx) && !selected && t.selection.is_empty() {
                        painter.rect_stroke(
                            resp.rect.shrink(1.0),
                            4.0,
                            Stroke::new(1.0_f32, focus_line),
                            egui::StrokeKind::Inside,
                        );
                    }
                    handle_item_click(t, &resp, r, idx, selected, &e, actions);
                    if e.is_dir {
                        drop_target_p(&painter, &resp, &e.path, accent, actions);
                    }
                    if resp.secondary_clicked()
                        && !(NATIVE_MENU && resp.ctx.input(|i| i.modifiers.shift))
                    {
                        let pos = resp.interact_pointer_pos().unwrap_or_default();
                        actions.push(Action::OpenMenu {
                            target: menu_target_items(t, &e.path, e.is_dir, in_search),
                            pos,
                        });
                    }
                });
            });

        // Sütun ayırıcıları yalnızca başlık satırında: Explorer'da da listenin
        // içinden çizgi geçmez, etiketler arasında kısa bir çizgi görünür.
        for r in head_cells.iter().skip(1) {
            let x = r.left() - 4.0;
            ui.painter().line_segment(
                [
                    egui::pos2(x, r.top() + 6.0),
                    egui::pos2(x, r.bottom() - 6.0),
                ],
                Stroke::new(1.0_f32, focus_line),
            );
        }

        ui.set_style(saved_style);

        // Kilidin hayalet serbest bırakması commit_rename'ın &mut self almasını
        // engeller; tablo çizildi, kilidi şimdi bırak.
        drop(dir_sizes);

        if let Some(label) = toggle_group {
            if !t.collapsed_groups.remove(&label) {
                t.collapsed_groups.insert(label);
            }
            t.view_dirty = true;
        }
        if cancel_rename {
            *renaming = None;
        }
        if let Some((p, text)) = commit {
            *renaming = None;
            self.commit_rename(p, text);
        }
    }

    fn grid_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let size = VIEW_LEVELS[self.view_level].0 as f32;
        // Explorer ölçüleri: 96 px simgede öğe aralığı ~122 px, ad en fazla 3 satır.
        // Yazı büyüdükçe etiket alanı da büyür; aksi hâlde adlar alt satırdaki grubun üstüne taşar.
        let f = text_factor();
        let tile_w = (size + 22.0 * f).max(80.0 * f);
        let tile_h = size + 76.0 * f;
        let gap = 4.0;
        let avail = ui.available_width() - 8.0;
        let cols = ((avail + gap) / (tile_w + gap)).floor().max(1.0) as usize;
        self.grid_cols = cols;
        let ctx = ui.ctx().clone();
        let px = icon_px(&ctx, size);

        let App {
            tabs,
            active,
            images,
            renaming,
            show_ext,
            accent,
            clipboard,
            item_rects,
            grid_scroll,
            type_names,
            ..
        } = self;
        let (show_ext, accent) = (*show_ext, *accent);
        let t = &mut tabs[*active];
        let in_search = t.search.is_some();
        let n = t.view.len();
        let rows = n.div_ceil(cols);
        let cut_set: std::collections::HashSet<PathBuf> = match clipboard {
            Some((v, true)) => v.iter().cloned().collect(),
            _ => Default::default(),
        };
        let mut commit: Option<(PathBuf, String)> = None;
        let mut cancel_rename = false;

        let pl = pal_of(ui);
        // Satırlar: (y, yükseklik, tür). Gruplama yoksa yalnızca öğe satırları.
        let head_h = 34.0;
        let mut glines: Vec<(f32, f32, GLine)> = Vec::new();
        let mut y = 4.0;
        let push_rows =
            |glines: &mut Vec<(f32, f32, GLine)>, y: &mut f32, start: usize, len: usize| {
                for k in 0..len.div_ceil(cols) {
                    let a = start + k * cols;
                    glines.push((*y, tile_h + gap, GLine::Row(a, (a + cols).min(start + len))));
                    *y += tile_h + gap;
                }
            };
        if t.groups.is_empty() {
            push_rows(&mut glines, &mut y, 0, n);
        } else {
            for (gi, g) in t.groups.iter().enumerate() {
                glines.push((y, head_h, GLine::Head(gi)));
                y += head_h;
                push_rows(&mut glines, &mut y, g.start, g.len);
            }
        }
        let total_h = y + 8.0;
        let _ = rows;

        let mut area = egui::ScrollArea::vertical()
            .auto_shrink(false)
            .drag_to_scroll(false);
        if t.scroll_to_cursor {
            if let Some(r) = t.cursor_row() {
                let top = glines
                    .iter()
                    .find(|(_, _, l)| matches!(l, GLine::Row(a, b) if r >= *a && r < *b))
                    .map(|(y, _, _)| *y)
                    .unwrap_or(0.0);
                let (off, vh) = *grid_scroll;
                if top < off {
                    area = area.vertical_scroll_offset(top);
                } else if top + tile_h > off + vh && vh > 0.0 {
                    area = area.vertical_scroll_offset(top + tile_h - vh + gap);
                }
            }
            t.scroll_to_cursor = false;
        }
        let mut toggle_group: Option<String> = None;
        let out = area.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            let width = ui.available_width();
            ui.set_min_height(total_h);
            ui.set_min_width(width);
            let weak = ui.visuals().weak_text_color();
            let text_color = ui.visuals().text_color();
            for (ly, lh, line) in glines.iter() {
                if ly + lh < viewport.min.y || *ly > viewport.max.y {
                    continue;
                }
                let line_rect =
                    Rect::from_min_size(origin + egui::vec2(0.0, *ly), egui::vec2(width, *lh));
                let (row_start, row_end) = match line {
                    GLine::Head(gi) => {
                        let g = &t.groups[*gi];
                        if group_header(
                            ui,
                            line_rect
                                .with_max_x(line_rect.left() + 260.0)
                                .shrink2(egui::vec2(0.0, 3.0)),
                            crate::i18n::t(&g.label),
                            !g.collapsed,
                            egui::Id::new(("ghead", &g.label)),
                        )
                        .clicked()
                        {
                            toggle_group = Some(g.label.clone());
                        }
                        continue;
                    }
                    GLine::Row(a, b) => (*a, *b),
                };
                let mut row_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(line_rect)
                        .layout(Layout::left_to_right(Align::Min)),
                );
                let ui = &mut row_ui;
                ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                {
                    ui.add_space(4.0);
                    for r in row_start..row_end {
                        let idx = t.view[r];
                        let e = t.entries[idx].clone();
                        let selected = t.selection.contains(&idx);
                        let (rect, resp) = ui.allocate_exact_size(
                            egui::vec2(tile_w, tile_h),
                            Sense::click_and_drag(),
                        );
                        item_rects.push((idx, rect));
                        let painter = ui.painter().clone();
                        if selected {
                            painter.rect_filled(
                                rect,
                                4.0,
                                if resp.hovered() { pl.sel_hover } else { pl.sel },
                            );
                        } else if resp.hovered() {
                            painter.rect_filled(rect, 4.0, pl.hover);
                        }
                        if t.cursor == Some(idx) && !selected && t.selection.is_empty() {
                            painter.rect_stroke(
                                rect,
                                4.0,
                                Stroke::new(1.0_f32, pl.line),
                                egui::StrokeKind::Inside,
                            );
                        }
                        // Görsel
                        let img_rect = Rect::from_min_size(
                            rect.min + egui::vec2((tile_w - size) / 2.0, 8.0),
                            Vec2::splat(size),
                        );
                        let thumb = !e.is_dir && fsutil::has_thumbnail(&e.ext);
                        let (key, icon_only) = if thumb {
                            (format!("t{px}:{}", e.path.display()), false)
                        } else {
                            (icon_key(&e.path, e.is_dir, &e.ext, px, e.plain_dir), true)
                        };
                        let dim = e.hidden || cut_set.contains(&e.path);
                        let tint = if dim {
                            Color32::from_white_alpha(130)
                        } else {
                            Color32::WHITE
                        };
                        match images.get(&key, &e.path, px, icon_only) {
                            Img::Ready(tex) => {
                                let r = fit_rect(tex.size_vec2(), img_rect);
                                painter.image(
                                    tex.id(),
                                    r,
                                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                    tint,
                                );
                                if thumb && !icon_only {
                                    painter.rect_stroke(
                                        r,
                                        0.0,
                                        Stroke::new(1.0_f32, Color32::from_black_alpha(40)),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                            }
                            Img::Failed => paint_fallback(&painter, img_rect, e.is_dir),
                            Img::Pending => {}
                        }
                        // Ad (en fazla 2 satır, ortalı) ya da yeniden adlandırma kutusu
                        let name_rect = Rect::from_min_max(
                            egui::pos2(rect.left() + 4.0, img_rect.bottom() + 6.0),
                            egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
                        );
                        match renaming.as_mut().filter(|rn| rn.path == e.path) {
                            Some(rn) => {
                                let mut child = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(name_rect.expand2(egui::vec2(18.0, 0.0))),
                                );
                                let out = egui::TextEdit::singleline(&mut rn.text)
                                    .id(egui::Id::new("inline_rename"))
                                    .horizontal_align(Align::Center)
                                    .desired_width(f32::INFINITY)
                                    .show(&mut child);
                                if rn.first {
                                    out.response.request_focus();
                                    let stem = if e.is_dir {
                                        rn.text.chars().count()
                                    } else {
                                        rn.text
                                            .rfind('.')
                                            .filter(|&d| d > 0)
                                            .map(|d| rn.text[..d].chars().count())
                                            .unwrap_or(rn.text.chars().count())
                                    };
                                    let mut st = out.state.clone();
                                    st.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                        egui::text::CCursor::new(0),
                                        egui::text::CCursor::new(stem),
                                    )));
                                    st.store(&ctx, out.response.id);
                                    rn.first = false;
                                } else if out.response.lost_focus() {
                                    if ctx.input(|i| i.key_pressed(Key::Escape)) {
                                        cancel_rename = true;
                                    } else {
                                        commit = Some((rn.path.clone(), rn.text.clone()));
                                    }
                                }
                            }
                            None => {
                                let mut job = egui::text::LayoutJob::simple(
                                    display_name(&e, show_ext),
                                    egui::TextStyle::Body.resolve(ui.style()),
                                    if dim { weak } else { text_color },
                                    name_rect.width(),
                                );
                                job.wrap.max_rows = 3;
                                job.wrap.break_anywhere = false;
                                job.halign = Align::Center;
                                let galley = painter.layout_job(job);
                                painter.galley(
                                    egui::pos2(name_rect.center().x, name_rect.top()),
                                    galley,
                                    text_color,
                                );
                            }
                        }

                        handle_item_click(t, &resp, r, idx, selected, &e, actions);
                        if e.is_dir {
                            drop_target_p(&painter, &resp, &e.path, accent, actions);
                        }
                        let tn = type_name_cached(type_names, &e);
                        let resp = resp.on_hover_ui(|ui| {
                            ui.label(RichText::new(&e.name).strong());
                            ui.label(crate::i18n::tf("Tür: {}", &[&tn]));
                            if !e.is_dir {
                                ui.label(crate::i18n::tf(
                                    "Boyut: {}",
                                    &[&fsutil::format_size(e.size)],
                                ));
                            }
                            ui.label(crate::i18n::tf(
                                "Değiştirme: {}",
                                &[&fsutil::format_time(e.modified)],
                            ));
                        });
                        if resp.secondary_clicked()
                            && !(NATIVE_MENU && resp.ctx.input(|i| i.modifiers.shift))
                        {
                            let pos = resp.interact_pointer_pos().unwrap_or_default();
                            actions.push(Action::OpenMenu {
                                target: menu_target_items(t, &e.path, e.is_dir, in_search),
                                pos,
                            });
                        }
                    }
                }
            }
        });
        *grid_scroll = (out.state.offset.y, out.inner_rect.height());
        if let Some(label) = toggle_group {
            if !t.collapsed_groups.remove(&label) {
                t.collapsed_groups.insert(label);
            }
            t.view_dirty = true;
        }

        if cancel_rename {
            *renaming = None;
        }
        if let Some((p, text)) = commit {
            *renaming = None;
            self.commit_rename(p, text);
        }
    }

    /// Önizleme bölmesi: yalnızca içerik (görsel küçük resim, metin önizleme)
    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        let (sel, home) = {
            let t = &self.tabs[self.active];
            let sel: Vec<Entry> = t.selected_entries().into_iter().cloned().collect();
            (sel, !t.lists_files() && !t.is_gallery())
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(8.0);
                let e = if home || sel.len() > 1 {
                    None
                } else {
                    sel.first().cloned()
                };
                let Some(e) = e else {
                    ui.vertical_centered(|ui| {
                        if sel.len() > 1 {
                            ui.label(icon_rt("📚").size(48.0));
                            ui.label(crate::i18n::tf("{} öğe seçildi", &[&sel.len()]));
                        }
                        ui.label(
                            RichText::new(crate::i18n::t("Önizlemek için bir öğe seçin.")).weak(),
                        )
                    });
                    return;
                };
                let path = e.path.clone();
                // Sanal konumlarda ham CLSID yerine görünen ad (Galeri, Giriş…)
                let title = if crate::tab::is_virtual(&path) {
                    self.loc_name(&path)
                } else {
                    path_label(&path)
                };
                let ctx = ui.ctx().clone();
                let px = icon_px(&ctx, 200.0);
                let thumb = !e.is_dir && fsutil::has_thumbnail(&e.ext);
                let key = if thumb {
                    format!("t{px}:{}", path.display())
                } else {
                    icon_key(&path, e.is_dir, &e.ext, px, e.plain_dir)
                };
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().min(260.0), 200.0),
                        Sense::hover(),
                    );
                    if let Img::Ready(tex) = self.images.get(&key, &path, px, !thumb) {
                        let r = fit_rect(tex.size_vec2(), rect);
                        ui.painter().image(
                            tex.id(),
                            r,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(title).size(17.0).strong()).wrap());
                });
                // Metin önizlemesi (arka planda okunur); küçük resimli türler ve klasörler
                // içerik önizlemez, diğer türler için yer tutucu göster
                if e.is_dir || thumb || e.size == 0 {
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(crate::i18n::t("Bu tür için önizleme yok.")).weak())
                    });
                    return;
                }
                if self.preview.path.as_ref() != Some(&e.path) {
                    self.preview.path = Some(e.path.clone());
                    *self.preview.data.lock().unwrap() = None;
                    let store = self.preview.data.clone();
                    let p = e.path.clone();
                    std::thread::spawn(move || {
                        let d = fsutil::read_preview(&p);
                        *store.lock().unwrap() = Some((p, d));
                        ctx.request_repaint();
                    });
                }
                let data = self.preview.data.lock().unwrap();
                if let Some((p, PreviewData::Error(err))) = data.as_ref() {
                    if p == &e.path {
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new(crate::i18n::tf("Önizleme yapılamadı: {}", &[&err]))
                                .weak(),
                        );
                    }
                }
                if let Some((p, PreviewData::Text(s))) = data.as_ref() {
                    if p == &e.path {
                        ui.add_space(10.0);
                        ui.label(RichText::new(crate::i18n::t("Önizleme")).strong());
                        ui.separator();
                        egui::Frame::new()
                            .fill(ui.visuals().extreme_bg_color)
                            .corner_radius(6.0)
                            .inner_margin(8)
                            .show(ui, |ui| {
                                egui::ScrollArea::both()
                                    .max_height(420.0)
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(RichText::new(s.as_str()).monospace())
                                                .extend(),
                                        );
                                    });
                            });
                    }
                }
            });
    }

    /// Ayrıntılar bölmesi: her tür dosya ve klasörün özellikleri
    fn details_panel(&mut self, ui: &mut egui::Ui) {
        let (sel, em, count, home) = {
            let t = &self.tabs[self.active];
            let sel: Vec<Entry> = t.selected_entries().into_iter().cloned().collect();
            (
                sel,
                t.path.clone(),
                t.view.len(),
                !t.lists_files() && !t.is_gallery(),
            )
        };
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.add_space(4.0);
                if home {
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(crate::i18n::t(
                                "Ayrıntılarını görmek için bir öğe seçin.",
                            ))
                            .weak(),
                        )
                    });
                    return;
                }
                if sel.len() > 1 {
                    let total: u64 = sel.iter().map(|e| e.size).sum();
                    ui.vertical_centered(|ui| {
                        ui.label(icon_rt("📚").size(48.0));
                        ui.label(
                            RichText::new(crate::i18n::tf("{} öğe seçildi", &[&sel.len()]))
                                .strong(),
                        );
                        ui.label(
                            RichText::new(crate::i18n::tf(
                                "Dosyaların toplam boyutu: {}",
                                &[&fsutil::format_size(total)],
                            ))
                            .weak(),
                        );
                    });
                    return;
                }
                let (path, entry) = match sel.first() {
                    Some(e) => (e.path.clone(), Some(e.clone())),
                    None => (em.clone(), None),
                };
                // Sanal konumlarda ham CLSID yerine görünen ad (Galeri, Giriş…)
                let title = if crate::tab::is_virtual(&path) {
                    self.loc_name(&path)
                } else {
                    path_label(&path)
                };
                let ctx = ui.ctx().clone();
                let px = icon_px(&ctx, 200.0);
                let (is_dir, ext, plain) = entry
                    .as_ref()
                    .map(|e| (e.is_dir, e.ext.clone(), e.plain_dir))
                    .unwrap_or((true, String::new(), false));
                let thumb = !is_dir && fsutil::has_thumbnail(&ext);
                let key = if thumb {
                    format!("t{px}:{}", path.display())
                } else {
                    icon_key(&path, is_dir, &ext, px, plain)
                };
                ui.vertical_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width().min(260.0), 200.0),
                        Sense::hover(),
                    );
                    if let Img::Ready(tex) = self.images.get(&key, &path, px, !thumb) {
                        let r = fit_rect(tex.size_vec2(), rect);
                        ui.painter().image(
                            tex.id(),
                            r,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(title).size(17.0).strong()).wrap());
                });
                ui.add_space(10.0);
                if let Some(e) = &entry {
                    let tn = type_name_cached(&mut self.type_names, e);
                    egui::Grid::new("dinfo")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            let mut row = |k: &str, v: String| {
                                ui.label(RichText::new(k).weak());
                                ui.add(egui::Label::new(v).wrap());
                                ui.end_row();
                            };
                            row(crate::i18n::t("Tür"), tn);
                            if !e.is_dir {
                                row(
                                    crate::i18n::t("Boyut"),
                                    crate::i18n::tf(
                                        "{}  ({} bayt)",
                                        &[&fsutil::format_size(e.size), &e.size],
                                    ),
                                );
                            } else if let Some(s) = self.dir_sizes.lock().unwrap().get(&e.path) {
                                row(crate::i18n::t("Boyut"), fsutil::format_size(*s));
                            }
                            row(
                                crate::i18n::t("Değiştirme"),
                                fsutil::format_time(e.modified),
                            );
                            if e.created.is_some() {
                                row(crate::i18n::t("Oluşturma"), fsutil::format_time(e.created));
                            }
                            row(
                                crate::i18n::t("Konum"),
                                e.path
                                    .parent()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default(),
                            );
                        });
                    if !e.is_dir {
                        ui.add_space(8.0);
                        if ui
                            .add_sized(
                                [ui.available_width(), 30.0],
                                egui::Button::new(crate::i18n::t("Aç")),
                            )
                            .clicked()
                        {
                            let _ = open::that_detached(&e.path);
                        }
                    }
                } else {
                    ui.label(RichText::new(crate::i18n::tf("{} öğe", &[&count])).weak());
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let p = pal_of(ui);
        ui.horizontal(|ui| {
            ui.set_min_height(23.0);
            ui.spacing_mut().item_spacing.x = 8.0;
            let bar = |ui: &mut egui::Ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(1.0, 14.0), Sense::hover());
                ui.painter()
                    .vline(r.center().x, r.y_range(), Stroke::new(1.0_f32, p.text));
            };
            let t = &self.tabs[self.active];
            let count = if t.lists_files() {
                t.view.len()
            } else if t.is_pc() {
                self.drives.lock().unwrap().len()
            } else {
                0
            };
            if t.lists_files() || t.is_pc() {
                ui.label(crate::i18n::tf("{} öğe", &[&fmt_count(count)]));
                bar(ui);
            }
            if t.lists_files() {
                let sel = t.view.iter().filter(|i| t.selection.contains(i)).count();
                if !t.filter.is_empty() && t.search.is_none() {
                    ui.label(
                        RichText::new(crate::i18n::tf(
                            "(filtre: {} / {})",
                            &[&t.view.len(), &t.entries.len()],
                        ))
                        .color(p.weak),
                    );
                }
                if sel > 0 {
                    let size: u64 = t
                        .view
                        .iter()
                        .filter(|i| t.selection.contains(i))
                        .map(|&i| t.entries[i].size)
                        .sum();
                    let s = if size > 0 {
                        crate::i18n::tf("{} öğe seçildi  {}", &[&sel, &fsutil::format_size(size)])
                    } else {
                        crate::i18n::tf("{} öğe seçildi", &[&sel])
                    };
                    ui.label(s);
                    bar(ui);
                }
                if let Some(srch) = &t.search {
                    let scanned = srch.handle.scanned.load(Relaxed);
                    if srch.done {
                        ui.label(
                            RichText::new(crate::i18n::tf(
                                "Arama tamamlandı · {} klasör · {} sn",
                                &[&scanned, &format!("{:.2}", srch.elapsed.as_secs_f32())],
                            ))
                            .color(p.weak),
                        );
                    } else {
                        ui.spinner();
                        ui.label(
                            RichText::new(crate::i18n::tf(
                                "Aranıyor… {} klasör tarandı",
                                &[&scanned],
                            ))
                            .color(p.weak),
                        );
                    }
                } else if t.is_loading() {
                    ui.spinner();
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.add_space(4.0);
                let grid = self.view_level > 0;
                let toggle = |ui: &mut egui::Ui, glyph: &str, on: bool| {
                    let (rect, r) = ui.allocate_exact_size(egui::vec2(26.0, 20.0), Sense::click());
                    if on {
                        ui.painter().rect(
                            rect,
                            3.0,
                            p.nav_sel,
                            Stroke::new(1.0_f32, p.line),
                            egui::StrokeKind::Inside,
                        );
                    } else if r.hovered() {
                        ui.painter().rect_filled(rect, 3.0, p.hover);
                    }
                    ui.painter().text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        glyph,
                        fluent_font(14.0),
                        p.text,
                    );
                    r
                };
                if toggle(ui, g::ICONS_VIEW, grid)
                    .on_hover_text(crate::i18n::t("Büyük simgeler (Ctrl+Shift+2)"))
                    .clicked()
                {
                    actions.push(Action::SetView(if grid { self.view_level } else { 3 }));
                }
                if toggle(ui, g::LIST, !grid)
                    .on_hover_text(crate::i18n::t("Ayrıntılar (Ctrl+Shift+6)"))
                    .clicked()
                {
                    actions.push(Action::SetView(0));
                }
            });
        });
    }

    /// Sağ altta ilerleme kartları ve bildirimler
    fn overlays(&mut self, ctx: &egui::Context) {
        self.toasts
            .retain(|(_, t, _)| t.elapsed() < Duration::from_secs(4));
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if self.jobs.is_empty() && self.toasts.is_empty() {
            return;
        }
        let accent = self.accent;
        egui::Area::new(egui::Id::new("overlays"))
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for job in &self.jobs {
                    egui::Frame::popup(ui.style())
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.set_width(360.0);
                            let total = job.total.load(Relaxed);
                            let done = job.done.load(Relaxed);
                            let frac = if total > 0 {
                                (done as f32 / total as f32).min(1.0)
                            } else {
                                0.0
                            };
                            let syncing = job.syncing.load(Relaxed);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&job.label).strong());
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    // Diske yazma yarıda kesilemez; iptal edilmiş gibi görünmesin
                                    if !syncing
                                        && ui
                                            .add(egui::Button::new(icon_rt("✕")).frame(false))
                                            .on_hover_text(crate::i18n::t("İptal"))
                                            .clicked()
                                    {
                                        job.cancel.store(true, Relaxed);
                                    }
                                });
                            });
                            let cur = job.current.lock().unwrap().clone();
                            if syncing {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(crate::i18n::t(
                                            "Diske yazılıyor, bitene kadar sürücüyü çıkarmayın",
                                        ))
                                        .small()
                                        .weak(),
                                    )
                                    .wrap(),
                                );
                            } else if !cur.is_empty() {
                                ui.add(
                                    egui::Label::new(RichText::new(cur).small().weak()).truncate(),
                                );
                            }
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .desired_height(6.0)
                                    .fill(accent)
                                    .corner_radius(3.0),
                            );
                            if job.kind == JobKind::Transfer && total > 0 && !syncing {
                                let el = job.started.elapsed().as_secs_f64().max(0.001);
                                let speed = done as f64 / el;
                                let left = if speed > 0.0 {
                                    (total - done.min(total)) as f64 / speed
                                } else {
                                    0.0
                                };
                                ui.label(
                                    RichText::new(crate::i18n::tf(
                                        "%{}  ·  {}/sn  ·  yaklaşık {} kaldı",
                                        &[
                                            &format!("{:.0}", frac * 100.0),
                                            &fsutil::format_size(speed as u64),
                                            &fmt_secs(left),
                                        ],
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                    ui.add_space(6.0);
                }
                for (msg, _, err) in &self.toasts {
                    egui::Frame::popup(ui.style())
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            let (icon, c) = if *err {
                                ("⚠", Color32::from_rgb(209, 52, 56))
                            } else {
                                ("✔", accent)
                            };
                            ui.horizontal(|ui| {
                                ui.label(icon_rt(icon).color(c));
                                ui.add(egui::Label::new(msg).wrap());
                            });
                        });
                    ui.add_space(6.0);
                }
            });
    }

    fn drag_overlay(&mut self, ctx: &egui::Context) {
        // Uygulama içi sürükleme etiketi
        if let Some(p) = DragAndDrop::payload::<DragPaths>(ctx) {
            if let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) {
                let (ctrl, shift) = ctx.input(|i| (i.modifiers.ctrl, i.modifiers.shift));
                let hint = if ctrl {
                    "Kopyala"
                } else if shift {
                    crate::i18n::t("Taşı")
                } else {
                    crate::i18n::t("Taşı / kopyala (Ctrl: kopyala, Shift: taşı)")
                };
                egui::Area::new(egui::Id::new("drag_label"))
                    .fixed_pos(pos + egui::vec2(18.0, 18.0))
                    .order(egui::Order::Tooltip)
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            let name = if p.0.len() == 1 {
                                p.0[0]
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            } else {
                                crate::i18n::tf("{} öğe", &[&p.0.len()])
                            };
                            ui.label(RichText::new(name).strong());
                            ui.label(RichText::new(hint).small().weak());
                        });
                    });
            }
        }
        // Dışarıdan dosya sürükleniyor
        let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
        let t = &self.tabs[self.active];
        if hovering_files && !t.is_home() && t.search.is_none() {
            let rect = ctx.screen_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("os_drop"),
            ));
            painter.rect(
                rect.shrink(6.0),
                10.0,
                self.accent.gamma_multiply(0.08),
                Stroke::new(2.0_f32, self.accent),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                crate::i18n::tf(
                    "\"{}\" klasörüne kopyalamak için bırakın",
                    &[&path_label(&t.path)],
                ),
                ui_font(20.0),
                self.accent,
            );
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut dialog = std::mem::replace(&mut self.dialog, Dialog::None);
        let mut keep = true;
        match &mut dialog {
            Dialog::None => keep = false,
            Dialog::ConfirmDelete { paths } => {
                let resp = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
                    ui.set_width(440.0);
                    ui.heading(crate::i18n::t("Kalıcı olarak silinsin mi?"));
                    ui.add_space(4.0);
                    let what = if paths.len() == 1 {
                        format!("\"{}\"", path_label(&paths[0]))
                    } else {
                        crate::i18n::tf("Bu {} öğe", &[&paths.len()])
                    };
                    ui.label(crate::i18n::tf(
                        "{} geri dönüşüm kutusuna gitmeden silinecek. Bu işlem geri alınamaz.",
                        &[&what],
                    ));
                    for p in paths.iter().take(5) {
                        ui.label(RichText::new(format!("•  {}", path_label(p))).weak());
                    }
                    if paths.len() > 5 {
                        ui.label(
                            RichText::new(crate::i18n::tf(
                                "… ve {} öğe daha",
                                &[&(paths.len() - 5)],
                            ))
                            .weak(),
                        );
                    }
                    ui.add_space(10.0);
                    let mut done = false;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(crate::i18n::t("Vazgeç"))
                                    .min_size(egui::vec2(90.0, 30.0)),
                            )
                            .clicked()
                        {
                            done = true;
                        }
                        let del = ui.add(
                            egui::Button::new(
                                RichText::new(crate::i18n::t("Kalıcı olarak sil"))
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(196, 43, 28))
                            .min_size(egui::vec2(140.0, 30.0)),
                        );
                        if del.clicked() || ui.input(|i| i.key_pressed(Key::Enter)) {
                            self.push_job(
                                fsutil::spawn_delete(std::mem::take(paths), true, ctx.clone()),
                                false,
                            );
                            done = true;
                        }
                    });
                    done
                });
                if resp.inner || resp.should_close() {
                    keep = false;
                }
            }
            Dialog::Conflict {
                files,
                dest,
                cut,
                names,
            } => {
                let mut choice: Option<Option<Conflict>> = None;
                let resp = egui::Modal::new(egui::Id::new("conflict")).show(ctx, |ui| {
                    ui.set_width(480.0);
                    ui.heading(if *cut {
                        crate::i18n::t("Dosyaları değiştir veya atla")
                    } else {
                        crate::i18n::t("Dosyaları değiştir veya atla")
                    });
                    ui.add_space(4.0);
                    ui.label(crate::i18n::tf(
                        "Hedef \"{}\" klasöründe aynı adlı {} öğe var:",
                        &[&path_label(dest), &names.len()],
                    ));
                    for n in names.iter().take(5) {
                        ui.label(RichText::new(format!("•  {n}")).weak());
                    }
                    if names.len() > 5 {
                        ui.label(
                            RichText::new(crate::i18n::tf(
                                "… ve {} öğe daha",
                                &[&(names.len() - 5)],
                            ))
                            .weak(),
                        );
                    }
                    ui.add_space(10.0);
                    let big = |ui: &mut egui::Ui, icon: &str, title: &str, sub: &str| {
                        ui.add_sized(
                            [ui.available_width(), 46.0],
                            egui::Button::new(icon_label(
                                ui,
                                icon,
                                None,
                                &format!("{title}\n{sub}"),
                            )),
                        )
                        .clicked()
                    };
                    if big(
                        ui,
                        "✔",
                        crate::i18n::t("Hedefteki dosyaları değiştir"),
                        crate::i18n::t(
                            "Klasörler birleştirilir, aynı adlı dosyaların üzerine yazılır",
                        ),
                    ) {
                        choice = Some(Some(Conflict::Replace));
                    }
                    if big(
                        ui,
                        "↷",
                        crate::i18n::t("Bu dosyaları atla"),
                        crate::i18n::t("Aynı adlı öğeler kopyalanmaz"),
                    ) {
                        choice = Some(Some(Conflict::Skip));
                    }
                    if big(
                        ui,
                        "⧉",
                        crate::i18n::t("İkisini de sakla"),
                        crate::i18n::t("Yeni öğeler \"- Kopya\" ekiyle adlandırılır"),
                    ) {
                        choice = Some(Some(Conflict::KeepBoth));
                    }
                    ui.add_space(6.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(crate::i18n::t("İptal")).clicked() {
                            choice = Some(None);
                        }
                    });
                });
                if resp.should_close() {
                    choice = Some(None);
                }
                if let Some(c) = choice {
                    keep = false;
                    if let Some(c) = c {
                        let j = fsutil::spawn_transfer(
                            std::mem::take(files),
                            dest.clone(),
                            *cut,
                            c,
                            ctx.clone(),
                        );
                        self.push_job(j, true);
                    }
                }
            }
            #[cfg(not(windows))]
            Dialog::Properties(pr) => {
                let images = &mut self.images;
                let out = pr.show(ctx, &mut |ui, path, is_dir, ext, size| {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
                    draw_icon(ui.painter(), images, path, is_dir, ext, rect);
                });
                if out.close {
                    keep = false;
                }
                if let Some((from, to)) = out.renamed {
                    self.undo.push(UndoOp::Rename {
                        from,
                        to: to.clone(),
                    });
                    self.reload_active(Some(to));
                } else if out.refresh {
                    self.reload_active(None);
                }
                if let Some(e) = out.error {
                    self.toast(e, true);
                }
            }
            Dialog::Help => {
                let resp = egui::Modal::new(egui::Id::new("help")).show(ctx, |ui| {
                    ui.set_width(560.0);
                    ui.heading(crate::i18n::t("Klavye kısayolları"));
                    ui.label(RichText::new(crate::i18n::t("Dosya Gezgini'ndeki kısayolların hepsi aynı; ekstralar ★ ile işaretli.")).weak());
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                        egui::Grid::new("keys").num_columns(2).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                            for (k, d) in HELP.iter().filter(|(k, _)| NATIVE_MENU || !k.contains("F10")) {
                                ui.label(RichText::new(crate::i18n::t(k)).monospace().strong());
                                ui.label(crate::i18n::t(d));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        // Sürüm ve hata bildirmek, güncelleme bakmak için proje sayfası
                        ui.label(RichText::new(format!("{} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION"))).weak());
                        ui.hyperlink_to("GitHub", env!("CARGO_PKG_REPOSITORY"));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.button(crate::i18n::t("Kapat")).clicked()).inner
                    })
                    .inner
                });
                if resp.inner || resp.should_close() {
                    keep = false;
                }
            }
        }
        if keep {
            self.dialog = dialog;
        }
    }
}

fn fmt_secs(s: f64) -> String {
    let s = s.round() as u64;
    if s < 60 {
        crate::i18n::tf("{} sn", &[&s])
    } else if s < 3600 {
        crate::i18n::tf("{} dk {} sn", &[&(s / 60), &(s % 60)])
    } else {
        crate::i18n::tf("{} sa {} dk", &[&(s / 3600), &((s % 3600) / 60)])
    }
}

const HELP: &[(&str, &str)] = &[
    ("Enter / çift tık", "Aç"),
    ("Alt+Sol / Alt+Sağ", "Geri / İleri (farenin yan tuşları da)"),
    ("Alt+Yukarı / Backspace", "Üst klasör"),
    (
        "Ctrl+L / Alt+D / F4",
        "Adres çubuğu (Tab ile otomatik tamamla)",
    ),
    (
        "Ctrl+E / Ctrl+F / F3",
        "Arama kutusu — yazdıkça filtreler, Enter alt klasörlerde arar ★",
    ),
    (
        "Ctrl+C / Ctrl+X / Ctrl+V",
        "Kopyala / Kes / Yapıştır (Explorer ile karşılıklı)",
    ),
    (
        "Ctrl+Z",
        "Geri al: yeniden adlandırma, taşıma, kopyalama, silme",
    ),
    ("F2", "Yeniden adlandır (yerinde)"),
    ("Del / Shift+Del", "Geri dönüşüm kutusuna / kalıcı sil"),
    ("Ctrl+Shift+N", "Yeni klasör"),
    ("Ctrl+A", "Tümünü seç"),
    ("Shift+ok / Ctrl+Boşluk", "Aralık seç / tek tek seç"),
    ("Harf yazmak", "O harfle başlayan öğeye atla"),
    ("Alt+Enter", "Özellikler"),
    ("Alt+P", "Önizleme bölmesi"),
    ("Alt+Shift+P", "Ayrıntılar bölmesi"),
    ("Boşluk", "Peek ile önizleme (PowerToys)"),
    ("Ctrl+Shift+1…6", "Çok büyük … ayrıntılar görünümü"),
    ("Ctrl+fare tekerleği", "Simge boyutunu değiştir"),
    ("Ctrl+T / Ctrl+W", "Yeni sekme / sekmeyi kapat"),
    ("Ctrl+Tab / Ctrl+1…9", "Sekmeler arası geçiş"),
    ("Ctrl+N", "Yeni pencere"),
    ("Shift+sağ tık / Shift+F10", "Windows'un tam sağ tık menüsü"),
    ("Orta tık", "Klasörü yeni sekmede aç / sekmeyi kapat ★"),
    (
        "Sürükle-bırak",
        "Aynı sürücü: taşı, farklı sürücü: kopyala (Ctrl: kopyala, Shift: taşı)",
    ),
    ("Ctrl+Shift+C", "Yolu kopyala"),
    ("Ctrl+D", "Hızlı erişime sabitle ★"),
    ("Ctrl+H", "Gizli öğeleri göster/gizle ★"),
    ("F5 / Ctrl+R", "Yenile"),
    ("F11", "Tam ekran"),
    ("Esc", "Aramayı kapat / filtreyi temizle / seçimi kaldır"),
];

// ================================================================ ana döngü

/// Kenarlıksız pencereyi kenarlarından boyutlandırma (Windows'ta bunu WM_NCHITTEST yapar)
#[cfg(not(windows))]
fn resize_edges(ctx: &egui::Context) {
    use egui::viewport::ResizeDirection as D;
    if ctx.input(|i| {
        i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false)
    }) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let r = ctx.screen_rect();
    let b = 6.0;
    let (l, rt, t, bt) = (
        pos.x < r.left() + b,
        pos.x > r.right() - b,
        pos.y < r.top() + b,
        pos.y > r.bottom() - b,
    );
    let (dir, icon) = match (l, rt, t, bt) {
        (true, _, true, _) => (D::NorthWest, egui::CursorIcon::ResizeNorthWest),
        (_, true, true, _) => (D::NorthEast, egui::CursorIcon::ResizeNorthEast),
        (true, _, _, true) => (D::SouthWest, egui::CursorIcon::ResizeSouthWest),
        (_, true, _, true) => (D::SouthEast, egui::CursorIcon::ResizeSouthEast),
        (true, ..) => (D::West, egui::CursorIcon::ResizeWest),
        (_, true, ..) => (D::East, egui::CursorIcon::ResizeEast),
        (_, _, true, _) => (D::North, egui::CursorIcon::ResizeNorth),
        (.., true) => (D::South, egui::CursorIcon::ResizeSouth),
        _ => return,
    };
    ctx.set_cursor_icon(icon);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let perf_t0 = std::time::Instant::now();
        #[cfg(feature = "screenshot")]
        crate::screenshot::tick(ctx);
        #[cfg(not(windows))]
        let _ = frame;
        #[cfg(not(windows))]
        resize_edges(ctx);
        #[cfg(windows)]
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        #[cfg(windows)]
        if self.hwnd.is_none() {
            if let Ok(h) = frame.window_handle() {
                if let RawWindowHandle::Win32(w) = h.as_raw() {
                    let hwnd = w.hwnd.get();
                    self.hwnd = Some(hwnd);
                    crate::system::set_hwnd(hwnd);
                    crate::system::apply_backdrop(ctx.style().visuals.dark_mode);
                    shell::install_subclass(hwnd);
                    // eframe pencereyi ilk kareden sonra gösterir; o an DWM henüz
                    // GL yüzeyini almadığından bir kare beyaz parlar. Hazır olana dek gizle.
                    shell::set_cloak(hwnd, true);
                }
            }
        }

        // Vurgu rengi, animasyon veya ölçek (DPI / metin boyutu) değiştiyse beklemeden uygula
        let scale_changed = {
            let ppp = ctx.pixels_per_point();
            (ppp - f32::from_bits(UI_PPP.load(Relaxed))).abs() > 0.01
        };
        if crate::system::take_dirty() || scale_changed {
            self.accent = theme_accent(self.theme);
            apply_system_scale(ctx);
            apply_theme(ctx, self.accent, self.theme);
            ctx.request_repaint();
        }
        if std::env::var_os("SWIFT_EXPLORER_WHEELLOG").is_some() {
            wheel_log(ctx);
        }

        if let Some(start) = win::take_reopen() {
            self.reopen(start);
        }
        self.poll_update();
        if self.update_installed && ctx.input(|i| i.viewport().close_requested()) {
            // Gizlenmek yerine çık; yeni sürüm arka planda bekleyecek
            if self.resident && self.background {
                crate::update::launch_hidden();
            }
        } else if self.resident && self.background && ctx.input(|i| i.viewport().close_requested())
        {
            if let Some(hwnd) = self.hwnd {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.park(hwnd);
            }
        }

        let mut actions = Vec::new();
        self.images.poll(ctx);
        self.repair_restored_window(ctx);
        self.minimized = ctx.input(|i| i.viewport().minimized == Some(true));
        self.reveal_when_ready(ctx);
        if self.revealed {
            crate::update::confirm_started();
        }

        self.tabs[self.active].ensure_loaded(ctx);
        self.poll_subdirs();
        for t in &mut self.tabs {
            t.poll(ctx);
        }
        let (sort, asc, hidden) = (self.sort, self.asc, self.show_hidden);
        for t in &mut self.tabs {
            if t.view_dirty {
                let grouped = self.grouped.contains(&t.path);
                t.rebuild_view(sort, asc, hidden, grouped);
            }
        }
        // Klasör değiştiyse o klasörün kendi görünümünü uygula (sekme, gezinti,
        // adres çubuğu, geri/ileri — hepsi buradan geçer).
        self.sync_folder_view();
        // Yeni oluşturulan öğe listeye gelince yerinde adlandırmaya geç
        {
            let t = &mut self.tabs[self.active];
            if let Some(p) = t.rename_after_load.clone() {
                if let Some(e) = t.entries.iter().find(|e| e.path == p) {
                    self.renaming = Some(Renaming {
                        path: p,
                        text: e.name.clone(),
                        first: true,
                    });
                    t.rename_after_load = None;
                } else if !t.is_loading() && t.select_after_load.is_none() {
                    t.rename_after_load = None;
                }
            }
        }
        // Son klasörler
        let cur = self.tabs[self.active].path.clone();
        if !crate::tab::is_virtual(&cur) && self.recent.first() != Some(&cur) {
            self.recent.retain(|p| p != &cur);
            self.recent.insert(0, cur);
            self.recent.truncate(30);
        }

        // Biten işler → geri-al kaydı + bildirim
        let mut finished = Vec::new();
        self.jobs.retain(|j| {
            let f = j.finished.load(Relaxed);
            if f {
                finished.push(j.clone());
            }
            !f
        });
        for j in finished {
            let key = Arc::as_ptr(&j) as usize;
            let silent = self.silent_jobs.contains(&key);
            self.silent_jobs.retain(|k| *k != key);
            let pairs = j.pairs.lock().unwrap().clone();
            let pair_count = pairs.len();
            if !silent && !pairs.is_empty() {
                match j.kind {
                    JobKind::Transfer if j.cut => self.undo.push(UndoOp::Moved(pairs)),
                    JobKind::Transfer => self
                        .undo
                        .push(UndoOp::Copied(pairs.into_iter().map(|(_, d)| d).collect())),
                    JobKind::Delete if !j.permanent => self
                        .undo
                        .push(UndoOp::Trashed(pairs.into_iter().map(|(s, _)| s).collect())),
                    _ => {}
                }
            }
            let errs = j.errors.lock().unwrap().clone();
            if let Some(first) = errs.first() {
                let more = if errs.len() > 1 {
                    crate::i18n::tf(" (+{} hata)", &[&(errs.len() - 1)])
                } else {
                    String::new()
                };
                self.toast(format!("{first}{more}"), true);
            } else if j.cancel.load(Relaxed) {
                self.toast(crate::i18n::tf("İptal edildi: {}", &[&j.label]), false);
            } else if j.kind == JobKind::Delete && !j.permanent {
                self.toast(
                    crate::i18n::tf(
                        "{} öğe geri dönüşüm kutusuna taşındı — geri almak için Ctrl+Z",
                        &[&pair_count],
                    ),
                    false,
                );
            } else if j.kind == JobKind::Restore {
                self.toast(
                    crate::i18n::t("Geri dönüşüm kutusundan geri yüklendi"),
                    false,
                );
            } else if j.kind == JobKind::Transfer && j.started.elapsed() > Duration::from_secs(2) {
                self.toast(
                    crate::i18n::tf("Diske yazıldı, sürücüyü çıkarabilirsiniz: {}", &[&j.label]),
                    false,
                );
            }
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        // Kaydırma sırasında diske yazma pahalı işler ertelensin; hiç durmayan
        // kaydırmalarda kayıt, hareketsizlikte yapılır (save_config UI dişi
        // disk yazımı — kaydırma sırasında 80 ms'lik takılmalar yapar).
        let scroll_motion = ctx.input(|i| {
            i.smooth_scroll_delta != egui::Vec2::ZERO || i.raw_scroll_delta != egui::Vec2::ZERO
        });
        if !scroll_motion {
            // Ayarları/sekmeleri değiştiyse arada bir kaydet (çökme veya zorla kapatmada kaybolmasın)
            if self.last_autosave.elapsed() > Duration::from_secs(3) {
                self.last_autosave = Instant::now();
                self.save_config();
            }
            if self.drives_refreshed.elapsed() > Duration::from_secs(20) {
                self.drives_refreshed = Instant::now();
                self.refresh_drives();
            }
        }
        ctx.request_repaint_after(Duration::from_secs(20));
        ctx.request_repaint_after(Duration::from_secs(20));

        // Dışarıdan sürüklenip bırakılan dosyalar → bu klasöre kopyala
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            let t = &self.tabs[self.active];
            if !t.is_home() && t.search.is_none() {
                let dest = t.path.clone();
                self.start_transfer(dropped, dest, false);
            }
        }

        self.handle_keys(ctx, &mut actions);
        let perf_pre = perf_t0.elapsed().as_secs_f32() * 1000.0;

        // Explorer düzeni: sekme şeridi, gezinme satırı, komut çubuğu (aralarında ince çizgiler),
        // solda gezinti bölmesi, altta durum çubuğu
        let pl = pal(ctx.style().visuals.dark_mode);
        let hline = |ctx: &egui::Context, rect: Rect| {
            ctx.layer_painter(egui::LayerId::background()).hline(
                rect.x_range(),
                rect.bottom() - 0.5,
                Stroke::new(1.0_f32, pl.line),
            );
        };
        egui::TopBottomPanel::top("tabs")
            .exact_height(title_h())
            .frame(
                egui::Frame::new()
                    .fill(pl.strip)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                self.tab_bar(ui, &mut actions);
            });
        let r = egui::TopBottomPanel::top("nav")
            .frame(egui::Frame::new().fill(pl.nav).inner_margin(egui::Margin {
                left: 8,
                right: 12,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show(ctx, |ui| {
                self.nav_bar(ui, &mut actions);
            });
        hline(ctx, r.response.rect);
        let r = egui::TopBottomPanel::top("cmd")
            .frame(
                egui::Frame::new()
                    .fill(pl.content)
                    .inner_margin(egui::Margin {
                        left: 12,
                        right: 12,
                        top: 7,
                        bottom: 7,
                    }),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                self.command_bar(ui, &mut actions);
            });
        hline(ctx, r.response.rect);
        egui::TopBottomPanel::bottom("status")
            .exact_height((23.0 * text_factor()).round())
            .frame(
                egui::Frame::new()
                    .fill(pl.status)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 8,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show_separator_line(false)
            .show(ctx, |ui| {
                self.status_bar(ui, &mut actions);
            });
        egui::SidePanel::left("nav_pane")
            .resizable(true)
            .default_width((210.0 * text_factor()).round())
            .width_range((150.0 * text_factor()).round()..=(480.0 * text_factor()).round())
            .frame(
                egui::Frame::new()
                    .fill(pl.content)
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                self.sidebar(ui, &mut actions);
            });
        if self.pane != 0 {
            egui::SidePanel::right("sidepane")
                .resizable(true)
                .default_width(320.0)
                .width_range(220.0..=900.0)
                .frame(egui::Frame::new().fill(pl.content).inner_margin(12))
                .show(ctx, |ui| {
                    if self.pane == 1 {
                        self.preview_panel(ui);
                    } else {
                        self.details_panel(ui);
                    }
                });
        }
        let perf_center = perf_t0.elapsed().as_secs_f32() * 1000.0;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(pl.content)
                    .inner_margin(egui::Margin {
                        left: 6,
                        right: 4,
                        top: 2,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                let t = &self.tabs[self.active];
                if t.is_gallery() || t.lists_files() {
                    self.file_area(ui, &mut actions);
                } else if t.is_pc() {
                    self.pc_page(ui, &mut actions);
                } else {
                    self.home_page(ui, &mut actions);
                }
            });

        let perf_dialogs = perf_t0.elapsed().as_secs_f32() * 1000.0;
        self.dialogs(ctx);
        self.overlays(ctx);
        self.drag_overlay(ctx);
        self.draw_context_menu(ctx, &mut actions);

        // Pencereden dışarı sürükleme → Windows'un sürükle-bırak'ı (masaüstü, Explorer, diğer uygulamalar)
        if let Some(p) = DragAndDrop::payload::<DragPaths>(ctx) {
            ctx.request_repaint();
            if let Some(hwnd) = self.hwnd {
                if shell::cursor_outside_window(hwnd) && ctx.input(|i| i.pointer.primary_down()) {
                    let paths = p.0.clone();
                    DragAndDrop::clear_payload(ctx);
                    self.marquee = None;
                    shell::drag_out(&paths);
                    self.inject_release =
                        Some(ctx.input(|i| i.pointer.latest_pos()).unwrap_or_default());
                    ctx.request_repaint();
                }
            }
        }

        for a in actions {
            self.apply(a);
        }

        let t = &self.tabs[self.active];
        let title = format!(
            "{} — Swift Explorer",
            if t.is_home() {
                self.loc_name(&t.path)
            } else {
                t.path.display().to_string()
            }
        );
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        if std::env::var_os("SWIFT_EXPLORER_WHEELLOG").is_some() {
            perf_log(
                ctx,
                &self.images,
                perf_pre,
                perf_center - perf_pre,
                perf_dialogs - perf_center,
                perf_t0.elapsed().as_secs_f32() * 1000.0 - perf_dialogs,
            );
        }
        // Kaydırma hareketi sırasında: Windows, art arda gelen giriş
        // mesajları yüzünden WM_PAINT'i erteleyebiliyor (~30 Hz'e kilitlenme);
        // zamanlayıcı nudge ile çerçeve hızını 60 Hz'de tut.
        let scroll_motion = ctx.input(|i| {
            i.smooth_scroll_delta != egui::Vec2::ZERO || i.raw_scroll_delta != egui::Vec2::ZERO
        });
        if scroll_motion {
            ctx.request_repaint_after(Duration::from_millis(15));
        }
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        #[cfg(feature = "screenshot")]
        crate::screenshot::input_hook(raw);
        // OLE sürükleme / yerel menü fare bırakma olayını yuttu; egui'ye bildir
        if let Some(pos) = self.inject_release.take() {
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: raw.modifiers,
            });
            raw.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Secondary,
                pressed: false,
                modifiers: raw.modifiers,
            });
        }
    }

    /// Simge durumundaki (küçültülmüş) pencerenin geometrisi eframe tarafından bozuk
    /// kaydedilir: konum (-32000, -32000), boyut ~160×33 piksel. O anki kaydı atlayıp
    /// son geçerli pencere durumunu geri yazarak bir sonraki açılışın bozulmasını önler.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.minimized {
            if let Some(prev) = &self.good_window {
                storage.set_string("window", prev.clone());
            }
            return;
        }
        if let Some(w) = storage.get_string("window") {
            self.good_window = Some(w);
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        for t in &mut self.tabs {
            t.cancel_search();
        }
        self.save_config();
    }
}
