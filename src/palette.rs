//! Ctrl+K command palette.
//!
//! This is a child module of `app` (declared with `#[path]`), so it can read the
//! application's private state without widening field visibility.
//!
//! The palette has two levels:
//!
//! * `Files` - entries from the current folder. `;` widens the scope to
//!   subfolders and `;;` to this PC; the prefix lives in the query text, so the
//!   scope can never drift away from what the user typed.
//! * `Actions` - the commands that apply to one highlighted entry. `→` enters
//!   this level, `←` (or backspace on an empty filter) returns to the results.
//!
//! Drawing uses the same theme palette, font family and pixel-rounded sizes as
//! the rest of the application (`pal`, `ui_font`, `text_size`), so the overlay
//! looks like another surface of the app rather than a separate dialog.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, FontId, Key, Margin, Modifiers, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Vec2,
};

use crate::fsutil::{self, Entry};
use crate::i18n;
use crate::query::{Query, SearchScope};
use crate::shell::Img;

use super::*;

// ---------------------------------------------------------------- constants

/// Wait after the last keystroke before starting a folder scan.
const SCAN_DEBOUNCE: Duration = Duration::from_millis(160);
/// How often to repaint while a scan is streaming results.
const SCAN_REPAINT: Duration = Duration::from_millis(100);
/// Results shown in the palette; the rest is one "show all" row.
const MAX_ROWS: usize = 8;
/// Recently modified items shown while the query is empty.
const RECENT_ROWS: usize = 6;
/// Scan hits kept in memory. A broad "this PC" scan can match hundreds of
/// thousands of files; the palette only ever shows a handful, so keeping a
/// bounded sample keeps the per-frame sort and clone cheap. `found_total`
/// still reports the true match count.
const SCAN_KEEP: usize = 2000;

/// `(entry, score, matched byte ranges)`
type Candidate = (Entry, i32, Vec<(usize, usize)>);

fn input_id() -> egui::Id {
    egui::Id::new("palette_input")
}

// ---------------------------------------------------------------- state

#[derive(Clone)]
pub enum Mode {
    Files,
    Actions(Entry),
}

#[derive(Clone)]
pub enum RowKind {
    Entry(Entry),
    Command(ItemCmd),
    /// Hands the query over to the toolbar search (current folder only).
    ShowAll,
}

#[derive(Clone)]
pub struct Row {
    pub kind: RowKind,
    pub label: String,
    pub spans: Vec<(usize, usize)>,
    pub detail: String,
    pub meta: String,
    pub enabled: bool,
    pub reason: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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

pub struct Scan {
    pub handle: fsutil::SearchHandle,
    pub key: String,
    pub found: Vec<Entry>,
    pub found_total: usize,
    pub scanned: u64,
    pub done: bool,
}

pub struct Palette {
    pub text: String,
    pub last_text: String,
    pub mode: Mode,
    /// Query and highlight restored when leaving `Actions`.
    pub saved_query: String,
    pub saved_cursor: usize,
    pub cursor: usize,
    pub rows: Vec<Row>,
    pub dirty: bool,
    pub scan: Option<Scan>,
    pub last_edit: Instant,
    /// Row that should be scrolled into view after keyboard navigation.
    pub scroll_to: Option<usize>,
}

impl Palette {
    pub fn new() -> Self {
        Palette {
            text: String::new(),
            last_text: String::new(),
            mode: Mode::Files,
            saved_query: String::new(),
            saved_cursor: 0,
            cursor: 0,
            rows: Vec::new(),
            dirty: true,
            scan: None,
            last_edit: Instant::now(),
            scroll_to: None,
        }
    }
}

// ---------------------------------------------------------------- parsing

/// Splits the scope prefix from the free-text query. A leading `;` is only a
/// prefix when it is alone or followed by a space, so a file named
/// `;notes.txt` can still be searched.
pub fn parse_prefix(text: &str) -> (SearchScope, &str) {
    if let Some(rest) = text.strip_prefix(";;") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return (SearchScope::ThisPc, rest.trim_start());
        }
    }
    if let Some(rest) = text.strip_prefix(';') {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return (SearchScope::Subfolders, rest.trim_start());
        }
    }
    (SearchScope::Folder, text)
}

fn prefix_for(scope: SearchScope) -> &'static str {
    match scope {
        SearchScope::Folder => "",
        SearchScope::Subfolders => "; ",
        SearchScope::ThisPc => ";; ",
    }
}

fn next_scope(scope: SearchScope) -> SearchScope {
    match scope {
        SearchScope::Folder => SearchScope::Subfolders,
        SearchScope::Subfolders => SearchScope::ThisPc,
        SearchScope::ThisPc => SearchScope::Folder,
    }
}

fn prev_scope(scope: SearchScope) -> SearchScope {
    match scope {
        SearchScope::Folder => SearchScope::ThisPc,
        SearchScope::Subfolders => SearchScope::Folder,
        SearchScope::ThisPc => SearchScope::Subfolders,
    }
}

// ---------------------------------------------------------------- scoring

/// Scores a name against the free query terms and returns the byte ranges that
/// matched, so the label can tint them with the accent color.
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
        for span in sp {
            if !spans.contains(&span) {
                spans.push(span);
            }
        }
    }
    Some((total, spans))
}

