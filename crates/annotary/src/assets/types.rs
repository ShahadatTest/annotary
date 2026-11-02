//! The supported Asset types — one constant, keyed by file extension (spec: "the type list … is
//! a single constant; extend without re-deciding"). Anything else referenced locally is warned
//! about and left as written.

/// What an Asset is, for the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Image,
    Video,
    Audio,
    Pdf,
}

/// One supported extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupportedType {
    /// Lower-case extension without the dot.
    pub ext: &'static str,
    /// The `type` recorded in the manifest and served by `/a/{hash}`.
    pub content_type: &'static str,
    pub kind: Kind,
    /// Scanned recursively for its own references (Markdown only).
    pub scan: bool,
    /// Needs a web-safe derivative (browsers do not display it natively everywhere).
    pub derivative: bool,
}

const fn t(ext: &'static str, content_type: &'static str, kind: Kind) -> SupportedType {
    SupportedType { ext, content_type, kind, scan: false, derivative: false }
}

/// Every supported extension. Text `.md .markdown .txt`; image `.png .jpg .jpeg .gif .webp .svg
/// .heic .heif .avif .bmp .tif .tiff`; video `.mp4 .mov .m4v .webm`; audio `.mp3 .m4a .aac .wav
/// .ogg .flac`; `.pdf`.
pub const SUPPORTED_TYPES: &[SupportedType] = &[
    SupportedType { scan: true, ..t("md", "text/markdown", Kind::Text) },
    SupportedType { scan: true, ..t("markdown", "text/markdown", Kind::Text) },
    t("txt", "text/plain", Kind::Text),
    t("png", "image/png", Kind::Image),
    t("jpg", "image/jpeg", Kind::Image),
    t("jpeg", "image/jpeg", Kind::Image),
    t("gif", "image/gif", Kind::Image),
    t("webp", "image/webp", Kind::Image),
    t("svg", "image/svg+xml", Kind::Image),
    SupportedType { derivative: true, ..t("heic", "image/heic", Kind::Image) },
    SupportedType { derivative: true, ..t("heif", "image/heif", Kind::Image) },
    SupportedType { derivative: true, ..t("avif", "image/avif", Kind::Image) },
    SupportedType { derivative: true, ..t("bmp", "image/bmp", Kind::Image) },
    SupportedType { derivative: true, ..t("tif", "image/tiff", Kind::Image) },
    SupportedType { derivative: true, ..t("tiff", "image/tiff", Kind::Image) },
    t("mp4", "video/mp4", Kind::Video),
    t("mov", "video/quicktime", Kind::Video),
    t("m4v", "video/x-m4v", Kind::Video),
    t("webm", "video/webm", Kind::Video),
    t("mp3", "audio/mpeg", Kind::Audio),
    t("m4a", "audio/mp4", Kind::Audio),
    t("aac", "audio/aac", Kind::Audio),
    t("wav", "audio/wav", Kind::Audio),
    t("ogg", "audio/ogg", Kind::Audio),
    t("flac", "audio/flac", Kind::Audio),
    t("pdf", "application/pdf", Kind::Pdf),
];

/// The extension of a path's last component, lower-cased (`img/A.PNG` → `png`); `None` when
/// there is none (`README`, `.hidden`).
pub fn extension_of(path: &str) -> Option<String> {
    let file = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = file.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// The supported type for a lower-case extension.
pub fn by_extension(ext: &str) -> Option<&'static SupportedType> {
    SUPPORTED_TYPES.iter().find(|t| t.ext == ext)
}

/// The supported type of a path, by its extension.
pub fn type_of(path: &str) -> Option<&'static SupportedType> {
    by_extension(&extension_of(path)?)
}

/// The supported type whose content type is `ct` (first match).
pub fn by_content_type(ct: &str) -> Option<&'static SupportedType> {
    let ct = ct.split(';').next().unwrap_or(ct).trim();
    SUPPORTED_TYPES.iter().find(|t| t.content_type == ct)
}

/// Sniff the type of a web-safe derivative from its bytes (PNG or JPEG — what
/// [`super::media::derivative`] produces).
pub fn sniff_derivative_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S03-02
    #[test]
    fn the_type_table_is_the_decided_list() {
        let exts: Vec<&str> = SUPPORTED_TYPES.iter().map(|t| t.ext).collect();
        assert_eq!(
            exts,
            vec![
                "md", "markdown", "txt", "png", "jpg", "jpeg", "gif", "webp", "svg", "heic", "heif", "avif", "bmp",
                "tif", "tiff", "mp4", "mov", "m4v", "webm", "mp3", "m4a", "aac", "wav", "ogg", "flac", "pdf"
            ]
        );
        assert_eq!(extension_of("img/Arch.PNG").as_deref(), Some("png"));
        assert_eq!(extension_of("README"), None);
        assert_eq!(extension_of(".hidden"), None);
        assert_eq!(extension_of("a.b/c"), None);
        assert_eq!(type_of("notes.md").unwrap().content_type, "text/markdown");
        assert!(type_of("notes.md").unwrap().scan && !type_of("notes.txt").unwrap().scan);
        assert!(type_of("x.heic").unwrap().derivative && type_of("x.avif").unwrap().derivative);
        assert!(type_of("x.tiff").unwrap().derivative && type_of("x.bmp").unwrap().derivative);
        assert!(!type_of("x.png").unwrap().derivative);
        assert_eq!(type_of("x.docx"), None);
        assert_eq!(type_of("x.mov").unwrap().kind, Kind::Video);
        assert_eq!(type_of("x.pdf").unwrap().kind, Kind::Pdf);
        assert_eq!(by_content_type("image/jpeg; charset=x").unwrap().ext, "jpg");
        assert_eq!(sniff_derivative_type(b"\x89PNG\r\n\x1a\nxxxx"), "image/png");
        assert_eq!(sniff_derivative_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_derivative_type(b"nope"), "application/octet-stream");
    }
}
