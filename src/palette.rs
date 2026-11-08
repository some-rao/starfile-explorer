//! Ctrl+K command palette.
//!
//! This is a child module of `app` (declared with `#[path]`), so it can read the
//! application's private state without widening field visibility.
//!
//! The palette has two levels:
//!
//! * `Files` - the entry-oriented level. A leading symbol picks what it lists:
//!   nothing (or `;` / `;;`) searches entries in the current folder / its
//!   subfolders / this PC, `>` lists application commands, `/` jumps to a path
//!   and `@` lists recent folders. The prefix lives in the query text, so the
//!   mode can never drift away from what the user typed.
//! * `Actions` - the commands that apply to one highlighted entry. `→` enters
//!   this level, `←` (or backspace on an empty filter) returns to the results.
//!
//! Drawing uses the same theme palette, font family and pixel-rounded sizes as
//! the rest of the application (`pal`, `ui_font`, `text_size`), so the overlay
//! looks like another surface of the app rather than a separate dialog.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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
    /// An application-level command, listed under the `>` prefix.
    App(AppCmd),
    /// A path the palette can navigate to (`/` path jump and `@` recent folders).
    Jump(PathBuf),
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

/// An application command listed by the `>` prefix. Every variant maps to an
/// existing [`Action`], so the palette adds no new behaviour of its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppCmd {
    NewTab,
    NewWindow,
    CloseTab,
    NewFolder,
    NewTextDoc,
    Undo,
    Refresh,
    Up,
    SelectAll,
    SelectNone,
    InvertSelection,
    Copy,
    Cut,
    Paste,
    /// View level index into `VIEW_LEVELS`.
    View(usize),
    Sort(fsutil::SortKey),
    ToggleHidden,
    ToggleExt,
    ToggleSplit,
    ToggleGit,
    ToggleHighlight,
    GoHome,
    GoThisPc,
    GoGallery,
    /// Index into `App::known`.
    Known(usize),
    Terminal,
    Properties,
    CopyPath,
    CalcSize,
    Peek,
    /// Akrilik yerine bir arka plan resmi seç
    PickBackground,
    /// Arka plan resmini kaldırıp akriliğe dön
    ClearBackground,
    /// Seçili öğeleri kalıpla topluca yeniden adlandır
    BatchRename,
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

/// What the palette lists, derived from the query's leading symbol. Keeping it
/// in the text means the mode can never drift away from what the user typed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prefix {
    /// File search in a scope. `;` widens it to subfolders and `;;` to this PC.
    Files(SearchScope),
    /// `>` application commands.
    Commands,
    /// `/` path jump.
    Path,
    /// `@` recent folders.
    Recents,
}

/// Splits the leading prefix from the free-text query. `;` / `;;` are only a
/// prefix when alone or followed by a space, so a file named `;notes.txt` can
/// still be searched; `>` / `/` / `@` are immediate prefixes.
pub fn parse_prefix(text: &str) -> (Prefix, &str) {
    if let Some(rest) = text.strip_prefix(";;") {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return (Prefix::Files(SearchScope::ThisPc), rest.trim_start());
        }
    }
    if let Some(rest) = text.strip_prefix(';') {
        if rest.is_empty() || rest.starts_with(char::is_whitespace) {
            return (Prefix::Files(SearchScope::Subfolders), rest.trim_start());
        }
    }
    if let Some(rest) = text.strip_prefix('>') {
        return (Prefix::Commands, rest.trim_start());
    }
    if let Some(rest) = text.strip_prefix('/') {
        return (Prefix::Path, rest.trim_start());
    }
    if let Some(rest) = text.strip_prefix('@') {
        return (Prefix::Recents, rest.trim_start());
    }
    (Prefix::Files(SearchScope::Folder), text)
}

/// The pill that names what the palette currently lists.
fn prefix_label(prefix: Prefix) -> &'static str {
    match prefix {
        Prefix::Files(SearchScope::Folder) => "Bu klasör",
        Prefix::Files(SearchScope::Subfolders) => "Alt klasörler",
        Prefix::Files(SearchScope::ThisPc) => "Bu bilgisayar",
        Prefix::Commands => "Komutlar",
        Prefix::Path => "Yol",
        Prefix::Recents => "Son kullanılanlar",
    }
}

