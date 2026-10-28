//! Markdown önizlemesi.
//!
//! `pulldown-cmark` ile olay olay çözülür, egui widget'larına çevrilir. Windows
//! Gezgini'nde Markdown önizlemesi yok; bu bölme "okunabilir" olmayı hedefler:
//! başlıklar, listeler, alıntılar, tablolar ve (varsa dili tanınan) kod blokları
//! renklendirilir. Kalın yazı kullanılmaz — proje kuralı gereği vurgu renkle
//! yapılır. Satır içi biçimler (`**kalın**`, `*eğik*`, `` `kod` ``, bağlantı,
//! üstü çizili) tek bir `LayoutJob` içinde çizilir; böylece karışık satırlar
//! düz metne indirgenmez.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::highlight;

pub struct Md {
    pub c: highlight::Colors,
    pub size: f32,
}

/// Satır içi biçim yığını (iç içe `**` / `*` / `~~` sayılır).
#[derive(Default, Clone, Copy)]
struct Inline {
    strong: u32,
    emph: u32,
    strike: u32,
    link: bool,
    code: bool,
}

pub fn render(ui: &mut egui::Ui, src: &str, md: &Md) {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let mut job = egui::text::LayoutJob::default();
    let mut inline = Inline::default();
    let mut heading: Option<u8> = None;
    let mut cur_size = md.size;
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
                cur_size = match level as u8 {
                    1 => md.size * 1.6,
                    2 => md.size * 1.34,
                    3 => md.size * 1.16,
                    _ => md.size * 1.04,
                };
                job = egui::text::LayoutJob::default();
            }
            Event::End(TagEnd::Heading(_)) => {
                let lvl = heading.take().unwrap_or(1);
                cur_size = md.size;
                if !job.text.trim().is_empty() {
                    ui.add_space(if lvl <= 2 { 12.0 } else { 8.0 });
                    ui.add(egui::Label::new(job.clone()).wrap());
                    ui.add_space(3.0);
                }
                job = egui::text::LayoutJob::default();
            }
            Event::Start(Tag::Paragraph) => job = egui::text::LayoutJob::default(),
            Event::End(TagEnd::Paragraph) => {
                let m = marker.take().unwrap_or_default();
                let j = std::mem::take(&mut job);
                para(ui, md, j, &m, lists.len(), quote);
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
                if !job.text.trim().is_empty() {
                    let m = marker.take().unwrap_or_default();
                    let j = std::mem::take(&mut job);
                    para(ui, md, j, &m, lists.len(), quote);
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
                                // Aynı önizlemede iki kod bloğu olabilir: her blok
                                // kendi kaydırma kimliğini almalı (yoksa egui
                                // "ID clash" uyarısıyla kırmızı çerçeve çizer).
                                .id_salt(ui.next_auto_id().with("md_code"))
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
                    ui.add_space(6.0);
                    let id = ui.next_auto_id().with("md_table");
                    egui::Grid::new(id)
                        .striped(true)
                        .spacing([16.0, 6.0])
                        .show(ui, |ui| {
                            for (i, r) in rows.iter().enumerate() {
                                for c in r {
                                    let color = if i == 0 { md.c.accent } else { md.c.text };
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
            Event::Start(Tag::Strong) => inline.strong += 1,
            Event::End(TagEnd::Strong) => inline.strong = inline.strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => inline.emph += 1,
            Event::End(TagEnd::Emphasis) => inline.emph = inline.emph.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => inline.strike += 1,
            Event::End(TagEnd::Strikethrough) => inline.strike = inline.strike.saturating_sub(1),
            Event::Text(t) => {
                if let Some((_, s)) = &mut code {
                    s.push_str(&t);
                } else if let Some(c) = &mut cell {
                    c.push_str(&t);
                } else {
                    let f = run_format(&inline, md, cur_size, quote > 0);
                    job.append(&t, 0.0, f);
                }
            }
            Event::Code(t) => {
                if let Some((_, s)) = &mut code {
                    s.push_str(&t);
                } else if let Some(c) = &mut cell {
                    c.push_str(&t);
                } else {
                    let mut inl = inline;
                    inl.code = true;
                    let f = run_format(&inl, md, cur_size, quote > 0);
                    job.append(&t, 0.0, f);
                }
            }
            Event::TaskListMarker(checked) => {
                marker = Some(if checked { "☑" } else { "☐" }.to_string());
            }
            Event::SoftBreak => {
                // Yumuşak satır sonu tek boşluk olarak akar; biçim korunur.
                let f = run_format(&inline, md, cur_size, quote > 0);
                job.append(" ", 0.0, f);
            }
            Event::HardBreak => {
                let f = run_format(&inline, md, cur_size, quote > 0);
                job.append("\n", 0.0, f);
            }
            Event::Rule => {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);
            }
            Event::Start(Tag::Link { .. }) => inline.link = true,
            Event::End(TagEnd::Link) => inline.link = false,
            Event::Start(Tag::Image { .. }) => job.append(
                "[görsel] ",
                0.0,
                run_format(&inline, md, cur_size, quote > 0),
            ),
            Event::FootnoteReference(name) => {
                let f = run_format(&inline, md, cur_size, quote > 0);
                job.append("[", 0.0, f.clone());
                job.append(&name, 0.0, f.clone());
                job.append("]", 0.0, f);
            }
            _ => {}
        }
    }
    if !job.text.trim().is_empty() {
        para(ui, md, job, "", lists.len(), quote);
    }
}

/// Satır içi biçim: kalın yerine vurgu rengi, eğik yazı, satır içi kod arka planı.
fn run_format(inl: &Inline, md: &Md, size: f32, quote: bool) -> egui::TextFormat {
    let mut f = egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color: if quote { md.c.weak } else { md.c.text },
        ..Default::default()
    };
    if inl.code {
        f.font_id = egui::FontId::monospace((size - 1.0).max(9.0));
        f.color = md.c.accent;
        f.background = if md.c.dark {
            egui::Color32::from_white_alpha(18)
        } else {
            egui::Color32::from_black_alpha(14)
        };
    }
    if inl.emph > 0 {
        f.italics = true;
    }
    if inl.strike > 0 {
        f.strikethrough = egui::Stroke::new(1.0_f32, md.c.weak);
    }
    if inl.link {
        f.color = md.c.accent;
        f.underline = egui::Stroke::new(1.0_f32, md.c.accent);
    }
    if inl.strong > 0 && !inl.code && !inl.link {
        // Proje kuralı: kalın yazı yok; vurgu renkle yapılır.
        f.color = md.c.accent;
    }
    f
}

fn para(
    ui: &mut egui::Ui,
    md: &Md,
    mut job: egui::text::LayoutJob,
    marker: &str,
    depth: usize,
    quote: usize,
) {
    if job.text.trim().is_empty() {
        return;
    }
    job.wrap.max_width = f32::INFINITY;
    let indent = 16.0 * (depth.saturating_sub(1) + quote) as f32;
    ui.horizontal_top(|ui| {
        ui.add_space(indent);
        if !marker.is_empty() {
            ui.add(
                egui::Label::new(egui::RichText::new(marker).size(md.size).color(md.c.weak))
                    .selectable(false),
            );
            ui.add_space(4.0);
        }
        ui.add(egui::Label::new(job).wrap().selectable(true));
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
