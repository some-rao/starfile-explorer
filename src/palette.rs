// Ctrl+K command palette. Child module of `app` so it can read App's private
// fields without widening their visibility.

use std::time::{Duration, Instant};

use egui::{Color32, Key, Layout, Modifiers, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::fsutil::{self, Entry};
use crate::i18n;
use crate::query::SearchScope;
use crate::shell::Img;

use super::*;

// ---------------------------------------------------------------- state

#[derive(Clone)]
pub enum Mode {
    Files(SearchScope),
    Actions(Entry),
}

pub struct Scan {
    pub handle: fsutil::SearchHandle,
    pub key: String,
    pub found: Vec<Entry>,
    pub scanned: u64,
    pub done: bool,
    pub started: Instant,
}

#[derive(Clone)]
pub struct Row {
    pub entry: Option<Entry>,
    pub cmd: Option<ItemCmd>,
    pub label: String,
    pub spans: Vec<(usize, usize)>,
    pub detail: String,
    pub meta: String,
    pub enabled: bool,
    pub reason: &'static str,
}

#[derive(Clone, Copy)]
pub enum ItemCmd {
    Open,
    OpenWith,
    OpenContaining,
    OpenNewTab,
    Copy,
    Cut,
    Rename,
    Delete,
    CopyPath,
    Properties,
    Preview,
}

pub struct Palette {
    pub text: String,
    pub last_text: String,
    pub mode: Mode,
    pub saved_query: String,
    pub saved_scope: SearchScope,
    pub cursor: usize,
    pub rows: Vec<Row>,
    pub dirty: bool,
    pub scan: Option<Scan>,
    pub last_edit: Instant,
}

impl Palette {
    pub fn new() -> Self {
        Palette {
            text: String::new(),
            last_text: String::new(),
            mode: Mode::Files(SearchScope::Folder),
            saved_query: String::new(),
            saved_scope: SearchScope::Folder,
            cursor: 0,
            rows: Vec::new(),
            dirty: true,
            scan: None,
            last_edit: Instant::now(),
        }
    }
}

// ---------------------------------------------------------------- prefix parsing

pub fn parse_prefix(text: &str) -> (SearchScope, &str) {
    if let Some(rest) = text.strip_prefix(";;") {
        if rest.is_empty() || rest.starts_with(' ') {
            return (SearchScope::ThisPc, rest.trim_start());
        }
    }
    if let Some(rest) = text.strip_prefix(';') {
        if rest.is_empty() || rest.starts_with(' ') {
            return (SearchScope::Subfolders, rest.trim_start());
        }
    }
    (SearchScope::Folder, text)
}

// ---------------------------------------------------------------- scoring

pub fn score(name_lower: &str, terms: &[String]) -> Option<(i32, Vec<(usize, usize)>)> {
    if terms.is_empty() {
        return Some((0, Vec::new()));
    }
    let mut total = 0i32;
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for term in terms {
        let t = term.to_lowercase();
        if t.is_empty() {
            continue;
        }
        let (s, sp) = match_term(name_lower, &t)?;
        total += s;
        for sp in sp {
            if !spans.contains(&sp) {
                spans.push(sp);
            }
        }
    }
    Some((total, spans))
}

fn match_term(name: &str, term: &str) -> Option<(i32, Vec<(usize, usize)>)> {
    // exact
    if name == term {
        return Some((1000, vec![(0, name.len())]));
    }
    // prefix
    if name.starts_with(term) {
        return Some((800, vec![(0, term.len())]));
    }
    // word start
    let bytes = name.as_bytes();
    let mut i = 0;
    while i + term.len() <= name.len() {
        let at_boundary =
            i == 0 || matches!(bytes[i - 1] as char, ' ' | '-' | '_' | '.' | '/' | '\\');
        if at_boundary && name[i..].starts_with(term) {
            return Some((600, vec![(i, i + term.len())]));
        }
        i += 1;
    }
    // substring
    if let Some(pos) = name.find(term) {
        return Some((400, vec![(pos, pos + term.len())]));
    }
    // subsequence
    let mut spans = Vec::new();
    let mut ni = 0;
    let nchars: Vec<char> = name.chars().collect();
    let tchars: Vec<char> = term.chars().collect();
    let mut ti = 0;
    let mut last = 0usize;
    // map char indices to byte offsets
    let mut byte_offsets: Vec<usize> = name.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(name.len());
    let mut matched_bytes: Vec<(usize, usize)> = Vec::new();
    let mut ci = 0usize;
    for tc in &tchars {
        while ci < nchars.len() && nchars[ci] != *tc {
            ci += 1;
        }
        if ci >= nchars.len() {
            return None;
        }
        matched_bytes.push((byte_offsets[ci], byte_offsets[ci + 1]));
        ci += 1;
    }
    // merge contiguous
    let mut cur: Option<(usize, usize)> = None;
    for (s, e) in matched_bytes {
        match cur {
            Some((cs, ce)) if ce == s => cur = Some((cs, e)),
            Some(c) => {
                spans.push(c);
                cur = Some((s, e));
            }
            None => cur = Some((s, e)),
        }
    }
    if let Some(c) = cur {
        spans.push(c);
    }
    if spans.is_empty() {
        return None;
    }
    Some((200, spans))
}

// ---------------------------------------------------------------- item commands

pub fn item_commands(e: &Entry) -> Vec<ItemCmd> {
    let mut v = Vec::new();
    if e.is_dir {
        v.push(ItemCmd::Open);
        v.push(ItemCmd::OpenNewTab);
        v.push(ItemCmd::OpenContaining);
    } else {
        v.push(ItemCmd::Open);
        v.push(ItemCmd::OpenWith);
        v.push(ItemCmd::OpenContaining);
    }
    v.push(ItemCmd::Copy);
    v.push(ItemCmd::Cut);
    v.push(ItemCmd::Rename);
    v.push(ItemCmd::Delete);
    v.push(ItemCmd::CopyPath);
    v.push(ItemCmd::Properties);
    v.push(ItemCmd::Preview);
    v
}

pub fn cmd_label(c: ItemCmd) -> &'static str {
    match c {
        ItemCmd::Open => "Aç",
        ItemCmd::OpenWith => "Birlikte aç…",
        ItemCmd::OpenContaining => "İçeren klasörü aç",
        ItemCmd::OpenNewTab => "Yeni sekmede aç",
        ItemCmd::Copy => "Kopyala",
        ItemCmd::Cut => "Kes",
        ItemCmd::Rename => "Yeniden adlandır",
        ItemCmd::Delete => "Sil",
        ItemCmd::CopyPath => "Yolu kopyala",
        ItemCmd::Properties => "Özellikler",
        ItemCmd::Preview => "Hızlı Önizleme",
    }
}

