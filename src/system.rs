//! Windows kişiselleştirme ayarları: animasyon tercihi, DPI farkındalığı
//! ve uygulama kimliği. Linux'ta bu değerler sabittir; arayüz kendi fontunu kullanır.
//!
//! `WM_SETTINGCHANGE` / `WM_THEMECHANGED` geldiğinde `mark_dirty()` çağrılır; `App::update`
//! bir sonraki karede `take_dirty()` görüp temayı yeniden uygular.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::OnceLock;

static DIRTY: AtomicBool = AtomicBool::new(false);
static CTX: OnceLock<egui::Context> = OnceLock::new();
static HWND: AtomicIsize = AtomicIsize::new(0);
/// Akrilik (buzlu cam) katmanı başarıyla kuruldu mu; `App` tuvali buna göre çizer.
static ACRYLIC: AtomicBool = AtomicBool::new(false);

/// Ana pencere tutamacını kaydet; DWM çağrıları pencere oluştuktan sonra yapılabilir.
pub fn set_hwnd(hwnd: isize) {
    HWND.store(hwnd, Ordering::Relaxed);
}

/// Ayar değişti; bir sonraki karede yeniden uygulanmalı.
pub fn mark_dirty() {
    DIRTY.store(true, Ordering::Relaxed);
}

/// Değişiklik bayrağını okur ve temizler.
pub fn take_dirty() -> bool {
    DIRTY.swap(false, Ordering::Relaxed)
}

/// Akrilik buzlu cam katmanı etkin mi (bkz. `apply_backdrop`).
pub fn backdrop_ok() -> bool {
    ACRYLIC.load(Ordering::Relaxed)
}

/// Alt sınıf işlemcisinin repaint isteyebilmesi için egui bağlamı.
pub fn set_ctx(ctx: egui::Context) {
    let _ = CTX.set(ctx);
}

