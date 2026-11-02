//! Gömülü PDF / ses / video / HTML görüntüleyici (Windows).
//!
//! Edge WebView2 çalışma zamanı Windows 11'de kurulu gelir; PDF'yi kendi
//! görüntüleyicisiyle açar, ses/videoyu HTML5 oynatıcıyla çalar, yerel HTML
//! sayfalarını tarayıcı olarak işler. Görüntüleyici uygulamanın alt penceresi
//! olarak önizleme dikdörtgenine yerleştirilir.
//!
//! Oluşturma eşzamansızdır: ilk istek geldiğinde ortam/denetleyici istenir,
//! tamamlanma geri çağrısı bir sonraki ileti pompasında çalışır ve yeniden çizim
//! ister. Bu arada çağıran taraf küçük resim/meta veri gösterir.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use windows::core::{Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{BOOL, HWND, RECT};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Controller, ICoreWebView2Controller3,
    ICoreWebView2EnvironmentOptions, ICoreWebView2_3, COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
};
use webview2_com::{
    AcceleratorKeyPressedEventHandler, CoreWebView2EnvironmentOptions,
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    ExecuteScriptCompletedHandler,
};

use crate::media::MediaKind;

/// Görüntüleyicinin kaplaması istenen alan (egui noktaları, istemci köşesine göre).
#[derive(Clone)]
pub struct Want {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub rect: egui::Rect,
    /// Oynatıcı sayfasının teması (uygulamanın temasıyla aynı olsun).
    pub dark: bool,
    /// Vurgu rengi `#RRGGBB`.
    pub accent: String,
    /// Arka plan rengi `#RRGGBB`: oynatıcı sayfası çevresiyle aynı tonu kullanır.
    pub bg: String,
}

/// Oynatıcı içindeyken uygulamaya iletilen tuşlar.
///
/// WebView2 odak aldığında tuşlar egui'ye ulaşmaz; Boşluk/Esc yakalanıp
/// buraya yazılır, uygulama her karede okur (önizlemeyi kapatmak için).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HotKey {
    Space,
    Escape,
}

enum State {
    Idle,
    Creating,
    Ready(ICoreWebView2Controller),
    Failed,
}

pub struct WebView {
    /// WebView2 denetleyicisi yalnızca UI iş parçacığında kullanılır; bu yüzden
    /// `Send`/`Sync` gerekmez (COM geri çağrıları da aynı iş parçacığında çalışır).
    state: Rc<RefCell<State>>,
    /// En son yüklenen dosya ve türü; aynı dosya için yeniden gezinilmez.
    shown: Option<(PathBuf, MediaKind, bool)>,
    visible: bool,
    bounds: Option<[i32; 4]>,
    scale: f32,
    /// Oynatıcı içindeyken yakalanan tuşlar (bkz. `HotKey`).
    keys: Rc<RefCell<Vec<HotKey>>>,
}

