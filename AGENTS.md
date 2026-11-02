# AGENTS.md — StarFile Explorer (development orientation)

`StarFile Explorer` is a **Rust + egui/eframe** desktop file manager that clones the look and behaviour of
Windows 11 File Explorer. ~10k lines of Rust in `src/*.rs`, plus one translation table per language in
`src/i18n/`. Windows is the primary target (Win32 shell
integration); Linux is a supported but less-complete port (`shell_unix.rs`, `win_unix.rs`).

This file is the entry point. **Read these docs before opening source files** — they were written from a
full read of the tree at `v0.7.0` (commit `850c108`) and cite `file:line` evidence.

| Order | Document | Use it for |
|---|---|---|
| 1 | [`docs/dev/architecture.md`](docs/dev/architecture.md) | Layers, module map, process/thread model, state ownership, persistence, platform contract, build & release |
| 2 | [`docs/dev/workflows.md`](docs/dev/workflows.md) | End-to-end dataflow: startup, listing, search, file ops, undo, clipboard, drag & drop, update |
| 3 | [`docs/dev/low-level-design.md`](docs/dev/low-level-design.md) | Per-file type/function reference, frame order, shortcut tables, invariants, known bugs, recipes |

Upstream repo docs: `README.md` (features, build), `CONTRIBUTING.md` (PR rules), `README.*.md` (translations).

---

## Build / run / verify (Windows, PowerShell)

```powershell
cargo run                 # debug binary; ~8 min cold build, seconds warm
cargo build --release     # target\release\starfile.exe (LTO thin, strip)
cargo test                # unit tests; also fails if any i18n table is out of sync
cargo fmt --all
cargo clippy --all-targets
```

- Toolchain floor: **Rust 1.82** (`Cargo.toml` `rust-version`); verified working on 1.98.1.
- The `Makefile` is bash/awk-only (run/make targets, cross-compile via `cargo-zigbuild` + Zig). It does
  **not** work in PowerShell; use cargo directly on Windows.
- `cargo build --features screenshot` produces a self-screenshot build used to regenerate `docs/*.png`
  (env vars in `src/screenshot.rs:1-8`).
- Release builds set `windows_subsystem = "windows"` (`src/main.rs:1`), so `eprintln!` diagnostics vanish;
  run a debug build when diagnosing.

---

## The 7 rules that keep changes safe

1. **Platform twins must stay in sync.** `main.rs` aliases `mod shell` → `shell.rs` on Windows and
   `shell_unix.rs` elsewhere, and `mod win` → `win.rs` / `win_unix.rs` (`src/main.rs:16-19`). There is **no
   trait**; the contract is "same names and signatures". Adding a `shell::`/`win::` function means editing
   both files (or gating every call with `#[cfg(windows)]`). See `docs/dev/architecture.md` §10
   (platform abstraction contract).
2. **UI intents go through `Action`.** Widgets push into `Vec<Action>`; `App::apply` (`app.rs:1691`)
   executes them after all panels are drawn (`app.rs:4498-4500`). Effects are visible one frame later.
   Don't mutate file-system state directly inside a widget closure.
3. **Turkish strings are the i18n keys.** Write UI text in Turkish (`crate::i18n::t("…")`) and add the
   English pair to `TABLE` in `i18n.rs` plus **every** file in `src/i18n/`; `cargo test` asserts exact key
   equality (`i18n.rs:495-503`). Positional indices are persisted (`lang = index+1`, `theme = index+3`), so
   **append only** to `i18n::LANGS` and `scheme::SCHEMES`.
4. **`Tab` owns per-tab state, `App` owns global state.** Sort, ascending, hidden, view level and grouping set
   are global (`app.rs:297-376`); filter, selection, cursor, history, loader and search are
   per-tab (`tab.rs:37-74`). Selection/cursor/anchor are indices into `Tab.entries`, **not** into `view`.
5. **All I/O happens off the UI thread.** One thread per directory load / search / job; results arrive on
   `mpsc` channels polled each frame (`App::update`). New async work must call `ctx.request_repaint()` or
   the UI will look frozen (the app has no continuous repaint).
6. **Icons and thumbnails come from the platform shell**, cached as egui textures by `ImageCache`
   (`images.rs`). Never decode images in the UI thread; request through `ImageCache::get`.
7. **Explorer is the spec.** When behaviour is ambiguous, copy Windows 11 File Explorer. New code comments,
   commit messages and review notes are written in **English**; existing Turkish comments may stay until
   the file is touched. UI text is still authored as Turkish i18n keys (rule 3) — that is the lookup
   architecture, not a comment-language choice.

---

## 60-second architecture

