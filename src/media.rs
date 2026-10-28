//! Önizlemede gömülü oynatıcıya giden dosya türleri.
//!
//! PDF WebView2'nin yerleşik görüntüleyicisiyle, ses/video ise HTML5
//! `<video>`/`<audio>` öğesiyle oynatılır. Yalnızca Chromium'un
//! çözebildiği kapsayıcılar listelenir; diğerleri meta veri dökümüne düşer.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MediaKind {
    Pdf,
    Video,
    Audio,
}

pub fn of_ext(ext: &str) -> Option<MediaKind> {
    const VIDEO: &[&str] = &[
        "mp4", "m4v", "mov", "webm", "ogv", "mkv", "avi", "wmv", "mpg", "mpeg", "3gp",
    ];
    const AUDIO: &[&str] = &[
        "mp3", "m4a", "aac", "flac", "wav", "ogg", "oga", "opus", "weba",
    ];
    if ext.eq_ignore_ascii_case("pdf") {
        return Some(MediaKind::Pdf);
    }
    if VIDEO.iter().any(|v| ext.eq_ignore_ascii_case(v)) {
        return Some(MediaKind::Video);
    }
    if AUDIO.iter().any(|v| ext.eq_ignore_ascii_case(v)) {
        return Some(MediaKind::Audio);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_case_insensitive() {
        assert_eq!(of_ext("PDF"), Some(MediaKind::Pdf));
        assert_eq!(of_ext("mp4"), Some(MediaKind::Video));
        assert_eq!(of_ext("MP3"), Some(MediaKind::Audio));
        assert_eq!(of_ext("exe"), None);
    }
}
