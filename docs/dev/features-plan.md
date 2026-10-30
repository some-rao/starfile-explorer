# Feature plan — six additions beyond Explorer (Q4 2026)

Status: **approved scope, implementation in progress**
Anchor note: line numbers were verified at `app.rs` = 7341 lines, `tab.rs` = 662, `fsutil.rs` = 1410.
The older `docs/dev/*.md` anchors (written at ~4526 lines) have drifted; re-verify before editing.

Out of scope (explicitly deferred): NTFS MFT/USN background index, elevated indexer service.

Build order (each phase is a separate, self-contained change that must pass
`cargo fmt --all`, `cargo clippy --all-targets`, `cargo test` on its own):

| Phase | Feature | Effort | Risk |
|---|---|---|---|
| 1 | Per-folder view memory | 1 d | low |
| 2 | Copy to… / Move to… | 1 d | low |
| 3 | Search scope toggle + filter chips | 2–3 d | medium |
| 4 | Git pill + Git column | 3 d | medium |
| 5 | Preview: syntax highlighting, Markdown, Quick Look | 6 d | medium |
| 6 | Split view (two panes) | 4 d | **high** (touches the App spine) |

Rationale for moving Copy to… ahead of the search work: it is independent, reuses
`start_transfer`, and gives an early, visible win. Split view stays last because it
rewrites `App.tabs` → `App.panes` and must not share a commit with feature work.

---

## Shared conventions (apply to every phase)

- **i18n**: UI text is Turkish and *is* the key. Every new key is added to
  `i18n.rs` `TABLE` **and** all seven of `src/i18n/{de,es,fr,pt,ru,zh,ja}.rs`
  (`i18n.rs:561` parity test). 8 edits per string.
- **Config**: `struct Config` (`app.rs:39-66`) is struct-level `#[serde(default)]`;
  new fields go in `Config`, `impl Default for Config`, `save_config`
  (`app.rs:1790-1822`) and `App::new` (`app.rs:1685-1704`). Missing fields in old
  config files silently fall back — no migration code needed.
- **All I/O off the UI thread**, results over `mpsc`, workers must call
  `ctx.request_repaint()`.
- **No bold**: emphasis uses size/colour only (`RichText::strong()` changes colour,
  not weight). New preview headings follow the same rule.
- **Platform twins**: a Windows-only function needs a same-named stub in
  `shell_unix.rs` / `win_unix.rs` (there is no trait; `main.rs:16-19` aliases the
  modules by path).
- **Docs**: update `AGENTS.md` (common changes + traps), `architecture.md`
  (§3 modules, §9.1 config, §10 contracts), `workflows.md` (the flow), and
  `low-level-design.md` (recipes) as each phase lands.

---

## Phase 1 — Per-folder view memory

### What the user gets
Downloads opens in Details sorted by Date modified (newest first); Music opens in
Large icons sorted by Name; each folder keeps its own layout. A View-menu toggle
(`Her klasörün görünümünü hatırla`, default **on**) controls the whole feature, and
`Klasör görünümünü sıfırla` forgets the current folder.

### Behaviour rules
- Remembered per folder: `view_level`, `sort`, `asc`, `grouped`, `show_hidden`,
  `show_ext`, and column widths.
- Applied on: tab switch, navigation (any source), and app start for the restored
  tabs.
- Written on: any change to those settings while a folder is open, and when
  navigating away (captures the final state).
- Global fallbacks stay exactly as they are today when a folder has no record.
- Key = normalized path, lowercased (`C:\users\x\downloads`). Virtual locations
  (`Home`, `This PC`, `Gallery`) are not remembered.

### Data model
```rust
// app.rs, next to Config
#[derive(Serialize, Deserialize, Clone, Default)]
struct FolderView {
    view_level: Option<usize>,
    sort: Option<SortKey>,
    asc: Option<bool>,
    grouped: Option<bool>,
    show_hidden: Option<bool>,
    show_ext: Option<bool>,
}

// Config
remember_views: bool,                       // default true
folder_views: HashMap<String, FolderView>,   // lowercased path -> view
```
`Option<T>` = "this folder never overrode the global value", so a folder that only
changed the sort does not freeze today's view level.

### Implementation
1. `app.rs` Config block (`39-91`): add the two fields + defaults
   (`remember_views: true`, `folder_views: HashMap::new()`), extend `save_config`
   (`1790-1822`), and read them in `App::new` (`1685-1704`).
2. `App` fields: `folder_views`, `remember_views`, `view_key: Option<String>`
   (the folder currently applied) and `git_column`-style flags later.
3. New methods on `App`:
   - `fn folder_key(path: &Path) -> Option<String>` — lowercased normalized path,
     `None` for virtual locations.
   - `fn apply_folder_view(&mut self, path: &Path)` — set the six globals from the
     map when `remember_views`, else leave them; then `mark_views_dirty()`.
   - `fn store_folder_view(&mut self, path: &Path)` — write the six globals into
     the map (only when `remember_views`).
