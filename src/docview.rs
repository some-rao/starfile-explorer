//! Ofis belgesi görünümü: `ooxml::Doc` bloğunu okunur bir düzene çevirir.
//!
//! Explorer kurulu Office işleyicisiyle belgenin gerçek sayfasını gösterir; burada
//! aynı bilgi egui ile yeniden kurulur: başlıklar, paragraflar, madde işaretleri,
//! tablolar, çalışma sayfası ızgarası ve slayt kartları.

use egui::{Color32, RichText};

use crate::highlight::Colors;
use crate::ooxml::{Block, Doc};

/// Bir çalışma sayfasında en fazla çizilen satır/sütun (çok büyük sayfalar
/// yüz binlerce pencere öğesi üretmesin).
const MAX_ROWS: usize = 200;
const MAX_COLS: usize = 16;

pub fn render(ui: &mut egui::Ui, doc: &Doc, c: &Colors, size: f32) {
    match doc {
        Doc::Text(blocks) => blocks_(ui, blocks, c, size),
        Doc::Sheets(sheets) => sheets_(ui, sheets, c, size),
        Doc::Slides(slides) => slides_(ui, slides, c, size),
    }
}

fn blocks_(ui: &mut egui::Ui, blocks: &[Block], c: &Colors, size: f32) {
    for b in blocks {
        match b {
            Block::Heading(level, text) => {
                ui.add_space(if *level <= 2 { 10.0 } else { 6.0 });
                let h = match level {
                    1 => size * 1.6,
                    2 => size * 1.32,
                    3 => size * 1.14,
                    _ => size,
                };
                ui.add(egui::Label::new(RichText::new(text).size(h).color(c.text)).wrap());
                ui.add_space(2.0);
            }
            Block::Para(text) => {
                ui.add(
                    egui::Label::new(RichText::new(text).size(size).color(c.text))
                        .wrap()
                        .selectable(true),
                );
                ui.add_space(6.0);
            }
            Block::Bullet { level, text } => {
                ui.horizontal_top(|ui| {
                    ui.add_space(10.0 * *level as f32);
                    ui.add(
                        egui::Label::new(RichText::new("•").size(size).color(c.weak))
                            .selectable(false),
                    );
                    ui.add_space(2.0);
                    ui.add(
                        egui::Label::new(RichText::new(text).size(size).color(c.text))
                            .wrap()
                            .selectable(true),
                    );
                });
                ui.add_space(2.0);
            }
            Block::Table { rows } => {
                table(ui, rows, c, size);
                ui.add_space(10.0);
            }
        }
    }
}

fn table(ui: &mut egui::Ui, rows: &[Vec<String>], c: &Colors, size: f32) {
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    egui::Frame::new()
        .fill(if c.dark {
            Color32::from_white_alpha(8)
        } else {
            Color32::from_black_alpha(6)
        })
        .corner_radius(4.0)
        .inner_margin(6)
        .show(ui, |ui| {
            egui::Grid::new(ui.next_auto_id().with("doc_table"))
                .num_columns(cols)
                .spacing([14.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        for ci in 0..cols {
                            let text = row.get(ci).map(String::as_str).unwrap_or("");
                            let mut rt = RichText::new(text).size(size).color(c.text);
                            if i == 0 {
                                rt = rt.color(c.accent);
                            }
                            ui.add(egui::Label::new(rt).wrap());
                        }
                        ui.end_row();
                    }
                });
        });
}

fn sheets_(ui: &mut egui::Ui, sheets: &[crate::ooxml::Sheet], c: &Colors, size: f32) {
    for (si, sheet) in sheets.iter().enumerate() {
        if si > 0 {
            ui.add_space(14.0);
        }
        ui.label(RichText::new(&sheet.name).size(size + 2.0).color(c.text));
        ui.add_space(4.0);
        if sheet.rows.is_empty() {
            ui.label(RichText::new("(boş)").size(size).color(c.weak));
            continue;
        }
        let cols = sheet
            .rows
            .iter()
            .map(|r| r.len())
            .max()
            .unwrap_or(0)
            .min(MAX_COLS);
        egui::Grid::new(ui.next_auto_id().with(("sheet", si)))
            .num_columns(cols + 1)
            .spacing([12.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                for (ri, row) in sheet.rows.iter().take(MAX_ROWS).enumerate() {
                    // Satır numarası sütunu (Explorer'daki sayfa görünümü gibi).
                    ui.label(
                        RichText::new((ri + 1).to_string())
                            .size(size - 1.0)
                            .color(c.weak),
                    );
                    for ci in 0..cols {
                        let text = row.get(ci).map(String::as_str).unwrap_or("");
                        let mut rt = RichText::new(text).size(size).color(c.text);
                        if ri == 0 {
                            // İlk satır başlık: vurgu rengiyle ayrılır (kalın değil).
                            rt = rt.color(c.accent);
                        }
                        ui.add(egui::Label::new(rt).wrap());
                    }
                    ui.end_row();
                }
            });
        if sheet.rows.len() > MAX_ROWS {
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "… {} satır daha (önizlemede ilk {MAX_ROWS} satır gösterilir)",
                    sheet.rows.len() - MAX_ROWS
                ))
                .size(size - 1.0)
                .color(c.weak),
            );
        }
    }
}

fn slides_(ui: &mut egui::Ui, slides: &[crate::ooxml::Slide], c: &Colors, size: f32) {
    for (i, slide) in slides.iter().enumerate() {
        if i > 0 {
            ui.add_space(12.0);
        }
        let bg = if c.dark {
            Color32::from_white_alpha(6)
        } else {
            Color32::from_black_alpha(5)
        };
        egui::Frame::new()
            .fill(bg)
            .corner_radius(6.0)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Slayt {}", slide.no))
                            .size(size - 2.0)
                            .color(c.weak),
                    );
                });
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(RichText::new(&slide.title).size(size + 3.0).color(c.text))
                        .wrap(),
                );
                if !slide.lines.is_empty() {
                    ui.add_space(4.0);
                }
                for (level, text) in &slide.lines {
                    ui.horizontal_top(|ui| {
                        ui.add_space(8.0 + 10.0 * *level as f32);
                        ui.add(
                            egui::Label::new(RichText::new("•").size(size).color(c.weak))
                                .selectable(false),
                        );
                        ui.add_space(2.0);
                        ui.add(
                            egui::Label::new(RichText::new(text).size(size).color(c.text))
                                .wrap()
                                .selectable(true),
                        );
                    });
                    ui.add_space(2.0);
                }
            });
    }
}
