# Low-level design reference — Swift Explorer

Per-file reference for the code as of `v0.7.0` (`850c108`). Every claim is derivable from the cited lines.
Use this to avoid re-reading `app.rs` (4.5k lines).

Read first: [`architecture.md`](architecture.md) (layers/state), [`workflows.md`](workflows.md) (flows).

---

## 1. `src/app.rs` — UI, state, actions (4526 lines)

### 1.1 Constants and enums

| Item | Line | Notes |
|---|---|---|
| `VIEW_LEVELS: [(u32,&str);5]` | 21 | `0 = Details`, then 32/48/96/256 logical px icons (Small/Medium/Large/Extra large) |
| `ICONS`, `FLUENT` | 23-24 | egui font family names |
| `TITLE_H = 34.0` | 733 | custom title strip height |
| `NATIVE_MENU = cfg!(windows)` | 838 | compile-time; both branches must compile |
| `VIEW_LEVELS` labels | 21 | Turkish keys, translated via `t()` |
| `HELP` table | 4236-4266 | F1 shortcut list; rows hidden on Linux |
| `Action` | 229-267 | 39 UI intents (deferred; see below) |
| `Dialog` | 269-276 | `None`, `ConfirmDelete{paths}`, `Conflict{files,dest,cut,names}`, `Help`, Linux `Properties` |
| `UndoOp` | 278-284 | `Rename{from,to}`, `Created`, `Moved(pairs)`, `Copied(dsts)`, `Trashed(srcs)` |
| `Renaming` / `Preview` | 286-295 | inline edit state / preview worker slot |

`Action` variants: `Navigate`, `OpenEntry(path,new_tab)`, `OpenSelected`, `Up`, `Back`, `Forward`,
`Refresh`, `NewTab(path)`, `NewWindow`, `CloseTab(usize)`, `Copy`, `Cut`, `Paste`, `PasteInto(path)`,
`Delete{permanent}`, `Rename`, `NewFolder`, `NewFile`, `SelectAll`, `SelectNone`, `InvertSelection`,
`CopyPath`, `Terminal(path)`, `RunTerminal{name,args,dir}`, `Properties(path)`, `ShowInExplorer(path)`,
`OpenWith(path)`, `ShellMenu{extended}`, `CalcSize`, `AddBookmark(path)`, `RemoveBookmark(path)`,
`StartSearch`, `ExitSearch`, `Sort(SortKey)`, `SetView(usize)`, `DropPaths{paths,target,ctrl,shift}`, `Undo`.

### 1.2 `Config` (28-91)