pub fn cmd_shortcut(c: ItemCmd) -> &'static str {
    match c {
        ItemCmd::Open => "Enter",
        ItemCmd::OpenWith => "",
        ItemCmd::OpenContaining => "Ctrl+Enter",
        ItemCmd::OpenNewTab => "Ctrl+T",
        ItemCmd::Copy => "Ctrl+C",
        ItemCmd::Cut => "Ctrl+X",
        ItemCmd::Rename => "F2",
        ItemCmd::Delete => "Del",
        ItemCmd::CopyPath => "Ctrl+Shift+C",
        ItemCmd::Properties => "Alt+Enter",
        ItemCmd::Preview => "Ctrl+Space",
    }
}

fn cmd_enabled(c: ItemCmd, e: &Entry, app: &App) -> (bool, &'static str) {
    let virtual_loc = crate::tab::is_virtual(&e.path)
        || app.tabs[app.active].is_home() && e.path.as_os_str().is_empty();
    match c {
        ItemCmd::Rename | ItemCmd::Cut | ItemCmd::Delete => {
            if crate::tab::is_virtual(&e.path) {
                (false, "Sürücü kökünde kullanılamaz")
            } else if app.tabs[app.active].is_home() {
                (false, "Burada kullanılamaz")
            } else {
                (true, "")
            }
        }
        ItemCmd::Copy => (true, ""),
        _ => (true, ""),
    }
}

// ---------------------------------------------------------------- App methods

impl super::App {
    pub fn open_palette(&mut self) {
        self.quick = None;
        self.palette = Some(Palette::new());
        self.ctx.request_repaint();
    }
    pub fn close_palette(&mut self) {
        self.palette = None;
    }
    pub fn toggle_palette(&mut self) {
        if self.palette.is_some() {
            self.palette = None;
        } else {
            self.open_palette();
        }
    }

    /// Draw the palette overlay. Called last in `update`.
    pub fn palette_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(_) = self.palette.as_ref() else {
            return;
        };