4. Hook in `update` (`6942-7294`), right after the active tab poll/rebuild block
   (`7008-7019`): if `(active, path)` changed → `store_folder_view(old)` then
   `apply_folder_view(new)`. This covers every navigation source (sidebar, crumbs,
   address bar, back/forward, up, open-folder) without touching `Tab`.
5. Write-through: call `store_folder_view` in the `apply` arms that mutate the six
   settings — `Sort` (`2804-2811`), `SetView` (`2813-2814`), `ToggleHidden`
   (`2820`), `ToggleExt` (`2824`), and the `Grupla` submenu handler
   (`3743-3757`, which mutates `self.grouped` directly).
6. Column widths: `details_view` (`5346-5781`) already salts its `TableBuilder`
   id (`5416-5420`, `5432-5433`). Replace the constant `"details_cols_v3"` with
   `format!("details_cols_v3:{}", folder_key)` when `remember_views` is on (keep
   `"details_cols_boot"` for `cumulative_pass_nr() < 3`, it must stay global because
   frame 0 reports a bogus width). egui_extras then keeps one `TableState` per
   folder with no new code. When the toggle is off, keep the old global id so
   existing widths are not lost.
7. View menu (`build_view_menu`, `4770-4906`, "Göster" submenu `4813-4861`): add
   the checkbox row and `Klasör görünümünü sıfırla` (pushes
   `Action::ForgetFolderView`). `apply` removes the entry for the current path and
   re-applies defaults.
8. `prefetch_startup` (`206-239`) duplicates the sort/group logic for the first
   paint; apply the remembered view there too, otherwise the first frame flashes.

### New i18n keys
`Her klasörün görünümünü hatırla`, `Klasör görünümünü sıfırla`,
`Bu klasörün görünümü sıfırlandı`.

### Verification
Open two folders, set different view modes/sorts, restart, revisit both. Toggle the
option off → both follow the global setting again. Resize a column in folder A,
check folder B keeps its own width.

---

## Phase 2 — Copy to… / Move to…

### What the user gets
Right-click → `Şuraya kopyala…` / `Şuraya taşı…` (also in the toolbar **More** menu).
A native Windows folder picker opens at the current folder; the transfer runs as a
normal job with the standard progress card, conflict dialog and Ctrl+Z undo.

### Implementation
1. `shell.rs` (new, next to `drag_out` around `61-210`):
   ```rust
   pub fn pick_folder(owner: isize, start: Option<&Path>) -> Option<PathBuf>
   ```
   COM `IFileOpenDialog` + `FOS_PICKFOLDERS` + `SHCreateItemFromParsingName` for the
   start folder. All required `windows` crate features are already enabled
   (`Cargo.toml:39`: `Win32_UI_Shell`, `Win32_System_Com`, `Win32_System_Ole`), and
   `CoInitializeEx` is already used in `shell.rs:175` — no Cargo change needed.
   The dialog pumps its own message loop; set `self.inject_release = true` before
   the call and clear it after (same pattern as `app.rs:2730-2731`) so the mouse-up
   that opened the menu does not leak into the owner window.
2. `shell_unix.rs`: same name returning `None` (no GTK dependency). On Linux the
   command opens the in-app path modal instead (step 4).
3. `Action` (`283-358`): `CopyTo { paths: Vec<PathBuf>, dest: PathBuf }`,
   `MoveTo { paths, dest }`, `PickDest { paths, cut: bool }`.
   `apply` (`2459-2883`): `PickDest` calls `shell::pick_folder`;
   `CopyTo`/`MoveTo` call `self.start_transfer(paths, dest, cut)`
   (`app.rs:1984-2003` — it already checks conflicts and registers undo).
4. In-app fallback modal for Linux: `Dialog::PickDest { paths, cut, text }`
   in `dialogs` (`6625-6839`), modelled on the address-bar editor
   (`3347-3353`) — a text field with `subdirs_cached` autocomplete and two buttons.
5. Menu entries: `open_context_menu` → `MenuTarget::Items` (`4549-4640`) right
   after the quick Copy/Cut row (`4622-4639`), using `app_node(...)` (`5024-5040`)
   with `Action::PickDest { paths: sel_paths(), cut }` and `n.sep_before()` on the
   first. Command bar: **More** menu (`3785-3872`).
6. Guards: skip for virtual locations and when nothing is selected; `busy_here`
   handling is already in `apply` (`2461-2464`).

### New i18n keys
`Şuraya kopyala…`, `Şuraya taşı…`, `Hedef klasör`, `Klasör seç`.

### Verification
Copy 3 files into a deep folder, check the progress card, the conflict dialog when
the target has a name clash, and Ctrl+Z. Try on a network path and a read-only
target (error toast).

---

## Phase 3 — Search scope toggle + filter chips

### What the user gets
The search box gains a scope button: **Filter this folder** (instant, in memory),
**Including subfolders**, **This PC**. Typed filters become removable chips under
the search box: `ext:pdf`, `size:>10mb`, `modified:thisweek`, `kind:image`,
`-draft`. Chips work while browsing *and* on search results (filters narrowed
without restarting the search).