Fields and values: see [`architecture.md` §9.1](architecture.md#91-config-json-the-only-app-owned-state).
`load_config` (85-91) tries `SwiftExplorer/config.json`, then `HizliGezgin/config.json` (220-222), then
`Config::default()`. `save_config` (1177-1209) writes pretty JSON only when the text changed.

### 1.3 `App` field groups (297-376)

| Group | Fields (line) | Notes |
|---|---|---|
| tabs/view | `tabs` 298, `active` 299, `sort` 300, `asc` 301, `show_hidden` 302, `show_preview` 303, `show_ext` 304, `view_level` 305, `grid_cols` 360 | view settings are **global**, not per tab |
| places | `bookmarks` 308, `recent` 309, `grouped` 311, `known` 331, `tree_open` 333 | `grouped` = folders grouped by date (default Downloads) |
| lifecycle | `background` 312, `resident` 314, `autostart` 315, `start_hidden` 329, `revealed` 374, `frames` 373, `hwnd` 351 | resident = this process owns the pipe server |
| updater | `auto_update` 320, `update_rx` 321, `update_wake` 323, `last_update_check` 324, `update_installed` 326 | atomics shared with the checker thread |
| ops | `clipboard` 336, `jobs` 337, `dir_sizes` 338, `undo` 353, `silent_jobs` 367, `renaming` 352 | `silent_jobs` keyed by `Arc::as_ptr` |
| input/UI | `dialog` 339, `address_edit` 340, `address_focus` 341, `focus_search` 342, `type_buf` 343, `type_time` 344, `toasts` 345, `marquee` 364, `item_rects` 365, `grid_scroll` 366, `zoom_acc` 363, `inject_release` 361 | |
| caches | `images` 349, `type_names` 350, `subdirs` 356-359, `drives` 334-335 | |
| persistence | `saved_config` 368, `last_autosave` 369, `started` 372 | |

### 1.4 Function index (verified anchors)

Lifecycle / config:

| Function | Line | Purpose |
|---|---|---|
| `spawn_update_checker` | 102 | returns `(rx, wake_tx)`; 15 s delay, hourly + jitter; exits after install |
| `startup_tabs` | 152 | explicit path wins, else saved tabs filtered to existing/virtual, else Home |
| `prefetch_startup` | 168 | pre-warm listing + icons before the window exists |
| `App::new` | 1070 | init order documented in `architecture.md#5-startup-sequence-cold-start` |
| `save_config` | 1177 | serialize `Config`, diff-write |
| `refresh_drives` | 1211 | thread → `drives` mutex + repaint |
| `watch_drives` | 1223 | 1 Hz `win::drive_mask()`, repaint only on change |
| `reveal_when_ready` | 1256 (win) / 1243 (unix) | uncloak when list+icons ready or 350 ms |
| `park` / `reopen` | 1289 / 1296 | hide-and-wait / restore from pipe request |
| `push_job` (see 1434) | — | adds a job and optional undo silence flag |

Actions / operations:

| Function | Line | Purpose |
|---|---|---|
| `start_transfer` | 1331 | conflict check → dialog or `spawn_transfer` |
| `create_new` | 1344 | new folder/text file, unique name, undo entry, inline rename |
| `commit_rename` | 1367 | validate + `fs::rename` + undo entry |
| `do_undo` | 1392 | LIFO replay, silent jobs |
| `handle_keys` | 1446 | see [§1.9](#19-shortcuts) |
| `apply` | 1691 | executes every `Action` (clipboard, transfer, menus, tabs…) |
| `subdirs_cached` | 1982 | sidebar/breadcrumb children, background read, `Arc<[PathBuf]>` |
| `poll_subdirs` | 1997 | applies results, drops stale hidden-flag results |
| `loc_name` | 2008 | localized display name (Home, This PC, drives, known folders) |
| `expand_path` | 3085 | `~`, `shell:*`, `%VAR%`, slash normalization |
| `type_name_cached` | 3129 | memoized `shell::type_name(ext)` |

Rendering:

| Function | Line | Notes |
|---|---|---|
| `tab_bar` | 2035 | 34 px strip; drag region added *first*; tabs max 236 px; caption buttons 46×30 |
| `nav_bar` | 2152 | back/forward/up/refresh (40×32) + search box (25 % width, 180–480 px) |
| `crumbs` / `crumb_children` | ~2241 / ~2269 | longest known-folder ancestor; overflow into `«` |
| `address_bar` | 2281 | edit mode with autocomplete from `subdirs_cached` (never hits disk inline) |
| `command_bar` | 2486 | New/Cut/Copy/Paste/Rename/Delete, Sort, View, More |
| `nav_row` / `tree_node` / `sidebar` | 2745 / 2777 / 2824 | 24 px rows, 16 px indent, max 2000 children |
| `home_page` / `pc_page` | 2933 / 3014 | tiles 250×62 / drive cards 262×54 |
| `file_area` | 3362 | background clicks, marquee, drop, empty states |
| `details_view` | 3446 | `TableBuilder`, fixed columns by width, group headers 34 px |
| `grid_view` | 3647 | manual virtualization, tile = `max(size+22,80)` × `size+76`, ≤3 name lines |
| `preview_panel` | 3838 | 0/1/N selection branches |
| `status_bar` | 3944 | counts, selection size, search progress, view toggles |
| `overlays` | 4007 | job cards + toasts, bottom-right |
| `drag_overlay` | 4064 | drag label + external drop hint |
| `dialogs` | 4090 | one modal at a time; `mem::replace` + reinstall |
| `resize_edges` | 4272 | non-Windows 6 px `BeginResize` |
| `update` | 4299 | frame order (`architecture.md#6-frame-lifecycle`) |
| `raw_input_hook` / `on_exit` | 4510 / 4520 | synthetic releases / cancel searches + save |

### 1.5 Panel geometry (Explorer metrics)

| Panel | Height/width | Frame |
|---|---|---|
| `TopBottomPanel::top("tabs")` | exactly 34 | fill `Pal.strip`, margin left 8 |
| `top("nav")` | auto | fill `Pal.nav`, margins 8/12/8/8 |
| `top("cmd")` | auto | fill `Pal.content`, margins 12/12/7/7 |
| `bottom("status")` | exactly 23 | fill `Pal.status`, margins 14/8 |
| `SidePanel::left("nav_pane")` | default 158, 120–450, resizable | `Pal.content` |
| `SidePanel::right("preview")` | default 320, 220–900, resizable | `Pal.content`, margin 12 |
| `CentralPanel` | remainder | margins 6/4/2/0 |

Panels use `show_separator_line(false)` and draw 1 px `hline`s on the background layer (`A:4443-4445`).

### 1.6 Rendering helpers

- `Pal` (489-511) / `pal()` (513-568) / `pal_of()` (570): Explorer-measured chrome colors, or scheme colors
  when `scheme::active()` brightness matches.
- `Chrome::of()` (679-730): surfaces egui paints itself (menus, popups, buttons, text edits).
- `Caption` + `caption_button` (736-769): minimize/maximize/restore/close, close is red
  `196,43,28` and pressed `148,22,16`.
- Glyph modules `g` (771-835): Segoe Fluent Icons on Windows, bundled `assets/fluent-icons.ttf` elsewhere.
- `flat_icon` (853), `glyph_job` (866), `icon_label` (926): toolbar/sidebar glyph buttons.
- `paint_icon` (1001) → `ImageCache::get`, `fit_rect` (1018) letterboxes; `paint_fallback` (983) draws an
  emoji/shape when the shell fails.
- `display_name` (1023) strips the extension for display when `show_ext == false` (byte slicing).
- `fmt_count` (909) always uses Turkish `.` thousands separators.
- `valid_name` (1031) rejects `\ / : * ? " < > |`, `.`, `..`, empty; does **not** reject `CON`, `NUL`,
  trailing dot/space.
- `same_volume` (1037 Unix `st_dev`, 1044 Windows first path component, case-insensitive).
- `drop_target` (1058) / `drop_target_p` (3298): highlight + emit `DropPaths`; refuses self/ancestor.

### 1.7 Deferred action model

Widgets never mutate FS state; they push `Action`s into the frame-local vector. `App::apply` (1691) runs
**after** all panels (`A:4498-4500`). Consequences:

- A newly opened dialog/tab/reload is visible next frame.
- Reading `self.tabs[active]` inside a widget sees the state before the current frame's queued actions.
- Some trivial mutations are inline and immediate: tab activation, sort direction, theme, language toggles,
  `show_preview`, `show_hidden` (menu), address edit state.

### 1.8 Dialogs, overlays, toasts

- Only one `Dialog` at a time; opening another requires closing the first (`A:4090-4222`).
- `ConfirmDelete`: Enter confirms (yes, even without focusing the button) (`A:4115`).
- `Conflict`: single choice for the whole batch — Replace / Skip / Keep both.
- Jobs render as non-modal cards: label, cancel (hidden while `syncing`), current file, 6 px bar, %/speed/ETA.
- Toasts: `(text, born, is_error)`, 4 s lifetime, 250 ms repaint while visible.
- Job completion maps to undo + toast in `A:4386-4413`.

### 1.9 Shortcuts

`shortcut()` (943-955) **removes** the matching key event and requires exact modifier equality.
Digits also match by `physical_key` (Turkish-Q layouts).

**Always active (even while typing)** — `A:1484-1548`:

| Keys | Action |
|---|---|
| Ctrl+T / Ctrl+W | new tab / close active tab |
| Ctrl+N | new window (new process) |
| Ctrl+L, Alt+D, F4 | focus address bar (select all) |
| Ctrl+F, Ctrl+E, F3 | focus search |
| Ctrl+Tab / Ctrl+Shift+Tab | next / previous tab |
| F5, Ctrl+R | refresh |
| F1 | Help |
| F11 | fullscreen toggle |
| Alt+Left / mouse Extra1, Alt+Right / Extra2 | back / forward |
| Alt+Up | up |
| Alt+P, Ctrl+P | toggle Details pane |
| Ctrl+H | toggle hidden items (does **not** clear subdir cache) |
| Ctrl+Shift+1 / 2 / 3 / 4 / 6 | view: 256 px / 96 / 48 / 32 / Details (5 unbound) |
| Ctrl+1..9 | activate tab n |
| Ctrl+V | paste; polled via `win::ctrl_v_down` on Windows because egui emits no event for file clipboards |
| Ctrl+wheel | cycle view level (accumulated threshold 0.12) |

**Only with a real listing and no text focus** (`A:1550-1688`):

| Keys | Action |
|---|---|
| Ctrl+Shift+N | new folder |
| Ctrl+Z | undo |
| Ctrl+A | select all |
| Ctrl+Shift+C | copy selected paths (quoted, CRLF) |
| Ctrl+D | pin current folder |
| Shift+Delete / Delete | permanent delete (dialog) / Recycle Bin |
| F2 | rename first selected in view order |
| Shift+F10 (Windows) | native shell menu |
| Backspace | up |
| Alt+Enter | Properties of first selected |
| Enter | open selected |
| Esc | exit search → clear filter → clear selection |
| Arrows / PageUp/Down / Home/End | move cursor (grid = `grid_cols`, page = `cols*4` or 20) |
| Shift+Arrow | extend range; Ctrl+Arrow moves cursor only |
| Ctrl+Space | toggle cursor item |
| any printable | type-ahead (900 ms, prefix, wrap) |

Dialogs swallow all keys except the Ctrl+V edge probe (`A:1453`, `1467-1469`).

### 1.10 app.rs invariants / traps

1. `tabs` is indexed by `active` in dozens of places — keep it valid; `CloseTab` must clamp.
2. View/sort/hidden/group settings are global; only `filter`, selection, cursor, history, scroll are per tab.
3. `item_rects` and `grid_scroll` are rebuilt each frame; marquee depends on the previous frame's rects.
4. `subdirs` cache keyed by path (not by `show_hidden`); stale results carry the hidden flag.
5. Opening a nested `Dialog` from inside `dialogs()` is not supported.
6. `inject_release` must be set after any blocking native menu/OLE call, or egui keeps the mouse down.
7. `fmt_count` and `display_name` have locale/edge-case limits (see §11).
8. `prefetch_startup` duplicates sort/group logic — keep it in sync with `Tab::rebuild_view`.

---

## 2. `src/tab.rs` — per-tab state machine (553 lines)

### 2.1 Types

| Type | Line | Fields |
|---|---|---|
| `THIS_PC: &str` | 14 | `::{20D04FE0-3AEA-1069-A2D8-08002B30309D}` (also the icon path) |
| `is_virtual(p)` | 17 | empty path or `THIS_PC` |
| `Group` | 22 | `label`, `start`, `len`, `collapsed` → `view[start..start+len]` |
| `Search` | 29 | `query`, `handle: SearchHandle`, `done`, `started`, `elapsed` |
| `Tab` | 37 | see below |
| `normalize` | 78 (unix) / 93 (win) | `~` expansion (unix), `/`→`\`, `C:`→`C:\`, trim trailing separator |

`Tab` fields: `path` 38, `entries` 39, `pending` 40, `view` 42, `view_dirty` 43, `selection` 45,
`cursor`/`anchor` 46-47, `scroll_to_cursor` 48, `back`/`forward` 49-50, `filter` 51, `lazy` 53,
`loader` 54, `progressive` 55, `load_started`/`load_time` 56-57, `error` 58, `select_after_load` 59,
`rename_after_load` 60-61, `watcher` 62, `watch_rx` 63, `refresh_at` 64, `search` 65,
`free_space` 66, `free_rx` 68, `groups` 70, `collapsed_groups` 71, `cursor_path` 73.

### 2.2 Methods

| Method | Line | Behaviour |
|---|---|---|
| `blank` / `new_lazy` / `ensure_loaded` | ~105 / 145 / 150 | lazy tabs load on first display; `ensure_loaded` takes `path` with `mem::take` |
| `title` | ~157 | `🔍 query` during search, `Giriş`/`Bu bilgisayar`, else file name |
| `is_home` / `is_pc` / `lists_files` | 174 / 178 / 183 | `is_home` = virtual (Home **and** This PC) |
| `navigate` | 191 | push history, cancel search, clear all state, start load/watch |
| `go_up` | 240 | above root → This PC; inside search → exit search |
| `load` | 256 | `progressive` = show batches as they arrive; else stage and swap |
| `watch` | 275 | `notify` non-recursive watcher → channel |
| `start_search` | 297 | build `Pattern`, cancel previous, clear entries/filter, spawn |
| `cancel_search` | 320 | set cancel flag, drop receiver |
| `exit_search` | 326 | cancel; virtual → clear; else reload |
| `swap_in` | 341 | replace entries, remap selection/cursor/anchor **by name** |
| `refresh_free_space` | 364 | thread → `free_rx` |
| `poll` | 375 | drain free/load/watch/search channels each frame |
| `rebuild_view` | 463 | filter → sort (rayon >4000) → group → resolve `select_after_load` |
| `cursor_row` | 518 | cursor index → visible row |
| `selected_entries` / `selected_paths` | 523 / 535 | visible selection, cursor fallback |
| `select_only` / `select_range_to` | 539 / 546 | click / shift-range primitives |

### 2.3 Invariants / traps

1. Selection/cursor/anchor index `entries`, but `selected_entries` filters through `view` → invisible
   selections silently drop out of operations.
2. `swap_in` matches by **name**; search results from multiple roots could collide.
3. Non-progressive reload with an error keeps the old entries and sets `error`; `pending` is discarded.
4. `refresh_at` is not cleared on navigate; an already-armed refresh can fire right after navigation.
5. Group labels come from hardcoded Turkish `DATE_GROUPS` and double as `collapsed_groups` keys.
6. Collapsed groups produce zero-length `Group`s; all share `start == view.len()`.
7. Grouping is a **stable** sort by bucket after the chosen sort → chosen order is preserved inside a bucket,
   buckets always newest→oldest.
8. `Pattern` is built from `filter` once; subsequent typing filters the existing result set.
9. No long-path prefixing.

---

## 3. `src/fsutil.rs` — FS layer, sorting, jobs (1134 lines)

### 3.1 Attributes and `Entry`

- Attribute bits: read-only `0x1`, hidden `0x2`, system `0x4`, directory `0x10`, reparse `0x400`
  (`F:17-21`). Windows uses `MetadataExt::file_attributes`; Unix synthesizes reparse for symlinks, dir flag
  from followed metadata, hidden from a leading dot (`F:26-46`).
- Hidden test = hidden **or** system (`F:49-51`).
- `Entry` (54-68): `name`, `name_lower`, `path`, `is_dir`, `size` (0 for dirs), `modified`, `created`,
  `hidden`, `ext` (lowercase, empty for dirs), `plain_dir` (ordinary folder → shared standard icon).
  `from_metadata` (71-95) uses attributes for `is_dir` (so junctions count), silently drops missing times,
  and sets `plain_dir` only on Windows for dirs without read-only/system/reparse.

### 3.2 Sorting

- `natural_cmp` (112-145): allocation-free byte comparison; digit runs compared by length after leading
  zeros, then lexicographically → `dosya2 < dosya10`; non-digits by UTF-8 byte (= code point) order.
- `compare` (147-164): dirs first for every key **except** Modified; tie-break natural name for
  Size/Modified/Type; descending reverses the whole ordering (so dirs move last).
- `SortKey` (101-107): Name (default) / Modified / Type / Size, serde-serializable.

### 3.3 Filtering

`Pattern` (170-219): trimmed lowercased query. If it contains `*`/`?`, it becomes a char glob (spaces
literal, classic two-pointer matcher). Otherwise whitespace-separated words must **all** be substrings.
Matches only `name_lower`, never paths.

### 3.4 Listing

- `LoadMsg` (316): `Batch(Vec<Entry>)`, `Done`, `Error`.
- `spawn_load` (324) / `prefetch` (336) / `take_prefetch` (341) / `spawn_load_with` (349):
  one-slot global prefetch; batches of ≤8192 or 60 ms; repaint per batch; skipped metadata errors.
- Hidden entries are always loaded; filtering happens in `rebuild_view`.

### 3.5 Search

- `SearchHandle` (388): `rx`, `cancel: Arc<AtomicBool>`, `scanned: Arc<AtomicU64>`.
- `spawn_search` (394): outer thread; `roots.par_iter()`; nested `subdirs.par_iter()` recursion; one batch
  per directory; `Done` after all roots.
- `search_dir` (407-432): cancel check, skip unreadable dirs, prune `DIR && !REPARSE` subdirs, match names,
  `from_metadata`; send failure = global cancel.
- No depth/result limits, no hidden filtering, no network pre-check.

### 3.6 Jobs

| Type | Line | Notes |
|---|---|---|
| `JobKind` | 563 | `Transfer`, `Delete`, `Size`, `Restore` |
| `Conflict` | 572 | `KeepBoth`, `Replace`, `Skip` |
| `Job` | 578 | `label`, `kind`, `cut`, `permanent`, `started`, `total/done/files: AtomicU64`, `finished`, `syncing`, `cancel`, `current: Mutex<String>`, `errors: Mutex<Vec<String>>`, `pairs: Mutex<Vec<(src,dst)>>` |
| `RUNNING` / `jobs_running` | 598 / 600 | self-update waits for zero |
| `unique_destination` | 641 | `name`, `name - Copy`, `name - Copy (2)`… (TOCTOU by design) |
| `tree_size` | 665 | symlink-aware; dirs via `dir_size` |
| `copy_tree` | 674 | skips reparse dirs; `fs::copy` <32 MiB; 4 MiB buffered loop ≥32 MiB with cancel cleanup; restores mtime; records written files |
| `sync_to_disk` | 729/758 | Windows: parallel `sync_all` per file (read-only temporarily cleared); Unix: `syncfs(dest)` |
| `spawn_transfer` | 771 | rename fast path, `total` pre-walk, copy, sync, delete sources only after sync, record pairs |
| `conflicts` | 848 | names present in dest, excluding same-folder pastes |
| `restore_from_trash` | 858 | match parent + name **or stem**, newest `time_deleted` |
| `spawn_delete` | 884 | permanent per-item (cancellable), Recycle Bin = `trash::delete_all` (atomic, no cancel) |
| `spawn_sizes` | 915 | sequential dirs, rayon inside; skips publishing canceled results |
| `spawn_restore` | 936 | wrapper |

Transfer pipeline (details in [`workflows.md#10-copy--move-job`](workflows.md#10-copy--move-job)):

1. lexical self-copy guard (`dest.starts_with(src)`);
2. cut into same dir = no-op;
3. target naming per conflict choice;
4. same-volume cut via `fs::rename` (instant);
5. copy remaining, tracking bytes;
6. `syncing = true` → flush;
7. cut sources deleted only if flush succeeded;
8. undo `pairs` recorded per completed source.

### 3.7 Formatting / preview / tests

- `format_size` (223-236): binary divisors but SI-looking labels, comma decimal.
- `format_time` (238-243): `%d.%m.%Y %H:%M` local.
- `DATE_GROUPS` (246): Turkish-only bucket names; `date_group` (249-279) maps to 8 buckets
  (today/future, yesterday, earlier this week, last week, earlier this month, last month, earlier this
  year, long ago). Buckets 4-6 are effectively unreachable for 8-13-day-old items.
- `kind_label` (281): `Klasör` / `Dosya` / `{EXT} dosyası`.
- `has_thumbnail` (950): images, video, PDF, HEIC/AVIF/TIFF/PSD/RAW, audio — i.e. whatever the shell can
  thumbnail.
- `read_preview` (967-987): 64 KiB, UTF-16LE BOM supported, NUL → binary, else UTF-8 lossy.
- Tests (989-1134, Unix-only ones are `cfg`-gated): Unix mode strings, tree stats, natural sort equivalence
  (3k + 15k names), copy with read-only file + `syncing` flag. `shell.rs` also has
  `shell_icons_have_valid_premultiplied_alpha`. `update::tests::install_swaps_the_exe` is `#[ignore]`d
  (network).

---

## 4. `src/images.rs` — texture cache (317 lines)

| Item | Line | Notes |
|---|---|---|
| `Img` | 10 | `Pending`, `Ready(TextureHandle)`, `Failed` |
| `Req` | 17 | `key` (empty = startup prefetch), `path`, `size`, `icon_only` |
| `Early` / `EarlyKey` | 25-31 | startup warm-up table |
| `WORKERS` (OnceLock), `EARLY`, `UI_CTX` | 42-46 | 4 threads, warm-up map, repaint target |
| `start` / `prefetch` / `end_prefetch` | 123 / 129 / 149 | warm-up handoff; unclaimed results dropped |
| `ImageCache::new` | ~186 | **panics** if constructed twice |
| `get` | 199 | non-blocking; miss inserts `Pending` + queues |
| `busy` / `drop_pending` | 236 / 242 | reveal gate / navigation purge |
| `poll` | 255 | drains results, increments frame |
| `texture_for` | 269 | `ahash` pixel digest dedupe, uploads otherwise |
| `evict` / `prune_unique` | 286 / 304 | evict oldest in chunks of 64 down to 75 % of 160 MiB |

- Workers pop from the **tail** (LIFO) so visible rows win during scrolling (`I:83-91`).
- `produce` retries 3× with 30 ms sleeps; second retry falls back to `icon_only` (`I:54-70`).
- No cancellation, no disk cache, no key invalidation on file change.
- Known trap: a stuck `Pending` slot is never retried/evicted; `busy()` can stay true and the icon blank.

---

## 5. Platform modules

### 5.1 `src/shell.rs` (Windows, 490) / `src/shell_unix.rs` (Linux, 170)

Contract table: `architecture.md`, section "10.1 shell".

Windows implementation notes:

- Icons: `SHCreateItemFromParsingName` → `IShellItemImageFactory::GetImage` (`SIIGBF_BIGGERSIZEOK`,
  optional `ICONONLY`) → HBITMAP → `GetDIBits` 32-bit top-down BGRA → RGBA; JPEG alpha forced opaque,
  straight alpha premultiplied in gamma space (`S:49-121`).
- Worker threads call `CoInitializeEx(COINIT_APARTMENTTHREADED)` (`S:142-146`).
- Cloak: `WH_CBT` hook matches the top-level window titled exactly `Swift Explorer`; subclass id `0x5345`;
  uncloak via `DWMWA_CLOAK` (`S:184-269`).
- Resize: `WM_NCHITTEST` subclass id `0x4847`, 6 px × DPI band, disabled when maximized (`S:273-334`).
- Native menu: PIDLs → `IShellItemArray` → `IContextMenu`; thread-local `ACTIVE_MENU`; message forwarding
  for owner-draw; `rename` verb intercepted (`S:343-387`).
- Drag-out: custom `IDropSource`, `DoDragDrop(COPY|MOVE|LINK)`; blocks until drop (`S:391-421`).
- Accent: `DwmGetColorizationColor` (`S:438-445`).

Linux implementation notes:

- Theme order: GTK default, Adwaita, breeze, Papirus, hicolor; sizes clamped 16-512 (`shell_unix.rs:17-42`).
- SVG via `resvg`/tiny-skia; raster via `image` + `thumbnail` (`:44-66`).
- Special names: `THIS_PC` → `computer`, `::…` → `user-home`, `/` → `drive-harddisk`, known dirs,
  mount points → `drive-removable-media` (`:68-105`).
- Thumbnails only for png/jpg/jpeg/gif/webp/bmp, no cache dir, no video/PDF (`:107-112`).
- File icons by MIME → exact type → category → `unknown` (`:113-126`).

### 5.2 `src/win.rs` (Windows, 472) / `src/win_unix.rs` (Linux, 325)

Contract table: `architecture.md`, section "10.2 win".

Windows:

- Drives: `GetLogicalDrives`, `GetDriveTypeW`, volume label → `"Label (C:)"`, free/total (`W:31-82`).
- Clipboard: `clipboard-win` CF_HDROP + `Preferred DropEffect` (`W:169-209`).
- Single instance: pipe `\\.\pipe\SwiftExplorer-<session>-<USERNAME>`; `FILE_FLAG_FIRST_PIPE_INSTANCE`;
  message = u32 byte length + UTF-16LE path; reply byte 1 = accepted; `AllowSetForegroundWindow(ASFW_ANY)`
  (`W:236-340`).
- Autostart: HKCU `…\Run\SwiftExplorer = "<exe>" --background`; existence-only check (`W:363-385`).
- Default FM: HKCU `Directory\shell`, `Drive\shell`, Win+E CLSID; uninstall is destructive and does not
  restore the previous handler (`W:389-469`).
- Properties: `ShellExecuteExW` verb `properties` with `SEE_MASK_INVOKEIDLIST` (`W:93-105`).

Linux:

- Mounts + `drive_mask` hash + `statvfs` (`win_unix.rs:35-84`).
- `xdg-mime` for MIME/default app; `.desktop` scan in `desktop_apps.rs`.
- FileManager1 D-Bus via `dbus-send`; URI encoder escapes outside `[A-Za-z0-9/-_.~]`.
- Flatpak: `/.flatpak-info` → all host tools through `flatpak-spawn --host`.
- Terminals: `$TERMINAL`, then `x-terminal-emulator`, `gnome-terminal`, `kgx`, `ptyxis`, `konsole`,
  `xfce4-terminal`, `kitty`, `alacritty`, `foot`, `wezterm`, `xterm`.
- Clipboard: newline-separated absolute paths; any non-absolute/missing path invalidates the whole list.

### 5.3 `src/properties.rs` (Linux only, 869)

- Separate viewport (`show_viewport_immediate`) with embedded-window fallback; tabs General / Digital
  Signatures / Security / Details / Previous Versions (`properties.rs:38-45`).
- General: icon, inline rename, type, "opens with" + chooser, location, size/on-disk/contains (background
  `TreeStats`, cancellable), timestamps, read-only + hidden (`:430-500`).
- Security: owner/group, rwx grid, octal + mode string; read-only toggles write bits (`:556-607`).
- Apply: single selection sets exact mode; multi-selection flips write bits only (`:293-313`); rename uses
  `app::valid_name` so Windows-invalid characters are rejected on Linux too.
- `Drop` cancels the size scan (`:99-105`).

### 5.4 `src/desktop_apps.rs` (Linux, 162)

Scans XDG data dirs + Flatpak exports, dedupes by id, skips `Hidden=true`, prefers localized `Name[xx]`,
separates recommended vs other by `MimeType`, sets default via `xdg-mime default`.

---

## 6. `src/update.rs` (273)

| Item | Line | Notes |
|---|---|---|
| manifest URL | 11-12 | `releases/latest/download/latest.json` (no API rate limits) |
| `Found` | 18-23 | `Installed(version)` / `Available(version, page)` |
| version compare | 38-45 | strips `v`, numeric components only; pre-releases ignored |
| `enabled` | 52-61 | disabled for Flatpak (`FLATPAK_ID`), Scoop (`install.json`/`manifest.json` beside exe), debug/`target/` exes |
| `.skip` check | ~93 | suppresses a rolled-back version |
| `install` | 172-209 | download `.new`, SHA-256 stream check, `sync_all`, rename swap, write `.pending` |
| `remove_old` | 112-120 | only when no `.pending` |
| `startup_guard` / `roll_back_if_failing` | 127 / 135 | 2 failed starts → restore `.old`, write `.skip`, relaunch |
| `confirm_started` | 161-170 | once per process |
| `launch_hidden` | 212 | spawns current exe with `--after-update` |

Limits: manifest ≤64 KiB, exe ≤256 MiB, 10 s connect / 30 s read timeouts, no signature, no arch check.

---

## 7. `src/i18n.rs` (523) and `src/scheme.rs` (85)

### i18n

- `LANGS` (20-30) order: `tr, en, de, es, fr, pt, ru, zh, ja`; `EN = 1`; `LANG: AtomicUsize`.
- `lang_index` (46-52) prefix-matches lowercased codes; rejects Traditional Chinese; `zh-CN`/`zh-SG` map to
  Simplified.
- `tables()` (79-85) is positionally aligned with `LANGS`; index 0 (Turkish) is an empty map because `t()`
  returns the key itself for Turkish.
- `t()` (88-95): language 0 → key; else table → English → key.
- `tf()` (98-129): `{}`, `{0}`, `{{`; `singular()` (131-160) applies literal replacements for
  en/de/es/fr/pt only.
- Tests (495-503): every language from index 2 must have **exactly** the English key set (set equality, not
  subset).

### scheme

- `Scheme` (8-25): `name`, `dark`, `crust` (tab strip), `mantle` (nav/pane), `base` (content),
  `surface` (pops), `border`, `disabled`, `weak`, `text`, `accent`.
- `SCHEMES` (48-61): 12 schemes; `FIRST = 3`; `ACTIVE: AtomicU8`.
- `get` (68), `set_active` (73), `active` (77), `mix` (82).

---

## 8. `src/screenshot.rs` and `build.rs`

- Feature `screenshot`; env vars `SWIFT_EXPLORER_SCREENSHOT` (BMP path, required), `_DELAY` (default 4 s),
  `_SIZE` (`WxH`), `_INPUT` (`click:x,y;key:F1`, every 500 ms from 1.5 s). Writes a 32-bit top-down BMP and
  closes (`screenshot.rs:1-8`, `74-122`).
- `build.rs`: Windows only; icon `assets/icon.ico`; for `CARGO_CFG_TARGET_ENV == "gnu"` (also gnullvm)
  embeds `packaging/windows/swift-explorer.manifest` (asInvoker, Common Controls 6, supportedOS GUIDs).
  MSVC relies on the linker manifest; without one `SetWindowSubclass` fails.

---

## 9. Known bugs & traps

Ordered by risk. `*` = confirmed in code during this review; the rest are strong inferences from the code
paths (verify before filing).

| # | Issue | Evidence |
|---|---|---|
| 1 | **No long-path support** (`\\?\` absent repo-wide); >260 chars fails with raw OS errors | `T:93-102`, `F` copy/read paths |
| 2 | Recycle-Bin delete is all-or-nothing, non-cancellable, no undo pairs on failure | `F:903-908` |
| 3 | Stuck `Pending` icon slot: never retried/evicted, `busy()` may never clear | `I:199-211`, `236-238` |
| 4 | `Ctrl+H` does not invalidate `subdirs` cache (View-menu toggle does) → stale sidebar | `A:1528-1533` vs `2596-2599` |
| 5 | `is_home()` covers This PC, so New/Paste/Delete/terminal guards also fire there | `T:174-176` |
| 6 | `selected_entries()` cursor fallback + `view` filtering: invisible selections are silently skipped | `T:523-533` |
| 7 | Search: no depth/result cap, traverses hidden/system trees, unreadable dirs invisible | `F:407-432` |
| 8 | Self-copy guard is lexical (`starts_with`) — case/symlink variants bypass it | `F:781` |
| 9 | Sync failure skips undo recording for completed cut copies | `F:828-839` |
| 10 | Same-volume move progress never advances `done` (rename path) | `F:803-806` |
| 11 | Transfer pre-walk (`tree_size`) doubles metadata cost before copying | `F:810` |
| 12 | Free space is shown but never checked before copying | `F:771-845`, `W:85` |
| 13 | Date buckets are hardcoded Turkish, used as collapse keys; 8-13 day olds mis-bucket | `F:246`, `249-279` |
| 14 | Native menu Enter on permanent-delete dialog confirms by keyboard | `A:4115` |
| 15 | `restore_from_trash` matches by stem → can restore the wrong generation | `F:858-882` |
| 16 | Preview slot is global; late thread can overwrite current selection | `A:3910-3940` |
| 17 | `display_name` byte-slices by `ext.len()`; non-ASCII case folding can mis-slice | `A:1023-1029` |
| 18 | Reserved names (`CON`, `NUL`, trailing dot/space) pass `valid_name` | `A:1031-1035` |
| 19 | `App` writes config from several call sites without locking; two instances can race | `A:1177-1209` |
| 20 | Update manifest has one `windows.url`, no arch check; unsigned, SHA-256 from the same host | `U:25-36`, `172-209` |
| 21 | Autostart value only checked for existence, not path | `W:366-370` |
| 22 | Windows default-FM uninstall deletes Win+E CLSID tree and does not restore previous handler | `W:457-469` |
| 23 | `Makefile build-local` omits `.exe` on Windows | `Makefile:74-77` |
| 24 | Group scan is O(n log n) chrono conversions + rescan; expensive on huge folders | `T:482-497` |
| 25 | `ImageCache` single-instance `expect` and unchecked byte accounting | `I:187`, `315` |

---

## 10. Recipes

### Recipe: add a command

1. Add a variant to `Action` (`A:229-267`).
2. Push it from `command_bar` (`A:2486`), `item_menu` (`A:3149`) and/or `background_menu` (`A:3233`).
3. Handle it in `apply` (`A:1691`). Use `self.tabs[self.active]`, `selected_paths()`, and
   `push_job(..., undoable)` for long work.
4. If it should be undoable, extend `UndoOp` + `do_undo` (`A:1392`) and the job-finish mapping
   (`A:4392-4399`).
5. Add shortcut in `handle_keys` if needed; add a Help row in `HELP` (`A:4236`).
6. Add Turkish text + English table entry + all 7 language files; `cargo test`.

### Recipe: add a keyboard shortcut

Use `shortcut(i, Modifiers::CTRL | …, Key::X)` inside `ctx.input_mut` (it removes the event and enforces
exact modifiers). Put it in the "always" block (`A:1484-1548`) if it must work while typing, otherwise after
the `if typing || home { return }` guard (`A:1550`).

### Recipe: add a file operation job

1. `JobKind` variant if it isn't a transfer/delete (`F:563`).
2. `pub fn spawn_xxx(...) -> Arc<Job>` that spawns a thread, updates `Job` atomics, honours
   `job.cancel`, and records `pairs` where applicable (`F:578-595`).
3. Call `job.finish()` and `ctx.request_repaint()`.
4. UI: `self.push_job(job, undoable)`, progress is automatic; add toast mapping in `A:4400-4412`.
5. Keep the durability rule: don't delete sources before `sync_to_disk` succeeded.

### Recipe: add a view mode

`VIEW_LEVELS` (`A:21`) drives the menu, `Ctrl+Shift+1..6`, and `view_level`. `file_area` (`A:3362`) chooses
details vs grid; add a branch for a new layout, keep `grid_cols` updated for keyboard navigation, and handle
`item_rects` for marquee selection.

### Recipe: add a language

1. `cp src/i18n/de.rs src/i18n/xx.rs`, translate the second string of every pair (keys must stay identical).
2. `mod xx;` in `i18n.rs:5-11` (alphabetical).
3. Append `("xx", "Native name")` to `LANGS` (`i18n.rs:20-30`).
4. Append `xx::TABLE` **at the same position** in `tables()` (`i18n.rs:82`).
5. Add the Windows LANGID mapping in `system_lang()` (`i18n.rs:65-75`).
6. If the language needs plural forms, add literal rules to `singular()` (`i18n.rs:131-160`), otherwise
   phrase strings to avoid plurals.
7. `cargo test` — `every_language_has_the_same_keys` must pass.

### Recipe: add a theme

Append a `s()` entry to `scheme::SCHEMES` (`scheme.rs:48-61`) — never insert. It appears in View → Theme
automatically (`A:2636-2649`); `theme = index + 3` is persisted.

### Recipe: add a platform function

1. Add to `shell.rs` and `shell_unix.rs` (or `win.rs` / `win_unix.rs`) with the **same name**, and the same
   parameters order/type. Return values may differ in platform-specific ways (see contract table) but callers
   must compile on both.
2. If one side is genuinely impossible, add a stub (`None`/`false`/no-op) — `#[cfg]` is only acceptable when
   every call site is also gated.
3. Update the contract tables in `architecture.md`.

### Recipe: reset local state while debugging

- Settings: delete `%APPDATA%\SwiftExplorer\config.json` (or set a different `APPDATA` for a scratch run).
- Window geometry / first-run behaviour: delete `app.ron` from eframe's storage dir for `Swift Explorer`.
- Update markers: `.new`, `.old`, `.bad`, `.pending`, `.skip` next to the exe; delete `.pending` first if a
  rollback loop is suspected.
- Single instance: make sure no hidden process is parked (`Stop-Process -Name swift-explorer`).
- Screenshots: `cargo run --features screenshot` with `SWIFT_EXPLORER_SCREENSHOT=…`.

---

## 11. Debugging notes

- Debug builds keep `eprintln!`; release builds (`windows_subsystem = "windows"`) discard them. Update and
  job errors are surfaced as toasts, not logs.
- Config is written every 3 s; if the app crashes, at most ~3 s of settings are lost.
- The UI only repaints on demand. If a new background task "does nothing", it is usually missing
  `ctx.request_repaint()`.
- Every modal blocks `handle_keys`; when shortcuts stop working, check `self.dialog`.
- When the window is invisible on Windows, check the cloak path: HWND captured? `revealed`?
  `images.busy()`? (`A:1256-1286`).
- `App::update` order is load-bearing: polling before `rebuild_view`, keys before panels, actions last.

---

## 12. Parity modules (2026 additions)

Line numbers in sections 1-11 are from 0.7.0; these modules were added after that pass.

### 12.1 `src/menu.rs` — modern context menu

- `Node::new(label)` builds an item; builders `with_icon`, `with_colored_icon`, `with_mock`,
  `with_cmd`, `shortcut`, `disabled`, `checked`, `sep_before`; `children` makes a submenu.
- `Menu::new(pos)` holds the root; `menu::show(ctx, &mut menu)` draws it as an `egui::Area` and returns
  `Outcome::Command(Cmd)`, `Close`, or `None`. `Cmd` = `App(usize)` (index into the action table built by
  `item_menu`/`background_menu`), `Shell(verb)`, or `More`.
- `menu::parts("label  Shift+F10")` splits the trailing shortcut for right alignment.
- `menu::shell_node(&ShellMenuItem, ctx, path)` converts a shell verb into a `Node`; IDs are stable strings
  so icon/submenu caches survive frames.
- Quick-action strip (`quick_row`) is the icon row at the bottom (copy path, open, …), Explorer style.
- `mock_view_icon` draws the tiny view diagrams (no glyph dependency): arms must all return `()`
  (use blocks; `rect_filled` returns `ShapeIdx`).
- `rect_stroke` in egui 0.31 needs `StrokeKind`; `CornerRadius::same(u8)`; float literals need `_f32`
  when inference is ambiguous.

### 12.2 `src/system.rs` — Windows integration

- `text_scale()` reads the accessibility text-size setting; `App::new` applies `ctx.set_zoom_factor`.
- `animations_enabled()` reads the "Show animations" setting; gates hover/scroll animation.
- `set_hwnd(hwnd)` stores the window; `apply_backdrop(hwnd)` sets rounded corners +
  `DWMWA_USE_IMMERSIVE_DARK_MODE` (Mica is deliberately absent — the glow surface is opaque).
- `aumid()`/`set_aumid` support taskbar identity; `dpi` helpers feed `px_bucket`.
- `set_hwnd` must be called **once** on the first frame, before `apply_backdrop`.

### 12.3 `src/wsl.rs` — Linux locations

- `distros()` → `Vec<Distro { name, root }>`; runs `wsl.exe --list --quiet` with `WSL_UTF8=1`, falls back
  to UTF-16LE, trims `\r`/NULs, filters `docker-desktop*`, caches for 10 s.
- Roots are `\\wsl.localhost\<name>`; hidden when the list is empty or on non-Windows.
- Sidebar rows reuse `nav_row`; do not add a separate listing path.

### 12.4 Gallery internals

- `tab::GALLERY` = Explorer's Gallery CLSID (icon comes from the shell). The correct GUID ends in
  `...417AA8`; `...417A2A` is invalid and silently yields a folder icon.
- `fsutil::spawn_gallery` walks `dirs::picture_dir()` up to depth 6, skips hidden + reparse dirs, keeps
  png/jpg/jpeg/gif/bmp/webp/tif/tiff/heic/heif/avif/jxl and mp4/mov/m4v/avi/mkv/wmv/webm/3gp/mpg/mpeg,
  sorts modified-desc, caps 5000.
- `Tab::rebuild_view` overrides sort to `Modified` desc and groups by `%B %Y` (English month names today).
- `is_home()` stays true for Gallery (read-only guards); `is_gallery()` is the exception in
  `navigate`, `load`, `go_up`, `preview_panel` and the central-panel dispatch.
- `images::decode_photo` is the local fallback for still images the shell cannot thumbnail (offline
  OneDrive placeholders, unusual PNGs); it is capped at 48 MB and only runs on non-icon requests.

### New traps

- `Tab::is_home()` is true for Home, This PC **and Gallery**; check `is_gallery()` before any
  "virtual ⇒ no files" assumption.
- `preview_panel` computes `home = !lists_files() && !is_gallery()`; dropping the second term brings back
  "select an item" on Gallery.
- Gallery ignores the Sort menu by design; changing sort keys there has no effect.
- `Action::Navigate(GALLERY)` forces `view_level = 3` (Large icons); other tabs keep their view.
- `cargo fmt --all` is **not** run on this fork: it reflows the whole tree and makes upstream merges noisy.