fn match_term(name: &str, term: &str) -> Option<(i32, Vec<(usize, usize)>)> {
    if name == term {
        return Some((1000, vec![(0, name.len())]));
    }
    if name.starts_with(term) {
        return Some((800, vec![(0, term.len())]));
    }

    // Word start (`report` in `yearly-report.pdf`).
    let bytes = name.as_bytes();
    for i in 0..name.len() {
        if i + term.len() > name.len() {
            break;
        }
        let boundary = i == 0
            || matches!(
                bytes[i - 1] as char,
                ' ' | '-' | '_' | '.' | '/' | '\\' | '('
            );
        if boundary && name[i..].starts_with(term) {
            return Some((600, vec![(i, i + term.len())]));
        }
    }

    if let Some(pos) = name.find(term) {
        return Some((400, vec![(pos, pos + term.len())]));
    }

    // Loose subsequence match (`rpt` in `report`).
    let nchars: Vec<char> = name.chars().collect();
    let mut byte_offsets: Vec<usize> = name.char_indices().map(|(b, _)| b).collect();
    byte_offsets.push(name.len());
    let mut matched: Vec<(usize, usize)> = Vec::new();
    let mut ci = 0usize;
    for tc in term.chars() {
        while ci < nchars.len() && nchars[ci] != tc {
            ci += 1;
        }
        if ci >= nchars.len() {
            return None;
        }
        matched.push((byte_offsets[ci], byte_offsets[ci + 1]));
        ci += 1;
    }

    let mut merged: Vec<(usize, usize)> = Vec::new();
    let mut cur: Option<(usize, usize)> = None;
    for (start, end) in matched {
        match cur {
            Some((cs, ce)) if ce == start => cur = Some((cs, end)),
            Some(previous) => {
                merged.push(previous);
                cur = Some((start, end));
            }
            None => cur = Some((start, end)),
        }
    }
    if let Some(last) = cur {
        merged.push(last);
    }
    Some((200, merged))
}

// ---------------------------------------------------------------- item commands

pub fn item_commands(entry: &Entry) -> Vec<ItemCmd> {
    let mut commands = Vec::new();
    commands.push(ItemCmd::Open);
    if entry.is_dir {
        commands.push(ItemCmd::OpenNewTab);
    } else {
        commands.push(ItemCmd::OpenWith);
    }
    commands.push(ItemCmd::OpenContaining);
    commands.push(ItemCmd::Copy);
    commands.push(ItemCmd::Cut);
    commands.push(ItemCmd::Rename);
    commands.push(ItemCmd::Delete);
    commands.push(ItemCmd::CopyPath);
    commands.push(ItemCmd::Properties);
    commands.push(ItemCmd::Preview);
    commands
}

