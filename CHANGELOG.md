# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.8.0] - 2026-11-02

### Rebranding from Swift Explorer

**Changed**
- Project renamed from "Swift Explorer" to "StarFile Explorer"
- Application ID: `io.github.some_rao.StarFile`
- New branding: blue folder icon with white star
- Updated installer with proper App ID GUID
- Binary name: `StarFile.exe` / `starfile`
- Installer output: `StarFile-Setup.exe`

### Added

- Command palette (Ctrl+Shift+P): search files, run commands, jump to paths
- Split view (Ctrl+Shift+S): browse two folders side by side
- Batch rename with patterns (`{name}`, `{n}`, `{date}`)
- Zip archive support: extract and create from context menu
- Multiple color themes: Catppuccin, Gruvbox, Everforest, Nord, Dracula, Tokyo Night, Rosé Pine, Solarized

### Fixed

- Linux build compatibility with Rust 1.82+
- Window management stub functions for Linux platform
- Icon size compliance for Flatpak (512x512)

### Security

- In-app update verification via SHA-256 checksum

[0.8.0]: https://github.com/some-rao/starfile-explorer/releases/tag/v0.8.0

---

## Previous Versions (from Swift Explorer)

Versions prior to 0.8.0 were released as part of the Swift Explorer project. See the [original changelog](https://github.com/misutesu-desu/swift-explorer/blob/main/CHANGELOG.md) for details.