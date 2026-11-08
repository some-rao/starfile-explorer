# StarFile Explorer

A free, open-source file manager for **Windows and Linux** with a Windows 11 File Explorer-inspired interface, built in Rust with egui/eframe.

[Latest release](https://github.com/some-rao/starfile-explorer/releases/latest) · [Source](https://github.com/some-rao/starfile-explorer) · [Changelog](CHANGELOG.md)

## Screenshots

<table>
  <tr>
    <td width="33%">
      <a href="docs/screenshots/screenshot1.png"><img src="docs/screenshots/screenshot1.png" alt="StarFile Explorer main interface" width="100%"></a><br>
      <sub>Main interface (release screenshot)</sub>
    </td>
    <td width="33%">
      <a href="docs/screenshots/screenshot2.png"><img src="docs/screenshots/screenshot2.png" alt="StarFile Explorer command palette" width="100%"></a><br>
      <sub>Command palette (release screenshot)</sub>
    </td>
    <td width="33%">
      <a href="docs/screenshots/screenshot3.png"><img src="docs/screenshots/screenshot3.png" alt="StarFile Explorer Quick Look preview" width="100%"></a><br>
      <sub>Quick Look preview (release screenshot)</sub>
    </td>
  </tr>
</table>

## Download

Get binaries from the [latest GitHub release](https://github.com/some-rao/starfile-explorer/releases/latest) (use the newest release assets; version numbers change over time):

| Platform | Asset | Notes |
|---|---|---|
| Windows | `StarFile-Setup.exe` | Per-user installer (no admin required) |
| Windows | `StarFile.exe` | Portable executable |
| Linux | `starfile-linux-x86_64`, `starfile-linux-x86_64.tar.gz` | x86_64 release artifacts; Flatpak is also built in CI releases |

## Install

**Windows:** download and run `StarFile-Setup.exe`, or run the portable `StarFile.exe` from the [latest release](https://github.com/some-rao/starfile-explorer/releases/latest).

**Linux:**

```bash
curl -fsSL https://raw.githubusercontent.com/some-rao/starfile-explorer/main/install.sh | sh
```

## Features

- **Explorer-inspired interface:** tabs, toolbar/address bar, navigation pane, Home/This PC pages, and multiple view modes.
- **Command palette (`Ctrl+K`, also `Ctrl+E` / `Ctrl+F` / `F3`):** search files, run commands, and jump to paths.
- **Split view (`Ctrl+\`):** browse two folders side by side.
- **Quick Look (`Space`):** preview selected items. Embedded PDF/media/HTML playback uses a WebView2-based viewer where available on Windows; behavior depends on the local Windows/WebView2 setup.
- **Batch rename:** rename multiple selected items with patterns (`{name}`, `{n}`, `{date}`), live preview, collision checks, and undo (available from the context menu / `…` menu and command palette).
- **Background file operations:** folder loading, search, and file jobs run off the UI thread to reduce freezes.
- **Recycle Bin integration (Windows):** delete to Recycle Bin with undo support for supported operations.
- **Git status column:** shows repository state when enabled.
- **Custom wallpaper:** optional background image behind translucent panels.
- **New-file templates:** create documents/templates from the New menu, including an empty `.zip` archive. On Windows, installed shell extensions may add their own archive/context-menu actions.
- **Themes and languages:** multiple themes and UI translations (English, Turkish, German, Spanish, French, Portuguese, Russian, Chinese, Japanese).
- **Automatic updates (Windows release builds):** checks release metadata and verifies downloads with SHA-256 before applying.

## Windows and Linux differences

StarFile Explorer aims for the same overall workflow, but platform integration differs:

- **Windows:** uses Win32 shell integration for icons/thumbnails, native/extended context-menu behavior, OLE drag-out, clipboard file operations, properties, and other Explorer-style features.
- **Linux:** uses freedesktop/XDG integrations (desktop theme icons, app context menus where applicable, D-Bus/file-manager integration, its own properties/dialog behavior) and does not provide the same Windows shell extension behavior.

## Building from source

Requires Rust 1.82+:

```bash
cargo build --release
```

See [AGENTS.md](AGENTS.md) and [`docs/dev/`](docs/dev/) for development notes.

## License and credits

Based on [Swift Explorer](https://github.com/misutesu-desu/swift-explorer) by misutesu-desu. Licensed under [MIT](LICENSE).
