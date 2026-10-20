//! Windows kişiselleştirme ayarları: metin ölçeği, animasyon tercihi, DPI farkındalığı
//! ve uygulama kimliği. Linux'ta bu değerler sabittir; arayüz kendi fontunu kullanır.
//!
//! `WM_SETTINGCHANGE` / `WM_THEMECHANGED` geldiğinde `mark_dirty()` çağrılır; `App::update`
//! bir sonraki karede `take_dirty()` görüp temayı ve ölçeği yeniden uygular.

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
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `HKCU` altındaki bir DWORD değeri; yoksa veya farklı türdeyse `None`.
    fn dword(path: &str, name: &str) -> Option<u32> {
        let path = wide(path);
        let name = wide(name);
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut value as *mut u32 as *mut c_void,
                &mut size,
            )
        };
        (rc == 0).then_some(value)
    }

    /// Windows Ayarlar > Erişilebilirlik > Metin boyutu (100–225).
    ///
    /// Bu ayar yalnızca metni büyütür; tüm arayüzü tutarlı büyütmek için egui zoom çarpanı
    /// olarak uygulanır (satır yükseklikleri, simgeler ve boşluklar da ölçeklenir).
    pub fn text_scale() -> f32 {
        dword(r"Software\Microsoft\Accessibility", "TextScaleFactor")
            .filter(|v| (100..=225).contains(v))
            .map(|v| v as f32 / 100.0)
            .unwrap_or(1.0)
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
    pub fn text_scale() -> f32 {
        1.0
    }

    pub fn animations() -> bool {
        true
    }

    pub fn apply_dpi_awareness() {}

    pub fn set_app_id() {}

    pub fn apply_backdrop(_dark: bool) {}
}

pub use imp::*;