impl WebView {
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(State::Idle)),
            shown: None,
            visible: false,
            bounds: None,
            scale: 0.0,
            keys: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Görüntüleyici içindeyken basılan Boşluk/Esc tuşları (bir kez okunur).
    pub fn take_keys(&self) -> Vec<HotKey> {
        std::mem::take(&mut *self.keys.borrow_mut())
    }

    /// Görüntüleyici hazır mı? Değilse önizleme küçük resim/meta veri gösterir.
    pub fn ready(&self) -> bool {
        matches!(&*self.state.borrow(), State::Ready(_))
    }

    /// Ortam/denetleyici oluşturulamadı (WebView2 çalışma zamanı yok ya da hata).
    pub fn failed(&self) -> bool {
        matches!(&*self.state.borrow(), State::Failed)
    }

    /// Her karede çağrılır: istenen alanı ve görünürlüğü uygular.
    ///
    /// `allow` yanlışsa (menü/dialog açık) görüntüleyici gizlenir; egui içeriği
    /// alt pencerenin altında kalmasın.
    pub fn sync(
        &mut self,
        hwnd: Option<isize>,
        want: Option<&Want>,
        allow: bool,
        ctx: &egui::Context,
    ) {
        let idle = matches!(&*self.state.borrow(), State::Idle);
        if idle && want.is_some() {
            if let Some(h) = hwnd {
                self.start(h, ctx);
            }
        }
        let controller = {
            let st = self.state.borrow();
            match &*st {
                State::Ready(c) => c.clone(),
                _ => return,
            }
        };
        let w = match want {
            Some(w) if allow && w.rect.width() >= 8.0 && w.rect.height() >= 8.0 => w,
            _ => {
                if self.visible {
                    // Gizlenirken ses/video duraklasın: önizleme kapanınca çalmaya
                    // devam eden bir oynatıcı kalmasın.
                    pause(&controller);
                    unsafe {
                        let _ = controller.SetIsVisible(BOOL(0));
                    }
                    self.visible = false;
                }
                return;
            }
        };

        let scale = ctx.pixels_per_point();
        let b = [
            w.rect.left().round() as i32,
            w.rect.top().round() as i32,
            w.rect.right().round() as i32,
            w.rect.bottom().round() as i32,
        ];
        if self.bounds != Some(b) {
            let rc = RECT {
                left: b[0],
                top: b[1],
                right: b[2],
                bottom: b[3],
            };
            unsafe {
                let _ = controller.SetBounds(rc);
            }
            self.bounds = Some(b);
        }
        if (scale - self.scale).abs() > 0.001 {
            if let Ok(c3) = controller.cast::<ICoreWebView2Controller3>() {
                unsafe {
                    let _ = c3.SetRasterizationScale(scale as f64);
                }
            }
            self.scale = scale;
        }

        let changed = self
            .shown
            .as_ref()
            .is_none_or(|(p, k, d)| p != &w.path || *k != w.kind || *d != w.dark);
        if changed {
            if let Ok(core) = unsafe { controller.CoreWebView2() } {
                match w.kind {
                    // PDF ve HTML doğrudan `file:///` adresinden açılır: HTML'in
                    // göreli CSS/görsel/script bağlantıları böylece çözülür.
                    MediaKind::Pdf | MediaKind::Html => {
                        let uri = file_uri(&w.path);
                        unsafe {
                            let _ = core.Navigate(&HSTRING::from(uri));
                        }
                    }
                    kind => {
                        let player = player_page(kind, &w.path, w.dark, &w.accent, &w.bg);
                        if let Some(page) = player {
                            unsafe {
                                if let Ok(core3) = core.cast::<ICoreWebView2_3>() {
                                    let _ = core3.SetVirtualHostNameToFolderMapping(
                                        &HSTRING::from(APP_HOST),
                                        &HSTRING::from(assets_dir().to_string_lossy().as_ref()),
                                        COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                                    );
                                    if let Some(parent) = w.path.parent() {
                                        let _ = core3.SetVirtualHostNameToFolderMapping(
                                            &HSTRING::from(MEDIA_HOST),
                                            &HSTRING::from(parent.to_string_lossy().as_ref()),
                                            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
                                        );
                                    }
                                }
                                let _ = core.Navigate(&HSTRING::from(page));
                            }
                        }
                    }
                }
            }
            self.shown = Some((w.path.clone(), w.kind, w.dark));
        }

        if !self.visible {
            unsafe {
                let _ = controller.SetIsVisible(BOOL(1));
                // Odak egui'de kalsın: Boşluk/ok tuşları listeyi yönetmeye devam etsin.
                if let Some(h) = hwnd {
                    windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(
                        h as *mut core::ffi::c_void,
                    );
                }
            }
            self.visible = true;
        }
    }

    /// Ortamı ve denetleyiciyi eşzamansız ister (bir kez).
    fn start(&mut self, hwnd: isize, ctx: &egui::Context) {
        // WebView2 STA COM ister; pencere iş parçacığı henüz başlatılmamış olabilir.
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
            );
        }
        *self.state.borrow_mut() = State::Creating;
        let data = data_dir();
        let _ = std::fs::create_dir_all(&data);
        let assets = assets_dir();
        let _ = std::fs::create_dir_all(&assets);
        let _ = std::fs::write(assets.join("player.html"), PLAYER_HTML);

        let state = Rc::clone(&self.state);
        let keys = Rc::clone(&self.keys);
        let ctx2 = ctx.clone();
        let handler =
            CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |hr, env| {
                let Some(env) = env.filter(|_| hr.is_ok()) else {
                    *state.borrow_mut() = State::Failed;
                    ctx2.request_repaint();
                    return Ok(());
                };
                let state = Rc::clone(&state);
                let failed = Rc::clone(&state);
                let keys = Rc::clone(&keys);
                let ctx3 = ctx2.clone();
                let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |hr, controller| {
                        match controller.filter(|_| hr.is_ok()) {
                            Some(c) => {
                                configure(&c);
                                hook_keys(&c, Rc::clone(&keys), ctx3.clone());
                                *state.borrow_mut() = State::Ready(c);
                            }
                            None => *state.borrow_mut() = State::Failed,
                        }
                        ctx3.request_repaint();
                        Ok(())
                    },
                ));
                if let Err(e) =
                    unsafe { env.CreateCoreWebView2Controller(HWND(hwnd as *mut _), &handler) }
                {
                    eprintln!("webview: denetleyici oluşturulamadı: {e}");
                    *failed.borrow_mut() = State::Failed;
                    ctx2.request_repaint();
                }
                Ok(())
            }));

        // Ses/video açılır açılmaz başlasın (Chromium varsayılan olarak kullanıcı
        // hareketi bekler).
        let opts = CoreWebView2EnvironmentOptions::default();
        unsafe {
            opts.set_additional_browser_arguments(String::from(
                "--autoplay-policy=no-user-gesture-required",
            ));
        }
        let options: ICoreWebView2EnvironmentOptions = opts.into();
        let folder = HSTRING::from(data.to_string_lossy().as_ref());
        let result = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(None::<&PCWSTR>, &folder, &options, &handler)
        };
        if let Err(e) = result {
            eprintln!("webview: ortam oluşturulamadı: {e}");
            *self.state.borrow_mut() = State::Failed;
        }
    }
}

