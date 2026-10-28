//! Ofis belgelerinden **yapılandırılmış** içerik çıkarma (DOCX, XLSX, PPTX ve ODF).
//!
//! Explorer'ın önizleme bölmesi kurulu Office önizleme işleyicisini kullanır; biz
//! aynı görselliği sunamayız ama belgenin başlıklarını, paragraflarını, listelerini,
//! tablolarını, çalışma sayfalarını ve slaytlarını çıkarıp `docview` ile çizebiliriz.
//! OOXML dosyaları zip + XML olduğundan dışa bağımlılık (Office, COM) gerekmez.

use std::io::Read;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

/// Önizlemesi yapılandırılmış olarak çıkarılabilen ofis uzantıları.
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

/// Belge gövdesindeki blok.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Başlık (1–4 arası; daha derin başlıklar 4 sayılır).
    Heading(u8, String),
    Para(String),
    /// Madde işaretli satır; `level` girinti seviyesi (0 tabanlı).
    Bullet {
        level: u8,
        text: String,
    },
    /// Tablo satırları; ilk satır çoğunlukla başlıktır.
    Table {
        rows: Vec<Vec<String>>,
    },
}

/// Çalışma sayfası (XLSX/ODS): ad + hücre satırları.
#[derive(Debug, Clone, PartialEq)]
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

/// Slayt (PPTX/ODP): başlık + girintili satırlar.
#[derive(Debug, Clone, PartialEq)]
pub struct Slide {
    pub no: u32,
    pub title: String,
    pub lines: Vec<(u8, String)>,
}

/// Çıkarılan belge.
#[derive(Debug, Clone, PartialEq)]
pub enum Doc {
    /// Metin belgesi (DOCX/ODT): başlık, paragraf, liste, tablo.
    Text(Vec<Block>),
    /// Hesap tablosu (XLSX/ODS).
    Sheets(Vec<Sheet>),
    /// Sunum (PPTX/ODP).
    Slides(Vec<Slide>),
}

impl Doc {
    pub fn is_empty(&self) -> bool {
        match self {
            Doc::Text(b) => b.is_empty(),
            Doc::Sheets(s) => s.iter().all(|s| s.rows.is_empty()),
            Doc::Slides(s) => s.is_empty(),
        }
    }
}

/// Belgeyi çıkarır. Okunamıyorsa `None` döner (çağıran meta veriye düşer).
pub fn document(path: &Path) -> Option<Doc> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !is_office(&ext) {
        return None;
    }
    let doc = if ext.starts_with("od") {
        odf(path)?
    } else {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).ok()?).ok()?;
        match ext.as_str() {
            "docx" | "docm" | "dotx" => Doc::Text(docx(&mut zip)?),
            "xlsx" | "xlsm" | "xltx" => Doc::Sheets(xlsx(&mut zip)?),
            _ => Doc::Slides(pptx(&mut zip)?),
        }
    };
    if doc.is_empty() {
        None
    } else {
        Some(doc)
    }
}

// ---------------------------------------------------------------- OOXML

