//! README görüntülerini yeniden üretmek için: `--features screenshot` ile derlenip
//! `SWIFT_EXPLORER_SCREENSHOT=çıktı.bmp` verilince pencere birkaç saniye sonra kendi
//! görüntüsünü BMP olarak kaydeder ve kapanır. Pencere gizli bir masaüstünde olsa da çalışır.
//!
//! `SWIFT_EXPLORER_SCREENSHOT_SIZE=1280x800` pencere boyutunu,
//! `SWIFT_EXPLORER_SCREENSHOT_DELAY=4` beklenecek saniyeyi belirler.
//! `SWIFT_EXPLORER_SCREENSHOT_INPUT=click:820,130;key:F1` görüntüden önce sırayla tıklar ve tuşa basar
//! (bir menüyü ya da pencereyi açık göstermek için).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Shot {
    path: PathBuf,
    at: Instant,
    size: Option<egui::Vec2>,
    requested: bool,
    input: Vec<String>,
    next_input: Instant,
    release: Option<egui::Event>,
}

static SHOT: Mutex<Option<Shot>> = Mutex::new(None);

pub fn init() {
    let Some(path) = std::env::var_os("SWIFT_EXPLORER_SCREENSHOT") else { return };
    let delay = std::env::var("SWIFT_EXPLORER_SCREENSHOT_DELAY").ok().and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let size = std::env::var("SWIFT_EXPLORER_SCREENSHOT_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some(egui::vec2(w.parse().ok()?, h.parse().ok()?))
    });
    let input = std::env::var("SWIFT_EXPLORER_SCREENSHOT_INPUT").map(|s| s.split(';').rev().map(str::to_string).collect()).unwrap_or_default();
    let now = Instant::now();
    *SHOT.lock().unwrap() = Some(Shot {
        path: path.into(),
        at: now + Duration::from_secs_f32(delay),
        size,
        requested: false,
        input,
        next_input: now + Duration::from_millis(1500),
        release: None,
    });
}

/// `raw_input_hook`'tan çağrılır: sıradaki tıklamayı ya da tuşu ekler
pub fn input_hook(raw: &mut egui::RawInput) {
    let mut guard = SHOT.lock().unwrap();
    let Some(shot) = guard.as_mut() else { return };
    if let Some(e) = shot.release.take() {
        raw.events.push(e);
        return;
    }
    if Instant::now() < shot.next_input {
        return;
    }
    let Some(action) = shot.input.pop() else { return };
    shot.next_input = Instant::now() + Duration::from_millis(500);
    let modifiers = egui::Modifiers::NONE;
    let point = |p: &str| -> Option<egui::Pos2> {
        let (x, y) = p.split_once(',')?;
        Some(egui::pos2(x.parse().ok()?, y.parse().ok()?))
    };
    if let Some(pos) = action.strip_prefix("hover:").and_then(point) {
        raw.events.push(egui::Event::PointerMoved(pos));
    } else if let Some((pos, button)) = action
        .strip_prefix("rclick:")
        .and_then(point)
        .map(|p| (p, egui::PointerButton::Secondary))
        .or_else(|| action.strip_prefix("click:").and_then(point).map(|p| (p, egui::PointerButton::Primary)))
    {
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers });
        shot.release = Some(egui::Event::PointerButton { pos, button, pressed: false, modifiers });
    } else if let Some(key) = action.strip_prefix("key:").and_then(egui::Key::from_name) {
        raw.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
        shot.release = Some(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    }
}

/// Her karede çağrılır
pub fn tick(ctx: &egui::Context) {
    let mut guard = SHOT.lock().unwrap();
    let Some(shot) = guard.as_mut() else { return };
    if let Some(size) = shot.size.take() {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
    }
    let image = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    });
    if let Some(image) = image {
        if let Err(e) = save_bmp(&shot.path, &image) {
            eprintln!("screenshot: {e}");
        }
        *guard = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        return;
    }
    if !shot.requested && Instant::now() >= shot.at {
        shot.requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
    }
    ctx.request_repaint_after(Duration::from_millis(100));
}

/// 32 bit, üstten alta BMP
fn save_bmp(path: &std::path::Path, img: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = img.size;
    let data_len = (w * h * 4) as u32;
    let mut out = Vec::with_capacity(54 + data_len as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + data_len).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for p in &img.pixels {
        let [r, g, b, a] = p.to_srgba_unmultiplied();
        out.extend_from_slice(&[b, g, r, a]);
    }
    std::fs::write(path, out)
}