/// Ayar değişikliğinden sonra arayüzü hemen çizdir.
pub fn repaint() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::Ordering;

    use windows::Win32::Foundation::{BOOL, HWND};
    use windows::Win32::Graphics::Dwm::{
        DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMSBT_TRANSIENTWINDOW,
        DWMWA_BORDER_COLOR, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    };
    use windows::Win32::UI::Controls::MARGINS;
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETWHEELSCROLLLINES,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Ayarlar > Erişilebilirlik > Görsel efektler > Animasyonlar.
    pub fn animations() -> bool {
        let mut on: i32 = 1;
        unsafe {
            SystemParametersInfoW(
                SPI_GETCLIENTAREAANIMATION,
                0,
                &mut on as *mut i32 as *mut c_void,
                0,
            );
        }
        on != 0
    }

    /// Pencere oluşturulmadan önce çağrılmalı; winit de aynısını yapar ama yarış olmaması için
    /// burada erkenden ayarlanır. Böylece ilk kare doğru DPI ile çizilir.
    pub fn apply_dpi_awareness() {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        unsafe {
            let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
    }

    /// Görev çubuğu gruplaması, sabitleme ve bildirim kimliği için süreç AUMID'si.
    /// Kurulumun oluşturduğu kısayol da aynı kimliği taşımalı.
    pub fn set_app_id() {
        let id = wide(crate::APP_ID);
        unsafe {
            let _ = SetCurrentProcessExplicitAppUserModelID(id.as_ptr());
        }
    }

    /// Windows "Metin boyutu" ayarı (Ayarlar > Erişilebilirlik > Metin boyutu), 100 = %100.
    /// Explorer ve diğer Win32 uygulamaları bu ayarı arayüzlerine uygular; bulunamazsa 1.0.
    pub fn text_scale() -> f32 {
        use windows_sys::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
        };
        let key = wide("Software\\Microsoft\\Accessibility");
        let value = wide("TextScaleFactor");
        let mut data: u32 = 100;
        let mut len = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut c_void,
                &mut len,
            )
        };
        if rc != 0 {
            return 1.0;
        }
        (data as f32 / 100.0).clamp(1.0, 2.25)
    }

    /// Sistemin ileti (message) yazı tipi yüksekliği, piksel cinsinden.
    ///
    /// `SPI_GETNONCLIENTMETRICS` değerleri DPI **ve** Windows "Metin boyutu"
    /// ayarını birlikte yansıtır; %100 ölçekte 12, %162'de 19 döner. Explorer
    /// liste/gezinme yazısını bu boyda çizer. Alınamazsa `None`.
    pub fn message_font_px() -> Option<f32> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS,
        };
        let mut ncm: NONCLIENTMETRICSW = unsafe { std::mem::zeroed() };
        ncm.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETNONCLIENTMETRICS,
                ncm.cbSize,
                &mut ncm as *mut _ as *mut c_void,
                0,
            )
        };
        if ok == 0 {
            return None;
        }
        let height = ncm.lfMessageFont.lfHeight.unsigned_abs() as f32;
        (height >= 6.0).then_some(height)
    }

    /// Windows "Fare tekerleği bir diş döndürüldüğünde kaç satır kaydırılsın"
    /// ayarı (SPI_GETWHEELSCROLLLINES, Ayarlar > Bluetooth ve cihazlar > Fare).
    /// u32::MAX (-1) "bir sayfa" demektir; kabaca beş satır diyelim.
    /// Alınamazsa Windows'un varsayılanı olan 3 döner.
    pub fn wheel_scroll_lines() -> u32 {
        let mut lines: u32 = 3;
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                &mut lines as *mut u32 as *mut c_void,
                0,
            )
        };
        if ok == 0 || lines == u32::MAX {
            return 5;
        }
        lines.clamp(1, 12)
    }

    /// Pencerenin GL piksel biçimindeki alfa biti sayısı.
    ///
    /// Mica camının görünmesi için yüzeyin gerçekten saydam olması gerekir; bu sayı
    /// 0 ise saydamlık yoktur ve tuval opak kalmalıdır (aksi halde arka plan olduğundan
    /// daha koyu görünür). glutin pencere DC'sine biçimi kurduktan sonra okunur.
    pub fn window_alpha_bits(hwnd: isize) -> u8 {
        use windows_sys::Win32::Graphics::Gdi::{GetDC, ReleaseDC};
        use windows_sys::Win32::Graphics::OpenGL::{
            DescribePixelFormat, GetPixelFormat, PIXELFORMATDESCRIPTOR,
        };
        unsafe {
            let hwnd = hwnd as *mut c_void;
            let hdc = GetDC(hwnd);
            if hdc.is_null() {
                return 0;
            }
            let mut pfd: PIXELFORMATDESCRIPTOR = std::mem::zeroed();
            let fmt = GetPixelFormat(hdc);
            let n = DescribePixelFormat(
                hdc,
                fmt,
                std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u32,
                &mut pfd,
            );
            ReleaseDC(hwnd, hdc);
            if n == 0 {
                0
            } else {
                pfd.cAlphaBits
            }
        }
    }

    /// Başlık çubuğu stillerini kaldırır.
    ///
    /// `DwmExtendFrameIntoClientArea` pencerenin tamamını çerçeveye çevirdiğinde,
    /// `WS_CAPTION`/`WS_SYSMENU` hâlâ duruyorsa DWM temanın kendi küçült/büyüt/kapat
    /// düğmelerini de çizer ve uygulamanın kendi düğmelerinin yanında ikinci bir takım
    /// belirir. Stilleri kaldırınca çerçeve yine saydam kalır (cam/saydamlık için
    /// yeterli) ama sistem düğmeleri çizilmez.
    unsafe fn strip_caption(hwnd: isize) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, SWP_FRAMECHANGED,
            SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_SYSMENU,
        };
        let hwnd = hwnd as *mut c_void;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let stripped = style & !(WS_CAPTION | WS_SYSMENU);
        if stripped == style {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_STYLE, stripped as isize);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }

    /// winit pencere bayrakları değiştiğinde stilleri yeniden yazar ve `WS_CAPTION`'ı
    /// geri getirir (bkz. winit `window_state.rs`); DWM de o zaman ikinci bir sistem
    /// düğmesi takımı çizer. Her karede ucuzca geri alınır: stil zaten temizse
    /// fonksiyon hiçbir şey yapmadan döner.
    pub fn ensure_no_caption(hwnd: isize) {
        unsafe { strip_caption(hwnd) }
    }

    /// Yuvarlatılmış köşeler, temaya uygun kenarlık ve akrilik (buzlu cam) arka plan.
    ///
    /// - `DWMWA_USE_IMMERSIVE_DARK_MODE`: özel başlık çubuğu kullanan pencerelerde ince
    ///   çerçeveyi/gölgeyi koyu moda uyarlar.
    /// - `DWMWA_WINDOW_CORNER_PREFERENCE`: Windows 11'in yuvarlatılmış köşeleri.
    /// - `DWMWA_BORDER_COLOR`: varsayılan gri kenar çizgisi yerine temanın ayraç rengi.
    /// - `DWMWA_SYSTEMBACKDROP_TYPE = TRANSIENTWINDOW`: akrilik. Mica'nın düz tonunun
    ///   aksine gürültülü/dokulu bir buzlanma verir. Görünmesi için `clear_color`
    ///   saydam ve `Pal::canvas`/`strip` yarı saydam çizilir.
    /// - `DwmExtendFrameIntoClientArea(-1)`: istemci alanını çerçeveye çevirir; DWM'nin
    ///   pencerenin alfa kanalını onurlandırması (yani camın görünmesi) buna bağlıdır.
    ///   Bu yüzden `strip_caption` ile sistem düğmeleri kapatılır.
    ///
    /// Windows 10'da desteklenmeyen çağrılar sessizce başarısız olur, zararsız.
    pub fn apply_backdrop(dark: bool, border: (u8, u8, u8)) {
        let raw = super::HWND.load(Ordering::Relaxed);
        if raw == 0 {
            return;
        }
        let hwnd = HWND(raw as *mut c_void);
        unsafe {
            let dark_flag = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark_flag as *const BOOL as *const c_void,
                std::mem::size_of::<BOOL>() as u32,
            );
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const c_void,
                std::mem::size_of_val(&corner) as u32,
            );
            // DWM kenarlık rengi COLORREF'tir: 0x00BBGGRR
            let color: u32 = border.0 as u32 | ((border.1 as u32) << 8) | ((border.2 as u32) << 16);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &color as *const _ as *const c_void,
                std::mem::size_of::<u32>() as u32,
            );
            // Sistem düğmeleri ikinci kez çizilmesin diye önce başlık stillerini kaldır.
            strip_caption(raw);
            let backdrop = DWMSBT_TRANSIENTWINDOW;
            let ok = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                &backdrop as *const _ as *const c_void,
                std::mem::size_of_val(&backdrop) as u32,
            )
            .is_ok();
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            let extended = DwmExtendFrameIntoClientArea(hwnd, &margins).is_ok();
            super::ACRYLIC.store(ok && extended, Ordering::Relaxed);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn animations() -> bool {
        true
    }

    pub fn apply_dpi_awareness() {}

    pub fn set_app_id() {}

    /// Linux'ta metin ölçeği ayrı bir ayar değil; masaüstü ölçeği DPI'ya yansır.
    pub fn text_scale() -> f32 {
        1.0
    }

    /// Linux'ta sistem yazı tipi metriği pencerelerden alınmaz.
    pub fn message_font_px() -> Option<f32> {
        None
    }

    /// Linux'ta tekerlek ayari okunmaz; Explorer'in varsayilani.
    pub fn wheel_scroll_lines() -> u32 {
        3
    }

    /// Linux'ta Mica/akrilik yok; pencere yüzeyi saydam sayılmaz.
    pub fn window_alpha_bits(_hwnd: isize) -> u8 {
        0
    }

    /// Linux'ta sistem başlık çubuğu yok.
    pub fn ensure_no_caption(_hwnd: isize) {}

    /// Linux'ta akrilik katman yok.
    pub fn backdrop_ok() -> bool {
        false
    }

    pub fn apply_backdrop(_dark: bool, _border: (u8, u8, u8)) {}
}

pub use imp::*;