fn configure(c: &ICoreWebView2Controller) {
    unsafe {
        let _ = c.SetIsVisible(BOOL(0));
        if let Ok(c3) = c.cast::<ICoreWebView2Controller3>() {
            let _ = c3.SetRasterizationScale(1.0);
        }
        if let Ok(core) = c.CoreWebView2() {
            if let Ok(s) = core.Settings() {
                let _ = s.SetAreDevToolsEnabled(BOOL(0));
                let _ = s.SetIsStatusBarEnabled(BOOL(0));
                let _ = s.SetIsZoomControlEnabled(BOOL(0));
            }
        }
    }
}

/// Oynatıcıdaki ses/videoyu duraklatır (önizleme kapanırken/gizlenirken).
fn pause(c: &ICoreWebView2Controller) {
    if let Ok(core) = unsafe { c.CoreWebView2() } {
        let done = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
        unsafe {
            let _ = core.ExecuteScript(
                &HSTRING::from(
                    "document.querySelectorAll('video,audio').forEach(function(m){try{m.pause()}catch(e){}})",
                ),
                &done,
            );
        }
    }
}

/// Boşluk ve Esc tuşlarını uygulamaya iletir: WebView2 odaklıyken de önizleme
/// kapanabilsin (tuşlar aksi hâlde egui'ye hiç ulaşmaz).
fn hook_keys(c: &ICoreWebView2Controller, keys: Rc<RefCell<Vec<HotKey>>>, ctx: egui::Context) {
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(a) = args else {
            return Ok(());
        };
        let mut vk = 0u32;
        if unsafe { a.VirtualKey(&mut vk) }.is_err() {
            return Ok(());
        }
        let key = match vk {
            0x20 => Some(HotKey::Space),
            0x1B => Some(HotKey::Escape),
            _ => None,
        };
        if let Some(k) = key {
            unsafe {
                let _ = a.SetHandled(BOOL(1));
            }
            keys.borrow_mut().push(k);
            // Tuş egui'ye hiç ulaşmadığı için kareyi kendimiz istemeliyiz.
            ctx.request_repaint();
        }
        Ok(())
    }));
    let mut token = Default::default();
    unsafe {
        let _ = c.add_AcceleratorKeyPressed(&handler, &mut token);
    }
}

const APP_HOST: &str = "swift-app.local";
const MEDIA_HOST: &str = "swift-media.local";
const PLAYER_HTML: &str = include_str!("player.html");

fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("StarFile")
        .join("WebView2")
}

fn assets_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("StarFile")
        .join("web")
}

/// `file:///C:/...` adresi (PDF görüntüleyicisi yerel dosyayı doğrudan açar).
fn file_uri(path: &Path) -> String {
    let s = path.to_string_lossy().replace('\\', "/");
    format!("file:///{}", percent(&s, true))
}

/// Oynatıcı sayfasının adresi; medya, ebeveyn klasörünün sanal ana bilgisayarından gelir.
fn player_page(kind: MediaKind, path: &Path, dark: bool, accent: &str, bg: &str) -> Option<String> {
    let name = path.file_name()?.to_string_lossy();
    let kind = match kind {
        MediaKind::Audio => "audio",
        MediaKind::Video => "video",
        MediaKind::Pdf | MediaKind::Html => return None,
    };
    let theme = if dark { "dark" } else { "light" };
    Some(format!(
        "https://{APP_HOST}/player.html?kind={kind}&theme={theme}&accent={}&bg={}&name={}&src=https%3A%2F%2F{MEDIA_HOST}%2F{}",
        percent(accent, false),
        percent(bg, false),
        percent(&name, false),
        percent(&name, false)
    ))
}

/// Yüzde kodlama: yol ayırıcıları korunabilir, diğer her şey UTF-8 baytlarıyla kodlanır.
fn percent(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric()
            || matches!(c, '-' | '_' | '.' | '~' | ':')
            || (keep_slash && c == '/')
        {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_keep_drive_and_slashes() {
        assert_eq!(
            file_uri(Path::new("C:\\Users\\a b\\x.pdf")),
            "file:///C:/Users/a%20b/x.pdf"
        );
    }

    #[test]
    fn media_urls_are_encoded() {
        let page = player_page(
            MediaKind::Audio,
            Path::new("C:\\m\\şarkı #1.mp3"),
            true,
            "#0078D4",
            "#202020",
        )
        .unwrap();
        assert!(page.contains("kind=audio"));
        assert!(page.contains("theme=dark"));
        assert!(page.contains("accent=%230078D4"));
        assert!(page.contains("bg=%23202020"));
        assert!(page.contains("%C5%9Fark%C4%B1%20%231.mp3"));
    }
}
