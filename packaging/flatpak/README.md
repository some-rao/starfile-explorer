# Flatpak

`io.github.some_rao.StarFile.yml` builds the app from this checkout. CI builds it on every push and attaches `starfile.flatpak` to tagged releases.

Building locally needs `flatpak-builder` and the GNOME 49 SDK with the Rust extension:

```bash
flatpak install flathub org.gnome.Sdk//49 org.gnome.Platform//49 org.freedesktop.Sdk.Extension.rust-stable//25.08
python3 flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/io.github.some_rao.StarFile.yml
flatpak run io.github.some_rao.StarFile
```

`flatpak-cargo-generator.py` comes from [flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo). `cargo-sources.json` is not committed; regenerate it whenever `Cargo.lock` changes.

## Flathub

The Flathub submission uses the same manifest with the `dir` source replaced by the release tag:

```yaml
      - type: git
        url: https://github.com/some-rao/starfile-explorer.git
        tag: vX.Y.Z
        commit: <commit hash of the tag>
```

and `cargo-sources.json` committed next to it in the Flathub repo.