/// Placeholder shown inside the empty input for the current prefix.
fn prefix_hint(prefix: Prefix) -> &'static str {
    match prefix {
        Prefix::Files(_) => "Dosya veya klasör ara — komutlar için > yaz",
        Prefix::Commands => "Komut ara",
        Prefix::Path => "Yol yaz",
        Prefix::Recents => "Son klasör ara",
    }
}

/// Footer key legend for the current prefix.
fn prefix_footer(prefix: Prefix) -> &'static str {
    match prefix {
        Prefix::Files(_) => "↑↓ taşı   ↵ aç   → komutlar   ← geri   Esc kapat",
        Prefix::Commands => "↑↓ taşı   ↵ çalıştır   Esc kapat",
        Prefix::Path => "↑↓ taşı   ↵ git   Esc kapat",
        Prefix::Recents => "↑↓ taşı   ↵ aç   Esc kapat",
    }
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

// ---------------------------------------------------------------- app commands

/// The `>` command list, grouped the way Explorer's menus are.
pub fn app_commands(known: usize) -> Vec<AppCmd> {
    let mut commands = vec![
        AppCmd::NewTab,
        AppCmd::NewWindow,
        AppCmd::CloseTab,
        AppCmd::NewFolder,
        AppCmd::NewTextDoc,
        AppCmd::Undo,
        AppCmd::Refresh,
        AppCmd::Up,
        AppCmd::SelectAll,
        AppCmd::SelectNone,
        AppCmd::InvertSelection,
        AppCmd::Copy,
        AppCmd::Cut,
        AppCmd::Paste,
    ];
    for level in 0..VIEW_LEVELS.len() {
        commands.push(AppCmd::View(level));
    }
    for key in [
        fsutil::SortKey::Name,
        fsutil::SortKey::Modified,
        fsutil::SortKey::Type,
        fsutil::SortKey::Size,
    ] {
        commands.push(AppCmd::Sort(key));
    }
    commands.extend([
        AppCmd::ToggleHidden,
        AppCmd::ToggleExt,
        AppCmd::ToggleSplit,
        AppCmd::ToggleGit,
        AppCmd::ToggleHighlight,
        AppCmd::PickBackground,
        AppCmd::ClearBackground,
        AppCmd::GoHome,
        AppCmd::GoThisPc,
        AppCmd::GoGallery,
    ]);
    for index in 0..known {
        commands.push(AppCmd::Known(index));
    }
    commands.extend([
        AppCmd::Terminal,
        AppCmd::Properties,
        AppCmd::CopyPath,
        AppCmd::CalcSize,
        AppCmd::BatchRename,
        AppCmd::Peek,
    ]);
    commands
}

fn sort_label(key: fsutil::SortKey) -> &'static str {
    match key {
        fsutil::SortKey::Name => "Ad",
        fsutil::SortKey::Modified => "Değiştirme tarihi",
        fsutil::SortKey::Type => "Tür",
        fsutil::SortKey::Size => "Boyut",
    }
}

