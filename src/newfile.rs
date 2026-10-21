//! "Yeni" menüsünde oluşturulan dosya türleri: her türün menü etiketi, ikonu ve
//! başlangıç içeriği. Office türleri 0 baytlık dosya yerine geçerli, boş OOXML
//! paketleri üretir; yoksa Office dosyayı bozuk sanıp onarım ister. Zip paketleri
//! sıkıştırmasız (STORE) yöntemle elle yazılır, böylece yeni bağımlılık gerekmez.

use egui::Color32;

/// "Yeni" menüsündeki tek bir tür
pub struct NewSpec {
    /// Menü ikonu için emoji glifi (tek renkli çizilir, tint ile boyanır)
    pub emoji: &'static str,
    /// İkon rengi; None = satırın kendi metin rengi
    pub tint: Option<Color32>,
    /// Menü etiketi (Türkçe i18n anahtarı)
    pub label_key: &'static str,
    /// Oluşturulan dosyanın öntanımlı adı (i18n anahtarı)
    pub name_key: &'static str,
    /// Dosyanın başlangıç içeriği (klasör ve metin belgesi boş)
    pub template: fn() -> Vec<u8>,
}

fn empty() -> Vec<u8> {
    Vec::new()
}

/// 0 = klasör (özel davranış), 1 = metin belgesi, diğerleri şablonlu belgeler
pub const KINDS: &[NewSpec] = &[
    NewSpec {
        emoji: "📁",
        tint: Some(Color32::from_rgb(232, 184, 74)),
        label_key: "Yeni klasör",
        name_key: "Yeni klasör",
        template: empty,
    },
    NewSpec {
        emoji: "📝",
        tint: None,
        label_key: "Metin Belgesi",
        name_key: "Yeni Metin Belgesi.txt",
        template: empty,
    },
    NewSpec {
        emoji: "📘",
        tint: Some(Color32::from_rgb(43, 87, 154)),
        label_key: "Word Belgesi",
        name_key: "Yeni Word Belgesi.docx",
        template: docx,
    },
    NewSpec {
        emoji: "📊",
        tint: Some(Color32::from_rgb(33, 115, 70)),
        label_key: "Excel Çalışma Kitabı",
        name_key: "Yeni Excel Çalışma Kitabı.xlsx",
        template: xlsx,
    },
    NewSpec {
        emoji: "📽",
        tint: Some(Color32::from_rgb(210, 71, 38)),
        label_key: "PowerPoint Sunusu",
        name_key: "Yeni PowerPoint Sunusu.pptx",
        template: pptx,
    },
    NewSpec {
        emoji: "📜",
        tint: Some(Color32::from_rgb(140, 120, 80)),
        label_key: "Zengin Metin Belgesi",
        name_key: "Yeni Zengin Metin Belgesi.rtf",
        template: rtf,
    },
    NewSpec {
        emoji: "📕",
        tint: Some(Color32::from_rgb(201, 49, 43)),
        label_key: "PDF Belgesi",
        name_key: "Yeni PDF Belgesi.pdf",
        template: pdf,
    },
    NewSpec {
        emoji: "🗜",
        tint: Some(Color32::from_rgb(106, 158, 222)),
        label_key: "Sıkıştırılmış klasör",
        name_key: "Yeni Sıkıştırılmış klasör.zip",
        template: zip,
    },
];

// ---------------------------------------------------------------- küçük zip yazıcısı

