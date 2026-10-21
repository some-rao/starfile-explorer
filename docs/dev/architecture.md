# Architecture — Swift Explorer

Snapshot: `v0.7.0`, commit `850c108`, fork `some-rao/swift-explorer` (upstream
`misutesu-desu/swift-explorer`). All line references are relative to the repo root at that commit.

Companion docs: [`workflows.md`](workflows.md) (end-to-end dataflow),
[`low-level-design.md`](low-level-design.md) (per-file API reference and traps).

---

## 1. What the program is

A single-process, single-window desktop file manager for **Windows** (Win11 look, Win32 shell integration)
and **Linux** (freedesktop port). Written in Rust with **egui 0.31 / eframe 0.31** — immediate-mode UI, GL
rendering, no web view, no Tauri/Electron.

| Property | Value |
|---|---|
| Crate | `swift-explorer` 0.7.0, edition 2021, MSRV 1.82 (`Cargo.toml:1-9`) |
| UI | `eframe`/`egui`/`egui_extras` 0.31 with `persistence` |
| Concurrency | `std::thread` + `mpsc`, `rayon` for bulk sort/search/size |
| FS | `std::fs`, `trash` (Recycle Bin/XDG trash), `notify` (watcher) |
| Net | `ureq` + `sha2` (self-update only) |
| Platform | `windows`/`windows-sys`/`clipboard-win` on Windows; `libc`, `freedesktop-icons`, `resvg`, `image` on Linux |
| Binary | one static exe; release profile `opt-level=3`, `lto="thin"`, `codegen-units=1`, `strip` (`Cargo.toml:48-52`) |

Non-obvious decisions that shape everything:

1. **No framework/state library.** `App` is a plain struct holding all state; the UI is a set of
   `fn(&mut self, ui, &mut Vec<Action>)` methods. Mutations go through an action queue.
2. **No async runtime.** Threads + channels; every frame polls what finished.
3. **Virtual locations are sentinel paths**, not a separate enum: `""` = Home, `"::{20D04FE0-…}"` =
   This PC, `"::{24AD3AD4-A569-4530-98E1-AB02F9417AA8}"` = Gallery (`tab.rs:14-24`).
   `is_home()` covers all three; `is_gallery()` is the exception that still lists files.
4. **Native shell is used for heavy lifting on Windows**: icons, thumbnails, type names, context menu,
   OLE drag-out, clipboard, properties sheet. This keeps the exe small and native-feeling.
5. **Settings are one JSON file** rewritten wholesale (not eframe storage).

---

## 2. Process model

```
swift-explorer.exe [path]
   │
   ├─ Windows: is another instance serving a named pipe?
   │     ├─ yes + hidden (parked) window → forward path, this process exits   (win.rs:245-254)
   │     ├─ yes + window visible         → server replies 0, this process opens its own window
   │     └─ no                           → become the pipe server             (win.rs:283-308)
   ├─ startup_guard()  → roll back a broken self-update (update.rs:127-158)
   ├─ prefetch_startup() → warm first folder + icons before the window exists (app.rs:168-217)
   └─ eframe::run_native → App::new → per-frame App::update
```

- **Windows** keeps a resident process alive after the window is closed when View → "Wait in the
  background" is on (`App.resident && background`): close is cancelled and the window is hidden
  (`app.rs:4331-4335`, `win.rs:343-346`). A later launch is forwarded over `\\.\pipe\SwiftExplorer-<session>-<user>`
  (`win.rs:236-241`) and the hidden window is re-shown (`win.rs:311-340`).
- **Linux** has no resident mode: `win_unix::serve` returns `false` (`win_unix.rs:278-280`), so every launch
  is a new process.
- The **update checker** runs in the resident process only on Windows (`app.rs:1085-1087`), so exactly one
  process polls/downloads.

---

## 3. Module map