fn app_cmd_label(cmd: AppCmd, app: &App) -> String {
    match cmd {
        AppCmd::NewTab => i18n::t("Yeni sekme").into(),
        AppCmd::NewWindow => i18n::t("Yeni pencere").into(),
        AppCmd::CloseTab => i18n::t("Sekmeyi kapat").into(),
        AppCmd::NewFolder => i18n::t("Yeni klasör").into(),
        AppCmd::NewTextDoc => i18n::t("Metin Belgesi").into(),
        AppCmd::Undo => i18n::t("Geri al").into(),
        AppCmd::Refresh => i18n::t("Yenile").into(),
        AppCmd::Up => i18n::t("Üst klasör").into(),
        AppCmd::SelectAll => i18n::t("Tümünü seç").into(),
        AppCmd::SelectNone => i18n::t("Hiçbirini seçme").into(),
        AppCmd::InvertSelection => i18n::t("Seçimi tersine çevir").into(),
        AppCmd::Copy => i18n::t("Kopyala").into(),
        AppCmd::Cut => i18n::t("Kes").into(),
        AppCmd::Paste => i18n::t("Yapıştır").into(),
        AppCmd::View(level) => {
            let level = level.min(VIEW_LEVELS.len() - 1);
            i18n::t(VIEW_LEVELS[level].1).into()
        }
        AppCmd::Sort(key) => i18n::t(sort_label(key)).into(),
        AppCmd::ToggleHidden => i18n::t("Gizli öğeler").into(),
        AppCmd::ToggleExt => i18n::t("Dosya adı uzantıları").into(),
        AppCmd::ToggleSplit => i18n::t("Bölünmüş görünüm").into(),
        AppCmd::ToggleGit => i18n::t("Git sütununu göster").into(),
        AppCmd::ToggleHighlight => i18n::t("Kod vurgulama").into(),
        AppCmd::PickBackground => i18n::t("Arka plan resmi seç…").into(),
        AppCmd::ClearBackground => i18n::t("Arka planı kaldır").into(),
        AppCmd::GoHome => i18n::t("Giriş").into(),
        AppCmd::GoThisPc => i18n::t("Bu bilgisayar").into(),
        AppCmd::GoGallery => i18n::t("Galeri").into(),
        AppCmd::Known(index) => app
            .known
            .get(index)
            .map(|known| known.1.to_string())
            .unwrap_or_default(),
        AppCmd::Terminal => i18n::t("Terminalde aç").into(),
        AppCmd::Properties => i18n::t("Özellikler").into(),
        AppCmd::CopyPath => i18n::t("Yolu kopyala").into(),
        AppCmd::CalcSize => i18n::t("Klasör boyutlarını hesapla").into(),
        AppCmd::BatchRename => i18n::t("Toplu yeniden adlandır…").into(),
        AppCmd::Peek => i18n::t("Hızlı Önizleme").into(),
    }
}