/// Standart CRC-32 (0xEDB88320 polinomu), tablosuz bit bazında
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB88320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Saklama (sıkıştırmasız) yöntemiyle çok parçalı zip üretir. Tarih alanı 0 olursa
/// bazı arşivciler şüphelenir, geçerli bir DOS tarihi yazılır (21 Eylül 2023).
fn zip_store(files: &[(&str, &[u8])]) -> Vec<u8> {
    const DOS_DATE: u16 = 0x5735;
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let ofs = out.len() as u32;
        let crc = crc32(data);
        let len = data.len() as u32;
        let nm = name.as_bytes();
        out.extend_from_slice(&0x04034B50_u32.to_le_bytes()); // yerel başlık imzası
        out.extend_from_slice(&(20_u16).to_le_bytes()); // sürüm
        out.extend_from_slice(&(0_u16).to_le_bytes()); // bayraklar
        out.extend_from_slice(&(0_u16).to_le_bytes()); // yöntem: STORE
        out.extend_from_slice(&(0_u16).to_le_bytes()); // saat
        out.extend_from_slice(&DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(nm.len() as u16).to_le_bytes());
        out.extend_from_slice(&(0_u16).to_le_bytes()); // ek alan
        out.extend_from_slice(nm);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x02014B50_u32.to_le_bytes()); // merkezi dizin imzası
        central.extend_from_slice(&(20_u16).to_le_bytes()); // üreten sürüm
        central.extend_from_slice(&(20_u16).to_le_bytes()); // gereken sürüm
        central.extend_from_slice(&(0_u16).to_le_bytes()); // bayraklar
        central.extend_from_slice(&(0_u16).to_le_bytes()); // yöntem
        central.extend_from_slice(&(0_u16).to_le_bytes()); // saat
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&(nm.len() as u16).to_le_bytes());
        central.extend_from_slice(&(0_u16).to_le_bytes()); // ek alan
        central.extend_from_slice(&(0_u16).to_le_bytes()); // yorum
        central.extend_from_slice(&(0_u16).to_le_bytes()); // disk numarası
        central.extend_from_slice(&(0_u16).to_le_bytes()); // iç öznitelik
        central.extend_from_slice(&(0_u32).to_le_bytes()); // dış öznitelik
        central.extend_from_slice(&ofs.to_le_bytes());
        central.extend_from_slice(nm);
    }
    let body = out.len() as u32;
    let clen = central.len() as u32;
    let count = files.len() as u16;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054B50_u32.to_le_bytes()); // EOCD imzası
    out.extend_from_slice(&(0_u16).to_le_bytes()); // bu disk
    out.extend_from_slice(&(0_u16).to_le_bytes()); // merkezi dizinin diski
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&clen.to_le_bytes());
    out.extend_from_slice(&body.to_le_bytes());
    out.extend_from_slice(&(0_u16).to_le_bytes()); // yorum
    out
}

// ---------------------------------------------------------------- Office şablonları

const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

fn docx() -> Vec<u8> {
    let content_types = format!(
        "{DECL}\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/></Types>"
    );
    let rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/officeDocument\" Target=\"word/document.xml\"/></Relationships>"
    );
    let document =
        format!("{DECL}\n<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p/></w:body></w:document>");
    zip_store(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("word/document.xml", document.as_bytes()),
    ])
}

fn xlsx() -> Vec<u8> {
    let content_types = format!(
        "{DECL}\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"
    );
    let rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>"
    );
    let workbook = format!(
        "{DECL}\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"{NS_R}\">\
<sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>"
    );
    let sheet = format!("{DECL}\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData/></worksheet>");
    zip_store(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
    ])
}

