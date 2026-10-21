//! Windows 11 tarzı sağ tık menüsü. Windows'ta içerik gerçek kabuk menüsünden
//! (`IContextMenu`) okunur; çizim egui ile yapılır ki modern Explorer görünümü elde edilsin.
//! Linux'ta yalnızca uygulamanın kendi komutları görünür.

use std::time::Instant;

use egui::{
    Align2, Color32, CornerRadius, Id, Margin, Order, Pos2, Rect, Sense, Shadow, Stroke, Vec2,
};

use crate::app::{fluent_font, g, ui_font};
use crate::shell::ShellMenu;

/// Menü satırının ürettiği komut. Kabuk komutları `ShellMenu::invoke` ile çalışır.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    App(u32),
    Shell(u32),
    More,
}

pub enum Icon {
    Glyph(&'static str),
    /// Glifi verilen renkle çizer; None ise satırın kendi rengi
    ColoredGlyph(&'static str, Option<Color32>),
    /// Görünüm menüsü için çizilmiş mini şema (ikon boyutları, liste, ayrıntılar…)
    Mock(usize),
    Image(egui::TextureHandle),
}

pub struct Node {
    pub label: String,
    pub icon: Option<Icon>,
    pub cmd: Option<Cmd>,
    pub enabled: bool,
    pub checked: bool,
    pub shortcut: Option<String>,
    /// Bu satırdan önce ince ayraç çizilir
    pub sep: bool,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(label: impl Into<String>) -> Node {
        Node {
            label: label.into(),
            icon: None,
            cmd: None,
            enabled: true,
            checked: false,
            shortcut: None,
            sep: false,
            children: vec![],
        }
    }

    pub fn with_icon(mut self, glyph: &'static str) -> Node {
        self.icon = Some(Icon::Glyph(glyph));
        self
    }

    /// Menü satırına renkli emoji ikonu ekler; None ise satır rengiyle çizilir
    pub fn with_colored_icon(mut self, glyph: &'static str, tint: Option<Color32>) -> Node {
        self.icon = Some(Icon::ColoredGlyph(glyph, tint));
        self
    }

    /// Görünüm menüsü satırına çizilmiş şema ikonu ekler
    pub fn with_mock(mut self, kind: usize) -> Node {
        self.icon = Some(Icon::Mock(kind));
        self
    }

    pub fn with_cmd(mut self, cmd: Cmd) -> Node {
        self.cmd = Some(cmd);
        self
    }

    pub fn shortcut(mut self, s: &str) -> Node {
        self.shortcut = Some(s.to_string());
        self
    }

    pub fn disabled(mut self) -> Node {
        self.enabled = false;
        self
    }

    pub fn checked(mut self, on: bool) -> Node {
        self.checked = on;
        self
    }

    pub fn sep_before(mut self) -> Node {
        self.sep = true;
        self
    }
}

/// Üstteki simge düğmeleri (Win11: Kes/Kopyala/Yeniden adlandır/Sil).
pub struct Quick {
    pub cmd: Cmd,
    pub glyph: &'static str,
    pub label: String,
    pub enabled: bool,
}

pub struct Menu {
    pub nodes: Vec<Node>,
    pub pos: Pos2,
    /// Açık alt menü zinciri; her düzey için satır indeksi
    pub open: Vec<usize>,
    pub opened: Instant,
    pub quick: Vec<Quick>,
    pub shell: Option<ShellMenu>,
}

pub enum Outcome {
    Command(Cmd),
    Close,
}

impl Menu {
    pub fn new(pos: Pos2) -> Menu {
        Menu {
            nodes: vec![],
            pos,
            open: vec![],
            opened: Instant::now(),
            quick: vec![],
            shell: None,
        }
    }
}

/// i18n anahtarını etiket ve kısayola ayırır. Tablodaki eski anahtarlar kısayolu
/// boşluklarla ya da `(Ctrl+X)` biçiminde taşıyor; menü kısayolu ayrı sütunda çizer.
pub fn parts(key: &str) -> (String, Option<String>) {
    let s = crate::i18n::t(key);
    if let Some(i) = s.find("  ") {
        let (a, b) = s.split_at(i);
        let sc = b.trim();
        if !sc.is_empty() && sc.len() <= 24 {
            return (a.trim_end().to_string(), Some(sc.to_string()));
        }
    }
    if s.ends_with(')') {
        if let Some(i) = s.rfind(" (") {
            let inner = &s[i + 2..s.len() - 1];
            if is_shortcut(inner) {
                return (s[..i].to_string(), Some(inner.to_string()));
            }
        }
    }
    (s.to_string(), None)
}

fn is_shortcut(s: &str) -> bool {
    let keyed = ["Ctrl", "Alt", "Shift", "Del", "Enter", "Esc", "Tab"]
        .iter()
        .any(|k| s.starts_with(k));
    keyed || (s.len() > 1 && s.starts_with('F') && s[1..].chars().all(|c| c.is_ascii_digit()))
}

struct Style {
    bg: Color32,
    border: Color32,
    text: Color32,
    weak: Color32,
    disabled: Color32,
    hover: Color32,
    accent: Color32,
    dark: bool,
}

impl Style {
    fn new(ctx: &egui::Context) -> Style {
        let dark = ctx.style().visuals.dark_mode;
        let accent = ctx.style().visuals.selection.bg_fill;
        if dark {
            Style {
                bg: Color32::from_rgb(43, 43, 43),
                border: Color32::from_rgb(58, 58, 58),
                text: Color32::from_rgb(243, 243, 243),
                weak: Color32::from_rgb(160, 160, 160),
                disabled: Color32::from_rgb(105, 105, 105),
                hover: Color32::from_rgb(61, 61, 61),
                accent,
                dark: true,
            }
        } else {
            Style {
                bg: Color32::from_rgb(249, 249, 249),
                border: Color32::from_rgb(229, 229, 229),
                text: Color32::from_rgb(26, 26, 26),
                weak: Color32::from_rgb(107, 107, 107),
                disabled: Color32::from_rgb(160, 160, 160),
                hover: Color32::from_rgb(238, 238, 238),
                accent,
                dark: false,
            }
        }
    }
}

const PAD: f32 = 4.0;
const ROW_H: f32 = 32.0;
const SEP_H: f32 = 9.0;
const QUICK_H: f32 = 60.0;
const TEXT_X: f32 = 42.0;
const MIN_W: f32 = 320.0;
const MAX_W: f32 = 380.0;

/// Menüyü çizer. Komut seçilirse ya da dışına tıklanırsa bir sonuç döner.
pub fn show(ctx: &egui::Context, menu: &mut Menu) -> Option<Outcome> {
    let st = Style::new(ctx);
    let mut outcome: Option<Outcome> = None;
    let mut union: Option<Rect> = None;
    let screen = ctx.screen_rect();
    let w = panel_width(ctx, &menu.nodes, !menu.quick.is_empty());
    let mut h = panel_height(&menu.nodes, !menu.quick.is_empty());
    h = h.min(screen.height() - PAD * 2.0);
    let mut pos = menu.pos;
    if pos.x + w > screen.right() - 4.0 {
        pos.x = (screen.right() - w - 4.0).max(screen.left() + 4.0);
    }
    if pos.y + h > screen.bottom() - 4.0 {
        pos.y = (screen.bottom() - h - 4.0).max(screen.top() + 4.0);
    }
    draw_panel(
        ctx,
        &menu.nodes,
        pos,
        w,
        &st,
        &mut menu.open,
        &mut outcome,
        &mut union,
        &menu.quick,
        0,
    );

    if outcome.is_none() {
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            outcome = Some(Outcome::Close);
        } else if menu.opened.elapsed().as_millis() > 120 && ctx.input(|i| i.pointer.any_pressed())
        {
            let p = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
            if !union.is_some_and(|u| u.contains(p)) {
                outcome = Some(Outcome::Close);
            }
        }
    }
    outcome
}

fn measure(ctx: &egui::Context, text: &str, size: f32) -> f32 {
    ctx.fonts(|f| {
        f.layout_no_wrap(text.to_string(), ui_font(size), Color32::WHITE)
            .size()
            .x
    })
}

fn panel_width(ctx: &egui::Context, nodes: &[Node], quick: bool) -> f32 {
    let mut w: f32 = MIN_W;
    if quick {
        w = w.max(58.0 * 5.0);
    }
    for n in nodes {
        if n.sep && n.label.is_empty() {
            continue;
        }
        let mut row = TEXT_X + measure(ctx, &n.label, 14.0) + 16.0;
        if let Some(sc) = &n.shortcut {
            row += measure(ctx, sc, 12.0) + 14.0;
        }
        if !n.children.is_empty() {
            row += 18.0;
        }
        w = w.max(row);
    }
    w.min(MAX_W)
}

fn panel_height(nodes: &[Node], quick: bool) -> f32 {
    let mut h = PAD * 2.0;
    if quick {
        h += QUICK_H + SEP_H;
    }
    let mut first = true;
    for n in nodes {
        if n.sep && !first {
            h += SEP_H;
        }
        if !n.sep {
            first = false;
        }
        h += ROW_H;
    }
    h
}

#[allow(clippy::too_many_arguments)]
fn draw_panel<'a>(
    ctx: &egui::Context,
    nodes: &'a [Node],
    pos: Pos2,
    width: f32,
    st: &Style,
    open: &mut Vec<usize>,
    outcome: &mut Option<Outcome>,
    union: &mut Option<Rect>,
    quick: &[Quick],
    depth: usize,
) {
    if nodes.is_empty() {
        return;
    }
    let id = Id::new(("swift_menu", depth));
    let mut child: Option<(Rect, &'a [Node])> = None;
    let area = egui::Area::new(id)
        .order(Order::Foreground)
        .fade_in(false)
        .fixed_pos(pos)
        .interactable(true)
        .show(ctx, |ui| {
            let shadow = Shadow {
                offset: [0, 8],
                blur: 24,
                spread: 0,
                color: Color32::from_black_alpha(if st.dark { 130 } else { 50 }),
            };
            egui::Frame::new()
                .fill(st.bg)
                .stroke(Stroke::new(1.0_f32, st.border))
                .corner_radius(8.0)
                .inner_margin(Margin::symmetric(4, 4))
                .shadow(shadow)
                .show(ui, |ui| {
                    let inner = width - 8.0;
                    ui.set_width(inner);
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    if depth == 0 && !quick.is_empty() {
                        quick_row(ui, quick, st, outcome, inner);
                        separator(ui, st, inner);
                    }
                    draw_items(ui, nodes, depth, st, open, outcome, inner, &mut child);
                });
        });
    let rect = area.response.rect;
    *union = Some(union.map_or(rect, |u| u.union(rect)));

    if let Some((row, children)) = child {
        let cw = panel_width(ctx, children, false);
        let mut x = rect.right() + 2.0;
        if x + cw > ctx.screen_rect().right() {
            x = rect.left() - cw - 2.0;
        }
        let mut y = row.top() - PAD;
        if y + panel_height(children, false) > ctx.screen_rect().bottom() {
            y = (ctx.screen_rect().bottom() - panel_height(children, false))
                .max(ctx.screen_rect().top());
        }
        draw_panel(
            ctx,
            children,
            Pos2::new(x, y),
            cw,
            st,
            open,
            outcome,
            union,
            &[],
            depth + 1,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_items<'a>(
    ui: &mut egui::Ui,
    nodes: &'a [Node],
    depth: usize,
    st: &Style,
    open: &mut Vec<usize>,
    outcome: &mut Option<Outcome>,
    width: f32,
    child: &mut Option<(Rect, &'a [Node])>,
) {
    let mut first = true;
    for (i, n) in nodes.iter().enumerate() {
        if n.sep && !first {
            separator(ui, st, width);
        }
        if !n.sep {
            first = false;
        }
        let rect = row(ui, n, i, depth, st, open, outcome, width);
        let open_here = !n.children.is_empty() && open.get(depth).copied() == Some(i);
        if open_here {
            *child = Some((rect, &n.children));
        }
    }
}

fn separator(ui: &mut egui::Ui, st: &Style, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, SEP_H), Sense::hover());
    let y = rect.center().y + 0.5;
    ui.painter().hline(
        (rect.left() + 8.0)..=(rect.right() - 8.0),
        y,
        Stroke::new(1.0_f32, st.border),
    );
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    n: &Node,
    idx: usize,
    depth: usize,
    st: &Style,
    open: &mut Vec<usize>,
    outcome: &mut Option<Outcome>,
    width: f32,
) -> Rect {
    let clickable = n.enabled && (n.cmd.is_some() || !n.children.is_empty());
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), sense);
    let hovered = resp.hovered() && n.enabled;
    if hovered {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            CornerRadius::same(4),
            st.hover,
        );
    }
    let color = if n.enabled { st.text } else { st.disabled };
    let icon_center = Pos2::new(rect.left() + 20.0, rect.center().y);

    if let Some(icon) = &n.icon {
        // Radyo satırı: seçiliyse ikonun solunda küçük vurgu noktası (Explorer gibi)
        if n.checked {
            ui.painter()
                .circle_filled(Pos2::new(rect.left() + 9.0, icon_center.y), 2.2, st.accent);
        }
        match icon {
            Icon::Glyph(glyph) => {
                ui.painter().text(
                    icon_center,
                    Align2::CENTER_CENTER,
                    *glyph,
                    fluent_font(17.0),
                    color,
                );
            }
            Icon::ColoredGlyph(glyph, tint) => {
                ui.painter().text(
                    icon_center,
                    Align2::CENTER_CENTER,
                    *glyph,
                    fluent_font(17.0),
                    tint.unwrap_or(color),
                );
            }
            Icon::Mock(kind) => {
                mock_view_icon(ui, *kind, icon_center, color);
            }
            Icon::Image(tex) => {
                let r = Rect::from_center_size(icon_center, Vec2::splat(16.0));
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().image(tex.id(), r, uv, Color32::WHITE);
            }
        }
    } else if n.checked {
        let c = icon_center;
        let path = vec![
            Pos2::new(c.x - 4.5, c.y + 0.2),
            Pos2::new(c.x - 1.5, c.y + 3.2),
            Pos2::new(c.x + 4.8, c.y - 3.4),
        ];
        ui.painter()
            .add(egui::Shape::line(path, Stroke::new(1.7_f32, st.accent)));
    }

    ui.painter().text(
        Pos2::new(rect.left() + TEXT_X, rect.center().y),
        Align2::LEFT_CENTER,
        &n.label,
        ui_font(14.0),
        color,
    );

    if let Some(sc) = &n.shortcut {
        let right = rect.right() - if n.children.is_empty() { 12.0 } else { 26.0 };
        ui.painter().text(
            Pos2::new(right, rect.center().y),
            Align2::RIGHT_CENTER,
            sc,
            ui_font(12.0),
            st.weak,
        );
    }

    if !n.children.is_empty() {
        ui.painter().text(
            Pos2::new(rect.right() - 13.0, rect.center().y),
            Align2::CENTER_CENTER,
            g::CHEVRON_RIGHT,
            fluent_font(11.0),
            color,
        );
    }

    if resp.hovered() {
        open.truncate(depth);
        if !n.children.is_empty() && n.enabled {
            open.push(idx);
        }
    }
    if resp.clicked() && clickable {
        if !n.children.is_empty() {
            open.truncate(depth);
            open.push(idx);
        } else if let Some(c) = n.cmd {
            *outcome = Some(Outcome::Command(c));
        }
    }
    rect
}

