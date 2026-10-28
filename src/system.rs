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
        DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_ROUND,
    };
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

    /// Yuvarlatılmış köşeler ve koyu pencere çerçevesi.
    ///
    /// `DWMWA_USE_IMMERSIVE_DARK_MODE` özel başlık çubuğu kullanan pencerelerde ince
    /// çerçeveyi/gölgeyi koyu moda uyarlar; `DWMWA_WINDOW_CORNER_PREFERENCE` Windows 11'in
    /// yuvarlatılmış köşelerini etkinleştirir. Windows 10'da ikinci çağrı hata verir, zararsız.
    pub fn apply_backdrop(dark: bool) {
        let hwnd = super::HWND.load(Ordering::Relaxed);
        if hwnd == 0 {
            return;
        }
        let hwnd = HWND(hwnd as *mut c_void);
        unsafe {
            let dark = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const BOOL as *const c_void,
                std::mem::size_of::<BOOL>() as u32,
            );
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &corner as *const _ as *const c_void,
                std::mem::size_of_val(&corner) as u32,
            );
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

    pub fn apply_backdrop(_dark: bool) {}
}

pub use imp::*;
