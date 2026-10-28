//! Windows dışında gömülü PDF/ses/video görüntüleyici yoktur; önizleme
//! küçük resim ve meta veriyle yetinir. Windows tarafı `webview.rs`.

/// Görüntüleyicinin kaplaması istenen alan (Windows'ta WebView2 kullanılır).
#[allow(dead_code)]
#[derive(Clone)]
pub struct Want {
    pub path: std::path::PathBuf,
    pub kind: crate::media::MediaKind,
    pub rect: egui::Rect,
    pub overlay: bool,
    pub dark: bool,
    pub accent: String,
}

/// Windows tarafında oynatıcıdan iletilen tuşlar; burada yok.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HotKey {
    Space,
    Escape,
}

pub struct WebView;

impl WebView {
    pub fn new() -> Self {
        WebView
    }

    pub fn ready(&self) -> bool {
        false
    }

    pub fn failed(&self) -> bool {
        true
    }

    pub fn take_keys(&self) -> Vec<HotKey> {
        Vec::new()
    }

    pub fn sync(
        &mut self,
        _hwnd: Option<isize>,
        _want: Option<&Want>,
        _allow: bool,
        _ctx: &egui::Context,
    ) {
    }
}