/// Görünüm menüsü satırları için çizilmiş mini şemalar: ikon fontuna bağlı olmadan,
/// Explorer'daki gibi her görünüm türünün kendi küçük resmi olur.
fn mock_view_icon(ui: &mut egui::Ui, kind: usize, c: Pos2, color: Color32) {
    let p = ui.painter();
    let rect = |x0: f32, y0: f32, w: f32, h: f32| {
        Rect::from_min_size(Pos2::new(c.x + x0, c.y + y0), Vec2::new(w, h))
    };
    match kind {
        // Çok büyük simgeler: tek büyük kare
        0 => {
            p.rect_filled(rect(-6.0, -6.0, 12.0, 12.0), 3.0, color);
        }
        // Büyük simgeler: tek orta kare
        1 => {
            p.rect_filled(rect(-4.5, -4.5, 9.0, 9.0), 2.5, color);
        }
        // Orta simgeler: 3×3 ızgara
        2 => {
            for j in 0..3 {
                for i in 0..3 {
                    p.rect_filled(
                        rect(-6.6 + i as f32 * 4.9, -6.6 + j as f32 * 4.9, 3.6, 3.6),
                        1.0,
                        color,
                    );
                }
            }
        }
        // Küçük simgeler: 4×4 ızgara
        3 => {
            for j in 0..4 {
                for i in 0..4 {
                    p.rect_filled(
                        rect(-6.9 + i as f32 * 3.7, -6.9 + j as f32 * 3.7, 2.8, 2.8),
                        0.9,
                        color,
                    );
                }
            }
        }
        // Liste: tek sütun çizgiler
        4 => {
            for j in 0..4 {
                p.rect_filled(rect(-6.5, -5.4 + j as f32 * 3.6, 9.5, 1.8), 0.9, color);
            }
        }
        // Ayrıntılar: noktalar + çizgiler
        5 => {
            for j in 0..3 {
                let y = -4.8 + j as f32 * 4.8;
                p.circle_filled(Pos2::new(c.x - 5.5, c.y + y + 0.9), 1.1, color);
                p.rect_filled(rect(-2.0, y, 8.0, 1.8), 0.9, color);
            }
        }
        // Döşemeler: kare + metin çizgisi
        6 => {
            for j in 0..2 {
                let y = -5.2 + j as f32 * 6.4;
                p.rect_filled(rect(-6.0, y, 4.6, 4.6), 1.2, color);
                p.rect_filled(rect(-0.3, y + 1.4, 6.3, 1.8), 0.9, color);
            }
        }
        // İçerik: paragraf blokları
        7 => {
            for j in 0..2 {
                let y = -5.6 + j as f32 * 6.8;
                p.rect_filled(rect(-6.0, y, 12.0, 1.8), 0.9, color);
                p.rect_filled(rect(-6.0, y + 2.8, 5.5, 1.8), 0.9, color);
            }
        }
        // Ayrıntılar bölmesi: sağ bölme çizili pencere
        8 => {
            let r = rect(-6.5, -6.0, 13.0, 12.0);
            p.rect_stroke(
                r,
                CornerRadius::same(2),
                Stroke::new(1.4_f32, color),
                egui::StrokeKind::Inside,
            );
            p.vline(
                c.x - 0.9,
                (r.top() + 1.4)..=(r.bottom() - 1.4),
                Stroke::new(1.4_f32, color),
            );
        }
        _ => {}
    }
}

