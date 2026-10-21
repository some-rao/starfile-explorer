//! Terminal ve editörlerden bilinen renk şemaları. Ayarlardaki `theme` değeri 0-2 için
//! Sistem/Açık/Koyu, 3 ve sonrası için bu listedeki sıra + 3'tür; yeni şemalar sona eklenir.

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use egui::Color32;

pub struct Scheme {
    pub name: &'static str,
    pub dark: bool,
    /// Sekme şeridi
    pub crust: Color32,
    /// Gezinme satırı, gezinti bölmesi
    pub mantle: Color32,
    /// İçerik
    pub base: Color32,
    /// Kutular ve açılır menüler
    pub surface: Color32,
    /// Çizgiler ve kenarlıklar
    pub border: Color32,
    pub disabled: Color32,
    pub weak: Color32,
    pub text: Color32,
    pub accent: Color32,
}

const fn hex(x: u32) -> Color32 {
    Color32::from_rgb((x >> 16) as u8, (x >> 8) as u8, x as u8)
}

#[allow(clippy::too_many_arguments)]
const fn s(
    name: &'static str,
    dark: bool,
    crust: u32,
    mantle: u32,
    base: u32,
    surface: u32,
    border: u32,
    disabled: u32,
    weak: u32,
    text: u32,
    accent: u32,
) -> Scheme {
    Scheme {
        name,
        dark,
        crust: hex(crust),
        mantle: hex(mantle),
        base: hex(base),
        surface: hex(surface),
        border: hex(border),
        disabled: hex(disabled),
        weak: hex(weak),
        text: hex(text),
        accent: hex(accent),
    }
}

pub const SCHEMES: &[Scheme] = &[
    s(
        "Catppuccin Mocha",
        true,
        0x11111b,
        0x181825,
        0x1e1e2e,
        0x313244,
        0x45475a,
        0x6c7086,
        0xa6adc8,
        0xcdd6f4,
        0xcba6f7,
    ),
    s(
        "Catppuccin Latte",
        false,
        0xdce0e8,
        0xe6e9ef,
        0xeff1f5,
        0xccd0da,
        0xbcc0cc,
        0x9ca0b0,
        0x6c6f85,
        0x4c4f69,
        0x8839ef,
    ),
    s(
        "Gruvbox Dark",
        true,
        0x1d2021,
        0x32302f,
        0x282828,
        0x3c3836,
        0x504945,
        0x7c6f64,
        0xa89984,
        0xebdbb2,
        0xd79921,
    ),
    s(
        "Gruvbox Light",
        false,
        0xebdbb2,
        0xf2e5bc,
        0xfbf1c7,
        0xebdbb2,
        0xd5c4a1,
        0xa89984,
        0x7c6f64,
        0x3c3836,
        0xb57614,
    ),
    s(
        "Everforest Dark",
        true,
        0x232a2e,
        0x343f44,
        0x2d353b,
        0x3d484d,
        0x475258,
        0x7a8478,
        0x9da9a0,
        0xd3c6aa,
        0xa7c080,
    ),
    s(
        "Everforest Light",
        false,
        0xefebd4,
        0xf4f0d9,
        0xfdf6e3,
        0xe6e2cc,
        0xe0dcc7,
        0xa6b0a0,
        0x829181,
        0x5c6a72,
        0x8da101,
    ),
    s(
        "Nord", true, 0x242933, 0x3b4252, 0x2e3440, 0x3b4252, 0x434c5e, 0x616e88, 0xd8dee9,
        0xeceff4, 0x88c0d0,
    ),
    s(
        "Dracula", true, 0x191a21, 0x21222c, 0x282a36, 0x343746, 0x44475a, 0x6272a4, 0xbfbfc6,
        0xf8f8f2, 0xbd93f9,
    ),
    s(
        "Tokyo Night",
        true,
        0x16161e,
        0x1f2335,
        0x1a1b26,
        0x292e42,
        0x3b4261,
        0x565f89,
        0xa9b1d6,
        0xc0caf5,
        0x7aa2f7,
    ),
    s(
        "Rosé Pine",
        true,
        0x191724,
        0x1f1d2e,
        0x191724,
        0x26233a,
        0x403d52,
        0x6e6a86,
        0x908caa,
        0xe0def4,
        0xc4a7e7,
    ),
    s(
        "Solarized Dark",
        true,
        0x00212b,
        0x073642,
        0x002b36,
        0x073642,
        0x0e4b5a,
        0x586e75,
        0x839496,
        0x93a1a1,
        0x268bd2,
    ),
    s(
        "Solarized Light",
        false,
        0xeee8d5,
        0xeee8d5,
        0xfdf6e3,
        0xeee8d5,
        0xddd6c1,
        0x93a1a1,
        0x839496,
        0x586e75,
        0x268bd2,
    ),
];

/// Ayarlardaki ilk şemanın değeri
pub const FIRST: u8 = 3;

static ACTIVE: AtomicU8 = AtomicU8::new(0);

pub fn get(theme: u8) -> Option<&'static Scheme> {
    SCHEMES.get(theme.checked_sub(FIRST)? as usize)
}

/// Seçili tema; `Pal` ve egui görünümü bunu okur
pub fn set_active(theme: u8) {
    ACTIVE.store(theme, Relaxed);
}

pub fn active() -> Option<&'static Scheme> {
    get(ACTIVE.load(Relaxed))
}

/// İki renk arası, `t` = 0 ise `a`, 1 ise `b`
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}