fn pptx() -> Vec<u8> {
    let content_types = format!(
        "{DECL}\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/slides/slide1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/></Types>"
    );
    let rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/officeDocument\" Target=\"ppt/presentation.xml\"/></Relationships>"
    );
    let presentation = format!(
        "{DECL}\n<p:presentation xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
<p:sldIdLst><p:sldId id=\"256\" r:id=\"rId2\"/></p:sldIdLst>\
<p:sldSz cx=\"9144000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>"
    );
    let presentation_rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"{NS_R}/slide\" Target=\"slides/slide1.xml\"/></Relationships>"
    );
    let master = format!(
        "{DECL}\n<p:sldMaster xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">\
<p:cSld><p:spTree><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst></p:sldMaster>"
    );
    let master_rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"{NS_R}/theme\" Target=\"../theme/theme1.xml\"/></Relationships>"
    );
    let layout = format!(
        "{DECL}\n<p:sldLayout xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\" type=\"blank\">\
<p:cSld><p:spTree><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"
    );
    let layout_rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>"
    );
    let slide = format!(
        "{DECL}\n<p:sld xmlns:a=\"{NS_A}\" xmlns:r=\"{NS_R}\" xmlns:p=\"{NS_P}\">\
<p:cSld><p:spTree><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>\
<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    );
    let slide_rels = format!(
        "{DECL}\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"{NS_R}/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/></Relationships>"
    );
    let theme = format!(
        "{DECL}\n<a:theme xmlns:a=\"{NS_A}\" name=\"Office\"><a:themeElements>\
<a:clrScheme name=\"Office\">\
<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"44546A\"/></a:dk2><a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1><a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2>\
<a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3><a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4>\
<a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5><a:accent6><a:srgbClr val=\"70AD46\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Office\"><a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Office\">\
<a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:fillStyleLst>\
<a:lnStyleLst>\
<a:ln w=\"6350\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln>\
<a:ln w=\"19050\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:ln></a:lnStyleLst>\
<a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>\
<a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill></a:bgFillStyleLst>\
</a:fmtScheme></a:themeElements></a:theme>"
    );
    zip_store(&[
        ("[Content_Types].xml", content_types.as_bytes()),
        ("_rels/.rels", rels.as_bytes()),
        ("ppt/presentation.xml", presentation.as_bytes()),
        (
            "ppt/_rels/presentation.xml.rels",
            presentation_rels.as_bytes(),
        ),
        ("ppt/slideMasters/slideMaster1.xml", master.as_bytes()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            master_rels.as_bytes(),
        ),
        ("ppt/slideLayouts/slideLayout1.xml", layout.as_bytes()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            layout_rels.as_bytes(),
        ),
        ("ppt/slides/slide1.xml", slide.as_bytes()),
        ("ppt/slides/_rels/slide1.xml.rels", slide_rels.as_bytes()),
        ("ppt/theme/theme1.xml", theme.as_bytes()),
    ])
}

fn rtf() -> Vec<u8> {
    br"{\rtf1\ansi\deff0{\fonttbl{\f0\fswiss Arial;}}\f0\fs24\par}".to_vec()
}

fn pdf() -> Vec<u8> {
    let mut out = b"%PDF-1.4\n".to_vec();
    let objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".as_slice(),
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".as_slice(),
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n".as_slice(),
    ];
    let mut offsets = Vec::new();
    for obj in objs {
        offsets.push(out.len() as u32);
        out.extend_from_slice(obj);
    }
    let xref_at = out.len() as u32;
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f\r\n");
    for ofs in offsets {
        out.extend_from_slice(format!("{ofs:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n");
    out.extend_from_slice(xref_at.to_string().as_bytes());
    out.extend_from_slice(b"\n%%EOF\n");
    out
}

/// Boş zip arşivi: yalnızca EOCD kaydı
fn zip() -> Vec<u8> {
    let mut out = vec![0x50, 0x4B, 0x05, 0x06];
    out.extend_from_slice(&[0_u8; 18]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_reference() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn every_kind_has_content() {
        for (i, k) in KINDS.iter().enumerate().skip(1) {
            let data = (k.template)();
            if i == 1 {
                assert!(data.is_empty(), "metin belgesi boş olmalı");
            } else {
                assert!(!data.is_empty(), "kind {i}");
            }
        }
    }

    #[test]
    fn zip_templates_have_valid_magic() {
        for i in [4, 7] {
            let data = (KINDS[i].template)();
            let n = data.len();
            if i == 7 {
                assert_eq!(&data[..4], &[0x50, 0x4B, 0x05, 0x06]);
            } else {
                assert_eq!(&data[n - 22..n - 18], &0x06054B50_u32.to_le_bytes());
            }
        }
    }

    #[test]
    fn pdf_ends_with_eof() {
        assert!(pdf().ends_with(b"%%EOF\n"));
    }
}