pub fn cmd_label(command: ItemCmd) -> &'static str {
    match command {
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

/// `Some(reason)` when the command cannot run for this target.
fn command_disabled(command: ItemCmd, entry: &Entry, app: &App) -> Option<&'static str> {
    let virtual_location = crate::tab::is_virtual(&entry.path) || entry.path.as_os_str().is_empty();
    let home = app.tabs[app.active].is_home();
    let root_reason = "Sürücü kökünde kullanılamaz";
    let here_reason = "Burada kullanılamaz";
    match command {
        ItemCmd::Rename | ItemCmd::Cut | ItemCmd::Delete => {
            if virtual_location {
                Some(root_reason)
            } else if home {
                Some(here_reason)
            } else {
                None
            }
        }
        ItemCmd::OpenContaining => {
            if virtual_location || entry.path.parent().is_none() {
                Some(root_reason)
            } else {
                None
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- app glue

impl super::App {
    pub fn open_palette(&mut self) {
        crate::diag::log("palette: open");
        self.quick = None;
        self.menu = None;
        self.palette = Some(Palette::new());
        self.ctx.request_repaint();
    }

    pub fn close_palette(&mut self) {
        crate::diag::log("palette: close");
        if let Some(palette) = self.palette.as_mut() {
            if let Some(scan) = &palette.scan {
                scan.handle.cancel.store(true, Ordering::Relaxed);
            }
        }
        self.palette = None;
        self.ctx.request_repaint();
    }

    pub fn toggle_palette(&mut self) {
        if self.palette.is_some() {
            self.close_palette();
        } else if matches!(self.dialog, Dialog::None) {
            self.open_palette();
        }
    }

    /// `Esc` and `←` step back one level; at the top level they close.
    pub fn palette_back(&mut self, ctx: &egui::Context) {
        let mut close = false;
        {
            let Some(palette) = self.palette.as_mut() else {
                return;
            };
            match &palette.mode {
                Mode::Actions(entry) => {
                    crate::diag::log(format!("palette: back from actions for {:?}", entry.path));
                    palette.mode = Mode::Files;
                    palette.text = palette.saved_query.clone();
                    palette.last_text = palette.text.clone();
                    palette.cursor = palette.saved_cursor;
                    palette.dirty = true;
                    palette.scroll_to = Some(palette.cursor);
                }
                Mode::Files => {
                    crate::diag::log("palette: back from results (close)");
                    close = true;
                }
            }
        }
        if close {
            self.close_palette();
        } else {
            set_caret(ctx, usize::MAX);
        }
    }

    // ------------------------------------------------------------ lifecycle

    pub(super) fn palette_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.palette.is_none() {
            return;
        }

        // Keep the query focused even after a row click moved focus away.
        if !ctx.memory(|memory| memory.has_focus(input_id())) {
            ctx.memory_mut(|memory| memory.request_focus(input_id()));
        }

        self.palette_note_edit();
        self.palette_poll_scan(ctx);
        self.palette_start_scan(ctx);
        self.palette_keys(ctx, actions);

        if self.palette.is_none() {
            return;
        }
        if self.palette.as_ref().map(|p| p.dirty).unwrap_or(false) {
            self.palette_rebuild();
        }
        self.palette_draw(ctx, actions);
    }

    fn palette_note_edit(&mut self) {
        let changed = self
            .palette
            .as_ref()
            .map(|p| p.text != p.last_text)
            .unwrap_or(false);
        if !changed {
            return;
        }
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if let Some(scan) = &palette.scan {
            scan.handle.cancel.store(true, Ordering::Relaxed);
        }
        palette.scan = None;
        palette.last_text = palette.text.clone();
        palette.last_edit = Instant::now();
        palette.dirty = true;
    }

    fn palette_poll_scan(&mut self, ctx: &egui::Context) {
        let mut dirty = false;
        let scanning;
        {
            let Some(palette) = self.palette.as_mut() else {
                return;
            };
            if let Some(scan) = &mut palette.scan {
                while let Ok(message) = scan.handle.rx.try_recv() {
                    match message {
                        fsutil::SearchMsg::Batch(entries) => {
                            scan.found_total += entries.len();
                            if scan.found.len() < SCAN_KEEP {
                                let room = SCAN_KEEP - scan.found.len();
                                scan.found.extend(entries.into_iter().take(room));
                            }
                            dirty = true;
                        }
                        fsutil::SearchMsg::Done => {
                            scan.done = true;
                            dirty = true;
                        }
                    }
                }
                scan.scanned = scan.handle.scanned.load(Ordering::Relaxed);
            }
            if dirty {
                palette.dirty = true;
            }
            scanning = palette.scan.as_ref().map(|s| !s.done).unwrap_or(false);
        }
        if scanning {
            ctx.request_repaint_after(SCAN_REPAINT);
        }
    }

    fn palette_start_scan(&mut self, ctx: &egui::Context) {
        let (scope, rest) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            match palette.mode {
                Mode::Files => {
                    let (scope, rest) = parse_prefix(&palette.text);
                    (scope, rest.trim().to_string())
                }
                Mode::Actions(_) => (SearchScope::Folder, String::new()),
            }
        };

        if !matches!(scope, SearchScope::Subfolders | SearchScope::ThisPc) || rest.is_empty() {
            let Some(palette) = self.palette.as_mut() else {
                return;
            };
            if let Some(scan) = &palette.scan {
                scan.handle.cancel.store(true, Ordering::Relaxed);
            }
            if palette.scan.take().is_some() {
                palette.dirty = true;
            }
            return;
        }

        let key = format!("{scope:?}|{rest}");
        let elapsed;
        let (needs_start, running_wrong) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            elapsed = palette.last_edit.elapsed();
            match &palette.scan {
                None => (true, false),
                Some(scan) if scan.key != key => (scan.done, !scan.done),
                Some(_) => (false, false),
            }
        };

        if running_wrong {
            let Some(palette) = self.palette.as_mut() else {
                return;
            };
            if let Some(scan) = palette.scan.take() {
                scan.handle.cancel.store(true, Ordering::Relaxed);
            }
            palette.dirty = true;
        }

        if !needs_start {
            return;
        }
        if elapsed < SCAN_DEBOUNCE {
            ctx.request_repaint_after(SCAN_DEBOUNCE - elapsed);
            return;
        }

        let roots = match scope {
            SearchScope::Subfolders => vec![self.tabs[self.active].path.clone()],
            SearchScope::ThisPc => {
                let drives = self.drives.lock().unwrap();
                drives.iter().map(|drive| drive.root.clone()).collect()
            }
            SearchScope::Folder => return,
        };
        let query = Query::parse(&rest);
        let handle = fsutil::spawn_search(roots, query, ctx.clone());

        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.scan = Some(Scan {
            handle,
            key,
            found: Vec::new(),
            found_total: 0,
            scanned: 0,
            done: false,
        });
        palette.dirty = true;
    }

    // ------------------------------------------------------------ keys

    fn palette_keys(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let (files_mode, text, empty) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            (
                matches!(palette.mode, Mode::Files),
                palette.text.clone(),
                palette.text.is_empty(),
            )
        };

        let mut up = false;
        let mut down = false;
        let mut page_up = false;
        let mut page_down = false;
        let mut enter = false;
        let mut reveal = false;
        let mut peek = false;
        let mut drill = false;
        let mut back = false;
        let mut cycle: Option<bool> = None;

        // Read the caret position before taking the input lock: loading the
        // text-edit state goes through `Context::data_mut`, which takes the
        // same context lock as `input_mut` and would deadlock the UI thread.
        let caret_end = caret_at_end(ctx, input_id(), &text);
        let caret_start = caret_at_start(ctx, input_id());

        ctx.input_mut(|input| {
            up = input.consume_key(Modifiers::NONE, Key::ArrowUp);
            down = input.consume_key(Modifiers::NONE, Key::ArrowDown);
            page_up = input.consume_key(Modifiers::NONE, Key::PageUp);
            page_down = input.consume_key(Modifiers::NONE, Key::PageDown);

            if input.key_pressed(Key::Enter) {
                if input.modifiers.ctrl {
                    reveal = true;
                    input.consume_key(Modifiers::CTRL, Key::Enter);
                } else if input.modifiers == Modifiers::NONE {
                    enter = true;
                    input.consume_key(Modifiers::NONE, Key::Enter);
                }
            }
            if input.modifiers.ctrl && input.key_pressed(Key::Space) {
                peek = true;
                input.consume_key(Modifiers::CTRL, Key::Space);
            }

            // Tab / Shift+Tab cycle the search scope. Consume it so the text
            // edit never loses focus.
            if input.consume_key(Modifiers::NONE, Key::Tab) {
                cycle = Some(false);
            } else if input.consume_key(Modifiers::SHIFT, Key::Tab) {
                cycle = Some(true);
            }

            let plain = !input.modifiers.ctrl && !input.modifiers.alt && !input.modifiers.shift;
            if plain {
                if files_mode {
                    // `→` drills into the item commands only when the caret is
                    // already at the end of the query; otherwise the text edit
                    // gets the key and moves the caret as usual.
                    if input.key_pressed(Key::ArrowRight)
                        && caret_end
                        && input.consume_key(Modifiers::NONE, Key::ArrowRight)
                    {
                        drill = true;
                    }
                } else {
                    if input.key_pressed(Key::ArrowLeft)
                        && caret_start
                        && input.consume_key(Modifiers::NONE, Key::ArrowLeft)
                    {
                        back = true;
                    }
                    if empty && input.consume_key(Modifiers::NONE, Key::Backspace) {
                        back = true;
                    }
                }
            }
        });

        if drill || back || cycle.is_some() || enter || reveal || peek {
            crate::diag::log(format!(
                "palette: keys drill={drill} back={back} cycle={cycle:?} enter={enter} reveal={reveal} peek={peek}"
            ));
        }

        if back {
            self.palette_back(ctx);
            return;
        }
        if drill {
            self.palette_drill(ctx);
            return;
        }
        if let Some(backwards) = cycle {
            self.palette_cycle_scope(ctx, backwards);
            return;
        }

        if let Some(palette) = self.palette.as_mut() {
            let count = palette.rows.len();
            if count > 0 {
                let mut cursor = palette.cursor.min(count - 1);
                if up {
                    cursor = cursor.saturating_sub(1);
                }
                if down {
                    cursor = (cursor + 1).min(count - 1);
                }
                if page_up {
                    cursor = cursor.saturating_sub(8);
                }
                if page_down {
                    cursor = (cursor + 8).min(count - 1);
                }
                if cursor != palette.cursor {
                    palette.cursor = cursor;
                    palette.scroll_to = Some(cursor);
                }
            }
        }

        if enter {
            self.palette_activate(actions);
        } else if reveal {
            self.palette_reveal(actions);
        } else if peek {
            self.palette_peek();
        }
    }

    fn palette_cycle_scope(&mut self, ctx: &egui::Context, backwards: bool) {
        let (scope, rest) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            if !matches!(palette.mode, Mode::Files) {
                return;
            }
            let (scope, rest) = parse_prefix(&palette.text);
            (scope, rest.to_string())
        };
        let scope = if backwards {
            prev_scope(scope)
        } else {
            next_scope(scope)
        };
        let text = format!("{}{}", prefix_for(scope), rest);

        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if let Some(scan) = &palette.scan {
            scan.handle.cancel.store(true, Ordering::Relaxed);
        }
        palette.scan = None;
        palette.text = text.clone();
        palette.last_text = text;
        palette.cursor = 0;
        palette.last_edit = Instant::now();
        palette.dirty = true;
        set_caret(ctx, usize::MAX);
    }

    fn palette_drill(&mut self, ctx: &egui::Context) {
        let target = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            palette
                .rows
                .get(palette.cursor)
                .and_then(|row| match &row.kind {
                    RowKind::Entry(entry) => Some(entry.clone()),
                    _ => None,
                })
        };
        let Some(entry) = target else {
            return;
        };
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        crate::diag::log(format!(
            "palette: drill into {:?} (cursor {})",
            entry.path, palette.cursor
        ));
        palette.saved_query = palette.text.clone();
        palette.saved_cursor = palette.cursor;
        palette.mode = Mode::Actions(entry);
        palette.text.clear();
        palette.last_text.clear();
        palette.cursor = 0;
        palette.dirty = true;
        if let Some(scan) = &palette.scan {
            scan.handle.cancel.store(true, Ordering::Relaxed);
        }
        palette.scan = None;
        set_caret(ctx, 0);
    }

    fn palette_reveal(&mut self, actions: &mut Vec<Action>) {
        if let Some(entry) = highlighted_entry(self) {
            self.close_palette();
            actions.push(Action::RevealPath(entry.path));
        }
    }

    fn palette_peek(&mut self) {
        if let Some(entry) = highlighted_entry(self) {
            self.palette = None;
            self.quick = Some(QuickLook::new(entry));
            self.ctx.request_repaint();
        }
    }

    fn palette_activate(&mut self, actions: &mut Vec<Action>) {
        let (mode, row) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            let Some(row) = palette.rows.get(palette.cursor) else {
                return;
            };
            if !row.enabled {
                return;
            }
            (palette.mode.clone(), row.clone())
        };

        crate::diag::log(format!(
            "palette: activate {:?} ({})",
            row.label,
            if matches!(mode, Mode::Files) {
                "results"
            } else {
                "commands"
            }
        ));

        match mode {
            Mode::Files => match row.kind {
                RowKind::Entry(entry) => {
                    self.close_palette();
                    actions.push(Action::OpenEntry(entry.path, entry.is_dir));
                }
                RowKind::ShowAll => {
                    let filter = self
                        .palette
                        .as_ref()
                        .map(|palette| parse_prefix(&palette.text).1.trim().to_string())
                        .unwrap_or_default();
                    let tab = &mut self.tabs[self.active];
                    tab.filter = filter;
                    tab.view_dirty = true;
                    self.close_palette();
                }
                RowKind::Command(_) => {}
            },
            Mode::Actions(target) => {
                let RowKind::Command(command) = row.kind else {
                    return;
                };
                self.close_palette();
                match command {
                    ItemCmd::Open => {
                        actions.push(Action::OpenEntry(target.path, target.is_dir));
                    }
                    ItemCmd::OpenWith => actions.push(Action::OpenWith(target.path)),
                    ItemCmd::OpenContaining => actions.push(Action::RevealPath(target.path)),
                    ItemCmd::OpenNewTab => actions.push(Action::NewTab(target.path)),
                    ItemCmd::Copy => actions.push(Action::ClipboardPaths {
                        paths: vec![target.path],
                        cut: false,
                    }),
                    ItemCmd::Cut => actions.push(Action::ClipboardPaths {
                        paths: vec![target.path],
                        cut: true,
                    }),
                    ItemCmd::Rename => actions.push(Action::RenamePath(target.path)),
                    ItemCmd::Delete => actions.push(Action::DeletePaths {
                        paths: vec![target.path],
                        permanent: false,
                    }),
                    ItemCmd::CopyPath => actions.push(Action::CopyPathOf(vec![target.path])),
                    ItemCmd::Properties => actions.push(Action::Properties(target.path)),
                    ItemCmd::Preview => {
                        self.quick = Some(QuickLook::new(target));
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ rebuild

    fn palette_rebuild(&mut self) {
        let (mode, text) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            (palette.mode.clone(), palette.text.clone())
        };
        let base = self.tabs[self.active].path.clone();
        let show_hidden = self.show_hidden;
        let mut rows: Vec<Row> = Vec::new();

        match mode {
            Mode::Files => {
                let (scope, rest) = parse_prefix(&text);
                let rest = rest.trim();
                if rest.is_empty() {
                    let mut recent: Vec<&Entry> = self.tabs[self.active]
                        .entries
                        .iter()
                        .filter(|entry| show_hidden || !entry.hidden)
                        .collect();
                    recent.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
                    for entry in recent.into_iter().take(RECENT_ROWS) {
                        rows.push(Row::file(entry, &base, Vec::new()));
                    }
                } else {
                    let query = Query::parse(rest);
                    let now = chrono::Local::now();
                    let source: Vec<Entry> = match scope {
                        SearchScope::Folder => self.tabs[self.active].entries.clone(),
                        SearchScope::Subfolders | SearchScope::ThisPc => self
                            .palette
                            .as_ref()
                            .and_then(|palette| palette.scan.as_ref())
                            .map(|scan| scan.found.clone())
                            .unwrap_or_default(),
                    };

                    // Glob terms are already enforced by `Query`; scoring them
                    // would fail because `*` and `?` are not in the name.
                    let terms: Vec<String> = query
                        .free_terms()
                        .iter()
                        .filter(|term| !term.contains('*') && !term.contains('?'))
                        .cloned()
                        .collect();

                    let mut candidates: Vec<Candidate> = Vec::new();
                    for entry in source {
                        if !show_hidden && entry.hidden {
                            continue;
                        }
                        if !query.matches(&entry, now) {
                            continue;
                        }
                        let (mut value, spans) =
                            score(&entry.name_lower, &terms).unwrap_or((0, Vec::new()));
                        if entry.is_dir {
                            value += 60;
                        }
                        candidates.push((entry, value, spans));
                    }
                    candidates.sort_by(|a, b| {
                        b.1.cmp(&a.1)
                            .then_with(|| b.0.is_dir.cmp(&a.0.is_dir))
                            .then_with(|| fsutil::natural_cmp(&a.0.name_lower, &b.0.name_lower))
                    });

                    let total = candidates.len();
                    for (entry, _, spans) in candidates.into_iter().take(MAX_ROWS) {
                        rows.push(Row::file(&entry, &base, spans));
                    }
                    // The toolbar filter only sees the current folder, so a
                    // "show all" hand-off only makes sense there.
                    if scope == SearchScope::Folder && total > MAX_ROWS {
                        rows.push(Row {
                            kind: RowKind::ShowAll,
                            label: i18n::tf("Tüm sonuçları göster ({})", &[&total.to_string()]),
                            spans: Vec::new(),
                            detail: String::new(),
                            meta: String::new(),
                            enabled: true,
                            reason: "",
                        });
                    }
                }
            }
            Mode::Actions(target) => {
                let filter = text.to_lowercase();
                for command in item_commands(&target) {
                    let label = i18n::t(cmd_label(command)).to_string();
                    if !filter.is_empty() && !label.to_lowercase().contains(&filter) {
                        continue;
                    }
                    let reason = command_disabled(command, &target, self);
                    rows.push(Row {
                        kind: RowKind::Command(command),
                        label,
                        spans: Vec::new(),
                        detail: String::new(),
                        meta: String::new(),
                        enabled: reason.is_none(),
                        reason: reason.unwrap_or(""),
                    });
                }
            }
        }

        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        palette.rows = rows;
        if palette.cursor >= palette.rows.len() {
            palette.cursor = palette.rows.len().saturating_sub(1);
        }
        palette.dirty = false;
        crate::diag::log(format!(
            "palette: rebuilt {} rows ({})",
            palette.rows.len(),
            if matches!(palette.mode, Mode::Files) {
                "results"
            } else {
                "commands"
            }
        ));
    }

    // ------------------------------------------------------------ drawing

    fn palette_draw(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let screen = ctx.screen_rect();
        let width = (screen.width() - 48.0)
            .clamp(300.0, 720.0)
            .min((screen.width() - 16.0).max(240.0));
        let max_height = (screen.height() * 0.62)
            .clamp(220.0, 540.0)
            .min((screen.height() - 120.0).max(200.0));
        let input_height = super::text_size(46.0);
        let footer_height = super::text_size(28.0);
        // Input, the two separator lines and the 6 px bottom padding.
        let list_max =
            (max_height - input_height - footer_height - 8.0).max(super::text_size(80.0));

        let mut card_rect = Rect::NOTHING;
        let mut activated: Option<usize> = None;

        // A transparent click catcher: clicks outside the card close the
        // palette, but nothing is dimmed. Overlays must stay as bright as the
        // rest of the app.
        let backdrop = egui::Area::new(egui::Id::new("palette_backdrop"))
            .fixed_pos(screen.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                let (_, response) = ui.allocate_exact_size(screen.size(), Sense::click());
                response
            });

        egui::Area::new(egui::Id::new("palette_card"))
            .anchor(
                Align2::CENTER_TOP,
                Vec2::new(0.0, (screen.height() * 0.12).max(40.0)),
            )
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.set_width(width);
                let palette = super::pal(ui.visuals().dark_mode);

                let card = egui::Frame::new()
                    .fill(palette.content)
                    .stroke(Stroke::new(1.0_f32, palette.line))
                    .corner_radius(10.0)
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 10],
                        blur: 28,
                        spread: 0,
                        color: Color32::from_black_alpha(if ui.visuals().dark_mode {
                            120
                        } else {
                            46
                        }),
                    })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        // Every row is laid out by hand, so the default gap
                        // between widgets would widen the card and push the
                        // right-aligned meta text against the border.
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        self.palette_input(ui, &palette);
                        palette_separator(ui, &palette);

                        egui::ScrollArea::vertical()
                            .id_salt("palette_list")
                            .max_height(list_max)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                activated = self.palette_rows(ui, &palette);
                            });

                        palette_separator(ui, &palette);
                        self.palette_footer(ui, &palette);
                        ui.add_space(6.0);
                    });
                card_rect = card.response.rect;
            });

        if let Some(index) = activated {
            if let Some(palette) = self.palette.as_mut() {
                palette.cursor = index;
            }
            self.palette_activate(actions);
        }

        if backdrop.inner.clicked() {
            let inside = ctx
                .input(|input| input.pointer.interact_pos())
                .map(|pos| card_rect.contains(pos))
                .unwrap_or(false);
            if !inside {
                self.close_palette();
            }
        }
    }

    fn palette_input(&mut self, ui: &mut egui::Ui, palette: &super::Pal) {
        let (mode_label, hint, chip) = {
            let Some(state) = self.palette.as_ref() else {
                return;
            };
            match &state.mode {
                Mode::Files => {
                    let (scope, _) = parse_prefix(&state.text);
                    let label = match scope {
                        SearchScope::Folder => i18n::t("Bu klasör"),
                        SearchScope::Subfolders => i18n::t("Alt klasörler"),
                        SearchScope::ThisPc => i18n::t("Bu bilgisayar"),
                    };
                    (
                        label.to_string(),
                        i18n::t("Dosya veya klasör ara").to_string(),
                        None,
                    )
                }
                Mode::Actions(entry) => (
                    i18n::t("Komutlar").to_string(),
                    i18n::t("Komut ara").to_string(),
                    Some(entry.name.clone()),
                ),
            }
        };

        let row_height = super::text_size(46.0);
        let control_height = super::text_size(22.0);
        let pill_font = super::ui_font(10.5);
        let pill_width =
            (measure_text(ui, &mode_label, &pill_font) + 20.0).min(ui.available_width() * 0.42);

        ui.add_space((row_height - super::text_size(24.0)) / 2.0);
        ui.horizontal(|ui| {
            ui.add_space(12.0);

            let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
            paint_search_icon(ui.painter(), icon_rect, palette.weak);
            ui.add_space(8.0);

            if let Some(name) = chip {
                let chip_font = super::ui_font(12.0);
                let chip_width = (measure_text(ui, &name, &chip_font) + 18.0)
                    .min(ui.available_width() * 0.40)
                    .max(36.0);
                let (chip_rect, _) =
                    ui.allocate_exact_size(Vec2::new(chip_width, control_height), Sense::hover());
                ui.painter().rect(
                    chip_rect,
                    4.0,
                    palette.field,
                    Stroke::new(1.0_f32, palette.line),
                    StrokeKind::Inside,
                );
                let label = elide_text(ui, &name, &chip_font, chip_width - 12.0);
                ui.painter().with_clip_rect(chip_rect).text(
                    chip_rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    chip_font,
                    palette.text,
                );
                ui.add_space(6.0);
                let (chevron_rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                paint_chevron_right(ui.painter(), chevron_rect, palette.weak);
                ui.add_space(6.0);
            }

            let text_width = (ui.available_width() - pill_width - 12.0).max(60.0);
            {
                let Some(state) = self.palette.as_mut() else {
                    return;
                };
                ui.add_sized(
                    Vec2::new(text_width, super::text_size(24.0)),
                    egui::TextEdit::singleline(&mut state.text)
                        .id(input_id())
                        .font(super::ui_font(14.0))
                        .text_color(palette.text)
                        .hint_text(
                            RichText::new(hint)
                                .font(super::ui_font(13.0))
                                .color(palette.disabled),
                        )
                        .margin(Margin::ZERO)
                        .frame(false)
                        .desired_width(text_width),
                );
            }

            let (pill_rect, _) =
                ui.allocate_exact_size(Vec2::new(pill_width, control_height), Sense::hover());
            ui.painter().rect(
                pill_rect,
                10.0,
                palette.field,
                Stroke::new(1.0_f32, palette.line),
                StrokeKind::Inside,
            );
            ui.painter().with_clip_rect(pill_rect).text(
                pill_rect.center(),
                Align2::CENTER_CENTER,
                &mode_label,
                pill_font,
                palette.weak,
            );
            ui.add_space(12.0);
        });
        ui.add_space((row_height - super::text_size(24.0)) / 2.0);
    }

    fn palette_rows(&mut self, ui: &mut egui::Ui, palette: &super::Pal) -> Option<usize> {
        let (rows, cursor, scroll_to, show_header, waiting) = {
            let state = self.palette.as_ref()?;
            let (scope, rest) = parse_prefix(&state.text);
            let files_mode = matches!(state.mode, Mode::Files);
            let waiting = files_mode
                && matches!(scope, SearchScope::Subfolders | SearchScope::ThisPc)
                && !rest.trim().is_empty();
            (
                state.rows.clone(),
                state.cursor,
                state.scroll_to,
                files_mode && rest.trim().is_empty(),
                waiting,
            )
        };

        let accent = self.accent;
        let row_height = super::text_size(40.0);

        if show_header && !rows.is_empty() {
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), super::text_size(26.0)),
                Sense::hover(),
            );
            ui.painter().text(
                Pos2::new(rect.left() + 18.0, rect.center().y),
                Align2::LEFT_CENTER,
                i18n::t("Son değiştirilenler"),
                super::ui_font(10.5),
                palette.weak,
            );
        }

        if rows.is_empty() {
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), super::text_size(88.0)),
                Sense::hover(),
            );
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                if waiting {
                    i18n::t("Taranıyor…")
                } else {
                    i18n::t("Sonuç yok")
                },
                super::ui_font(12.0),
                palette.weak,
            );
            return None;
        }

        let mut activated = None;
        for (index, row) in rows.iter().enumerate() {
            let selected = index == cursor;
            let (outer, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), row_height), Sense::hover());
            let rect = Rect::from_min_max(
                outer.min + Vec2::new(6.0, 1.0),
                outer.max - Vec2::new(6.0, 1.0),
            );
            let response = ui.interact(rect, ui.id().with(("palette_row", index)), Sense::click());

            let fill = if selected {
                if response.hovered() {
                    palette.sel_hover
                } else {
                    palette.sel
                }
            } else if response.hovered() {
                palette.hover
            } else {
                Color32::TRANSPARENT
            };
            if fill != Color32::TRANSPARENT {
                ui.painter().rect_filled(rect, 5.0, fill);
            }

            let icon_rect = Rect::from_min_size(
                Pos2::new(rect.left() + 10.0, rect.center().y - 9.0),
                Vec2::splat(18.0),
            );
            match &row.kind {
                RowKind::Entry(entry) => {
                    let px = super::icon_px(ui.ctx(), 18.0);
                    let key =
                        super::icon_key(&entry.path, entry.is_dir, &entry.ext, px, entry.plain_dir);
                    match self.images.get(&key, &entry.path, px, true) {
                        Img::Ready(texture) => {
                            ui.painter().image(
                                texture.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        _ => super::paint_fallback(ui.painter(), icon_rect, entry.is_dir),
                    }
                }
                RowKind::Command(_) => paint_chevron_right(ui.painter(), icon_rect, palette.weak),
                RowKind::ShowAll => {
                    paint_search_icon(ui.painter(), icon_rect.shrink(2.0), palette.weak)
                }
            }

            let detail_font = super::ui_font(10.5);
            let meta_width = if row.meta.is_empty() {
                0.0
            } else {
                measure_text(ui, &row.meta, &detail_font) + 14.0
            };
            let reason_width = if !row.enabled && !row.reason.is_empty() {
                measure_text(ui, i18n::t(row.reason), &detail_font) + 14.0
            } else {
                0.0
            };
            let right_reserve = meta_width.max(reason_width) + 8.0;
            let label_x = rect.left() + 40.0;
            let label_top = if row.detail.is_empty() {
                rect.center().y - super::text_size(16.0) / 2.0
            } else {
                rect.top() + 4.0
            };
            let label_clip = Rect::from_min_max(
                Pos2::new(label_x, label_top),
                Pos2::new(rect.right() - right_reserve, rect.bottom() - 2.0),
            );

            let base_color = if row.enabled {
                palette.text
            } else {
                palette.disabled
            };
            let job = label_job(row, base_color, accent);
            let galley = ui.fonts(|fonts| fonts.layout_job(job));
            ui.painter().with_clip_rect(label_clip).galley(
                Pos2::new(label_x, label_top),
                galley,
                base_color,
            );

            if !row.detail.is_empty() {
                ui.painter().with_clip_rect(rect).text(
                    Pos2::new(label_x, rect.top() + 22.0),
                    Align2::LEFT_TOP,
                    &row.detail,
                    detail_font.clone(),
                    palette.weak,
                );
            }

            if !row.meta.is_empty() {
                ui.painter().with_clip_rect(rect).text(
                    Pos2::new(rect.right() - 8.0, label_top),
                    Align2::RIGHT_TOP,
                    &row.meta,
                    detail_font.clone(),
                    palette.weak,
                );
            }
            if !row.enabled && !row.reason.is_empty() {
                ui.painter().with_clip_rect(rect).text(
                    Pos2::new(rect.right() - 8.0, rect.top() + 22.0),
                    Align2::RIGHT_TOP,
                    i18n::t(row.reason),
                    detail_font,
                    palette.disabled,
                );
            }

            if response.clicked() {
                if let Some(state) = self.palette.as_mut() {
                    state.cursor = index;
                    state.scroll_to = None;
                }
            }
            if response.double_clicked() && row.enabled {
                activated = Some(index);
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if scroll_to == Some(index) {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        }

        if scroll_to.is_some() {
            if let Some(state) = self.palette.as_mut() {
                state.scroll_to = None;
            }
        }
        activated
    }

    fn palette_footer(&mut self, ui: &mut egui::Ui, palette: &super::Pal) {
        let (hint, status) = {
            let Some(state) = self.palette.as_ref() else {
                return;
            };
            let hint = i18n::t("↑↓ taşı   ↵ aç   → komutlar   ← geri   Esc kapat");
            let status = if let Some(scan) = &state.scan {
                let scanned = scan.scanned.to_string();
                let found = scan.found_total.to_string();
                let counts = i18n::tf("{} klasör · {} sonuç", &[&scanned, &found]);
                if scan.done {
                    counts
                } else {
                    format!("{} {}", i18n::t("Taranıyor…"), counts)
                }
            } else if matches!(state.mode, Mode::Files) {
                format!(
                    "; {}    ;; {}",
                    i18n::t("Alt klasörler"),
                    i18n::t("Bu bilgisayar")
                )
            } else {
                String::new()
            };
            (hint, status)
        };

        let height = super::text_size(28.0);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let font = super::ui_font(10.0);

        ui.painter().with_clip_rect(rect).text(
            Pos2::new(rect.left() + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            hint,
            font.clone(),
            palette.weak,
        );

        if !status.is_empty() {
            let status_width = measure_text(ui, &status, &font);
            let hint_width = measure_text(ui, hint, &font);
            let room = rect.width() - 32.0 - hint_width - 16.0;
            if status_width <= room {
                ui.painter().with_clip_rect(rect).text(
                    Pos2::new(rect.right() - 16.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    &status,
                    font,
                    palette.weak,
                );
            }
        }
    }
}

// ---------------------------------------------------------------- row helpers

impl Row {
    fn file(entry: &Entry, base: &Path, spans: Vec<(usize, usize)>) -> Row {
        Row {
            kind: RowKind::Entry(entry.clone()),
            label: entry.name.clone(),
            spans,
            detail: detail_for(entry, base),
            meta: meta_for(entry),
            enabled: true,
            reason: "",
        }
    }
}

/// Second line: the path relative to the open folder. It is empty when the item
/// is a direct child, because the name is already the label.
fn detail_for(entry: &Entry, base: &Path) -> String {
    // The second line is the containing folder. The item name is already the
    // label, so repeating it here would double every search hit.
    let Some(parent) = entry.path.parent() else {
        return String::new();
    };
    if parent == base {
        return String::new();
    }
    match parent.strip_prefix(base) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into_owned(),
        _ => parent.to_string_lossy().into_owned(),
    }
}

fn meta_for(entry: &Entry) -> String {
    if entry.time_label.is_empty() {
        if entry.is_dir {
            String::new()
        } else {
            fsutil::format_size(entry.size)
        }
    } else if entry.is_dir {
        entry.time_label.clone()
    } else {
        format!("{} · {}", fsutil::format_size(entry.size), entry.time_label)
    }
}

fn highlighted_entry(app: &App) -> Option<Entry> {
    let palette = app.palette.as_ref()?;
    match &palette.mode {
        Mode::Files => palette
            .rows
            .get(palette.cursor)
            .and_then(|row| match &row.kind {
                RowKind::Entry(entry) => Some(entry.clone()),
                _ => None,
            }),
        Mode::Actions(entry) => Some(entry.clone()),
    }
}

// ---------------------------------------------------------------- text helpers

fn label_job(row: &Row, base: Color32, accent: Color32) -> egui::text::LayoutJob {
    let font = super::ui_font(13.0);
    let mut job = egui::text::LayoutJob::default();
    let mut spans = row.spans.clone();
    spans.retain(|(start, end)| {
        *start < *end
            && *end <= row.label.len()
            && row.label.is_char_boundary(*start)
            && row.label.is_char_boundary(*end)
    });
    spans.sort();
    let mut cursor = 0usize;
    for (start, end) in spans {
        if start < cursor {
            continue;
        }
        if start > cursor {
            job.append(
                &row.label[cursor..start],
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color: base,
                    ..Default::default()
                },
            );
        }
        job.append(
            &row.label[start..end],
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                color: accent,
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
                font_id: font,
                color: base,
                ..Default::default()
            },
        );
    }
    job
}

