# Contributing to StarFile Explorer

Thank you for your interest in contributing to StarFile Explorer! This document provides guidelines and instructions for contributing.

## How to Contribute

### Reporting Bugs

If you find a bug, please open an issue with:
- A clear description of the problem
- Steps to reproduce (if applicable)
- Your operating system and version
- Screenshots if helpful
- The version number (visible in Help > About)

### Suggesting Features

Feature requests are welcome! Please open an issue with:
- A clear description of the proposed feature
- Why it would be useful
- Any mockups or screenshots

### Pull Requests

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes
4. Run tests: `cargo test`
5. Commit your changes (`git commit -m 'Add amazing feature'`)
6. Push to the branch (`git push origin feature/amazing-feature`)
7. Open a Pull Request

## Development Setup

### Prerequisites

- Rust 1.82 or newer
- Git
- For Windows installer: Inno Setup 6

### Building

```bash
# Build the project
cargo build --release

# Run tests
cargo test

# Format code
cargo fmt --all

# Run linter
cargo clippy --all-targets
```

### Building the Windows Installer

```powershell
# Install Inno Setup 6 first
ISCC.exe /DAppVersion=0.8.0 packaging\windows/starfile.iss

# Output: dist/StarFile-Setup-0.8.0.exe
```

## Code Style

- Follow existing Rust conventions
- Use `cargo fmt` to auto-format
- Ensure `cargo clippy` passes without warnings
- Add your name to contributors if this is your first contribution

## Translation

The project uses Turkish strings as i18n keys. To add or update translations:

1. Create a new file in `src/i18n/{lang}.rs` (copy from `de.rs`)
2. Translate all English strings
3. Add `mod {lang};` in `src/i18n.rs`
4. Add entries in `LANGS`, `tables()`, and `system_lang()`
5. Run `cargo test` to verify

See the [German translation commit](https://github.com/misutesu-desu/swift-explorer/commit/main/src/i18n/de.rs) as a reference.

## Project Structure

```
starfile/
├── src/                    # Rust source code
│   ├── i18n/              # Translations
│   └── *.rs               # Modules
├── assets/                # Icons, fonts
├── packaging/             # Installer and package files
├── docs/                  # Documentation
└── CONTRIBUTING.md        # This file
```

## Questions?

Join the discussion or ask questions in the [GitHub Discussions](https://github.com/some-rao/starfile-explorer/discussions) section.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.