/// DOCX: paragraflar, başlık stilleri, madde işaretleri ve tablolar.
fn docx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<Vec<Block>> {
    let xml = read_zip(zip, "word/document.xml")?;
    let mut blocks: Vec<Block> = Vec::new();
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Vec<String> = Vec::new();
    let mut cell: Option<String> = None;
    let mut para = String::new();
    let mut style: Option<String> = None;
    let mut list_level: Option<u8> = None;
    let mut in_text = false;
    let mut total = 0usize;

    let mut reader = Reader::from_str(&xml);
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"p" => {
                    para.clear();
                    style = None;
                    list_level = None;
                }
                b"pStyle" => style = attr(&e, b"val"),
                b"numPr" => {
                    list_level.get_or_insert(0);
                }
                b"ilvl" => {
                    if let Some(v) = attr(&e, b"val").and_then(|v| v.parse::<u8>().ok()) {
                        list_level = Some(v.min(8));
                    }
                }
                b"t" => in_text = true,
                b"tab" => para.push('\t'),
                b"br" | b"cr" => para.push(' '),
                b"tbl" => table = Some(Vec::new()),
                b"tr" => row.clear(),
                b"tc" => cell = Some(String::new()),
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"tab" => para.push('\t'),
                b"br" | b"cr" => para.push(' '),
                b"pStyle" => style = attr(&e, b"val"),
                b"ilvl" => {
                    if let Some(v) = attr(&e, b"val").and_then(|v| v.parse::<u8>().ok()) {
                        list_level = Some(v.min(8));
                    }
                }
                _ => {}
            },
            Event::Text(t) if in_text => {
                if let Some(s) = text_of(&t) {
                    total += s.len();
                    para.push_str(&s);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"t" => in_text = false,
                b"p" => {
                    if let Some(c) = cell.as_mut() {
                        // Hücre içi paragraflar alt satıra yazılır.
                        let t = para.trim();
                        if !t.is_empty() {
                            if !c.is_empty() {
                                c.push('\n');
                            }
                            c.push_str(t);
                        }
                    } else {
                        push_block(&mut blocks, &mut para, &style, list_level);
                    }
                    para.clear();
                }
                b"tc" => {
                    let t = cell.take().unwrap_or_default();
                    row.push(t.trim().to_string());
                }
                b"tr" => {
                    if !row.is_empty() {
                        if let Some(t) = table.as_mut() {
                            t.push(std::mem::take(&mut row));
                        }
                    }
                }
                b"tbl" => {
                    if let Some(rows) = table.take() {
                        if !rows.is_empty() {
                            blocks.push(Block::Table { rows });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        if total > MAX_TEXT {
            break;
        }
    }
    Some(blocks)
}

/// Paragrafı bloğa çevirir (başlık / madde / düz metin).
fn push_block(
    blocks: &mut Vec<Block>,
    para: &mut str,
    style: &Option<String>,
    list_level: Option<u8>,
) {
    let text = para.trim().to_string();
    if text.is_empty() {
        return;
    }
    if let Some(level) = list_level {
        blocks.push(Block::Bullet { level, text });
        return;
    }
    let style = style.as_deref().unwrap_or_default();
    let heading = if let Some(rest) = style.strip_prefix("Heading") {
        rest.trim().parse::<u8>().ok()
    } else if style == "Title" {
        Some(1)
    } else if style.eq_ignore_ascii_case("subtitle") {
        Some(2)
    } else {
        None
    };
    match heading {
        Some(n) => blocks.push(Block::Heading(n.clamp(1, 4), text)),
        None => blocks.push(Block::Para(text)),
    }
}

/// PPTX: slayt başına başlık ve girintili satırlar.
fn pptx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<Vec<Slide>> {
    // Slayt dosyaları sıra numarasına göre okunur (slide1, slide2…).
    let mut files: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        let name = file.name().to_string();
        let Some(rest) = name.strip_prefix("ppt/slides/slide") else {
            continue;
        };
        let Some(num) = rest.strip_suffix(".xml").and_then(|n| n.parse().ok()) else {
            continue;
        };
        files.push((num, name));
    }
    files.sort_by_key(|(n, _)| *n);
    if files.is_empty() {
        return None;
    }
    let mut slides = Vec::new();
    let mut total = 0usize;
    for (no, name) in files {
        let Some(xml) = read_zip(zip, &name) else {
            continue;
        };
        let mut reader = Reader::from_str(&xml);
        let mut paras: Vec<(u8, String)> = Vec::new();
        let mut para = String::new();
        let mut level = 0u8;
        let mut in_text = false;
        loop {
            match reader.read_event().ok()? {
                Event::Start(e) => match e.local_name().as_ref() {
                    b"pPr" => {
                        level = attr(&e, b"lvl")
                            .and_then(|v| v.parse::<u8>().ok())
                            .unwrap_or(0)
                            .min(8)
                    }
                    b"t" => in_text = true,
                    b"br" => para.push(' '),
                    _ => {}
                },
                Event::Empty(e) if e.local_name().as_ref() == b"br" => para.push(' '),
                Event::Text(t) if in_text => {
                    if let Some(s) = text_of(&t) {
                        total += s.len();
                        para.push_str(&s);
                    }
                }
                Event::End(e) => match e.local_name().as_ref() {
                    b"t" => in_text = false,
                    b"p" => {
                        let t = para.trim().to_string();
                        if !t.is_empty() {
                            paras.push((level, t));
                        }
                        para.clear();
                    }
                    _ => {}
                },
                Event::Eof => break,
                _ => {}
            }
            if total > MAX_TEXT {
                break;
            }
        }
        // PowerPoint başlığı genellikle ilk paragraftır (başlık yer tutucusu).
        let title = paras
            .first()
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| format!("Slayt {no}"));
        let lines = paras.into_iter().skip(1).collect();
        slides.push(Slide { no, title, lines });
        if total > MAX_TEXT {
            break;
        }
    }
    Some(slides)
}

/// XLSX: sayfa başına hücre satırları.
fn xlsx(zip: &mut zip::ZipArchive<std::fs::File>) -> Option<Vec<Sheet>> {
    let shared = read_zip(zip, "xl/sharedStrings.xml")
        .map(|x| shared_strings(&x))
        .unwrap_or_default();
    let names = sheet_names(zip);
    let mut files: Vec<(u32, String)> = Vec::new();
    for i in 0..zip.len() {
        let Ok(file) = zip.by_index(i) else { continue };
        let name = file.name().to_string();
        if let Some(num) = name
            .strip_prefix("xl/worksheets/sheet")
            .and_then(|r| r.strip_suffix(".xml"))
            .and_then(|n| n.parse().ok())
        {
            files.push((num, name));
        }
    }
    files.sort_by_key(|(n, _)| *n);
    let mut sheets = Vec::new();
    let mut total = 0usize;
    for (num, name) in files {
        let Some(xml) = read_zip(zip, &name) else {
            continue;
        };
        let title = names
            .get((num - 1) as usize)
            .cloned()
            .unwrap_or_else(|| format!("Sheet {num}"));
        let rows = sheet_rows(&xml, &shared);
        total += rows
            .iter()
            .map(|r| r.iter().map(|c| c.len()).sum::<usize>())
            .sum::<usize>();
        sheets.push(Sheet { name: title, rows });
        if total > MAX_TEXT {
            break;
        }
    }
    Some(sheets)
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
                if let Some(v) = attr(&e, b"name") {
                    names.push(v);
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

/// Bir çalışma sayfasını satır/sütun ızgarasına çevirir (boşluklar doldurulur).
fn sheet_rows(xml: &str, shared: &[String]) -> Vec<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    let mut out: Vec<Vec<String>> = Vec::new();
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
                    shared_cell = attr(&e, b"t").is_some_and(|v| v == "s");
                    col = attr(&e, b"r").map(|r| col_index(&r)).unwrap_or(col + 1);
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
                    row.push((col, text.replace(['\r', '\n'], " ")));
                }
                b"row" => {
                    row.sort_by_key(|(c, _)| *c);
                    let mut line: Vec<String> = Vec::new();
                    for (c, v) in row.drain(..).filter(|(c, _)| *c < MAX_COLS) {
                        while line.len() < c {
                            line.push(String::new());
                        }
                        line.push(v);
                    }
                    if !line.is_empty() {
                        out.push(line);
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        if out.len() > MAX_ROWS {
            break;
        }
    }
    out
}

/// Test/eski çağrılar için sekmeyle ayrılmış düz metin.
#[allow(dead_code)]
fn sheet_text(xml: &str, shared: &[String]) -> String {
    let mut out = String::new();
    for row in sheet_rows(xml, shared) {
        out.push_str(&row.join("\t"));
        out.push('\n');
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

fn odf(path: &Path) -> Option<Doc> {
    let xml = read_zip_path(path, "content.xml")?;
    let mut reader = Reader::from_str(&xml);
    let mut blocks: Vec<Block> = Vec::new();
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut row: Vec<String> = Vec::new();
    let mut cell: Option<String> = None;
    let mut para = String::new();
    let mut heading = 0u8;
    let mut list_level: Option<u8> = None;
    let mut in_par = false;
    let mut total = 0usize;
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => match e.local_name().as_ref() {
                b"h" => {
                    in_par = true;
                    heading = attr(&e, b"outline-level")
                        .and_then(|v| v.parse::<u8>().ok())
                        .unwrap_or(1)
                        .clamp(1, 4);
                }
                b"p" => {
                    in_par = true;
                    heading = 0;
                }
                b"list" => {
                    list_level.get_or_insert(0);
                }
                b"s" => para.push(' '),
                b"tab" => para.push('\t'),
                b"line-break" => para.push(' '),
                b"table" => table = Some(Vec::new()),
                b"table-row" => row.clear(),
                b"table-cell" => cell = Some(String::new()),
                _ => {}
            },
            Event::Text(t) if in_par => {
                if let Some(s) = text_of(&t) {
                    total += s.len();
                    para.push_str(&s);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"p" | b"h" => {
                    in_par = false;
                    let text = para.trim().to_string();
                    para.clear();
                    if text.is_empty() {
                    } else if let Some(c) = cell.as_mut() {
                        if !c.is_empty() {
                            c.push('\n');
                        }
                        c.push_str(&text);
                    } else if heading > 0 {
                        blocks.push(Block::Heading(heading, text));
                    } else if let Some(level) = list_level {
                        blocks.push(Block::Bullet {
                            level: level.min(8),
                            text,
                        });
                    } else {
                        blocks.push(Block::Para(text));
                    }
                }
                b"list" => list_level = None,
                b"table-cell" => {
                    let t = cell.take().unwrap_or_default();
                    row.push(t.trim().to_string());
                }
                b"table-row" => {
                    if !row.is_empty() {
                        if let Some(t) = table.as_mut() {
                            t.push(std::mem::take(&mut row));
                        }
                    }
                }
                b"table" => {
                    list_level = None;
                    if let Some(rows) = table.take() {
                        if !rows.is_empty() {
                            blocks.push(Block::Table { rows });
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        if total > MAX_TEXT {
            break;
        }
    }
    Some(Doc::Text(blocks))
}

// ---------------------------------------------------------------- yardımcılar

const MAX_TEXT: usize = 200 * 1024;
const MAX_COLS: usize = 40;
const MAX_ROWS: usize = 5_000;

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

/// Bir öğenin (yerel adı `name` olan) niteliğini okur.
fn attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.local_name().as_ref() == name)
        .and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
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
    fn sheet_rows_keep_columns_aligned() {
        let xml = r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1"><v>7</v></c></row></sheetData></worksheet>"#;
        let shared = vec!["Ad".to_string()];
        let rows = sheet_rows(xml, &shared);
        assert_eq!(rows[0], vec!["Ad", "", "7"]);
    }

    #[test]
    fn docx_headings_and_lists_become_blocks() {
        let xml = r#"<w:document><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Başlık</w:t></w:r></w:p>
            <w:p><w:pPr><w:numPr><w:ilvl w:val="1"/></w:numPr></w:pPr><w:r><w:t>Madde</w:t></w:r></w:p>
            <w:p><w:r><w:t>Düz</w:t></w:r></w:p>
        </w:body></w:document>"#;
        let blocks = docx_blocks(xml);
        assert_eq!(blocks[0], Block::Heading(2, "Başlık".into()));
        assert_eq!(
            blocks[1],
            Block::Bullet {
                level: 1,
                text: "Madde".into()
            }
        );
        assert_eq!(blocks[2], Block::Para("Düz".into()));
    }

    #[test]
    fn docx_tables_become_rows() {
        let xml = r#"<w:document><w:body><w:tbl>
            <w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc></w:tr>
            <w:tr><w:tc><w:p><w:r><w:t>C</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>D</w:t></w:r></w:p></w:tc></w:tr>
        </w:tbl></w:body></w:document>"#;
        let blocks = docx_blocks(xml);
        assert_eq!(
            blocks[0],
            Block::Table {
                rows: vec![vec!["A".into(), "B".into()], vec!["C".into(), "D".into()]]
            }
        );
    }

    /// `docx()` bir zip ister; aynı olay mantığını XML üzerinde sınamak için
    /// zip'siz bir sarmalayıcı: geçici bir zip yazmak yerine doğrudan ayrıştırır.
    fn docx_blocks(xml: &str) -> Vec<Block> {
        // docx() ile aynı kod yolu: geçici zip oluşturup "word/document.xml" ekleriz.
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "swift-ooxml-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join("t.docx");
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            z.write_all(xml.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        let mut z = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let blocks = docx(&mut z).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        blocks
    }
}