fn measure_text(ui: &egui::Ui, text: &str, font: &FontId) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

fn elide_text(ui: &egui::Ui, text: &str, font: &FontId, max_width: f32) -> String {
    if measure_text(ui, text, font) <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let ellipsis_width = measure_text(ui, ellipsis, font);
    let mut out = String::new();
    for character in text.chars() {
        let mut probe = out.clone();
        probe.push(character);
        if measure_text(ui, &probe, font) + ellipsis_width > max_width {
            break;
        }
        out.push(character);
    }
    out.push('…');
    out
}

/// Keeps the text edit caret at the start (0) or at the end (`usize::MAX`).
fn set_caret(ctx: &egui::Context, position: usize) {
    let Some(mut state) = egui::text_edit::TextEditState::load(ctx, input_id()) else {
        return;
    };
    let index = if position == usize::MAX {
        state
            .cursor
            .char_range()
            .map(|range| range.primary.index.max(range.secondary.index))
            .unwrap_or(0)
    } else {
        position
    };
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(index),
        )));
    state.store(ctx, input_id());
}

fn caret_at_end(ctx: &egui::Context, id: egui::Id, text: &str) -> bool {
    let Some(state) = egui::text_edit::TextEditState::load(ctx, id) else {
        return true;
    };
    let Some(range) = state.cursor.char_range() else {
        return true;
    };
    range.primary.index >= text.chars().count()
}