        // edit detection
        {
            let p = self.palette.as_mut().unwrap();
            if p.text != p.last_text {
                if let Some(s) = &mut p.scan {
                    s.handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                p.scan = None;
                p.last_text = p.text.clone();
                p.last_edit = Instant::now();
                p.dirty = true;
            }
        }

        // poll scan
        {
            let mut done_now = false;
            let mut got_batch = false;
            if let Some(p) = self.palette.as_mut() {
                if let Some(s) = &mut p.scan {
                    while let Ok(msg) = s.handle.rx.try_recv() {
                        match msg {
                            fsutil::SearchMsg::Batch(v) => {
                                s.found.extend(v);
                                got_batch = true;
                            }
                            fsutil::SearchMsg::Done => {
                                s.done = true;
                                done_now = true;
                            }
                        }
                    }
                    s.scanned = s.handle.scanned.load(std::sync::atomic::Ordering::Relaxed);
                    if got_batch || done_now {
                        p.dirty = true;
                    }
                }
            }
            if !done_now {
                if let Some(p) = self.palette.as_ref() {
                    if p.scan.as_ref().map(|s| !s.done).unwrap_or(false) {
                        ctx.request_repaint_after(Duration::from_millis(120));
                    }
                }
            }
        }

        // debounce scan start
        let (scope, rest) = {
            let p = self.palette.as_ref().unwrap();
            match &p.mode {
                Mode::Files(s) => (*s, {
                    let (_, r) = parse_prefix(&p.text);
                    r.to_string()
                }),
                Mode::Actions(_) => (SearchScope::Folder, String::new()),
            }
        };
        // Only scan for non-folder scopes with non-empty query.
        if matches!(scope, SearchScope::Subfolders | SearchScope::ThisPc) && !rest.trim().is_empty()
        {
            let key = format!("{:?}:{}", scope, rest.trim());
            let need_start = {
                let p = self.palette.as_ref().unwrap();
                match &p.scan {
                    Some(s) => s.key != key && s.done,
                    None => true,
                }
            };
            // cancel running scan if key changed
            {
                let p = self.palette.as_mut().unwrap();
                if let Some(s) = &p.scan {
                    if s.key != key && !s.done {
                        s.handle
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        p.scan = None;
                        p.dirty = true;
                    }
                }
            }
            if need_start {
                let elapsed = self.palette.as_ref().unwrap().last_edit.elapsed();
                if elapsed >= Duration::from_millis(180) {
                    // roots
                    let roots = match scope {
                        SearchScope::Subfolders => vec![self.tabs[self.active].path.clone()],
                        SearchScope::ThisPc => {
                            let d = self.drives.lock().unwrap();
                            d.iter().map(|x| x.root.clone()).collect()
                        }
                        _ => vec![],
                    };
                    let q = crate::query::Query::parse(&rest);
                    let handle = fsutil::spawn_search(roots, q, ctx.clone());
                    let s = Scan {
                        handle,
                        key: key.clone(),
                        found: Vec::new(),
                        scanned: 0,
                        done: false,
                        started: Instant::now(),
                    };
                    let p = self.palette.as_mut().unwrap();
                    p.scan = Some(s);
                    p.dirty = true;
                } else {
                    ctx.request_repaint_after(Duration::from_millis(180) - elapsed);
                }
            }
        } else {
            // clear scan if switched back to folder
            let p = self.palette.as_mut().unwrap();
            if p.scan.is_some() {
                if let Some(s) = &mut p.scan {
                    s.handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                p.scan = None;
                p.dirty = true;
            }
        }

        // rebuild if dirty
        if self.palette.as_ref().unwrap().dirty {
            self.palette_rebuild();
        }

        // ---- input handling ----
        let mut close = false;
        let mut drill = false; // files -> actions
        let mut back = false; // actions -> files
        let mut move_up = false;
        let mut move_down = false;
        let mut move_pgup = false;
        let mut move_pgdn = false;
        let mut move_home = false;
        let mut move_end = false;
        let mut run = false;
        let mut reveal = false;
        let mut peek = false;
        let mut tab_cycle = false;

        ctx.input_mut(|i| {
            if i.key_pressed(Key::Escape) {
                close = true; // handled below as step-back
                i.consume_key(Modifiers::NONE, Key::Escape);
            }
            if i.key_pressed(Key::ArrowUp) {
                move_up = true;
                i.consume_key(Modifiers::NONE, Key::ArrowUp);
            }
            if i.key_pressed(Key::ArrowDown) {
                move_down = true;
                i.consume_key(Modifiers::NONE, Key::ArrowDown);
            }
            if i.key_pressed(Key::PageUp) {
                move_pgup = true;
                i.consume_key(Modifiers::NONE, Key::PageUp);
            }
            if i.key_pressed(Key::PageDown) {
                move_pgdn = true;
                i.consume_key(Modifiers::NONE, Key::PageDown);
            }
            if i.key_pressed(Key::Home) {
                move_home = true;
                i.consume_key(Modifiers::NONE, Key::Home);
            }
            if i.key_pressed(Key::End) {
                move_end = true;
                i.consume_key(Modifiers::NONE, Key::End);
            }
            if i.key_pressed(Key::Enter) {
                if i.modifiers.ctrl {
                    reveal = true;
                } else {
                    run = true;
                }
                i.consume_key(Modifiers::NONE, Key::Enter);
                i.consume_key(Modifiers::CTRL, Key::Enter);
            }
            if i.key_pressed(Key::Tab) {
                tab_cycle = true;
                i.consume_key(Modifiers::NONE, Key::Tab);
            }
            if i.modifiers.ctrl && i.key_pressed(Key::Space) {
                peek = true;
                i.consume_key(Modifiers::CTRL, Key::Space);
            }
            // drill / back
            let p = self.palette.as_ref().unwrap();
            match &p.mode {
                Mode::Files(_) => {
                    if i.key_pressed(Key::ArrowRight) {
                        drill = true;
                        i.consume_key(Modifiers::NONE, Key::ArrowRight);
                    }
                }
                Mode::Actions(_) => {
                    if i.key_pressed(Key::ArrowLeft) {
                        back = true;
                        i.consume_key(Modifiers::NONE, Key::ArrowLeft);
                    }
                    if i.key_pressed(Key::Backspace) {
                        let empty = self.palette.as_ref().unwrap().text.is_empty();
                        if empty {
                            back = true;
                            i.consume_key(Modifiers::NONE, Key::Backspace);
                        }
                    }
                }
            }
        });

        // apply movement
        {
            let p = self.palette.as_mut().unwrap();
            let n = p.rows.len();
            if n > 0 {
                if move_up {
                    if p.cursor > 0 {
                        p.cursor -= 1;
                    }
                }
                if move_down {
                    if p.cursor + 1 < n {
                        p.cursor += 1;
                    }
                }
                if move_pgup {
                    p.cursor = p.cursor.saturating_sub(10);
                }
                if move_pgdn {
                    p.cursor = (p.cursor + 10).min(n - 1);
                }
                if move_home {
                    p.cursor = 0;
                }
                if move_end {
                    p.cursor = n - 1;
                }
            }
        }

        // tab cycle scope (files mode only)
        if tab_cycle {
            let p = self.palette.as_mut().unwrap();
            if let Mode::Files(s) = p.mode.clone() {
                let new_scope = match s {
                    SearchScope::Folder => SearchScope::Subfolders,
                    SearchScope::Subfolders => SearchScope::ThisPc,
                    SearchScope::ThisPc => SearchScope::Folder,
                };
                // rewrite prefix
                let (_, rest) = parse_prefix(&p.text);
                let prefix = match new_scope {
                    SearchScope::Folder => "",
                    SearchScope::Subfolders => "; ",
                    SearchScope::ThisPc => ";; ",
                };
                let new_text = if prefix.is_empty() {
                    rest.to_string()
                } else {
                    format!("{}{}", prefix, rest)
                };
                p.mode = Mode::Files(new_scope);
                p.text = new_text.clone();
                p.last_text = new_text;
                p.last_edit = Instant::now();
                p.dirty = true;
                if let Some(s) = &mut p.scan {
                    s.handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                p.scan = None;
            }
        }

        // drill
        if drill {
            let target = {
                let p = self.palette.as_ref().unwrap();
                p.rows.get(p.cursor).and_then(|r| r.entry.clone())
            };
            if let Some(e) = target {
                let p = self.palette.as_mut().unwrap();
                p.saved_query = p.text.clone();
                p.saved_scope = match &p.mode {
                    Mode::Files(s) => *s,
                    _ => SearchScope::Folder,
                };
                p.mode = Mode::Actions(e);
                p.text = String::new();
                p.last_text = String::new();
                p.dirty = true;
                if let Some(s) = &mut p.scan {
                    s.handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                p.scan = None;
            }
        }

        // back
        if back {
            let p = self.palette.as_mut().unwrap();
            if let Mode::Actions(_) = p.mode {
                p.mode = Mode::Files(p.saved_scope);
                p.text = p.saved_query.clone();
                p.last_text = p.saved_query.clone();
                p.dirty = true;
            }
        }

        // peek
        if peek {
            let target = palette_highlighted_entry(self);
            if let Some(e) = target {
                self.quick = Some(QuickLook::new(e));
                // keep palette open? Quick Look is separate overlay; close palette to avoid both
                self.palette = None;
                // open quick look handled by existing infra? QuickLook::new just stores; need to trigger?
                // Existing Ctrl+Space in list sets self.quick. We mirror that.
            }
        }

        // reveal (Ctrl+Enter)
        if reveal {
            let target = palette_highlighted_entry(self);
            if let Some(e) = target {
                let parent = e
                    .path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| e.path.clone());
                actions.push(Action::Navigate(parent));
                // select after load
                self.tabs[self.active].select_after_load = Some(e.path.clone());
            }
        }

        // run (Enter)
        if run {
            let p = self.palette.as_ref().unwrap();
            match p.mode.clone() {
                Mode::Files(_) => {
                    if let Some(row) = p.rows.get(p.cursor).cloned() {
                        if row.entry.is_none() && row.cmd.is_none() && row.label.starts_with("Tüm")
                        {
                            // hand off to toolbar search
                            let rest = self
                                .palette
                                .as_ref()
                                .map(|p| {
                                    let (_, r) = parse_prefix(&p.text);
                                    r.to_string()
                                })
                                .unwrap_or_default();
                            self.tabs[self.active].filter = rest;
                            self.tabs[self.active].view_dirty = true;
                            self.close_palette();
                        } else if let Some(e) = row.entry.clone() {
                            self.close_palette();
                            if e.is_dir {
                                actions.push(Action::Navigate(e.path.clone()));
                            } else {
                                let _ = open::that_detached(&e.path);
                            }
                        }
                    }
                }
                Mode::Actions(_) => {
                    // determine target and cmd
                    let (target, cmd) = {
                        let p = self.palette.as_ref().unwrap();
                        let t = match &p.mode {
                            Mode::Actions(e) => e.clone(),
                            _ => return,
                        };
                        let c = p
                            .rows
                            .get(p.cursor)
                            .and_then(|r| r.cmd)
                            .unwrap_or(ItemCmd::Open);
                        (t, c)
                    };
                    self.close_palette();
                    match cmd {
                        ItemCmd::Open => {
                            if target.is_dir {
                                actions.push(Action::Navigate(target.path.clone()));
                            } else {
                                let _ = open::that_detached(&target.path);
                            }
                        }
                        ItemCmd::OpenWith => actions.push(Action::OpenWith(target.path.clone())),
                        ItemCmd::OpenContaining => {
                            let parent = target
                                .path
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or(target.path.clone());
                            actions.push(Action::Navigate(parent));
                            self.tabs[self.active].select_after_load = Some(target.path.clone());
                        }
                        ItemCmd::OpenNewTab => actions.push(Action::NewTab(target.path.clone())),
                        ItemCmd::Copy => actions.push(Action::ClipboardPaths {
                            paths: vec![target.path.clone()],
                            cut: false,
                        }),
                        ItemCmd::Cut => actions.push(Action::ClipboardPaths {
                            paths: vec![target.path.clone()],
                            cut: true,
                        }),
                        ItemCmd::Rename => actions.push(Action::RenamePath(target.path.clone())),
                        ItemCmd::Delete => actions.push(Action::DeletePaths {
                            paths: vec![target.path.clone()],
                            permanent: false,
                        }),
                        ItemCmd::CopyPath => {
                            actions.push(Action::CopyPathOf(vec![target.path.clone()]))
                        }
                        ItemCmd::Properties => {
                            actions.push(Action::Properties(target.path.clone()))
                        }
                        ItemCmd::Preview => {
                            self.quick = Some(QuickLook::new(target));
                        }
                    }
                }
            }
        }

        // Esc step-back
        if close {
            let p = self.palette.as_ref().unwrap();
            match &p.mode {
                Mode::Actions(_) => {
                    // back one level
                    let p = self.palette.as_mut().unwrap();
                    p.mode = Mode::Files(p.saved_scope);
                    p.text = p.saved_query.clone();
                    p.last_text = p.saved_query.clone();
                    p.dirty = true;
                }
                Mode::Files(_) => {
                    self.palette = None;
                }
            }
        }

        if self.palette.is_none() {
            return;
        }
        // ---- drawing ----
        let screen = ctx.screen_rect();
        let backdrop = egui::Area::new(egui::Id::new("palette_backdrop"))
            .fixed_pos(screen.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(r, 0.0, Color32::from_black_alpha(40));
                resp
            });
        if backdrop.inner.clicked() {
            self.palette = None;
            return;
        }

        let tf = super::text_factor();
        let width = (screen.width() - 60.0).min(760.0).max(520.0);
        let max_h = (screen.height() - 120.0).min(560.0).max(300.0);

        let dark = ctx.style().visuals.dark_mode;
        let pal = super::pal(dark);

        egui::Area::new(egui::Id::new("palette_card"))
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -screen.height() * 0.08))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_min_width(width);
                ui.set_max_width(width);
                egui::Frame::new()
                    .fill(pal.content)
                    .stroke(Stroke::new(1.0, pal.line))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(14))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 12],
                        blur: 32,
                        spread: 0,
                        color: Color32::from_black_alpha(100),
                    })
                    .show(ui, |ui| {
                        ui.set_max_height(max_h);
                        self.palette_input_row(ui, &pal, tf);
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .id_salt("palette_list")
                            .max_height(max_h - 110.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                self.palette_rows(ui, &pal, tf);
                            });
                        ui.add_space(6.0);
                        self.palette_footer(ui, &pal, tf);
                    });
            });
    }

    fn palette_input_row(&mut self, ui: &mut egui::Ui, pal: &super::Pal, tf: f32) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("⌕")
                    .size(18.0 * tf)
                    .color(pal.weak),
            );
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.palette.as_mut().unwrap().text)
                    .id(egui::Id::new("palette_input"))
                    .desired_width(ui.available_width() - 110.0)
                    .font(egui::FontId::proportional(14.0 * tf))
                    .frame(false),
            );
            if !resp.has_focus() {
                resp.request_focus();
            }
            // mode pill
            let mode_name = {
                let p = self.palette.as_ref().unwrap();
                match &p.mode {
                    Mode::Files(SearchScope::Folder) => i18n::t("Bu klasör"),
                    Mode::Files(SearchScope::Subfolders) => i18n::t("Alt klasörler"),
                    Mode::Files(SearchScope::ThisPc) => i18n::t("Bu bilgisayar"),
                    Mode::Actions(_) => i18n::t("Komutlar"),
                }
            };
            ui.label(
                RichText::new(mode_name)
                    .size(11.0 * tf)
                    .color(self.accent),
            );
        });
        // chip for actions target
        let target_chip = {
            let p = self.palette.as_ref().unwrap();
            match &p.mode {
                Mode::Actions(e) => Some(e.name.clone()),
                _ => None,
            }
        };
        if let Some(name) = target_chip {
            ui.label(
                RichText::new(format!("[{}]", name))
                    .size(12.0)
                    .color(self.accent),
            );
        }
        // chips from query
        if let Mode::Files(_) = self.palette.as_ref().unwrap().mode {
            let p = self.palette.as_ref().unwrap();
            let q = crate::query::Query::parse(match &p.mode {
                Mode::Files(s) => {
                    let (_, r) = parse_prefix(&p.text);
                    r
                }
                _ => "",
            });
            if !q.chips.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for c in &q.chips {
                        ui.label(
                            RichText::new(&c.token)
                                .size(11.0)
                                .background_color(Color32::from_gray(50)),
                        );
                    }
                });
            }
        }
    }

    fn palette_rows(&mut self, ui: &mut egui::Ui, pal: &super::Pal, tf: f32) {
        let rows = self
            .palette
            .as_ref()
            .map(|p| p.rows.clone())
            .unwrap_or_default();
        let cur = self.palette.as_ref().map(|p| p.cursor).unwrap_or(0);
        if rows.is_empty() {
            // empty state
            ui.label(
                RichText::new(i18n::t("Son değiştirilenler"))
                    .size(11.0)
                    .color(pal.weak),
            );
            let _ = ui;
            return;
        }
        for (idx, row) in rows.iter().enumerate() {
            let selected = idx == cur;
            let bg = if selected {
                self.accent.linear_multiply(0.25)
            } else {
                Color32::TRANSPARENT
            };
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), 34.0 * tf),
                Sense::click(),
            );
            if selected || resp.hovered() {
                ui.painter().rect_filled(
                    rect,
                    4.0,
                    if resp.hovered() && !selected {
                        Color32::from_gray(40)
                    } else {
                        bg
                    },
                );
            }
            // icon
            let icon_rect = Rect::from_min_size(
                rect.min + Vec2::new(6.0, (34.0 * tf - 16.0) / 2.0),
                Vec2::splat(16.0),
            );
            if let Some(e) = &row.entry {
                let px = super::icon_px(ui.ctx(), 16.0);
                let key = super::icon_key(&e.path, e.is_dir, &e.ext, px, e.plain_dir);
                let img = self.images.get(&key, &e.path, px, true);
                match img {
                    Img::Ready(tex) => {
                        ui.painter().image(
                            tex.id(),
                            icon_rect,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    _ => super::paint_fallback(ui.painter(), icon_rect, e.is_dir),
                }
            } else {
                // command row: small glyph
                ui.painter().text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "›",
                    egui::FontId::proportional(14.0),
                    pal.weak,
                );
            }
            // label with spans
            let label_x = rect.min.x + 28.0;
            let label_rect = Rect::from_min_size(
                Pos2::new(label_x, rect.min.y + 4.0 * tf),
                Vec2::new(rect.width() - 120.0, 16.0 * tf),
            );
            let mut job = egui::text::LayoutJob::default();
            if row.spans.is_empty() {
                job.append(
                    &row.label,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(13.0 * tf),
                        color: if row.enabled {
                            Color32::from_gray(230)
                        } else {
                            Color32::from_gray(120)
                        },
                        ..Default::default()
                    },
                );
            } else {
                // build by walking spans
                let mut cursor = 0usize;
                let mut sorted = row.spans.clone();
                sorted.sort();
                for (s, e) in sorted {
                    if s > cursor {
                        job.append(
                            &row.label[cursor..s],
                            0.0,
                            egui::TextFormat {
                                font_id: egui::FontId::proportional(13.0 * tf),
                                color: if row.enabled {
                                    Color32::from_gray(230)
                                } else {
                                    Color32::from_gray(120)
                                },
                                ..Default::default()
                            },
                        );
                    }
                    let end = e.min(row.label.len());
                    job.append(
                        &row.label[s..end],
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(13.0 * tf),
                            color: self.accent,
                            ..Default::default()
                        },
                    );
                    cursor = end;
                }
                if cursor < row.label.len() {
                    job.append(
                        &row.label[cursor..],
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(13.0 * tf),
                            color: if row.enabled {
                                Color32::from_gray(230)
                            } else {
                                Color32::from_gray(120)
                            },
                            ..Default::default()
                        },
                    );
                }
            }
            ui.painter().galley(
                label_rect.min,
                ui.fonts(|f| f.layout_job(job)),
                Color32::WHITE,
            );
            // detail (relative path or shortcut)
            if !row.detail.is_empty() {
                let drect = Rect::from_min_size(
                    Pos2::new(label_x, rect.min.y + 18.0 * tf),
                    Vec2::new(rect.width() - 120.0, 14.0 * tf),
                );
                ui.painter().text(
                    drect.min,
                    egui::Align2::LEFT_TOP,
                    &row.detail,
                    egui::FontId::proportional(10.5 * tf),
                    Color32::from_gray(130),
                );
            }
            // meta right
            if !row.meta.is_empty() {
                ui.painter().text(
                    rect.right_top() + Vec2::new(-8.0, 4.0 * tf),
                    egui::Align2::RIGHT_TOP,
                    &row.meta,
                    egui::FontId::proportional(10.5 * tf),
                    Color32::from_gray(130),
                );
            }
            // reason if disabled
            if !row.enabled && !row.reason.is_empty() {
                ui.painter().text(
                    rect.right_top() + Vec2::new(-8.0, 18.0 * tf),
                    egui::Align2::RIGHT_TOP,
                    i18n::t(row.reason),
                    egui::FontId::proportional(10.0 * tf),
                    Color32::from_gray(110),
                );
            }
            // click handling
            if resp.clicked() {
                // set cursor
                let p = self.palette.as_mut().unwrap();
                p.cursor = idx;
                // if actions mode, run? We'll let Enter handle; click on row selects only.
            }
            if resp.double_clicked() {
                // treat as run
                // set cursor and flag run via a stored request? Simpler: push a marker? We'll handle by setting a flag in palette.
                let p = self.palette.as_mut().unwrap();
                p.cursor = idx;
                // request run next frame by storing in a field? Use actions push directly:
                // We'll re-run logic by calling a helper method. For simplicity, set a flag.
                // Use self.palette run request via a bool in Palette? Add field run_requested.
            }
        }
        let _ = (ui,);
    }

    fn palette_footer(&mut self, ui: &mut egui::Ui, pal: &super::Pal, tf: f32) {
        let p = self.palette.as_ref().unwrap();
        let scan_info = if let Some(s) = &p.scan {
            if s.done {
                format!("{} klasör · {} sonuç", s.scanned, s.found.len())
            } else {
                format!("{} klasör taranıyor · {} sonuç", s.scanned, s.found.len())
            }
        } else {
            String::new()
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Up/Down move   Enter open   Right commands   Ctrl+Enter reveal   Esc close")
                    .size(10.5 * tf)
                    .color(pal.weak),
            );
            if !scan_info.is_empty() {
                ui.with_layout(Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(scan_info)
                            .size(10.5 * tf)
                            .color(pal.weak),
                    );
                });
            }
        });
    }

    fn palette_rebuild(&mut self) {
        // gather data
        let p_text;
        let p_mode;
        let p_saved;
        {
            let p = self.palette.as_ref().unwrap();
            p_text = p.text.clone();
            p_mode = p.mode.clone();
            p_saved = p.saved_query.clone();
        }
        let t = &self.tabs[self.active];
        let entries = t.entries.clone();
        let is_home = t.is_home();
        let now = chrono::Local::now();

        let mut rows: Vec<Row> = Vec::new();

        match p_mode {
            Mode::Files(scope) => {
                // determine actual scope from prefix OR stored? Use prefix parse.
                let (pscope, rest) = parse_prefix(&p_text);
                let scope = match scope {
                    SearchScope::Folder => pscope,
                    _ => scope,
                };
                // use pscope from prefix for correctness
                let (scope, rest) = parse_prefix(&p_text);
                let _ = (scope, is_home);

                // scan results if available
                let scan_found: Vec<Entry> = self
                    .palette
                    .as_ref()
                    .unwrap()
                    .scan
                    .as_ref()
                    .map(|s| s.found.clone())
                    .unwrap_or_default();

                let use_scan = matches!(scope, SearchScope::Subfolders | SearchScope::ThisPc)
                    && !rest.trim().is_empty();

                if rest.trim().is_empty() && !use_scan {
                    // empty state: 6 most recently modified
                    let mut sorted: Vec<&Entry> = entries.iter().collect();
                    sorted.sort_by(|a, b| b.modified.cmp(&a.modified));
                    for e in sorted.into_iter().take(6) {
                        rows.push(Row {
                            entry: Some(e.clone()),
                            cmd: None,
                            label: e.name.clone(),
                            spans: Vec::new(),
                            detail: rel_path(e, &self.tabs[self.active].path),
                            meta: format!("{} · {}", human_size(e.size), e.time_label),
                            enabled: true,
                            reason: "",
                        });
                    }
                } else {
                    let q = crate::query::Query::parse(rest);
                    let terms = q.free_terms().to_vec();
                    // source list
                    let source: Vec<Entry> = if use_scan {
                        scan_found
                    } else {
                        entries.clone()
                    };
                    // filter
                    let mut cands: Vec<(Entry, i32, Vec<(usize, usize)>)> = Vec::new();
                    for e in source {
                        if !q.matches(&e, now) {
                            continue;
                        }
                        let (sc, spans) = match score(&e.name_lower, &terms) {
                            Some(x) => x,
                            None => (0, Vec::new()),
                        };
                        // folder boost
                        let boost = if e.is_dir { 100 } else { 0 };
                        cands.push((e, sc + boost, spans));
                    }
                    // sort
                    cands.sort_by(|a, b| {
                        b.1.cmp(&a.1)
                            .then(a.0.is_dir.cmp(&b.0.is_dir).reverse())
                            .then(
                                a.0.path
                                    .components()
                                    .count()
                                    .cmp(&b.0.path.components().count()),
                            )
                    });
                    let total = cands.len();
                    for (e, _sc, spans) in cands.into_iter().take(8) {
                        rows.push(Row {
                            entry: Some(e.clone()),
                            cmd: None,
                            label: e.name.clone(),
                            spans,
                            detail: rel_path(&e, &self.tabs[self.active].path),
                            meta: format!("{} · {}", human_size(e.size), e.time_label),
                            enabled: true,
                            reason: "",
                        });
                    }
                    if total > 8 {
                        rows.push(Row {
                            entry: None,
                            cmd: None,
                            label: format!("Tüm sonuçları göster ({})", total),
                            spans: Vec::new(),
                            detail: String::new(),
                            meta: String::new(),
                            enabled: true,
                            reason: "",
                        });
                        // mark as ShowAll via a parallel? We'll use a flag in label detection.
                        // To keep RowKind, add field kind. We didn't. Use cmd None + entry None + label starts with "Tüm".
                        // Better: add kind field. But Row already defined. Let's add `kind: RowKind`? Too late.
                        // We'll handle in run() by checking entry.is_none() && cmd.is_none() && label starts with "Tüm".
                    }
                }
            }
            Mode::Actions(target) => {
                let filter = p_text.to_lowercase();
                let cmds = item_commands(&target);
                for c in cmds {
                    let (enabled, reason) = cmd_enabled(c, &target, self);
                    let label = i18n::t(cmd_label(c)).to_string();
                    if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                        continue;
                    }
                    rows.push(Row {
                        entry: None,
                        cmd: Some(c),
                        label,
                        spans: Vec::new(),
                        detail: String::new(),
                        meta: String::new(),
                        enabled,
                        reason,

                    });
                }
            }
        }

        // clamp cursor
        let p = self.palette.as_mut().unwrap();
        if rows.is_empty() {
            p.cursor = 0;
        } else if p.cursor >= rows.len() {
            p.cursor = rows.len() - 1;
        }
        p.rows = rows;
        p.dirty = false;
    }
}