### Query language (v1)
| Token | Meaning |
|---|---|
| bare words | all must appear in the name (current `Pattern` behaviour) |
| `ext:pdf,docx` | extension list |
| `type:file` / `type:folder` | entry kind |
| `size:>10mb`, `size:<500kb`, `size:1mb..1gb` | size comparisons |
| `modified:today\|week\|month\|year` | mtime window |
| `kind:image\|video\|audio\|doc\|archive\|code\|other` | extension classes |
| `-token` | negation (any of the above) |
| `*`/`?` inside a bare term | glob, as today |

### Implementation
1. New `src/query.rs`:
   ```rust
   pub enum ChipKind { Ext, Type, Size, Modified, Kind, Neg }
   pub struct Chip  { pub kind: ChipKind, pub value: String, pub span: (usize, usize) }
   pub struct Query { pub terms: Vec<String>, pub chips: Vec<Chip>, pattern: Pattern }
   impl Query { pub fn parse(s: &str) -> Query; pub fn matches(&self, e: &Entry) -> bool;
                pub fn is_empty(&self) -> bool; pub fn remove_chip(&self, i: usize) -> String; }
   ```
   `matches` = `pattern.matches(&e.name_lower)` **and** every chip predicate.
   Extension classes live in one `fn class(ext) -> Kind` table reusing
   `fsutil::is_image`/`has_thumbnail` where possible.
2. `Tab` (`tab.rs:50-87`): add `pub scope: SearchScope` and `pub query: Query`.
   Add
   ```rust
   pub enum SearchScope { Folder, Subfolders, ThisPc }   // default Subfolders
   ```
   and `Tab::set_filter(&mut self, s: String)` which updates `filter`, `query` and
   `view_dirty` together. Replace the direct `filter` writes at `app.rs:3226`
   (TextEdit), `3247` (clear), `2333-2335` (Escape), `tab.rs:224` (navigate) and
   `tab.rs:360` (start_search) with it.
3. `rebuild_view` (`tab.rs:515-610`): replace `Pattern::new(&self.filter)` +
   `pat.matches(...)` (`522-529`) with `self.query.matches(e)`.
4. Search engine: change `fsutil::spawn_search(roots, pattern, ctx)` (`491-508`)
   and `search_dir` (`510-543`) to take `&Query` (cloneable) instead of `Pattern`;
   `search_dir` builds the `Entry` first (needs size/mtime/ext for chips) and then
   calls `query.matches`. Keep the rayon recursion, cancellation and batching as is.
5. `Action::StartSearch` (`apply` `2786-2802`): `match t.scope` —
   `Folder` → do not spawn (the in-memory filter already did the work);
   `Subfolders` → `vec![t.path]`; `ThisPc` → drive roots; Gallery keeps its
   Pictures override.
6. UI — scope button in `nav_bar` (`3181-3269`): a small `menu_button` inside the
   search frame to the right of the text field (`3205-3216`), icon per scope, items
   for the three scopes → `Action::SetSearchScope(s)` (handled next to `StartSearch`;
   changing scope while a search is running restarts it, while filtering just marks
   the view dirty).
7. Chips row: in `file_area` (`5228-5298`) immediately above the list. Each chip
   is a small button `label ✕`; clicking ✕ pushes `Action::RemoveChip(i)`, which
   rewrites `filter` via `Query::remove_chip` and calls `set_filter`. Also shown in
   search mode so results can be narrowed without a restart (in-memory only).
8. Status bar (`6340-6454`): append the scope name and the active chip count.
9. `pc_page` drive cards (`4429-4437`) keep filtering on `query.pattern` via a
   `Query::name_pattern()` accessor.

### New i18n keys
`Şurada ara`, `Bu klasör`, `Alt klasörler dahil`, `Bu bilgisayar`, `Filtreler`,
`Filtreyi kaldır`.

### Risks
- `search_dir` now materialises an `Entry` per file instead of a name check; on
  huge drives this costs a few extra `metadata`-free field copies (the metadata is
  already fetched) — measure before/after with the status-bar scanned counter.
- Chips must survive `Escape` ordering: clear chips first, then filter, then
  selection.

---

## Phase 4 — Git pill + Git column

### What the user gets
A pill in the status bar next to the path: ` main ● 2↑1↓` for the repo containing
the current folder (click = copy branch, right-click = open terminal / git status).
A **Git** details column showing the branch on repo roots and — when `git` is
available — per-file status letters (`M`, `A`, `?`, `D`).

### Implementation
1. New `src/git.rs` (pure Rust, Windows + Linux):
   ```rust
   pub struct GitInfo { pub root: PathBuf, pub branch: String, pub dirty: bool,
                        pub ahead: u32, pub behind: u32, pub files: HashMap<PathBuf, char> }
   pub fn repo_root(dir: &Path) -> Option<PathBuf>;      // walk up for .git (dir or file)
   pub fn read_head(root: &Path) -> Option<(String, bool /*dirty-by-index*/)>;
   pub fn status(root: &Path) -> Option<HashMap<PathBuf, char>>;  // via `git`, cached
   ```
   - `repo_root` + `read_head` need no dependency: parse `.git/HEAD`, follow
     `ref: refs/heads/x` to `.git/refs/heads/x` or `.git/packed-refs`; worktrees
     (`.git` is a file) are handled by reading `gitdir:`.
   - `status` shells out to `git status --porcelain=v2 -z --branch --untracked-files=normal`
     with `Command`, `-C root`, stdin null; parse the NUL-separated records. If
     `git` is missing (`Command::spawn` fails), fall back to branch-only.