fn caret_at_start(ctx: &egui::Context, id: egui::Id) -> bool {
    let Some(state) = egui::text_edit::TextEditState::load(ctx, id) else {
        return true;
    };
    state
        .cursor
        .char_range()
        .map(|range| range.primary.index == 0 && range.secondary.index == 0)
        .unwrap_or(true)
}

// ---------------------------------------------------------------- paint helpers

fn palette_separator(ui: &mut egui::Ui, palette: &super::Pal) {
    let y = ui.cursor().top();
    ui.painter().hline(
        ui.max_rect().x_range(),
        y,
        Stroke::new(1.0_f32, palette.line),
    );
    ui.add_space(1.0);
}

fn paint_search_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center() + Vec2::new(-1.0, -1.0);
    let radius = rect.width() * 0.30;
    let stroke = Stroke::new(1.4_f32, color);
    painter.circle_stroke(center, radius, stroke);
    let start = center + Vec2::splat(radius * 0.72);
    let end = rect.right_bottom() - Vec2::splat(1.0);
    painter.line_segment([start, end], stroke);
}

fn paint_chevron_right(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let stroke = Stroke::new(1.5_f32, color);
    painter.line_segment(
        [
            Pos2::new(center.x - 2.5, center.y - 4.5),
            Pos2::new(center.x + 2.0, center.y),
        ],
        stroke,
    );
    painter.line_segment(
        [
            Pos2::new(center.x + 2.0, center.y),
            Pos2::new(center.x - 2.5, center.y + 4.5),
        ],
        stroke,
    );
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn entry(path: &str, is_dir: bool) -> Entry {
        let path = PathBuf::from(path);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Entry {
            name_lower: name.to_lowercase(),
            name,
            path,
            is_dir,
            size: 0,
            modified: Some(SystemTime::now()),
            created: None,
            hidden: false,
            ext: String::new(),
            plain_dir: is_dir,
            time_label: String::new(),
        }
    }

    #[test]
    fn prefix_cases() {
        assert_eq!(parse_prefix("rapor"), (SearchScope::Folder, "rapor"));
        assert_eq!(parse_prefix("; rapor"), (SearchScope::Subfolders, "rapor"));
        assert_eq!(parse_prefix(";"), (SearchScope::Subfolders, ""));
        assert_eq!(parse_prefix(";; rapor"), (SearchScope::ThisPc, "rapor"));
        assert_eq!(parse_prefix(";;"), (SearchScope::ThisPc, ""));
        // A file name that starts with a semicolon is not a scope prefix.
        assert_eq!(
            parse_prefix(";important.txt"),
            (SearchScope::Folder, ";important.txt")
        );
    }

    #[test]
    fn scope_cycle_is_reversible() {
        let mut scope = SearchScope::Folder;
        for _ in 0..3 {
            scope = next_scope(scope);
        }
        assert_eq!(scope, SearchScope::Folder);
        assert_eq!(
            next_scope(prev_scope(SearchScope::ThisPc)),
            SearchScope::ThisPc
        );
    }

    #[test]
    fn score_prefers_prefix_and_marks_it() {
        let terms = vec!["rep".to_string()];
        let (value, spans) = score("report.md", &terms).unwrap();
        assert!(value >= 800);
        assert_eq!(spans, vec![(0, 3)]);
        assert!(score("unrelated.txt", &terms).is_none());
    }

    #[test]
    fn score_merges_contiguous_subsequence_hits() {
        let terms = vec!["rpt".to_string()];
        let (value, spans) = score("report", &terms).unwrap();
        assert_eq!(value, 200);
        assert_eq!(spans, vec![(0, 1), (2, 3), (5, 6)]);
    }

    #[test]
    fn details_show_the_containing_folder_only() {
        let base = Path::new("C:/root");
        let direct = entry("C:/root/a.txt", false);
        assert_eq!(detail_for(&direct, base), "");
        let nested = entry("C:/root/sub/a.txt", false);
        assert_eq!(detail_for(&nested, base), "sub");
        // A hit from a wider scan shows its full parent folder.
        let outside = entry("C:/other/a.txt", false);
        assert_eq!(
            detail_for(&outside, base),
            outside.path.parent().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn command_list_matches_item_kind() {
        let file = entry("C:/root/a.txt", false);
        let commands = item_commands(&file);
        assert!(commands.contains(&ItemCmd::OpenWith));
        assert!(!commands.contains(&ItemCmd::OpenNewTab));

        let folder = entry("C:/root/sub", true);
        let commands = item_commands(&folder);
        assert!(commands.contains(&ItemCmd::OpenNewTab));
        assert!(!commands.contains(&ItemCmd::OpenWith));
    }

    #[test]
    fn every_command_has_a_label() {
        let folder = entry("C:/root/sub", true);
        for command in item_commands(&folder) {
            assert!(!cmd_label(command).is_empty());
        }
    }
}
