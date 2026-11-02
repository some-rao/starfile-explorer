#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[cfg(not(windows))]
mod desktop_apps;
mod diag;
mod fsutil;
mod git;
mod highlight;
mod i18n;
mod images;
mod markdown;
mod media;
mod menu;
mod newfile;
#[cfg(not(windows))]
mod properties;
mod query;
mod scheme;
#[cfg(feature = "screenshot")]
mod screenshot;
#[cfg_attr(not(windows), path = "shell_unix.rs")]
mod shell;
mod system;
mod tab;
mod update;
#[cfg_attr(not(windows), path = "webview_unix.rs")]
mod webview;
#[cfg_attr(not(windows), path = "win_unix.rs")]
mod win;
mod wsl;

use std::path::PathBuf;

pub const APP_NAME: &str = "StarFile Explorer";
/// Wayland/X11 pencere kimliği; `.desktop` dosyasının adıyla aynı olmalı ki masaüstü ikonu ve adı eşleştirsin
pub const APP_ID: &str = "io.github.some_rao.StarFile";

fn main() -> eframe::Result<()> {
    diag::install();
    // Eski adla (Swift Explorer) yazılmış ayarları devral; pencere açılmadan önce
    app::migrate_legacy_config();
    // Pencere oluşmadan: monitör başına DPI ve görev çubuğu kimliği
    system::apply_dpi_awareness();
    system::set_app_id();

    let arg = std::env::args().nth(1);
    // Kaldırıcı, dosyalar silinmeden önce kayıt defteri temizliği için çağırır
    if arg.as_deref() == Some("--uninstall-cleanup") {
        app::uninstall_cleanup();
        return Ok(());
    }
    let after_update = arg.as_deref() == Some(update::AFTER_UPDATE_ARG);
    let hidden = after_update || arg.as_deref() == Some(win::BACKGROUND_ARG);
    if after_update {
        // Eski sürüm boruyu bırakıp çıkana dek bekle
        for _ in 0..100 {
            if !win::is_serving() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    // Explorer sürücüyü "%1" ile "C:\" diye verir; Windows bunu C:" olarak ayrıştırır
    let start = arg
        .filter(|_| !hidden)
        .map(|a| PathBuf::from(a.trim_end_matches('"')));
    if hidden && win::is_serving() || !hidden && win::hand_off(start.as_deref()) {
        return Ok(());
    }
    // Açılışı başka sürece devredenler sayılmasın diye burada
    update::startup_guard();
    // İlk çalıştırmada (kayıtlı pencere konumu yoksa) ekranı kaplayarak aç
    let first_run = eframe::storage_dir(APP_NAME).is_none_or(|d| !d.join("app.ron").exists());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_maximized(first_run)
            // Oturum açılışında gizli başlarken kullanıcının penceresinden odağı almasın
            .with_active(!hidden)
            .with_title(APP_NAME)
            .with_app_id(APP_ID)
            // Başlık çubuğu yok: sekmeler ve pencere düğmeleri Explorer gibi en üstte
            .with_decorations(false)
            // Akrilik buzlu cam istemci alanının arkasına çizilebilsin diye saydam
            // yüzey iste. Sürücü/GL yapılandırması desteklemezse pencere opak kalır
            // (zararsız).
            .with_transparent(true)
            .with_inner_size([1060.0, 640.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .unwrap_or_default(),
            ),
        // vsync açık (eframe varsayılanı): sunum vblank'a kilitlenir, kare
        // aralığı düzenli olur ve kaydırma akıcı çıkar. Serbest döngüde
        // (vsync kapalı) dokunmatik pan'in kare ritmi giriş hızına kilitleniyordu.
        vsync: true,
        ..Default::default()
    };
    #[cfg(windows)]
    shell::begin_startup_cloak();
    app::prefetch_startup(start.clone());
    #[cfg(feature = "screenshot")]
    screenshot::init();
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, start, hidden)))),
    )
}