fn quick_row(
    ui: &mut egui::Ui,
    quick: &[Quick],
    st: &Style,
    outcome: &mut Option<Outcome>,
    width: f32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, QUICK_H), Sense::hover());
    let n = quick.len().max(1) as f32;
    let cell = width / n;
    for (i, q) in quick.iter().enumerate() {
        let br = Rect::from_min_max(
            Pos2::new(rect.left() + cell * i as f32 + 3.0, rect.top() + 2.0),
            Pos2::new(
                rect.left() + cell * (i as f32 + 1.0) - 3.0,
                rect.bottom() - 2.0,
            ),
        );
        let resp = ui.interact(
            br,
            Id::new(("swift_menu_quick", i)),
            if q.enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if resp.hovered() && q.enabled {
            ui.painter()
                .rect_filled(br, CornerRadius::same(5), st.hover);
        }
        let color = if q.enabled { st.text } else { st.disabled };
        ui.painter().text(
            Pos2::new(br.center().x, br.top() + 20.0),
            Align2::CENTER_CENTER,
            q.glyph,
            fluent_font(22.0),
            color,
        );
        ui.painter().text(
            Pos2::new(br.center().x, br.bottom() - 11.0),
            Align2::CENTER_CENTER,
            &q.label,
            ui_font(11.0),
            color,
        );
        if resp.clicked() && q.enabled {
            *outcome = Some(Outcome::Command(q.cmd));
        }
    }
}

/// Kabuk menü satırını egui satırına dönüştürür; ikonlar bir kez dokuya çevrilir.
pub fn shell_node(item: &crate::shell::ShellMenuItem, ctx: &egui::Context, path: &str) -> Node {
    let mut node = Node::new(item.label.clone());
    node.enabled = item.enabled;
    node.checked = item.checked;
    if item.separator {
        node.sep = true;
        return node;
    }
    if let Some(img) = &item.icon {
        let tex = ctx.load_texture(
            format!("shell_menu_{path}_{}", item.id),
            img.clone(),
            egui::TextureOptions::LINEAR,
        );
        node.icon = Some(Icon::Image(tex));
    }
    node.children = item
        .children
        .iter()
        .map(|c| shell_node(c, ctx, &format!("{path}_{}", c.id)))
        .collect();
    node
}
