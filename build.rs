fn main() {
    // Windows'ta exe dosyasina uygulama ikonunu ve surum bilgisini gom
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        // Dosya ozelliklerinde (sag tik > Ozellikler > Ayrintilar) gorunen alanlar
        res.set("ProductName", "StarFile Explorer");
        res.set("FileDescription", "StarFile Explorer");
        res.set("OriginalFilename", "starfile.exe");
        res.set("InternalName", "starfile");
        res.set("ProductVersion", env!("CARGO_PKG_VERSION"));
        res.set("FileVersion", env!("CARGO_PKG_VERSION"));
        res.set(
            "LegalCopyright",
            "MIT. Fork of Swift Explorer by Abdullah Safer.",
        );
        // MSVC baglayicisi uygulama bildirimini (comctl32 v6 dahil) kendisi ekler; MinGW'de eklemez.
        // Bildirim olmazsa SetWindowSubclass gibi islevler bulunamaz ve uygulama acilmaz.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
            res.set_manifest_file("packaging/windows/starfile.manifest");
            println!("cargo:rerun-if-changed=packaging/windows/starfile.manifest");
        }
        res.compile().expect("ikon kaynagi derlenemedi");
    }
}