2. Cache: `App.git_cache: HashMap<PathBuf, (Option<GitInfo>, Instant)>` (TTL 5 s)
   plus a single worker thread + `mpsc` so a `git status` never blocks the UI.
   Request the current folder's repo on navigation and on the 5 s tick; visible
   only when the column or pill is enabled.
3. Status-bar pill in `status_bar` (`6340-6454`): `ui.button` with the branch text,
   `on_hover_text` = repo root + ahead/behind, click → `ctx.copy_text(branch)`.
4. Column: `details_view` (`5346-5781`). Add, in this order:
   - `Config.git_column: bool` (default false) + View menu toggle.
   - width budget: extend `gaps` (`5404`) by one gap and subtract from `w_name`
     (`5408-5411`); width from the widest branch label (cap 120 px).
   - `table = table.column(Column::initial(w_git).at_least(48.0).at_most(160.0)
     .resizable(true).clip(true))` after the Type column (`5464-5472`).
   - header: `head(&mut h, &t("Git"), None, false)` after `5560` — no sort key in
     v1 (it is not a `SortKey`; sorting by git state would need a new key, defer).
   - body cell before the spacer (`5721`): branch for `e.is_dir && is_repo_root`,
     else the status letter for that path, else empty.
   - bump the table salt (`5419`) because the column count changed.
5. `Entry` stays untouched; git state is looked up by path from the cache in the
   cell closure (a `HashMap` hit per visible row).

### New i18n keys
`Git`, `Git sütununu göster`, `Dalı kopyala`, `Değişiklikler var`,
`Git deposu değil`.

### Risks
- `git status` on a monorepo can take seconds → hard requirement: worker thread,
  5 s TTL, and never more than one in flight per repo.
- `.git/index` parsing is *not* in v1 (the CLI path is simpler and exact); document
  that git is optional and the column degrades to branch-only without it.

---

## Phase 5 — Preview: syntax highlighting, Markdown, Quick Look

### What the user gets
- Code previews are coloured (keywords, strings, comments, numbers, types) with a
  line-number gutter, aligned to the active theme, no bold.
- `.md` files render (headings sized by level, lists, tables, code fences with
  highlighting, clickable links, local images).
- **Space** opens a floating Quick Look overlay (large, centred, arrows to step
  through files, Esc/click-outside to close). If PowerToys is installed, a setting
  chooses which one Space invokes; built-in is the default.

### Implementation
1. `src/highlight.rs` (no dependency):
   ```rust
   pub enum Lang { Rust, C, Cpp, CSharp, Java, Go, Python, Js, Ts, Json, Toml, Yaml,
                   Html, Css, Shell, PowerShell, Sql, Markdown }
   pub enum Tok { Plain, Comment, Str, Num, Kw, Type, Func, Punct }
   pub fn lang_for(path: &Path, head: &str) -> Option<Lang>;
   pub fn highlight(lang: Lang, text: &str) -> Vec<(std::ops::Range<usize>, Tok)>;
   ```
   A hand-written lexer per family (~700 lines total) with a shared
   `Lexer { bytes, i, out }` helper. Truncate input to 256 KiB for lexing.
2. Token colours: derive from the active palette instead of hardcoding —
   keywords = `accent`, strings = a green tuned by `scheme::mix(text, accent, ..)`,
   comments = `weak`, numbers = a warm tone. One `fn token_color(tok, pal, accent)
   -> Color32` in `app.rs`, so all 12 themes work.
3. Rendering: build an `egui::text::LayoutJob` with one section per token (colour
   only, same font) and lay it out with `FontId::monospace(text_size(13.0))`.
   Add a line-number gutter as a second monospace column painted alongside, and
   virtualise: only lay out the visible ±200 lines (line offset table first).
   Cache `(path, mtime, theme, lang) -> Arc<Vec<Section>>`.
4. Mono font: `setup_fonts` (`508-592`) currently never touches
   `FontFamily::Monospace`. Add Cascadia Mono → Consolas → egui default on Windows,
   DejaVu Sans Mono → Liberation Mono on Linux, CJK fallback included.
5. `fsutil::read_preview` (`fsutil.rs:1199-1222`): raise the 64 KiB literal cap to
   a named `PREVIEW_MAX = 256 * 1024`, add UTF-16 BE BOM handling, and return
   `Binary` for NUL bytes as today. Do not change `PreviewData`'s shape; whether a
   text file is *Markdown* is decided by extension in the UI.
6. `src/markdown.rs` (depends on the answer to Q2):
   `pub fn render(ui: &mut egui::Ui, src: &str, base: &Path, ctx: &MarkdownCtx)`.
   Headings by size + colour (no bold), lists, task lists, tables, blockquotes,
   fenced code → `highlight`, inline code, links via `open`, images via
   `ImageCache::get("md{px}:{path}", ..)` with paths resolved against `base`.
   Guard: only local `file`-relative images, no network fetch.