/// `Some(reason)` when the application command cannot run right now.
fn app_cmd_disabled(cmd: AppCmd, app: &App) -> Option<&'static str> {
    let virtual_location = crate::tab::is_virtual(&app.tabs[app.active].path);
    let here_reason = "Burada kullanılamaz";
    match cmd {
        AppCmd::Paste => {
            if virtual_location {
                Some(here_reason)
            } else if app.clipboard.is_none() {
                Some("Pano boş")
            } else {
                None
            }
        }
        AppCmd::Undo => {
            if app.undo.is_empty() {
                Some("Geri alınacak bir işlem yok")
            } else {
                None
            }
        }
        AppCmd::NewFolder
        | AppCmd::NewTextDoc
        | AppCmd::Terminal
        | AppCmd::Properties
        | AppCmd::CopyPath
        | AppCmd::CalcSize
        | AppCmd::BatchRename => {
            if virtual_location {
                Some(here_reason)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// The action an application command maps to. Every variant reuses an existing
/// [`Action`]; the palette only decides which one and for which folder.
fn app_cmd_action(cmd: AppCmd, app: &App) -> Action {
    let current = app.tabs[app.active].path.clone();
    match cmd {
        AppCmd::NewTab => Action::NewTab(PathBuf::new()),
        AppCmd::NewWindow => Action::NewWindow,
        AppCmd::CloseTab => Action::CloseTab(app.active),
        AppCmd::NewFolder => Action::NewFolder,
        AppCmd::NewTextDoc => Action::New(1),
        AppCmd::Undo => Action::Undo,
        AppCmd::Refresh => Action::Refresh,
        AppCmd::Up => Action::Up,
        AppCmd::SelectAll => Action::SelectAll,
        AppCmd::SelectNone => Action::SelectNone,
        AppCmd::InvertSelection => Action::InvertSelection,
        AppCmd::Copy => Action::Copy,
        AppCmd::Cut => Action::Cut,
        AppCmd::Paste => Action::Paste,
        AppCmd::View(level) => Action::SetView(level),
        AppCmd::Sort(key) => Action::Sort(key),
        AppCmd::ToggleHidden => Action::ToggleHidden,
        AppCmd::ToggleExt => Action::ToggleExt,
        AppCmd::ToggleSplit => Action::ToggleSplit,
        AppCmd::ToggleGit => Action::ToggleGitColumn,
        AppCmd::ToggleHighlight => Action::ToggleHighlight,
        AppCmd::PickBackground => Action::PickBackground,
        AppCmd::ClearBackground => Action::ClearBackground,
        AppCmd::GoHome => Action::Navigate(PathBuf::new()),
        AppCmd::GoThisPc => Action::Navigate(PathBuf::from(THIS_PC)),
        AppCmd::GoGallery => Action::Navigate(PathBuf::from(crate::tab::GALLERY)),
        AppCmd::Known(index) => Action::Navigate(
            app.known
                .get(index)
                .map(|known| known.2.clone())
                .unwrap_or_default(),
        ),
        AppCmd::Terminal => Action::Terminal(current),
        AppCmd::Properties => Action::Properties(current),
        AppCmd::CopyPath => Action::CopyPath,
        AppCmd::CalcSize => Action::CalcSize,
        AppCmd::BatchRename => Action::BatchRename,
        AppCmd::Peek => Action::Peek,
    }
}

// ---------------------------------------------------------------- path jump

/// Splits a filter into lowercased whitespace-separated terms.
fn query_terms(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

/// Label for a path that has no `Entry` behind it (drives, bookmarks, recents).
fn jump_label(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return i18n::t("Giriş").into();
    }
    if path.as_os_str() == THIS_PC {
        return i18n::t("Bu bilgisayar").into();
    }
    if path.as_os_str() == crate::tab::GALLERY {
        return i18n::t("Galeri").into();
    }
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            path.to_string_lossy()
                .trim_end_matches(['\\', '/'])
                .to_string()
        })
}

/// Expands a typed path the way Explorer's address bar would: `~` is the home
/// directory, `%VAR%` is an environment variable and a relative path is taken
/// from the open folder. No I/O: whether it exists is decided by the navigation
/// action, which reports a missing path with a toast.
fn resolve_path(input: &str, current: &Path) -> PathBuf {
    let trimmed = input.trim().trim_matches('"').trim();
    if trimmed.is_empty() {
        return current.to_path_buf();
    }
    if let Some(home) = dirs::home_dir() {
        if trimmed == "~" {
            return home;
        }
        if let Some(rest) = trimmed
            .strip_prefix("~/")
            .or_else(|| trimmed.strip_prefix("~\\"))
        {
            return if rest.is_empty() {
                home
            } else {
                home.join(rest)
            };
        }
    }
    let expanded = expand_env(trimmed);
    let path = PathBuf::from(&expanded);
    if path.is_absolute() {
        return path;
    }
    // `C:` on its own is drive-relative; a bare drive letter means its root.
    let bytes = expanded.as_bytes();
    if expanded.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return PathBuf::from(format!("{expanded}/"));
    }
    current.join(path)
}

