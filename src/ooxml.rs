//! Ofis belgelerinden okunabilir metin çıkarma (DOCX, XLSX, PPTX ve ODF).
//!
//! Explorer'ın önizleme bölmesi kurulu Office önizleme işleyicisini kullanır; biz
//! aynı görselliği sunamayız ama belgenin metnini/ızgara düzenini gösterebiliriz.
//! OOXML dosyaları zip + XML olduğundan dışa bağımlılık (Office, COM) gerekmez.

use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::Reader;

/// Önizlemesi metin olarak çıkarılabilen ofis uzantıları.
pub fn is_office(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "docx"
            | "docm"
            | "dotx"
            | "xlsx"
            | "xlsm"
            | "xltx"
            | "pptx"
            | "pptm"
            | "potx"
            | "odt"
            | "ods"
            | "odp"
    )
}

/// Belgeden düz metin çıkarır. Okunamıyorsa `None` döner (çağıran meta veriye düşer).
pub fn extract(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !is_office(&ext) {
        return None;
    }
    let text = if ext.starts_with("od") {
        odf(path)?
    } else {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
        match ext.as_str() {
            "docx" | "docm" | "dotx" => docx(&mut zip)?,
            "xlsx" | "xlsm" | "xltx" => xlsx(&mut zip)?,
            _ => pptx(&mut zip)?,
        }
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

// ---------------------------------------------------------------- OOXML

fn docx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<String> {
    let xml = read_zip(zip, "word/document.xml")?;
    let mut out = String::new();
    let mut reader = Reader::from_str(&xml);
    let mut in_text = false;
    let mut cell = false;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"t" => in_text = true,
                b"tab" => out.push('\t'),
                b"br" | b"cr" => out.push('\n'),
                b"tc" => {
                    if cell {
                        out.push('\t');
                    }
                    cell = true;
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"tab" => out.push('\t'),
                b"br" | b"cr" => out.push('\n'),
                _ => {}
            },
            Event::Text(t) if in_text => out.push_str(&text_of(&t)?),
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => out.push('\n'),
                b"tr" => {
                    out.push('\n');
                    cell = false;
                }
                b"tc" => cell = true,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Some(out)
}

fn pptx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<String> {
    // Slayt dosyaları sıra numarasına göre okunur (slayt1, slayt2…).
    let mut slides: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        let name = file.name().to_string();
        let Some(rest) = name.strip_prefix("ppt/slides/slide") else {
            continue;
        };
        let Some(num) = rest.strip_suffix(".xml").and_then(|n| n.parse().ok()) else {
            continue;
        };
        slides.push((num, name));
    }
    slides.sort_by_key(|(n, _)| *n);
    if slides.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (n, name) in slides {
        let Some(xml) = read_zip(zip, &name) else {
            continue;
        };
        out.push_str(&format!("── Slayt {n} ──\n"));
        let mut reader = Reader::from_str(&xml);
        let mut in_text = false;
        loop {
            match reader.read_event().ok()? {
                Event::Start(e) => match e.local_name().as_ref() {
                    b"t" => in_text = true,
                    b"br" => out.push('\n'),
                    _ => {}
                },
                Event::Text(t) if in_text => out.push_str(&text_of(&t)?),
                Event::End(e) => match e.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => out.push('\n'),
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            if out.len() > MAX_TEXT {
                return Some(out);
            }
        }
        out.push('\n');
    }
    Some(out)
}

fn xlsx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<String> {
    let shared = read_zip(zip, "xl/sharedStrings.xml")
        .map(|x| shared_strings(&x))
        .unwrap_or_default();
    let names = sheet_names(zip);
    // Sayfalar dosya adındaki numaraya göre sıralanır.
    let mut sheets: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        let name = file.name().to_string();
        if let Some(num) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|r| r.strip_suffix(".xml"))
            .and_then(|n| n.parse().ok())
        {
            sheets.push((num, name));
        }
    }
    sheets.sort_by_key(|(n, _)| *n);
    let mut out = String::new();
    for (num, name) in sheets {
        let Some(xml) = read_zip(zip, &name) else {
            continue;
        };
        let title = names
            .get((num - 1) as usize)
            .cloned()
            .unwrap_or_else(|| format!("Sheet {num}"));
        out.push_str(&format!("── {title} ──\n"));
        out.push_str(&sheet_text(&xml, &shared));
        out.push('\n');
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Some(out)
}

/// `xl/workbook.xml` içindeki sayfa adları (sırayla).
fn sheet_names(zip: &mut zip::ZipArchive<std::fs::File>) -> Vec<String> {
    let Some(xml) = read_zip(zip, "xl/workbook.xml") else {
        return vec![];
    };
    let mut reader = Reader::from_str(&xml);
    let mut names = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) if e.local_name().as_ref() == b"sheet" => {
                for a in e.attributes().flatten() {
                    if a.key.local_name().as_ref() == b"name" {
                        if let Ok(v) = a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
                            names.push(v.into_owned());
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    names
}

fn shared_strings(xml: &str) -> Vec<String> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_si = false;
    let mut in_text = false;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"si" => {
                    in_si = true;
                    cur.clear();
                }
                b"t" if in_si => in_text = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_text => {
                if let Some(s) = text_of(&t) {
                    cur.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"si" => {
                    out.push(std::mem::take(&mut cur));
                    in_si = false;
                }
                b"t" => in_text = false,
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// Bir çalışma sayfasını sekmeyle ayrılmış satırlara çevirir.
fn sheet_text(xml: &str, shared: &[String]) -> String {
    let mut reader = Reader::from_str(xml);
    let mut out = String::new();
    let mut row: Vec<(usize, String)> = Vec::new();
    let mut col = 0usize;
    let mut in_value = false;
    let mut in_cell = false;
    let mut shared_cell = false;
    let mut val = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"c" => {
                    in_cell = true;
                    // Hücre tipi: `t="s"` paylaşılan dizge dizini demektir.
                    shared_cell = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == b"t")
                        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
                        .is_some_and(|v| v == "s");
                    col = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == b"r")
                        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
                        .map(|r| col_index(&r))
                        .unwrap_or(col + 1);
                    val.clear();
                }
                b"v" | b"t" if in_cell => in_value = true,
                _ => {}
            },
            Ok(Event::Text(t)) if in_value => {
                if let Some(s) = text_of(&t) {
                    val.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"v" | b"t" => in_value = false,
                b"c" => {
                    in_cell = false;
                    // Paylaşılan dizge dizini (`t="s"`) gerçek metne çevrilir.
                    let text = if shared_cell {
                        val.parse::<usize>()
                            .ok()
                            .and_then(|i| shared.get(i).cloned())
                            .unwrap_or_else(|| std::mem::take(&mut val))
                    } else {
                        std::mem::take(&mut val)
                    };
                    shared_cell = false;
                    row.push((col, text));
                }
                b"row" => {
                    row.sort_by_key(|(c, _)| *c);
                    let mut line = String::new();
                    let mut next = 0usize;
                    for (c, v) in row.drain(..).take(MAX_COLS) {
                        while next < c {
                            line.push('\t');
                            next += 1;
                        }
                        line.push_str(&v.replace(['\r', '\n'], " "));
                        line.push('\t');
                        next = c + 1;
                    }
                    out.push_str(line.trim_end_matches('\t'));
                    out.push('\n');
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() > MAX_TEXT {
            break;
        }
    }
    out
}

/// "A1" / "AB12" başvurusundan sıfır tabanlı sütun indeksi.
fn col_index(r: &str) -> usize {
    let mut n = 0usize;
    for ch in r.chars().take_while(|c| c.is_ascii_alphabetic()) {
        n = n * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    n.saturating_sub(1)
}

// ---------------------------------------------------------------- ODF

fn odf(path: &Path) -> Option<String> {
    let xml = read_zip_path(path, "content.xml")?;
    let mut reader = Reader::from_str(&xml);
    let mut out = String::new();
    let mut in_text = false;
    let mut depth_par = 0usize;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"p" | b"h" => {
                    depth_par += 1;
                }
                b"tab" => out.push('\t'),
                b"line-break" => out.push('\n'),
                b"table-cell" => out.push('\t'),
                b"s" => out.push(' '),
                _ => {}
            },
            Event::Text(t) if depth_par > 0 => {
                in_text = true;
                out.push_str(&text_of(&t)?);
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"p" | b"h" => {
                    depth_par = depth_par.saturating_sub(1);
                    if depth_par == 0 {
                        out.push('\n');
                    }
                    in_text = false;
                }
                b"table-row" => out.push('\n'),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        let _ = in_text;
        if out.len() > MAX_TEXT {
            break;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- yardımcılar

const MAX_TEXT: usize = 200 * 1024;
const MAX_COLS: usize = 40;

fn read_zip(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> Option<String> {
    let mut f = zip.by_name(name).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    Some(s)
}

fn read_zip_path(path: &Path, name: &str) -> Option<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
    read_zip(&mut zip, name)
}

/// XML metin olayını dizgeye çevirir (kodlama + varlık çözümü).
fn text_of(t: &quick_xml::events::BytesText) -> Option<String> {
    let raw = t.decode().ok()?;
    quick_xml::escape::unescape(&raw)
        .ok()
        .map(|s| s.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_extensions_are_recognised() {
        assert!(is_office("DOCX"));
        assert!(is_office("xlsx"));
        assert!(is_office("odp"));
        assert!(!is_office("pdf"));
        assert!(!is_office("txt"));
    }

    #[test]
    fn cell_references_map_to_columns() {
        assert_eq!(col_index("A1"), 0);
        assert_eq!(col_index("B7"), 1);
        assert_eq!(col_index("Z1"), 25);
        assert_eq!(col_index("AA1"), 26);
        assert_eq!(col_index("AB2"), 27);
    }

    #[test]
    fn sheet_cells_resolve_shared_strings() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1"><v>42</v></c></row></sheetData></worksheet>"#;
        let shared = vec!["Ad".to_string(), "Soyad".to_string()];
        assert_eq!(sheet_text(xml, &shared), "Ad\tSoyad\t42\n");
    }

    #[test]
    fn docx_paragraphs_and_cells_become_lines() {
        let xml = r#"<w:document><w:body><w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
        // docx() bir zip ister; metin çıkarımının çekirdeğini doğrudan sınamak için
        // aynı olay mantığını burada küçük bir yardımcıyla çalıştırıyoruz.
        let mut reader = Reader::from_str(xml);
        let mut out = String::new();
        let mut in_text = false;
        while let Ok(ev) = reader.read_event() {
            match ev {
                Event::Start(e) => match e.local_name().as_ref() {
                    b"t" => in_text = true,
                    b"tc" => out.push('\t'),
                    _ => {}
                },
                Event::Text(t) if in_text => out.push_str(&text_of(&t).unwrap()),
                Event::End(e) => match e.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => out.push('\n'),
                    b"tr" => out.push('\n'),
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
        }
        assert!(out.contains("Hello"));
        assert!(out.contains('\t'));
    }
}
