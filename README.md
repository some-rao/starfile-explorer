# StarFile Explorer

[![Release](https://img.shields.io/github/v/release/some-rao/starfile-explorer)](../../releases/latest)
[![Build](https://github.com/some-rao/starfile-explorer/actions/workflows/build.yml/badge.svg)](../../actions/workflows/build.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

[Türkçe](README.tr.md)

A file manager for Windows and Linux that looks and behaves like the Windows 11 File Explorer, written
in Rust. Frameless acrylic window, tabs in the title bar, a command palette, quick look, galleries and
inline previews. Folders are read on background threads, so large ones open without freezing the window,
and search runs across subfolders in parallel.

![Large icons, grouped by date](docs/large-icons.png)

## Install

**Windows.** Download `StarFile-Setup-<version>.exe` from the [Releases](../../releases) page and run it.
It installs for the current user only (`%LOCALAPPDATA%\Programs\StarFile Explorer`), so it does not ask for
administrator rights, and it creates the Start menu entry itself.

- **Updates** happen in the app: a new release is downloaded from GitHub, checked against its SHA-256, and
  takes over when you close the window. Nothing to download by hand. View >
  Install updates automatically turns this off.
- **Uninstall** from Windows Settings > Apps, or from the Start menu entry. The uninstaller removes the
  program, the autostart entry and the file-manager registration; your settings in `%APPDATA%\StarFile`
  are kept on purpose, so a reinstall starts where you left off.
- The installer is **not code signed yet**, so the first run shows "Windows protected your PC". Choose
  *More info* > *Run anyway*. Every release is built by GitHub Actions from this repository.

`StarFile.exe` is also published on its own. That is the raw binary the updater uses; it works without
installing, but you then miss the Start menu entry, the uninstaller and the update hand-over.

**Linux.** Install it for your user with:

```bash
curl -fsSL https://raw.githubusercontent.com/some-rao/starfile-explorer/main/install.sh | sh
```

This puts `starfile` in `~/.local/bin` and adds it to the app menu. To remove it, run the same command with
`-s -- --uninstall` after `sh`. You can also download `starfile-linux-x86_64.tar.gz` and run `starfile` from
it without installing, or install `starfile.flatpak` with `flatpak install --user starfile.flatpak`.

## What it does

**Interface**

- Same layout as Explorer: tabs in the title bar, one toolbar card with the address bar, a navigation pane,
  Home, Gallery and This PC pages.
- The window has no system caption: the title bar, the tab strip and the window buttons are drawn by the
  app, over an acrylic backdrop that follows the desktop wallpaper. Maximizing stops exactly at the
  taskbar instead of sliding underneath it.
- Tabs are floating pills that can be dragged, reordered, duplicated and torn off. Ctrl+T, Ctrl+W,
  Ctrl+Tab and middle-click all behave like Explorer's.
- Uses Windows' own icons, thumbnails and file type names.
- Every Explorer shortcut works the same way. Press F1 for the full list.
- Besides System, Light and Dark, View > Theme has Catppuccin (Mocha, Latte), Gruvbox, Everforest, Nord,
  Dracula, Tokyo Night, Rosé Pine and Solarized.
- Interface in English, Turkish, German, Spanish, French, Brazilian Portuguese, Russian, Simplified
  Chinese and Japanese. The language follows Windows and can be changed from View > Language.
- Chinese, Japanese and Korean file names display with the system's CJK fonts.

**Finding things**

- Ctrl+K opens a command palette: it searches files as you type, runs any command, and jumps to a path or
  a recent folder. Ctrl+E, Ctrl+F and F3 open it too, and `*` and `?` wildcards work.
- Space opens Quick Look on the highlighted item: images, PDFs, audio and video play inline, text and
  Markdown are shown with syntax highlighting, and folders list their contents.
- Markdown, PDF and video previews render in an embedded WebView2 pane, so a README or a PDF opens where
  you are instead of in another application.
- Typing `cmd`, `powershell`, `pwsh` or `wt` in the address bar opens that terminal in the current folder,
  and `terminal` opens Windows Terminal. `\\wsl$\…` paths open like any other folder. On Linux, `terminal`
  opens your default terminal, and a terminal's name (`konsole`, `kitty`…) opens that one.
- A Git column shows the branch state of a repository in the folder itself.

**Files**

- Copy, cut, paste and drag and drop work in both directions with Explorer and the desktop.
- Shift + right-click opens the full Windows context menu, so 7-Zip, WinRAR and Send to are there.
- Ctrl+Z undoes renames, moves, copies and deletes to the Recycle Bin.
- A copy or move counts as done only after the files are written to the disk, like running `sync` in a
  terminal. Once the progress bar is gone, a USB stick can be pulled out. When moving to another drive,
  the originals are deleted only after that.
- When a file with the same name already exists, it asks whether to replace, skip or keep both.
- Folders update live when files change on disk.
- Downloads is grouped by date modified (Today, Yesterday, Last week…), like in Explorer. Other folders
  can be grouped from Sort > Group by.
- On Windows, closing the window keeps the app waiting in the background, so opening it again shows the
  window right away instead of waiting for the graphics driver to start. With View > Start in the
  background when signing in, the first launch after a reboot is quick too.

| Details | This PC |
| --- | --- |
| ![Details view](docs/details.png) | ![This PC](docs/this-pc.png) |

| Light theme | Turkish |
| --- | --- |
| ![Light theme](docs/light.png) | ![Turkish interface](docs/turkish.png) |

The Type column and the Shift + right-click menu come from Windows, so they stay in the Windows display
language even when the app is set to English.

## Linux

The Linux version is less tested than the Windows one. It keeps the same look and shortcuts, with these
differences:

- Icons come from your desktop's icon theme. Thumbnails are shown for PNG, JPEG, GIF, WebP and BMP.
- There is no Windows context menu, so Shift + right-click and Shift+F10 open the app's own menu.
- Copied files go to the clipboard as a list of paths. Pasting paths or `file://` links copied from other
  file managers works; pasting into other file managers may not.
- Dragging files out of the window to other apps doesn't work yet. Dropping files into it works on X11.
- "This PC" lists `/` and disks mounted under `/media`, `/run/media` and `/mnt`.
- Properties opens the app's own window with General, Security and Details tabs, where permissions and
  the default app can be changed.
- "Show in file manager" uses your file manager over D-Bus (Nautilus, Dolphin, Nemo and Thunar support it).

## Building

You need Rust 1.82 or newer.

```bash
cargo build --release
```

The executable ends up in `target/release/starfile.exe` on Windows and `target/release/starfile` on Linux.
`cargo build --release` needs the icon and manifest in `packaging/windows/` (Windows) in place;
`cargo test` also fails if a translation table is out of sync with the others.

To build the installer, install [Inno Setup 6](https://jrsoftware.org/isinfo.php) and run:

```powershell
ISCC.exe /DAppVersion=0.8.0 packaging\windows\starfile.iss
```

The output is `dist\StarFile-Setup-0.8.0.exe`. The release workflow does exactly this for every tag.

## Settings

Settings are stored in `%APPDATA%\StarFile\config.json` on Windows and `~/.config/StarFile/config.json` on
Linux. The first time the app starts it copies the settings of an earlier Swift Explorer installation
(`%APPDATA%\SwiftExplorer`), so existing preferences survive the rename. Setting `STARFILE_LANG` to a
language code (`en`, `tr`, `de`, `es`, `fr`, `pt`, `ru`, `zh`, `ja`) overrides the language setting;
`SWIFT_EXPLORER_LANG` still works for old shortcuts.

## Translations

Each language is one file in [`src/i18n/`](src/i18n). To add one, copy `de.rs`, translate the second string
on every line, then add a `mod` line and entries in `LANGS`, `tables()` and `system_lang()` in
[`src/i18n.rs`](src/i18n.rs). The [German commit](../../commits/main/src/i18n/de.rs) shows every change
needed. Fixes to the existing translations are welcome too; native speakers haven't checked them yet.

## Known limitations

- In Details view, the selection rectangle can only be started from the empty area below the rows.
- Windows can't move files with very long paths (around 260+ characters) to the Recycle Bin, so they are
  deleted directly and Ctrl+Z can't bring them back.
- Network locations open in Windows File Explorer.
- The installer is unsigned (see [Install](#install)).

## Credits

StarFile Explorer is a fork of [Swift Explorer](https://github.com/misutesu-desu/swift-explorer) by
Abdullah Şafer, which is where the file manager itself, the shell integration, the command palette, quick
look and most of the UI come from. This fork adds the name, the icon, the installer/update channel and the
frameless window work. Both projects are MIT licensed; the original copyright notice is kept in
[`LICENSE`](LICENSE), and [`NOTICE`](NOTICE) spells out the attribution.

## License

[MIT](LICENSE). The Linux build includes a few icons from Microsoft's
[Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)
([MIT](assets/LICENSE-fluent-icons.txt)). See [`NOTICE`](NOTICE) for the full list.