7. Quick Look overlay:
   - `App.quick_look: Option<QuickLook>` with `{ path, scroll: f32, kind }`;
     `Config.quick_look: u8` = `0` built-in (default), `1` PowerToys Peek, `2` off.
   - `handle_keys` (`2137-2458`): plain Space currently pushes `Action::Peek`
     (`2416-2427`). Change the arm to `Action::Peek` still, and in `apply`
     (`2867-2877`) branch on `quick_look`: built-in → open the overlay; PowerToys →
     `win::peek_file` as today.
   - Draw after `overlays` (`7257`) in a new `fn quick_look(&mut self, ctx)`, using
     `egui::Area` with `Order::Foreground` + `Frame::window`, 70 % of the screen,
     a full-screen click-catcher behind it, Esc/click-outside to close,
     `ArrowLeft`/`ArrowRight` to step through `Tab.view`.
   - Own preview cache slot (`QuickLook.data`), never `self.preview` (the side pane
     owns that).
8. View menu `Göster` submenu: `Hızlı önizleme` radio (built-in / PowerToys / off)
   and `Kod vurgulama` toggle.

### New i18n keys
`Kod vurgulama`, `Markdown önizlemesi`, `Hızlı önizleme`, `Yerleşik`,
`PowerToys Peek`, `Kapalı`, `Büyük dosya — önizleme kısaltıldı`, `Satırları kaydır`,
`Sonraki öğe`, `Önceki öğe`.

### Risks
- A hand-rolled lexer must never panic on invalid UTF-8 boundaries — iterate byte
  ranges only at ASCII delimiters.
- Markdown rendering inside a virtualised list needs a `Ui` per frame; keep it
  stateless and cache the parse (`pulldown_cmark::Parser` output is cheap to
  re-walk for <256 KiB, cache anyway keyed by mtime).

---

## Phase 6 — Split view

