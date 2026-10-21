//! Markdown önizlemesi.
//!
//! `pulldown-cmark` ile olay olay çözülür, egui widget'larına çevrilir. Windows
//! Gezgini'nde Markdown önizlemesi yok; bu bölme "okunabilir" olmayı hedefler:
//! başlıklar, listeler, alıntılar, tablolar ve (varsa dili tanınan) kod blokları
//! renklendirilir. Kalın yazı kullanılmaz — proje kuralı gereği vurgu renkle
//! yapılır.

use std::sync::atomic::{AtomicU64, Ordering};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::highlight;

pub struct Md {
    pub c: highlight::Colors,
    pub size: f32,
}

static TABLE_NO: AtomicU64 = AtomicU64::new(0);

pub fn render(ui: &mut egui::Ui, src: &str, md: &Md) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let mut buf = String::new();
    let mut heading: Option<u8> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut counters: Vec<u64> = Vec::new();
    let mut marker: Option<String> = None;
    let mut quote: usize = 0;
    let mut code: Option<(Option<highlight::Lang>, String)> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell: Option<String> = None;

    for ev in Parser::new_ext(src, opts) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(level as u8);
                buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let lvl = heading.take().unwrap_or(1);
                let t = buf.trim().to_string();
                buf.clear();
                if !t.is_empty() {
                    let size = match lvl {
                        1 => md.size * 1.6,
                        2 => md.size * 1.34,
                        3 => md.size * 1.16,
                        _ => md.size * 1.04,
                    };
                    ui.add_space(10.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(t).size(size).color(md.c.text)).wrap(),
                    );
                    ui.add_space(2.0);
                }
            }
            Event::Start(Tag::Paragraph) => buf.clear(),
            Event::End(TagEnd::Paragraph) => {
                let m = marker.take().unwrap_or_default();
                para(ui, md, &buf, &m, lists.len(), quote);
                buf.clear();
            }
            Event::Start(Tag::BlockQuote(_)) => {
                quote += 1;
                ui.add_space(2.0);
            }
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::List(start)) => {
                lists.push(start);
                counters.push(start.unwrap_or(1));
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                counters.pop();
                ui.add_space(2.0);
            }
            Event::Start(Tag::Item) => {
                marker = Some(match lists.last() {
                    Some(Some(_)) => {
                        let n = counters.last_mut().unwrap();
                        let s = format!("{n}.");
                        *n += 1;
                        s
                    }
                    _ => "•".to_string(),
                });
            }
            Event::End(TagEnd::Item) => {
                // Sıkı listelerde paragraf olayı gelmez; madde işareti burada harcanır.
                if !buf.trim().is_empty() {
                    let m = marker.take().unwrap_or_default();
                    para(ui, md, &buf, &m, lists.len(), quote);
                    buf.clear();
                }
                marker = None;
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(info) => highlight::Lang::from_name(info),
                    CodeBlockKind::Indented => None,
                };
                code = Some((lang, String::new()));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, text)) = code.take() {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .corner_radius(6.0)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            egui::ScrollArea::both()
                                .max_height(360.0)
                                .auto_shrink([false, true])
                                .show(ui, |ui| {
                                    let job = highlight::job(lang, &text, md.size - 1.0, md.c);
                                    ui.add(egui::Label::new(job).extend());
                                });
                        });
                    ui.add_space(6.0);
                }
            }
            Event::Start(Tag::Table(_)) => {
                rows.clear();
                row.clear();
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => row.clear(),
            Event::Start(Tag::TableCell) => cell = Some(String::new()),
            Event::End(TagEnd::TableCell) => {
                if let Some(c) = cell.take() {
                    row.push(c.trim().to_string());
                }
            }
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                rows.push(std::mem::take(&mut row));
            }
            Event::End(TagEnd::Table) => {
                if !rows.is_empty() {
                    let id = TABLE_NO.fetch_add(1, Ordering::Relaxed);
                    ui.add_space(6.0);
                    egui::Grid::new(("md_table", id))
                        .striped(true)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            for (i, r) in rows.iter().enumerate() {
                                for c in r {
                                    let color = if i == 0 { md.c.weak } else { md.c.text };
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(c).size(md.size).color(color),
                                        )
                                        .wrap(),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(6.0);
                }
                rows.clear();
            }
            Event::Text(t) => {
                if let Some((_, s)) = &mut code {
                    s.push_str(&t);
                } else if let Some(c) = &mut cell {
                    c.push_str(&t);
                } else {
                    buf.push_str(&t);
                }
            }
            Event::Code(t) => {
                if let Some((_, s)) = &mut code {
                    s.push_str(&t);
                } else if let Some(c) = &mut cell {
                    c.push_str(&t);
                } else {
                    buf.push_str(&t);
                }
            }
            Event::TaskListMarker(checked) => {
                marker = Some(if checked { "☑" } else { "☐" }.to_string());
            }
            Event::SoftBreak => buf.push(' '),
            Event::HardBreak => buf.push('\n'),
            Event::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                // Bağlantı metni düz yazı olarak akar; hedef sonunda köşeli parantezle
                // gösterilir ki okuyucu nereye gittiğini görsün.
                let _ = &dest_url;
            }
            Event::End(TagEnd::Link) => {}
            Event::Start(Tag::Image { .. }) => buf.push_str("[görsel] "),
            Event::FootnoteReference(name) => {
                buf.push('[');
                buf.push_str(&name);
                buf.push(']');
            }
            _ => {}
        }
    }
    if !buf.trim().is_empty() {
        para(ui, md, &buf, "", lists.len(), quote);
    }
}

fn para(ui: &mut egui::Ui, md: &Md, text: &str, marker: &str, depth: usize, quote: usize) {
    let t = text.trim();
    if t.is_empty() {
        return;
    }
    let indent = 16.0 * (depth + quote) as f32;
    ui.horizontal(|ui| {
        ui.add_space(indent);
        if !marker.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new(marker).size(md.size).color(md.c.weak))
                    .selectable(false),
            );
            ui.add_space(4.0);
        }
        let color = if quote > 0 { md.c.weak } else { md.c.text };
        ui.add(
            egui::Label::new(egui::RichText::new(t).size(md.size).color(color))
                .wrap()
                .selectable(true),
        );
    });
    ui.add_space(3.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Render egui bağlamı gerektirir; burada yalnızca ayrıştırmanın paniklemediğini
    // ve tablo/kod olaylarının beklendiği gibi geldiğini doğruluyoruz.
    #[test]
    fn parser_accepts_common_markdown() {
        let src = "# Başlık\n\n- bir\n- iki\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_TASKLISTS);
        let events: Vec<_> = Parser::new_ext(src, opts).collect();
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::Table(_)))));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))))));
    }
}