```
main.rs ──► single-instance handoff (win::hand_off) ──► eframe::run_native
                │
                ├─ prefetch_startup(): read first folder + warm icon cache before the window shows
                ▼
        App (app.rs, 4.5k lines)  ── egui immediate-mode UI, 34px custom title bar
          ├─ tabs: Vec<Tab>        (tab.rs)  one per Explorer tab: entries/view/selection/search/watcher
          ├─ actions: Vec<Action>  queued intents applied at end of frame
          ├─ jobs: Vec<Arc<Job>>   background copy/move/delete/size jobs (fsutil.rs)
          ├─ images: ImageCache    4 LIFO worker threads → shell icons/thumbnails
          └─ dialogs / overlays    one modal at a time + progress cards + toasts
                │
    ┌───────────┴─────────────┬──────────────────┬───────────────┐
 fsutil.rs                images.rs           shell.rs        win.rs
 listing/sort/search/     texture cache       Win32 icons,    drives, clipboard,
 copy/move/delete/undo    + worker pool       native menu,    single instance,
 jobs, patterns                              OLE drag-out    autostart, DWM
                                                │
                                     shell_unix.rs / win_unix.rs  (Linux twins)
```

Sizes: `app.rs` 8604 lines · `palette.rs` 2365 · `fsutil.rs` 1361 · `properties.rs` 1054 (Linux only) ·
`shell.rs` 978 · `highlight.rs` 833 · `menu.rs` 659 · `win.rs` 627 · `tab.rs` 625 · `i18n.rs` 611 ·
`markdown.rs` 441 · `webview.rs` 420 · `query.rs` 402 · `images.rs` 393 · `win_unix.rs` 311 ·
`update.rs` 282 · `shell_unix.rs` 275 · `git.rs` 214 · `scheme.rs` 211 · `desktop_apps.rs` 200 ·
`system.rs` 200 · `screenshot.rs` 190 · `main.rs` 100.

---

## Where to make common changes

| Task | Touch |
|---|---|
| New menu command / toolbar button | add `enum Action` variant (`app.rs:229`), push it from `toolbar_row`/`item_menu`/`background_menu`, handle in `apply` (`app.rs:1691`) |
| New keyboard shortcut | `App::handle_keys` (`app.rs:1446`); use `shortcut()` (`app.rs:943`) so the event is consumed and modifiers match exactly |
| New file operation | implement the job in `fsutil.rs` (`spawn_*` returning `Arc<Job>`), push via `App::push_job`, add undo mapping in `app.rs:4392-4399` |
| New view mode | `VIEW_LEVELS` (`app.rs:21`), `file_area` dispatch (`app.rs:3410`), plus `Ctrl+Shift+1..6` map (`app.rs:1535`) and the View menu |
| New window/panel | `App::update` panel stack (`app.rs:8369-8448`); remember Explorer panel order |
| New theme | append to `scheme::SCHEMES` (`scheme.rs:48`) — never insert |
| New language | copy `src/i18n/de.rs`, add `mod`, `LANGS`, `tables()`, Windows LANGID, run `cargo test` (`docs/dev/low-level-design.md#recipe-add-a-language`) |
| New Windows shell feature | `shell.rs`/`win.rs` **and** a stub or fallback in `shell_unix.rs`/`win_unix.rs` |
| Update flow / version | `update.rs`; version lives in `Cargo.toml`, git tag, and AppStream metainfo release entry |

---

## Top traps (full list: `docs/dev/low-level-design.md` §9)

- **No long-path support**: no `\\?\` prefixing anywhere; paths ≥260 chars fail with raw OS errors.
- **`Tab::is_home()` is true for Home *and* This PC** (`tab.rs:174-176`), so "can't paste/new/delete" guards
  also fire on the drive page.
- **`selected_entries()` falls back to the cursor** when the selection set is empty (`tab.rs:523-533`), so
  commands act on the focused row even with nothing highlighted.
- **Deferred actions**: reading `self.tabs[...]` inside a widget sees pre-action state; don't "fix" a
  one-frame lag by mutating in the closure.
- **`Ctrl+H` does not invalidate the sidebar subdir cache** (the View-menu checkbox does), so the tree can
  show stale hidden folders.
- **Recycle-Bin delete is all-or-nothing** (`trash::delete_all`) and produces no undo entry on failure.
- **Update manifest is a single `windows.url`** with no architecture check; SHA-256 only, no signature.
- **Date group labels are hardcoded Turkish** (`fsutil.rs:246`) and double as collapse keys.
- **`ImageCache` may be constructed once per process** (`images.rs:187`); `App::new` must stay the only caller.
- **`epaint` is vendored and patched** (`vendor/epaint`, `[patch.crates-io]`). It carries the glyph coverage
  curve that keeps UI text as sharp as Explorer's. Bumping `eframe`/`egui` to a new minor version requires
  copying the matching `epaint` release into `vendor/epaint` and re-applying `sharpen_coverage()`.

---

## Definition of done for a change

```powershell
cargo fmt --all
cargo clippy --all-targets
cargo test
cargo run              # exercise the affected view/flow by hand
```

A PR should stay to one change (upstream `CONTRIBUTING.md`), and behaviour that differs from Explorer is
usually a bug.