/// Replaces `%NAME%` with the environment variable, leaving unknown or lone
/// `%` characters untouched.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
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
        // Arama sürerken palet sorgusu sonuçları canlı daraltır; eskiden bunu
        // araç çubuğundaki arama kutusu yapardı.
        let query = match self.palette.as_ref() {
            Some(palette) => match parse_prefix(&palette.text) {
                (Prefix::Files(_), rest) => rest.trim().to_string(),
                _ => String::new(),
            },
            None => String::new(),
        };
        if self.tabs[self.active].search.is_some() {
            let tab = &mut self.tabs[self.active];
            if tab.filter != query {
                tab.filter = query;
                tab.view_dirty = true;
            }
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

    fn palette_cancel_scan(&mut self) {
        let Some(palette) = self.palette.as_mut() else {
            return;
        };
        if let Some(scan) = &palette.scan {
            scan.handle.cancel.store(true, Ordering::Relaxed);
        }
        if palette.scan.take().is_some() {
            palette.dirty = true;
        }
    }

    fn palette_start_scan(&mut self, ctx: &egui::Context) {
        let (scope, rest) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            match palette.mode {
                Mode::Files => match parse_prefix(&palette.text) {
                    (Prefix::Files(scope), rest) => (Some(scope), rest.trim().to_string()),
                    _ => (None, String::new()),
                },
                Mode::Actions(_) => (None, String::new()),
            }
        };

        let scope = match scope {
            Some(SearchScope::Subfolders) => SearchScope::Subfolders,
            Some(SearchScope::ThisPc) => SearchScope::ThisPc,
            _ => {
                self.palette_cancel_scan();
                return;
            }
        };

        if rest.is_empty() {
            self.palette_cancel_scan();
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
        let (files_mode, text, empty, drillable) = {
            let Some(palette) = self.palette.as_ref() else {
                return;
            };
            (
                matches!(palette.mode, Mode::Files),
                palette.text.clone(),
                palette.text.is_empty(),
                // `→` only drills when there is an entry row to drill into, so
                // the key is left to the text edit in the `>` / `/` / `@` lists.
                palette
                    .rows
                    .get(palette.cursor)
                    .map(|row| matches!(row.kind, RowKind::Entry(_)))
                    .unwrap_or(false),
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
                    // already at the end of the query and an entry is selected;
                    // otherwise the text edit gets the key and moves the caret.
                    if drillable && caret_end && input.consume_key(Modifiers::NONE, Key::ArrowRight)
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
            // Scope cycling only applies to the file-search prefix; the `>` / `/`
            // / `@` modes have no scope to cycle.
            match parse_prefix(&palette.text) {
                (Prefix::Files(scope), rest) => (scope, rest.to_string()),
                _ => return,
            }
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
                RowKind::Jump(path) => {
                    self.close_palette();
                    actions.push(Action::Navigate(path));
                }
                RowKind::App(cmd) => {
                    let action = app_cmd_action(cmd, self);
                    self.close_palette();
                    actions.push(action);
                }
                RowKind::ShowAll => {
                    let (prefix, filter) = {
                        let Some(palette) = self.palette.as_ref() else {
                            return;
                        };
                        let (prefix, rest) = parse_prefix(&palette.text);
                        (prefix, rest.trim().to_string())
                    };
                    // Kapsam palet ön ekinden gelir: `;` alt klasörler, `;;` bu bilgisayar.
                    let scope = match prefix {
                        Prefix::Files(SearchScope::Subfolders) => SearchScope::Subfolders,
                        Prefix::Files(SearchScope::ThisPc) => SearchScope::ThisPc,
                        _ => SearchScope::Folder,
                    };
                    let tab = &mut self.tabs[self.active];
                    tab.filter = filter;
                    tab.view_dirty = true;
                    self.close_palette();
                    actions.push(Action::SetSearchScope(scope));
                    actions.push(Action::StartSearch);
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

        let rows: Vec<Row> = match mode {
            Mode::Files => match parse_prefix(&text) {
                (Prefix::Files(scope), rest) => {
                    self.collect_file_rows(scope, rest.trim(), &base, show_hidden)
                }
                (Prefix::Commands, rest) => self.collect_command_rows(rest),
                (Prefix::Path, rest) => self.collect_path_rows(rest, &base, show_hidden),
                (Prefix::Recents, rest) => self.collect_recent_rows(rest),
            },
            Mode::Actions(target) => {
                let mut rows = Vec::new();
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
                rows
            }
        };

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

    /// Entries from the open folder (or the running scan for a wider scope).
    /// With an empty filter the six most recently modified entries are listed.
    fn collect_file_rows(
        &self,
        scope: SearchScope,
        rest: &str,
        base: &Path,
        show_hidden: bool,
    ) -> Vec<Row> {
        let mut rows: Vec<Row> = Vec::new();
        if rest.is_empty() {
            let mut recent: Vec<&Entry> = self.tabs[self.active]
                .entries
                .iter()
                .filter(|entry| show_hidden || !entry.hidden)
                .collect();
            recent.sort_by_key(|entry| std::cmp::Reverse(entry.modified));
            for entry in recent.into_iter().take(RECENT_ROWS) {
                rows.push(Row::file(entry, base, Vec::new()));
            }
            return rows;
        }

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

        // Glob terms are already enforced by `Query`; scoring them would fail
        // because `*` and `?` are not in the name.
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
            let (mut value, spans) = score(&entry.name_lower, &terms).unwrap_or((0, Vec::new()));
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
            rows.push(Row::file(&entry, base, spans));
        }
        // "Tüm sonuçları göster" satırı: sonuçları sekmenin tamamında gösterir
        // (kapsam palet ön ekinden gelir).
        if total > MAX_ROWS {
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
        rows
    }

    /// Application commands, fuzzy-filtered by the text after `>`.
    fn collect_command_rows(&self, rest: &str) -> Vec<Row> {
        let terms = query_terms(rest);
        let mut rows = Vec::new();
        for command in app_commands(self.known.len()) {
            let label = app_cmd_label(command, self);
            let Some((_, spans)) = score(&label.to_lowercase(), &terms) else {
                continue;
            };
            let reason = app_cmd_disabled(command, self);
            rows.push(Row::app(command, label, spans, reason));
        }
        rows
    }

    /// Path-jump targets for the `/` prefix: the typed path itself first, then
    /// matching drives, known folders, subfolders, bookmarks and recents.
    fn collect_path_rows(&self, rest: &str, current: &Path, show_hidden: bool) -> Vec<Row> {
        let terms = query_terms(rest);
        let mut rows: Vec<Row> = Vec::new();

        // What the user is spelling is always the first row, even before it
        // matches anything else in the list.
        let typed = if rest.trim().is_empty() {
            None
        } else {
            Some(resolve_path(rest, current))
        };
        if let Some(path) = &typed {
            rows.push(Row::jump(
                path.clone(),
                jump_label(path),
                path.to_string_lossy().into_owned(),
                Vec::new(),
            ));
        }

        let mut seen: HashSet<PathBuf> = HashSet::new();
        if let Some(path) = &typed {
            seen.insert(path.clone());
        }
        let mut candidates: Vec<(PathBuf, String, String)> = Vec::new();

        let drives: Vec<(PathBuf, String)> = self
            .drives
            .lock()
            .unwrap()
            .iter()
            .map(|drive| (drive.root.clone(), drive.label.clone()))
            .collect();
        for (root, label) in drives {
            push_unique(
                &mut seen,
                &mut candidates,
                root.clone(),
                jump_label(&root),
                label,
            );
        }
        for (_, name, path) in &self.known {
            push_unique(
                &mut seen,
                &mut candidates,
                path.clone(),
                name.to_string(),
                path.to_string_lossy().into_owned(),
            );
        }
        // Subfolders only help while a filter is being typed; listing all of
        // them for an empty query would drown the quick places above.
        if !rest.trim().is_empty() {
            for entry in self.tabs[self.active].entries.iter() {
                if !entry.is_dir || (!show_hidden && entry.hidden) {
                    continue;
                }
                push_unique(
                    &mut seen,
                    &mut candidates,
                    entry.path.clone(),
                    entry.name.clone(),
                    String::new(),
                );
            }
        }
        for path in self.bookmarks.iter().chain(self.recent.iter()) {
            push_unique(
                &mut seen,
                &mut candidates,
                path.clone(),
                jump_label(path),
                path.to_string_lossy().into_owned(),
            );
        }

        let mut matched: Vec<(i32, Row)> = Vec::new();
        for (path, label, detail) in candidates {
            let label_lower = label.to_lowercase();
            let path_lower = path.to_string_lossy().to_lowercase();
            let Some((value, spans)) = match_label(&label_lower, &path_lower, &terms) else {
                continue;
            };
            matched.push((value, Row::jump(path, label, detail, spans)));
        }
        matched.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| {
                fsutil::natural_cmp(&a.1.label.to_lowercase(), &b.1.label.to_lowercase())
            })
        });
        for (_, row) in matched
            .into_iter()
            .take(MAX_ROWS.saturating_sub(rows.len()))
        {
            rows.push(row);
        }
        rows.truncate(MAX_ROWS);
        rows
    }

    /// Recent folders (`@`), most recent first, filtered by text.
    fn collect_recent_rows(&self, rest: &str) -> Vec<Row> {
        let terms = query_terms(rest);
        let mut seen: HashSet<PathBuf> = HashSet::new();
        let mut candidates: Vec<(PathBuf, String, String)> = Vec::new();
        for path in self.recent.iter().chain(self.bookmarks.iter()) {
            push_unique(
                &mut seen,
                &mut candidates,
                path.clone(),
                jump_label(path),
                path.to_string_lossy().into_owned(),
            );
        }

        let mut matched: Vec<(i32, Row)> = Vec::new();
        for (path, label, detail) in candidates {
            let label_lower = label.to_lowercase();
            let path_lower = path.to_string_lossy().to_lowercase();
            let Some((value, spans)) = match_label(&label_lower, &path_lower, &terms) else {
                continue;
            };
            matched.push((value, Row::jump(path, label, detail, spans)));
        }
        // Stable sort: equal scores keep the recency order.
        matched.sort_by_key(|(value, _)| std::cmp::Reverse(*value));
        matched
            .into_iter()
            .take(MAX_ROWS)
            .map(|(_, row)| row)
            .collect()
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
                    let (prefix, _) = parse_prefix(&state.text);
                    (
                        i18n::t(prefix_label(prefix)).to_string(),
                        i18n::t(prefix_hint(prefix)).to_string(),
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
            let (prefix, rest) = parse_prefix(&state.text);
            let files_mode = matches!(state.mode, Mode::Files);
            let mut waiting = false;
            let mut show_header = false;
            if files_mode {
                match prefix {
                    Prefix::Files(scope) => {
                        waiting = matches!(scope, SearchScope::Subfolders | SearchScope::ThisPc)
                            && !rest.trim().is_empty();
                        show_header = scope == SearchScope::Folder && rest.trim().is_empty();
                    }
                    // The `>` / `/` / `@` lists have no "recently modified"
                    // heading; their pill already names them.
                    Prefix::Commands | Prefix::Path | Prefix::Recents => {}
                }
            }
            (
                state.rows.clone(),
                state.cursor,
                state.scroll_to,
                show_header,
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
                RowKind::Command(_) | RowKind::App(_) => {
                    paint_chevron_right(ui.painter(), icon_rect, palette.weak)
                }
                RowKind::Jump(path) => {
                    let px = super::icon_px(ui.ctx(), 18.0);
                    let key = super::icon_key(path, true, "", px, true);
                    match self.images.get(&key, path, px, true) {
                        Img::Ready(texture) => {
                            ui.painter().image(
                                texture.id(),
                                icon_rect,
                                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        _ => super::paint_fallback(ui.painter(), icon_rect, true),
                    }
                }
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
            let (prefix, _) = parse_prefix(&state.text);
            let files_mode = matches!(state.mode, Mode::Files);
            let hint = if files_mode {
                i18n::t(prefix_footer(prefix))
            } else {
                i18n::t(prefix_footer(Prefix::Commands))
            };
            let status = if let Some(scan) = &state.scan {
                let scanned = scan.scanned.to_string();
                let found = scan.found_total.to_string();
                let counts = i18n::tf("{} klasör · {} sonuç", &[&scanned, &found]);
                if scan.done {
                    counts
                } else {
                    format!("{} {}", i18n::t("Taranıyor…"), counts)
                }
            } else if files_mode && prefix == Prefix::Files(SearchScope::Folder) {
                format!(
                    "; {}    ;; {}    > {}    / {}    @ {}",
                    i18n::t("Alt klasörler"),
                    i18n::t("Bu bilgisayar"),
                    i18n::t("Explorer komutları"),
                    i18n::t("Yol"),
                    i18n::t("Son kullanılanlar"),
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

    /// A path row (path jump and recent folders): no entry metadata behind it.
    fn jump(path: PathBuf, label: String, detail: String, spans: Vec<(usize, usize)>) -> Row {
        Row {
            kind: RowKind::Jump(path),
            label,
            spans,
            detail,
            meta: String::new(),
            enabled: true,
            reason: "",
        }
    }

    /// An application command row.
    fn app(
        command: AppCmd,
        label: String,
        spans: Vec<(usize, usize)>,
        reason: Option<&'static str>,
    ) -> Row {
        Row {
            kind: RowKind::App(command),
            label,
            spans,
            detail: String::new(),
            meta: String::new(),
            enabled: reason.is_none(),
            reason: reason.unwrap_or(""),
        }
    }
}

/// Adds a path candidate unless it has already been seen.
fn push_unique(
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<(PathBuf, String, String)>,
    path: PathBuf,
    label: String,
    detail: String,
) {
    if seen.insert(path.clone()) {
        out.push((path, label, detail));
    }
}

/// Scores a path candidate against the filter. The label is matched first; when
/// only the full path matches, the row is kept but stays unhighlighted.
fn match_label(
    label_lower: &str,
    path_lower: &str,
    terms: &[String],
) -> Option<(i32, Vec<(usize, usize)>)> {
    if terms.is_empty() {
        return Some((0, Vec::new()));
    }
    if let Some((value, spans)) = score(label_lower, terms) {
        return Some((value, spans));
    }
    if terms.iter().all(|term| path_lower.contains(term)) {
        return Some((0, Vec::new()));
    }
    None
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
        use Prefix::Files;
        assert_eq!(parse_prefix("rapor"), (Files(SearchScope::Folder), "rapor"));
        assert_eq!(
            parse_prefix("; rapor"),
            (Files(SearchScope::Subfolders), "rapor")
        );
        assert_eq!(parse_prefix(";"), (Files(SearchScope::Subfolders), ""));
        assert_eq!(
            parse_prefix(";; rapor"),
            (Files(SearchScope::ThisPc), "rapor")
        );
        assert_eq!(parse_prefix(";;"), (Files(SearchScope::ThisPc), ""));
        // A file name that starts with a semicolon is not a scope prefix.
        assert_eq!(
            parse_prefix(";important.txt"),
            (Files(SearchScope::Folder), ";important.txt")
        );
    }

    #[test]
    fn symbol_prefixes_are_immediate() {
        assert_eq!(parse_prefix("> yeni"), (Prefix::Commands, "yeni"));
        assert_eq!(parse_prefix(">yeni"), (Prefix::Commands, "yeni"));
        assert_eq!(parse_prefix("/c/"), (Prefix::Path, "c/"));
        assert_eq!(parse_prefix("@"), (Prefix::Recents, ""));
        assert_eq!(parse_prefix("@ projeler"), (Prefix::Recents, "projeler"));
    }

    #[test]
    fn resolve_expands_home_and_environment() {
        let current = Path::new("C:/work");
        // Relative paths hang off the open folder.
        assert_eq!(resolve_path("sub", current), PathBuf::from("C:/work/sub"));
        // An absolute path is taken as-is.
        assert_eq!(
            resolve_path("C:/Windows", current),
            PathBuf::from("C:/Windows")
        );
        // Unknown variables and lone percents stay literal.
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("%NO_SUCH_VAR_XYZ%"), "%NO_SUCH_VAR_XYZ%");
        if let Some(home) = dirs::home_dir() {
            assert_eq!(resolve_path("~", current), home);
            assert_eq!(resolve_path("~/docs", current), home.join("docs"));
        }
    }

    #[test]
    fn app_commands_have_no_duplicates() {
        let commands = app_commands(4);
        for (index, command) in commands.iter().enumerate() {
            assert!(
                !commands[..index].contains(command),
                "{command:?} listed twice"
            );
        }
        assert!(commands.contains(&AppCmd::Peek));
        assert!(commands.contains(&AppCmd::Paste));
        assert!(commands.contains(&AppCmd::Known(3)));
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
