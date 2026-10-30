# Contributing

Bug reports, translations and pull requests are all welcome.

## Building

You need Rust 1.82 or newer.

```bash
cargo build --release
cargo test
```

The Windows build uses the Windows shell for icons, the context menu and the clipboard; the Linux build replaces those in `src/shell_unix.rs` and `src/win_unix.rs`.

## Translations

Interface text lives in [`src/i18n.rs`](src/i18n.rs) (Turkish keys and English) and one file per language in [`src/i18n/`](src/i18n). The [README](README.md#translations) explains how to add a language. `cargo test` fails if a language file is missing a line. Issues labeled [`translation`](../../labels/translation) list the languages that need a translator or a native-speaker check.

## Pull requests

- Keep a pull request to one change, so it's easy to review and to revert.
- The app copies Windows 11 File Explorer: if something behaves differently from Explorer, Explorer is usually right.
- New code comments and commit messages are in English. Existing Turkish comments can stay until the file is touched.

## Screenshots

`cargo build --release --features screenshot` builds a version that saves a picture of its own window and exits. See [`src/screenshot.rs`](src/screenshot.rs) for the environment variables. It's how the README images are made.