| File | Lines | Role | Compiled when |
|---|---|---|---|---|
| `src/main.rs` | 87 | Entry, args (`--background`, `--after-update`, `--uninstall-cleanup`), single-instance handoff, eframe options | always |
| `src/app.rs` | ~6740 | `App` state, every panel/view/menu/dialog, action dispatch, shortcuts, theme, fonts, update polling, Gallery/WSL sidebar | always |
| `src/tab.rs` | ~620 | `Tab`: path, entries, filtered view, selection, history, loader, watcher, search, groups; virtual `GALLERY` page | always |
| `src/fsutil.rs` | ~1330 | `Entry`, sorting, wildcards, listing loader, search, Gallery scan, jobs (copy/move/delete/restore/size), formatting, preview read | always |
| `src/images.rs` | ~390 | Icon/thumbnail worker pool + texture cache (keys, LRU, dedupe, startup warm-up); local image decode fallback | always |
| `src/menu.rs` | ~660 | Windows 11 style modern context menu renderer (shell verb tree, Fluent icons, submenus, quick actions) | always |
| `src/system.rs` | ~120 | Windows text scale, animation setting, DPI awareness, AUMID, DWM rounded corners/immersive dark frame | always |
| `src/wsl.rs` | ~75 | WSL distro discovery (`wsl.exe --list --quiet`, UTF-16LE fallback, 10 s cache) | Windows (stub elsewhere) |
| `src/newfile.rs` | ~370 | "New" submenu document templates (other contributors' feature) | always |
| `src/shell.rs` | ~940 | Win32: `IShellItemImageFactory` icons, shell verb tree, type names, native context menu, OLE drag-out, DWM cloak, resize subclass | Windows |
| `src/shell_unix.rs` | ~250 | freedesktop icons (theme lookup, SVG via resvg, `image` thumbnails), type names, stubs | non-Windows |
| `src/win.rs` | ~630 | Win32: drives, clipboard (CF_HDROP), single instance, autostart, default file manager, properties, terminals, DWM accent | Windows |
| `src/win_unix.rs` | ~310 | mounts, `statvfs`, xdg-mime, FileManager1 D-Bus, terminals, Flatpak `flatpak-spawn` | non-Windows |
| `src/properties.rs` | ~1050 | Own Properties window: General/Security/Details tabs, permissions, size scan, default-app chooser | non-Windows |
| `src/desktop_apps.rs` | ~200 | `.desktop` scanning, localized names, MIME associations | non-Windows |
| `src/update.rs` | ~280 | `latest.json` fetch, SHA-256, exe swap, `.pending` rollback, `launch_hidden` | always |
| `src/i18n.rs` | ~570 | Turkish-key tables, `LANGS`, `t()`, `tf()`, plural heuristics, language detection | always |
| `src/scheme.rs` | ~210 | 12 colour schemes + process-global active scheme | always |
| `src/screenshot.rs` | ~160 | Feature-gated self-screenshot/input harness (`click:`, `hover:`, `rclick:`) | `feature = "screenshot"` |
| `build.rs` | 13 | Embeds icon/manifest for GNU Windows targets via `winresource` | build script |
| `packaging/windows/swift-explorer.iss` | — | Inno Setup per-user installer/uninstaller (Start Menu, optional desktop shortcut, AUMID) | installer only |
| `src/i18n/{de,es,fr,pt,ru,zh,ja}.rs` | ~20 KB each | One table per language, key = Turkish source string | always |

`main.rs` wires platform files by *path aliasing*, not traits:

```rust
#[cfg_attr(not(windows), path = "shell_unix.rs")] mod shell;
#[cfg_attr(not(windows), path = "win_unix.rs")]   mod win;
```
(`src/main.rs:16-19`)

---

## 4. Layering and dependencies

Logical layers (top depends on bottom; there are no strict crate boundaries):

```
┌──────────────────────────────────────────────┐
│ main.rs        process, lifecycle, eframe     │
├──────────────────────────────────────────────┤
│ app.rs         UI + application state         │
├───────────────┬───────────────┬──────────────┤
│ tab.rs        │ images.rs     │ update.rs    │
│ per-tab state │ texture cache │ self-update  │
├───────────────┴───────┬───────┴──────────────┤
│ fsutil.rs             │ shell.rs / unix      │
│ FS, jobs, search      │ platform icons/menus │
├───────────────────────┴──────────────────────┤
│ win.rs / win_unix.rs, scheme.rs, i18n.rs      │
│ OS services, palettes, strings                │
└──────────────────────────────────────────────┘
```

Real dependency edges (verified from `use` statements):

- `app` → `tab`, `fsutil`, `images`/`shell`, `win`, `update`, `scheme`, `i18n`, `properties` (Linux only),
  `screenshot` (feature).
- `images` → `shell` (`shell_image`, `init_worker_thread`) — so Linux image decoding lives behind the same
  `shell::shell_image` API (`images.rs:58,80`).
- `shell_unix` → `win_unix` (`mount_roots`), `tab` (`THIS_PC`), `i18n`.
- `win_unix` → `properties` (`AppInfo`), `desktop_apps`.
- `properties` → `win_unix` (mime/default app), `shell` (app icon), `desktop_apps`.
- `win.rs` → `shell::set_cloak` only; `shell.rs` never calls `win`.
- `tab` → `fsutil`, `notify`; `fsutil` → `rayon`, `trash`, `libc` (Unix).

There are intentional cycles at the item level (`properties` ↔ `win_unix`) — Rust modules allow it; don't add
new ones if avoidable.

---

## 5. Startup sequence (cold start)

1. **`main`** (`main.rs:27-69`):
   - parse arg 1; `--after-update` waits up to 5 s for the old pipe server to exit (`main.rs:31-39`);
   - `start = arg` (strip a trailing `"` — Explorer passes `"C:\"`), `None` when hidden;
   - hand off to a resident instance (`win::hand_off`); exit if accepted;
   - `update::startup_guard()` (rollback logic);
   - `first_run` = no `app.ron` in eframe storage dir → open maximized (`main.rs:48`);
   - window: borderless, 1060×640, min 640×400, app id `io.github.misutesu_desu.SwiftExplorer`, DnD on;
   - Windows: `shell::begin_startup_cloak()` installs a `WH_CBT` hook to DWM-cloak the window at creation
     (`shell.rs:191-250`), preventing a white flash before the GL surface is ready;
   - `app::prefetch_startup(start)` then `eframe::run_native`.
2. **`prefetch_startup`** (`app.rs:168-217`) starts image workers (`images::start`), loads config, resolves
   the startup tab, calls `fsutil::prefetch` (starts the directory read), then in a thread:
   - `shell::system_scale()` (sets per-monitor-v2 DPI awareness, returns system DPI);
   - reads the folder itself, filters hidden, applies the **same** sort/group rules as `Tab::rebuild_view`;
   - takes the first 60 entries and warms 18 px icons (details) or 32/48/96/256 px icons/thumbnails;
   - warms 16 px icons for known folders and tab locations;
   - `images::prefetch(reqs)`.
   Invariant: if the prefetch ordering diverges from `rebuild_view`, the cache misses (comment `app.rs:183`).
3. **`App::new`** (`app.rs:1070-1161`) in order: fonts → config → accent → language → CJK fonts → theme →
   tabs (`Tab::new` for active, `Tab::new_lazy` otherwise) → pipe server (`win::serve`) → update checker
   thread → struct literal → `refresh_drives()` (thread) → `watch_drives()` (1 Hz poll).
4. **First frames**: HWND capture + subclass + cloak (`app.rs:4309-4320`), reveal gate
   (`reveal_when_ready`, `app.rs:1256-1286`) uncloaks once the list has been drawn and icons are resolved
   (or 350 ms elapsed), then `update::confirm_started()` clears the `.pending` marker.

Consequence: **the drive list is empty on frame 1** and fills in when the drive thread returns; the window
stays cloaked until the first list + icons are ready (Windows).

---

## 6. Frame lifecycle (`impl eframe::App for App`, `app.rs:4298-4526`)

Order matters; it is the de-facto contract of the UI.

| # | Step | Line |
|---|---|---|
| 1 | screenshot tick (feature) | 4300 |
| 2 | non-Windows edge-resize hit test (`BeginResize`) | 4302-4305, 4272-4296 |
| 3 | Windows first frame: store HWND, `install_subclass`, `set_cloak(true)` | 4306-4320 |
| 4 | `win::take_reopen()` → `reopen(start)` (hidden window woken by pipe) | 4322-4324 |
| 5 | `poll_update()` (drain updater channel) | 4325 |
| 6 | close handling: update-installed relaunch; else hide/park | 4326-4336 |
| 7 | `images.poll(ctx)` — drain finished textures | 4339 |
| 8 | `reveal_when_ready`; `update::confirm_started` once revealed | 4340-4343 |
| 9 | ensure active lazy tab loaded; `poll_subdirs`; `Tab::poll` for every tab | 4345-4349 |
| 10 | `rebuild_view` for every `view_dirty` tab using global sort/hidden/group | 4350-4356 |
| 11 | start inline rename for `rename_after_load` (new folder/file) | 4357-4368 |
| 12 | update `recent` (cap 30) | 4369-4375 |
| 13 | collect finished jobs → undo entries, toasts, repaint | 4377-4413 |
| 14 | 100 ms repaint while jobs run | 4414-4416 |
| 15 | autosave config every 3 s | 4417-4421 |
| 16 | refresh drives every 20 s; 20 s heartbeat repaint | 4422-4426 |
| 17 | external OS drop → copy into current folder | 4428-4436 |
| 18 | `handle_keys(ctx, &mut actions)` | 4438 |
| 19 | panels: tabs (34 px) → nav → command → status (23 px) → sidebar → preview → central | 4440-4477 |
| 20 | `dialogs` → `overlays` (jobs/toasts) → `drag_overlay` | 4479-4481 |
| 21 | drag-out to OS when cursor leaves the window with a payload | 4483-4496 |
| 22 | **apply all queued actions** | 4498-4500 |
| 23 | update window title if changed | 4502-4507 |

`raw_input_hook` (`app.rs:4510-4518`) injects synthetic primary/secondary *release* events after a native
menu or OLE drag, because those swallow the real mouse-up. `on_exit` cancels searches and saves config
(`app.rs:4520-4525`).

**Repaint policy** (no continuous repaint): input events; 100 ms while jobs run; 250 ms while toasts are
visible; 20 s heartbeat; `request_repaint()` from worker threads; `request_repaint_after(16..)` for
debounced refreshes. Any new background work must request a repaint itself.

---

## 7. State ownership

### 7.1 `App` (global, `app.rs:297-376`)

| Group | Fields |
|---|---|
| Tabs/view | `tabs: Vec<Tab>`, `active`, `sort: SortKey`, `asc`, `show_hidden`, `show_preview`, `show_ext`, `view_level: usize`, `grid_cols` |
| Places | `bookmarks`, `recent`, `grouped: Vec<PathBuf>` (folders grouped by date), `known` (localized known folders), `tree_open: HashSet<PathBuf>` |
| Lifecycle | `background`, `resident`, `autostart`, `start_hidden`, `revealed`, `frames`, `hwnd` |
| Update | `auto_update: Arc<AtomicBool>`, `update_rx`, `update_wake`, `last_update_check: Arc<AtomicU64>`, `update_installed` |
| File ops | `clipboard: Option<(Vec<PathBuf>, bool)>` (paths, cut), `jobs: Vec<Arc<Job>>`, `silent_jobs: Vec<usize>`, `dir_sizes`, `undo: Vec<UndoOp>` |
| UI transient | `dialog: Dialog`, `address_edit`, `address_focus`, `focus_search`, `type_buf`/`type_time` (type-ahead), `renaming`, `marquee`, `item_rects`, `inject_release`, `toasts` |
| Caches | `images: ImageCache`, `type_names: HashMap<String,String>`, `subdirs: HashMap<PathBuf, Arc<[PathBuf]>>` + pending set + channel, `drives: Arc<Mutex<Vec<DriveInfo>>>` |
| Persistence | `saved_config: String`, `last_autosave` |

### 7.2 `Tab` (per tab, `tab.rs:37-74`)

`path`, `entries: Vec<Entry>` (source of truth), `pending` (staging for non-progressive reload),
`view: Vec<usize>` (filtered+sorted indices into `entries`), `view_dirty`, `selection: HashSet<usize>`,
`cursor`, `anchor` (all **entry indices**), `scroll_to_cursor`, `back`/`forward`, `filter`, `lazy`,
`loader: Option<Receiver<LoadMsg>>`, `progressive`, load timing, `error`, `select_after_load`,
`rename_after_load`, `watcher`, `watch_rx`, `refresh_at`, `search: Option<Search>`, `free_space`,
`free_rx`, `groups: Vec<Group>`, `collapsed_groups: HashSet<String>`, `cursor_path` (This PC).

Why indices: sorting/filtering rebuilds `view` but not `entries`, so selection survives re-sort.

### 7.3 Process-global state

| Global | Where | Used for |
|---|---|---|
| `scheme::ACTIVE: AtomicU8` | `scheme.rs:66` | active palette read by `pal()`/`Chrome::of` |
| `i18n::LANG: AtomicUsize` | `i18n.rs:34` | current language |
| `images::WORKERS`, `EARLY`, `UI_CTX` | `images.rs:42-46` | worker pool, startup handoff, repaint target |
| `fsutil::PREFETCH` | `fsutil.rs:332` | one-slot startup listing handoff |
| `fsutil::RUNNING` | `fsutil.rs:598` | `jobs_running()` gate for update restart |
| `win::PARKED`, `win::REOPEN` | `win.rs:231-233` | hidden window + pending reopen path |
| `shell::ACTIVE_MENU`, `STARTUP_HOOK` | `shell.rs:180-185` | native menu message forwarding, CBT cloak |
| `update` marker files | next to the exe | `.new`, `.old`, `.bad`, `.pending`, `.skip` |

---

## 8. Concurrency model

| Producer | Threads | Delivery | Cancellation |
|---|---|---|---|
| Directory load | 1 per `load()` (`fsutil.rs:349-379`) | `mpsc<LoadMsg::Batch/Done/Error>`, batches ≤8192 or 60 ms | drop receiver → `send` fails → thread exits |
| Search | 1 outer thread + nested `rayon` (`fsutil.rs:394-432`) | `mpsc<SearchMsg::Batch/Done>` | `Arc<AtomicBool>` + drop receiver |
| Directory size | `rayon` inside caller thread (`fsutil.rs:436-455`) | return value / `Job` atomics | `Job.cancel` checked per dir |
| Transfer | 1 thread per job (`fsutil.rs:777-843`) | `Arc<Job>` atomics + mutexes | `Job.cancel` (not while syncing) |
| Delete | 1 thread per job (`fsutil.rs:884-913`) | `Arc<Job>` | permanent: per item; Recycle Bin: none |
| Restore from trash | 1 thread (`fsutil.rs:936-947`) | `Arc<Job>` | none |
| Folder size action | 1 thread (`fsutil.rs:915-934`) | `Arc<Mutex<HashMap>>` + `Job` | `Job.cancel` |
| Icons/thumbnails | 4 workers, LIFO queue (`images.rs:72-117`) | `mpsc<(key, ColorImage)>`, polled each frame | none (queue purge only) |
| Update check | 1 thread (`app.rs:102-149`) | `mpsc<update::Found>` + wake channel | receiver drop |
| Drive list | a thread per refresh (`app.rs:1211-1219`) | `Arc<Mutex<Vec<DriveInfo>>>` | none |
| Drive hot-plug | 1 thread, 1 Hz mask poll (`app.rs:1223-1238`) | mutex + repaint | process lifetime |
| Sidebar subdirs | 1 thread per uncached dir, deduped (`app.rs:1982-2005`) | `mpsc<(path, show_hidden, Vec<PathBuf>)>` | stale results dropped by hidden flag |
| Preview text | 1 thread per selection (`app.rs:3910-3940`) | `Arc<Mutex<Option<(path, PreviewData)>>>` | last-writer-wins |
| Named pipe server | 1 thread (Windows, `win.rs:291-307`) | statics + egui context repaint | process lifetime |

Staleness is handled by **channel identity**, not generation counters: navigating drops the old receiver.
The only generation-like guard is `show_hidden` carried with subdir results.

---

## 9. Persistence

### 9.1 Config JSON (the only app-owned state)

`dirs::config_dir()/SwiftExplorer/config.json` → Windows `%APPDATA%\SwiftExplorer\config.json`
(`app.rs:81-83`). Legacy fallback: `%APPDATA%\HizliGezgin\config.json` (`app.rs:220-222`).
Serialized pretty JSON, rewritten only when the string changes (`app.rs:1202-1206`). Every field is
`#[serde(default)]`, corrupt JSON silently falls back to `Config::default()` (`app.rs:85-91`).

| Field | Type | Meaning |
|---|---|---|
| `bookmarks` | `Vec<PathBuf>` | pinned Quick access entries |
| `tabs`, `active` | `Vec<PathBuf>`, `usize` | restore tab set; `""` = Home, `THIS_PC` GUID = This PC |
| `show_hidden`, `show_preview`, `show_ext` | bool | View toggles |
| `sort`, `asc` | `SortKey`, bool | Name/Modified/Type/Size + direction (global) |
| `view_level` | `usize` | 0 = Details, 1..4 = 32/48/96/256 px icons (`VIEW_LEVELS`, `app.rs:21`) |
| `theme` | `u8` | 0 system, 1 light, 2 dark, ≥3 → `SCHEMES[theme-3]` |
| `lang` | `u8` | 0 system, otherwise `LANGS[lang-1]` |
| `recent` | `Vec<PathBuf>` | recent folders, cap 30 |
| `grouped` | `Vec<PathBuf>` | folders grouped by date (default: Downloads) |
| `background` | bool | keep process resident after close (Windows) |
| `auto_update` | bool | update checker |
| `last_update_check` | u64 | unix seconds, for the hourly schedule |
| `previous_folder_app` | `Option<String>` | Linux: restore previous default file manager |
| `tips_dismissed` | bool | first-run tips |

Writes: every 3 s autosave, on park/close, on bookmark add/remove, `on_exit`.
`process::exit()` paths (update relaunch) rely on the 3 s autosave.

### 9.2 `app.ron` (eframe-owned)

eframe's `FileStorage` writes `app.ron` under `eframe::storage_dir("Swift Explorer")` with the window
settings (`STORAGE_WINDOW_KEY = "window"`) and egui memory. The app never calls `get_value`/`set_value`,
and `App` does not override `save()`. Window geometry restore is entirely eframe's.
Its existence is used as the "not first run → don't force maximize" heuristic (`main.rs:48`).

### 9.3 Runtime-only state (lost on exit)

Undo stack, clipboard snapshot, jobs, dialogs, tree expansion, scroll positions, preview, toasts, search
results, `subdirs` cache, image cache.

### 9.4 Index stability

`theme` and `lang` are **positional indices**. Only append to `scheme::SCHEMES` (`scheme.rs:48-61`) and
`i18n::LANGS` (`i18n.rs:20-30`); inserting/reordering silently changes users' settings.

---

## 10. Platform abstraction contract

`app.rs` calls `crate::shell::*` and `crate::win::*` unconditionally (some calls are `#[cfg]`-gated). The
two files per module must expose matching names/signatures.

### 10.1 `shell` — Windows `shell.rs` / Linux `shell_unix.rs`

| Function | Windows | Linux |
|---|---|---|
| `shell_image(path, size_px, icon_only) -> Option<ColorImage>` | `IShellItemImageFactory::GetImage` (`shell.rs:49-121`) | theme icon lookup + resvg/`image` (`shell_unix.rs:97-128`) |
| `type_name(ext) -> String` | `SHGetFileInfoW` `SHGFI_TYPENAME` (`shell.rs:123-139`) | localized `"{} Dosyası"` (`shell_unix.rs:141-146`) |
| `init_worker_thread()` | `CoInitializeEx(STA)` (`shell.rs:142-146`) | no-op |
| `system_scale() -> f32` | sets PMv2, `GetDpiForSystem()/96` (`shell.rs:254-260`) | `1.0` |
| `begin_startup_cloak()`, `end_startup_cloak()`, `set_cloak(hwnd, bool)`, `install_subclass(hwnd)` | CBT hook + `DWMWA_CLOAK` + `WM_NCHITTEST` subclass (`shell.rs:184-334`) | **absent** — callers are `#[cfg(windows)]` |
| `MenuResult`, `context_menu(hwnd, paths, extended)` | `IContextMenu` + `TrackPopupMenuEx` (`shell.rs:336-387`) | `MenuResult::None` stub |
| `drag_out(paths) -> bool` | OLE `DoDragDrop` (`shell.rs:391-421`) | `false` |
| `cursor_outside_window(hwnd) -> bool` | `WindowFromPoint` + root compare | `false` |
| `accent_color() -> Option<Color32>` | `DwmGetColorizationColor` | `None` |
| `app_icon(icon, size)` | **absent** | `.desktop` `Icon=` lookup (used by Linux properties) |

### 10.2 `win` — Windows `win.rs` / Linux `win_unix.rs`

| Function | Windows | Linux |
|---|---|---|
| `DriveInfo { root, label, free, total }`, `drives()`, `drive_mask()` | `GetLogicalDrives`/`GetDriveTypeW`/volume label (`win.rs:31-82`) | mounts under `/media`, `/run/media`, `/mnt` + `/` (`win_unix.rs:35-73`) |
| `disk_space(path)` | `GetDiskFreeSpaceExW` | `statvfs` |
| `show_properties(path)` | native shell property sheet (`ShellExecuteExW` `properties`) | **absent** — Linux uses `properties.rs` dialog |
| `open_with(path)` | `OpenAs_RunDLL` | default app (`open::that_detached`) |
| `show_in_explorer(path)` | `explorer.exe /select,` | `org.freedesktop.FileManager1.ShowItems` via `dbus-send`, fallback parent |
| `open_terminal(dir)` | `()`, spawns Windows Terminal/PowerShell | `bool`, `$TERMINAL` then a fixed list |
| `is_terminal(name)`, `run_terminal(name,args,dir)` | keywords `terminal/cmd/powershell/pwsh/wt` | `$TERMINAL`, `x-terminal-emulator`, `gnome-terminal`, … |
| `ctrl_v_down()` | `GetAsyncKeyState` edge (egui emits no Ctrl+V for file clipboards) | `false`; paste arrives as `Event::Paste` |
| `clipboard_set_files(paths, cut)` / `clipboard_get_files()` | `CF_HDROP` + `Preferred DropEffect` | newline-separated absolute paths as text |
| `note_paste_text`, `files_as_text` | **absent** | helpers for text-clipboard emulation |
| `hand_off`, `is_serving`, `serve`, `park`, `take_parked`, `take_reopen` | named-pipe single instance + hidden window | all no-ops / `false` |
| `autostart`, `set_autostart` | HKCU `Run` value `SwiftExplorer` (`win.rs:363-385`) | no-ops |
| `is_default_file_manager`, `set_default_file_manager(on, previous)` | HKCU `Directory\shell`, `Drive\shell`, Win+E CLSID (`win.rs:389-469`) | `xdg-mime default`, returns previous id |
| `mount_roots`, `mime_of`, `default_app_id`, `default_app`, `host_command` | **absent** | Linux-only helpers |

Known asymmetries (do not "fix" by assuming symmetry): `open_terminal` returns `()` vs `bool`;
`shell::app_icon` and `win::show_properties` are single-platform; `NATIVE_MENU = cfg!(windows)`
(`app.rs:838`) is a compile-time constant, so both branches of `if NATIVE_MENU` must type-check.

### 10.3 Virtual pages: Gallery and WSL

**Gallery** (`tab::GALLERY`) is the only virtual location that lists files.

| Piece | Behaviour |
|---|---|
| Path | Explorer's real Gallery CLSID `{24AD3AD4-A569-4530-98E1-AB02F9417AA8}`; the shell supplies the sidebar icon |
| Scan | `fsutil::spawn_gallery` walks `dirs::picture_dir()` (Pictures, including OneDrive redirection) recursively, skips hidden/reparse dirs, keeps image + video extensions, caps at 5000, returns one batch |
| Ordering | `Tab::rebuild_view` forces `SortKey::Modified`, descending, and groups by month/year (`October 2026`) |
| Loading | `Tab::navigate`/`load` special-case `is_gallery()` because `is_home()` is true for it; no watcher, no free-space query |
| Guards | `is_home()` stays true, so paste/new/delete guards keep Gallery read-only; `preview_panel` treats it as a file page, not the empty Home page |
| Search | `Action::StartSearch` uses `dirs::picture_dir()` as the root when the tab is Gallery |
| Thumbnails | `shell_image` first; on failure `images::decode_photo` decodes png/jpg/gif/webp/bmp/tiff locally (≤48 MB) so offline/OneDrive placeholders still render |

**WSL** (`wsl.rs`) feeds the sidebar "Linux" node:

- `wsl::distros()` runs `wsl.exe --list --quiet` with `WSL_UTF8=1`, falls back to UTF-16LE decoding,
  filters service distros (`docker-desktop*`), and caches the result for 10 s.
- Each distro root is `\\wsl.localhost\<name>`; rows reuse `nav_row` + the normal directory pipeline, so
  browsing, sorting and the address bar work without a second code path.
- On non-Windows targets `wsl.rs` returns an empty list, so the node is hidden.

---

## 11. Theme, fonts, DPI

- **Encoding**: `theme 0/1/2` = system/light/dark; `≥3` = `scheme::SCHEMES[theme-3]` (`FIRST = 3`,
  `scheme.rs:63-70`). 12 schemes: Catppuccin Mocha/Latte, Gruvbox Dark/Light, Everforest Dark/Light, Nord,
  Dracula, Tokyo Night, Rosé Pine, Solarized Dark/Light.
- **`apply_theme`** (`app.rs:591-661`) sets `scheme::set_active`, maps to `ThemePreference`, and applies one
  style to **both** light and dark visuals: 14 px body, 12 px small, 20 px heading, 26 px interact height,
  animation disabled (`animation_time = 0.0`, no scroll animation), 8/5 px radii, colors from `Chrome`.
- **`Pal`** (`app.rs:489-568`) holds Explorer-measured chrome colors (tab strip, nav row, content, status,
  hover, selection); if a scheme is active and its brightness matches, scheme colors are used, else the
  hardcoded Explorer grays.
- **Fonts** (`setup_fonts`, `app.rs:413-470`): the UI uses Explorer's own font — `SegUIVar.ttf`
  (Segoe UI Variable, default Regular Text instance) on Windows 11, `segoeui.ttf` on Windows 10.
  egui's `Ubuntu-Light` is dropped from the proportional family so glyphs missing from Segoe cannot
  fall back to a lighter face and change stroke weight; `seguisym.ttf` is the symbol fallback.
  Linux prepends the desktop UI font (`gsettings font-name` / `fc-match sans-serif`). A `fluent` family
  maps UI glyphs: `SegoeIcons.ttf` (Win11) or `segmdl2.ttf` (Win10); Linux uses bundled
  `assets/fluent-icons.ttf`. CJK fallbacks (`msyh.ttc`, `malgun.ttf`, optionally `YuGothM.ttc`) are loaded
  on a background thread at lowest priority (`app.rs:480-518`).
- **Text weight**: a single regular family is used everywhere; there is no semibold family and no bold
  menu row (`menu::Node::bold` was removed).
- **Text size** is *not* hardcoded: `system::message_font_px()` reads `NONCLIENTMETRICSW.lfMessageFont.lfHeight`
  (12 px at 100 % DPI, 19 px at the "Text size" setting 162 %), which is exactly what Explorer draws.
  `text_factor()` (`app.rs:955-960`) turns that into a ratio against the 14 px design base and every text
  size, row height, toolbar height and nav-pane width is derived from it. `text_size()` then snaps the
  result to a **whole device pixel** — a half-pixel em size spreads stems across two pixel columns and only
  a couple of letters per word stay sharp. Sizes never go below 11 pt.
- **Text rasterization** comes from a vendored, patched `epaint` (`vendor/epaint`, wired through
  `[patch.crates-io]` in `Cargo.toml`). Stock epaint rasterizes with grayscale antialiasing only; at 19 px
  that leaves a ~20 % coverage halo which makes Explorer-comparable text look bold and blurry (ClearType
  in Windows uses subpixel coverage and has ~15 % less ink). `sharpen_coverage()` in
  `vendor/epaint/src/text/font.rs` drops sub-threshold coverage before it enters the atlas, which brings
  measured ink density to ClearType level. The crate must stay on the same 0.31.x version as `eframe`.
- **DPI**: egui/winit supply `pixels_per_point`. The app keeps `zoom_factor = 1.0` and disables egui's
  keyboard zoom (Ctrl +, Ctrl -, Ctrl 0) in `apply_system_scale`, so the scale is exactly the Windows
  DPI scale used by File Explorer. Icon sizes are requested as
  `px_bucket(logical * ppp)` with buckets `{16,24,32,48,64,96,128,256,512}` (`app.rs:1080-1088`). Startup
  prefetch uses *system* DPI (`shell::system_scale`), which can mismatch per-monitor DPI.
- **Details columns** all have draggable dividers (`TableBuilder::resizable(true)`; every column is
  `.resizable(true)` with `at_least`/`at_most` thresholds, the trailing invisible `remainder()` column soaks
  up leftover width). Initial widths are measured from content at the current font
  (`px("00.00.0000 00:00")`, `px("Microsoft Edge PDF Document")`, ...), so a fresh profile shows most values
  unclipped; Name gets ~42 % of the table and any width the Type column does not need. Widths live in egui
  persisted memory (`app.ron`, `egui_extras` `serde` feature), and double-clicking a divider fits that column
  to its content. Header labels are left-aligned for Name and right-aligned for Date modified/Type/Size;
  cell text keeps Name/Date/Type left and Size right. The table id is salted with a `_boot` variant for the
  first passes because side-panel widths are not known on frame 0 — creating the column state there would
  freeze bogus widths (`details_view`, `app.rs:5300-5400`).
- **Restored window guard**: `App::repair_restored_window` (`app.rs:1830-1870`) un-minimizes/centres a
  stored window that is off-screen, maximized oddly or shrunk below 700×420, and `App::save` rewrites the
  last good `"window"` entry instead of persisting a minimized rect. `good_window` ignores `-32000`
  coordinates.
- No Mica/acrylic/rounded-corner DWM attributes; the only DWM calls are cloaking and accent color.

---

## 12. Internationalization

- Keys are **Turkish source strings**; `t(key)` returns the key for Turkish, else the table value, else
  English, else the key (`i18n.rs:87-95`).
- `LANGS`: `tr, en, de, es, fr, pt, ru, zh, ja` (`i18n.rs:20-30`); `TABLE` in `i18n.rs` is English; one file
  per other language in `src/i18n/`.
- `tf(key, args)` supports `{}`, `{0}`, `{{`; `singular()` applies literal "1 items" → "1 item" rules for
  en/de/es/fr/pt only (`i18n.rs:97-160`).
- Language resolution: `SWIFT_EXPLORER_LANG` > saved index > OS language (`app.rs:576-584`). Windows uses
  `GetUserDefaultUILanguage`; Linux `LC_ALL`/`LC_MESSAGES`/`LANG`. Traditional Chinese maps to English.
- Adding a language is a 6-file change + test; see the recipe in `low-level-design.md`.

---

## 13. Build, packaging, release

- **Local build**: `cargo build --release` → `target/release/swift-explorer.exe`; the exe embeds the icon
  and, for GNU targets, a manifest via `build.rs` (`asInvoker`, Common Controls 6, supportedOS GUIDs).
- **`Makefile`** (bash only) adds `run/dev/test/check/fmt/lint`, portable packages, checksums, and
  cross-compilation with `cargo-zigbuild` + Zig installed into `.cache/venv` via pip. Targets: Linux
  `x86_64`/`aarch64` (glibc 2.17), Windows `x86_64`/`i686` (GNU) and `aarch64` (gnullvm).
- **CI** (`.github/workflows/build.yml`): matrix `windows-latest` + `ubuntu-22.04`, `cargo build --release`,
  artifacts `SwiftExplorer.exe` and `swift-explorer-linux-x86_64.tar.gz`; on tags `v*`, a `manifest` job
  generates `latest.json` (`{version, page, windows:{url, sha256}}`) and attaches everything to the release;
  `flatpak-sources` + `flatpak` jobs build the Flatpak bundle.
- **Linux install**: `install.sh` downloads the tarball, installs binary + icon + `.desktop` + metainfo into
  XDG dirs (no checksum verification). Flatpak manifest in `packaging/flatpak/`.
- **Version bump locations**: `Cargo.toml` (source of `CARGO_PKG_VERSION`), git tag (drives `latest.json`),
  newest `<release>` in `packaging/linux/io.github.misutesu_desu.SwiftExplorer.metainfo.xml`.

Self-update mechanics: see [`workflows.md#17-self-update`](workflows.md#17-self-update).

---

## 14. Constraints and non-goals

- **Windows-first**: no Mica, no AUMID/jump lists, no long-path (`\\?\`) support anywhere.
- **One window per process**; "new window" spawns a second process (`Action::NewWindow`).
- **No plugin system, no scripting, no search index**; recursive search walks the FS live.
- **No disk cache** for thumbnails; icon cache is per-process, capped at 160 MiB of texture memory.
- **No redo** in the undo stack; undo journal is in-memory only.
- **Settings are not per-folder** except grouping; sort/view/hidden are global.