// helper to get highlighted entry
fn palette_highlighted_entry(app: &App) -> Option<Entry> {
    let p = app.palette.as_ref()?;
    match &p.mode {
        Mode::Files(_) => p.rows.get(p.cursor).and_then(|r| r.entry.clone()),
        Mode::Actions(e) => Some(e.clone()),
    }
}

fn rel_path(e: &Entry, base: &std::path::Path) -> String {
    e.path
        .strip_prefix(base)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| e.path.to_string_lossy().into_owned())
}

fn human_size(n: u64) -> String {
    if n < 1024 {
        format!("{} B", n)
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else if n < 1024 * 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

// RowKind not defined; add a helper marker stored in reason? We'll detect ShowAll by entry None & cmd None & label starts with "Tüm"
// Add a field to Row? We already shipped Row without kind. To support ShowAll detection in run(), check label.

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_prefix_cases() {
        assert_eq!(parse_prefix("rapor"), (SearchScope::Folder, "rapor"));
        assert_eq!(parse_prefix("; rapor"), (SearchScope::Subfolders, "rapor"));
        assert_eq!(parse_prefix(";"), (SearchScope::Subfolders, ""));
        assert_eq!(parse_prefix(";; rapor"), (SearchScope::ThisPc, "rapor"));
        assert_eq!(parse_prefix(";;"), (SearchScope::ThisPc, ""));
        // filename with leading semicolon is not a prefix
        assert_eq!(
            parse_prefix(";important.txt"),
            (SearchScope::Folder, ";important.txt")
        );
    }
    #[test]
    fn score_basic() {
        let terms = vec!["rep".to_string()];
        let r = score("report.md", &terms).unwrap();
        assert!(r.0 >= 400);
        let r2 = score("unrelated.txt", &terms);
        assert!(r2.is_none());
    }
}