### What the user gets
`Ctrl+\` splits the content into two panes, each with its own tab strip. Drag the
divider to resize. `Tab`/`F6` moves focus. A sync button makes the other pane
follow the same relative path. `Şuraya taşı` gains a "other pane" item. The layout
is persisted (`split`, `split_ratio`, second pane's tabs).

### Implementation (the only invasive change)
1. `app.rs`: new `struct Pane { tabs: Vec<Tab>, active: usize }` and
   `App { panes: [Pane; 2], focus: usize, split: Option<f32>, split_sync: bool }`.
   Rename the existing `pane: u8` (right-hand side panel) to `side_panel` first —
   13 sites — so the word is free.
2. Accessors: `fn cur(&self) -> &Pane`, `fn cur_mut(&mut self) -> &mut Pane`,
   `tab()`/`tab_mut()` retargeted to `self.panes[self.focus]`. The 37 existing
   `self.tab()` calls then stay unchanged; the 36 `self.tabs[self.active]` and the
   `active` arithmetic are retargeted one by one (compiler-checked).
3. Rendering: `file_area`, `details_view`, `grid_view`, `status_bar`,
   `command_bar`, `nav_bar`, `address_bar`, `sidebar`, `preview_panel` gain a
   `pane: usize` parameter, or (simpler and chosen) are only ever called for the
   focused pane, and the unfocused pane is drawn by a new thin
   `fn pane_body(&mut self, ui, actions, pane: usize, interactive: bool)` that
   borrows `self.panes[pane]` locally. Start with the focused-pane design and move
   the body renderers behind it as the compiler demands.
4. Panels: tab strip per pane (`tab_bar` already takes an index → add a pane
   index), a `SidePanel`-based splitter or a `CentralPanel` that lays out
   `[left, 6 px drag handle, right]` horizontally when `split` is set.
5. Action targeting: capture `self.focus` at *push* time. Add
   `Action::FocusPane(usize)` handled synchronously, and freeze the target for
   long-lived actions (`Navigate`, `OpenSelected`, `Paste`, …) via a
   `pending_pane: usize` set when the action is pushed. `apply` runs at frame end
   (`7277`), so without this a click in the unfocused pane would act on the
   focused one.
6. Transients that must become per-pane: `item_rects`, `marquee`, `grid_scroll`,
   `renaming`, `address_edit`. Either move them into `Pane` or key them by pane
   index. `self.preview`/`type_names` stay global.
7. Polling: `update`'s `poll`/`rebuild_view` loop must run for **both** panes each
   frame (search channels, watchers and loaders of the unfocused pane must keep
   draining) — this is the failure mode of the "swap state" alternative; do not
   use that design.
8. Config: `split: bool`, `split_ratio: f32` (0.5), `tabs2: Vec<PathBuf>`,
   `active2: usize`; `save_config` writes both panes; startup restores both.
9. `Action::ToggleSplit`, `Action::FocusPane`, `Action::SyncPanes`,
   `Action::CopyToOtherPane`; `Ctrl+\` in `handle_keys`.

### New i18n keys
`Bölünmüş görünüm`, `Bölmeleri değiştir`, `Diğer bölmeyle eşitle`,
`Diğer bölmeye taşı`, `Diğer bölmeye kopyala`.

### Verification
Both panes load independently, both keep watching for file changes, both keep
searching while the other is focused; split persists across restart; keyboard
navigation only affects the focused pane; undo works for jobs started from either.

---

## Open decisions (asked before implementation)

1. Commit the current typography/columns work as a baseline before phase 1.
2. Markdown: add `pulldown-cmark` (tiny, no transitive deps) vs hand-roll.
3. Git: allow calling the `git` CLI for `status` (exact, per-file letters) with a
   branch-only fallback when git is absent.
4. Destination picker: native `IFileOpenDialog` on Windows (in-app modal on Linux)
   vs in-app modal everywhere.
5. Quick Look default: built-in overlay first, PowerToys Peek as the alternative.

---

# Implementation status

All six phases are implemented, one commit each. No background index, no
admin/MFT index.

| Phase | Commit | What landed |
|---|---|---|
| Base | `771c3ab` | sharp text (vendored `epaint`), system text size, resizable details columns, roomier nav pane |
| 1 | `e8c201d` | `FolderView` + `Config.remember_views`/`folder_views`, per-folder sort/view/grouping/column widths, View-menu toggle + reset |
| 2 | `66fbe61` | `shell::pick_folder` (IFileOpenDialog / zenity-kdialog), `Action::PickDest`, Copy to… / Move to… |
| 3 | `bf54638` | `src/query.rs` (scope + `ext:`/`type:`/`size:`/`modified:`/`kind:`/`-word`), scope menu, removable chips row, scope in status bar |
| 4 | `5e8a231` | `src/git.rs` (`HEAD` + `git status --porcelain=v2`), status-bar branch pill, Git details column (branch on repo root, state letter per file) |
| 5 | `9aa07ac` | `src/highlight.rs` (single-pass lexer, 9 language families), `src/markdown.rs` (pulldown-cmark), preview rewrite, Space → built-in Quick Look overlay with `quick_look` setting |
| 6 | `52b5700` | split view: `pane2`/`focus`, `split_area`, Ctrl+\\, View-menu toggle |
| 6.1 | `2670c96` | unique egui ids per pane (`App.pane_no`), Space closes Quick Look again |
| 5.1 | (this commit) | preview v2 + column layout (see below) |

## Post-review fixes (preview v2 and details columns)

Raised by the user after the six phases:

1. **Red frames in split view** — both panes reused the same egui widget ids, so
   egui's debug id-clash warning painted red rects around both panes. Every
   pane-owned id (`list_bg`, `page_group`, the group headers, inline rename, the
   details `TableBuilder` salt) is now suffixed with `App.pane_no` (`0`/`1`).
2. **Space closes the preview** — the same key toggles: Quick Look open → close,
   otherwise open.
3. **Folder preview** — the panel (and Quick Look) now show the folder's own date
   and `{} Dosya, {} Klasör`, then its children (folders first, natural order,
   cap 500, no recursive size). Children are read on a worker thread and cached
   in `App.peek`, keyed by path.
4. **Non-previewable files** — a details card (`metadata_block`) with name, type,
   size, modified, created and full path replaces the old "no preview" text.
5. **Open with** — every preview has *Aç* and (files only) *Birlikte aç…*
   buttons; they queue `Action::OpenEntry` / `Action::OpenWith`.
6. **PDF, video, audio** — embedded **Edge WebView2** (`src/webview.rs`,
   `webview2-com`): PDFs go to the runtime's own viewer via `file:///`, media
   play through `player.html` served from a virtual host
   (`swift-app.local` → asset folder, `swift-media.local` → the file's parent).
   Creation is asynchronous (STA COM, `CoInitializeEx` in `start`); until the
   controller is ready the preview shows the thumbnail and
   *Önizleme hazırlanıyor…*. The child HWND is hidden while a menu or dialog is
   open, while the window is minimised, and for the side panel while Quick Look
   covers it. Bounds are DIPs, `RasterizationScale` = `pixels_per_point`.
7. **Word / Excel / PowerPoint / ODF** — thumbnail hero plus extracted text
   (`src/ooxml.rs`: zip + `quick-xml`; docx paragraphs/tables, pptx slides,
   xlsx sheets with shared-string resolution, ODF `content.xml`; 200 KB cap).
8. **Details columns** — `Değiştirme tarihi`, `Tür` and `Boyut` are left-aligned
   (heading and cells); `Name` is content-sized (longest of four candidates over
   the first 4000 rows, +34 px, clamped 220–460 px, then shrunk to fit), `Type`
   is content-sized, and the remaining space becomes a trailing spacer column so
   columns start right after the name like Explorer.

New i18n keys: `Klasör içeriği`, `Yol`, `Önizleme hazırlanıyor…`,
`Gömülü görüntüleyici kullanılamıyor` (all seven language files + `TABLE`).

## Deviations from the design above

- **Split view uses the swap design** the plan warned against, but with the
  warning addressed: both panes' tabs are polled and rebuilt every frame
  (`update` chains `self.tabs` and `self.pane2`), and the unfocused pane's
  actions are applied **while that pane is swapped in**, so no action needs a
  pane tag. Costs: one tab strip (the focused pane's), no persisted split
  state, and `Ctrl+Tab` / close-last-tab act on the focused pane only.
- **Quick Look** renders Markdown and highlighted text itself instead of only
  delegating to PowerToys Peek; Peek remains selectable in the View menu.
- Markdown tables/lists/headings are rendered; images and footnotes are shown as
  text, links are not clickable. Inline emphasis/code/links are drawn as runs of
  a single `LayoutJob` (see "Post-review fixes 2").
- Git column sits right after **Name** so the Size column keeps the right edge.

## Verification checklist (manual)

1. **Per-folder view** — set Details + sort by Date + widen a column in
   `Downloads`, go to `Documents` (Icons view), return to `Downloads`; restart
   and confirm both restore. View menu → *Reset this folder's view*.
2. **Copy to… / Move to…** — right-click → *Copy to…*; pick a folder; confirm
   the file appears, `Ctrl+Z` undoes it, and the conflict dialog appears when
   the name exists.
3. **Search** — type `ext:pdf size:>1mb`; chips appear under the search box;
   click a chip's ✕ to remove it; press Enter and switch scope with the button
   left of the search box (this folder / including subfolders / this PC).
4. **Git** — open this repo; the status bar shows the branch, a ● when dirty,
   ↑/↓ when ahead/behind; the Git column shows `M`/`?` next to changed files
   and the branch name on the repository root folder. Toggle it in View.
5. **Preview** — select `src/app.rs` (coloured), `README.md` (Markdown), then
   press Space in the list for the Quick Look overlay; ←/→ move to the next
   file while it is open; Esc closes. View → *Syntax highlighting* / the three
   Quick Look entries.
6. **Split view** — View → *Split view* or `Ctrl+\`; click a folder in the
   right pane (it takes focus), `Alt+Left` goes back only in that pane; the tab
   strip, status bar and side panel follow the focused pane. No red frames.
7. **Preview v2** — select a folder (contents list + counts), a `.txt` file
   (text), a `.pdf` (embedded viewer, scroll/zoom work), `.mp3`/`.mp4`
   (play/pause, seek), `.docx`/`.xlsx`/`.pptx` (thumbnail + text), an `.exe`
   (details card), and press *Birlikte aç…* on any file.
8. **Quick Look** — Space opens it over the list, Space again closes it; Esc and
   the window's ✕ also close it; ←/→ move through the folder while it is open;
   PDF/video/audio play inside the overlay.
9. **Columns** - in Details view drag the *Değiştirme tarihi* / *Tür* / *Boyut*
   headings: values start at the left edge of each column, the *Name* column is
   only as wide as the longest visible name (220-460 px) and Date/Type/Size
   follow immediately after it.

---

## Post-review fixes 2 (six-item review)

Raised by the user after preview v2 and the first split-view fix.

| # | Report | Cause | Fix |
|---|---|---|---|
| 1 | Red lines inside a **Markdown** preview in Quick Look | every fenced code block created `ScrollArea::both()` with the *default* salt, so two blocks in one document shared one id; same for `Grid::new(("md_table", counter))` | each code block salts with `ui.next_auto_id().with("md_code")`, tables with `ui.next_auto_id().with("md_table")` |
| 2 | Red lines in **split view** *and* scrolling one pane scrolled the other | egui gives sibling child `Ui`s the **same** `ui.id()` (only `unique_id` differs). Both pane bodies therefore shared every `ui.id()`-derived id — including the default `ScrollArea` id — so they also shared one scroll offset | `ui.push_id(("pane", pane))` wraps each pane body; `grid_scroll` is now `[(f32, f32); 2]` (per pane); every view/panel `ScrollArea` has an explicit `id_salt` (`nav_scroll`, `home_scroll`, `pc_scroll`, `grid_scroll`, `preview_scroll`, `details_scroll`, `help_scroll`, `addr_suggest_scroll`, `pv_scroll` + surface) |
| 3 | Quick Look "so basic"; audio player "looks so bad", must autoplay, kept playing after closing | plain `Window` with a raw `<audio controls>`; WebView2 needs the autoplay flag; nothing paused the element on hide | dim backdrop + rounded card with a real header; `player.html` rebuilt (album tile, seek bar with hover knob, time labels, play/pause, volume, speed; video keeps native controls but autoplays); `--autoplay-policy=no-user-gesture-required`; `pause()` script runs whenever the child window hides |
| 4 | Split-view button next to *Sırala* / *Görünüm*; ✕ in the pane's top-right corner | — | painted two-panel toggle in the command bar (accent when active); ✕ is an overlay hit-test at the right pane's top-right, drawn last so it wins clicks |
| 5 | *Aç* / *Birlikte aç…* in the Quick Look header; **no** file details there for images/PDF/… | details card was drawn for every surface | buttons moved into the header (window), removed from the body on the overlay surface; metadata only for folders, archives/binaries and failures |
| 6 | Word / Excel / PowerPoint / Markdown previews "not that great" | Office was a monospaced text dump; Markdown flattened inline runs | `ooxml::Doc { Text(Vec<Block>), Sheets, Slides }` (headings, paragraphs, bullets, tables; per-sheet grid; slide cards) rendered by the new `src/docview.rs`; Markdown paragraphs are one `LayoutJob` with accent for strong, italics, inline-code background, underlined links; header rows in accent colour |

Also: `WebView2` `AcceleratorKeyPressed` now forwards **Boşluk** / **Esc** to the
app (`WebView::take_keys`), so the preview can be closed while the player has
keyboard focus — previously those keys died inside the child window.

### Verification (round 3)

1. **Markdown** - open a `.md` with **two or more fenced code blocks** (e.g.
   `README.md` or any doc with ``` blocks) in Quick Look: no red frame, both
   blocks scroll on their own.
2. **Split view** - `Ctrl+\` (or the new toolbar button); scroll the left pane,
   the right pane must not move; no red frames; ✕ in the top-right of the right
   pane closes the split (and the toolbar toggle un-presses).
3. **Audio** - Space on an `.mp3`: it starts **playing immediately** without
   pressing play; with the player focused press Space - the preview closes and
   the sound stops. Same for Esc, ✕, and clicking the dimmed background.
4. **Quick Look chrome** - header shows icon, name, type · size, then *Aç*,
   *Birlikte aç…*, ✕; an image/PDF shows only content (no details card); a `.zip`
   or `.exe` shows the details card.
5. **Office** - `.docx` with headings/lists/table shows them as such; `.xlsx`
   shows sheet name, row numbers and a highlighted first row; `.pptx` shows one
   card per slide; `.md` shows **accent** bold, italic, inline code chips and
   underlined links.

---

## Post-review fixes 3 (split crash, audio, office, markdown tables)

Raised by the user after the round-3 build.

| # | Report | Cause | Fix |
|---|---|---|---|
| 1 | Split view **crashed the app**; the pane ✕ closed the whole application | `Action::ToggleSplit` cleared `pane2` **before** swapping the tab sets. A click on ✕ also focused the right pane first, so `swap_panes()` then moved the empty `pane2` into `tabs`; the next frame indexed `tabs[active]` on an empty `Vec` | closing goes through `merge_split_panes()`: the focused set wins, `pane2` is dropped afterwards, `active` is clamped, and an empty result falls back to *Bu bilgisayar*; ✕ clicks are excluded from the pane-focus hit test |
| 2 | Clicking the path bar of the right pane **moved its content to the primary pane** | panes were laid out as "focused set always on the left"; any focus change swapped `tabs`/`pane2` and the contents visually jumped sides | new `App::left_tabs` records which side draws `tabs`; focus changes swap the sets **and** flip the flag, so every pane keeps its position. Both panes now show their folder path, so nothing shifts vertically either |
| 3 | Audio player "pretty design in the middle, empty top and bottom" | the card was centred in a 220 px strip inside a much taller Quick Look window; the page background did not match the window fill | overlay audio fills the window height; `player.html` gets a tall layout (large album tile, centred column, full-bleed accent-tinted background) via `@media (min-height: 420px)`, and the page receives the app's exact `bg` colour (`Want::bg`) |
| 4 | Office preview "doesn't work at all — only an icon / empty box" | Quick Look drew a 640 px *thumbnail* above the document; for Office files the shell returns the file icon, which pushed the actual text below the fold | Office files no longer get the hero thumbnail (`hero && !office`); the document text starts at the top with the full body height |
| 5 | Markdown "tables / flowcharts / etc not great" | tables used `Grid` with unbounded label widths (columns overflowed, some malformed tables are not parsed at all by GFM), and wide ASCII diagrams scrolled sideways | tables get equal, wrapped columns inside a rounded frame; `normalize_tables()` repairs delimiter rows whose cell count differs from the header (they used to render as raw `\|` text); code blocks shrink their font (down to 7.5 px) when the longest line would not fit horizontally |

Mermaid blocks are deliberately **not** rendered as diagrams: the user chose to
keep them as highlighted code rather than bundle the ~3 MB Mermaid engine.

### Verification (round 4)

1. **Split view** - `Ctrl+\`, click into the right pane (its path bar, a file,
   empty space): the pane contents stay on their side; the ✕ in the top-right
   closes only the split (the app keeps running) and the toolbar toggle
   un-presses. Repeat with the focus on the right pane, then close via the
   toolbar button - no crash, and the folder you were using is the one left.
2. **Audio** - Space on an `.mp3`: the player fills the Quick Look window (no
   large empty bands above/below), starts playing, Space closes it and the sound
   stops.
3. **Office** - Space on a `.docx`/`.xlsx`/`.pptx`: the document text starts at
   the top of the window (no giant icon above it) and scrolls.
4. **Markdown tables** - open `docs/dev/architecture.md` in Quick Look: the
   *File / Lines / Role / Compiled when* table (5-cell delimiter over a 4-cell
   header) now renders as a table, not as raw `|` lines; long cells wrap instead
   of running off the window.
5. **ASCII diagrams** - `docs/dev/architecture.md` "60-second architecture"
   block fits the width (glyphs stay aligned, no horizontal scrollbar).



