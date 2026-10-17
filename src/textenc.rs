//! Text encodings and line endings: detection on open, conversion to the
//! UTF-8 buffer Scintilla edits, and back again on save.

use encoding_rs::Encoding;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enc {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Legacy(&'static Encoding),
}

/// Encodings offered in the "Reopen with / Save with encoding" pickers.
pub const LEGACY_LABELS: &[&str] = &[
    "windows-1252",
    "windows-1250",
    "windows-1251",
    "windows-1253",
    "windows-1254",
    "windows-1255",
    "windows-1256",
    "windows-1257",
    "windows-1258",
    "windows-874",
    "iso-8859-2",
    "iso-8859-5",
    "iso-8859-7",
    "iso-8859-15",
    "koi8-r",
    "koi8-u",
    "ibm866",
    "macintosh",
    "shift_jis",
    "euc-jp",
    "iso-2022-jp",
    "gbk",
    "gb18030",
    "big5",
    "euc-kr",
];

impl Enc {
    pub fn label(&self) -> String {
        match self {
            Enc::Utf8 => "UTF-8".into(),
            Enc::Utf8Bom => "UTF-8 with BOM".into(),
            Enc::Utf16Le => "UTF-16 LE".into(),
            Enc::Utf16Be => "UTF-16 BE".into(),
            Enc::Legacy(e) => e.name().to_string(),
        }
    }

    /// Stable identifier for settings and the session file.
    pub fn id(&self) -> String {
        match self {
            Enc::Utf8 => "utf-8".into(),
            Enc::Utf8Bom => "utf-8-bom".into(),
            Enc::Utf16Le => "utf-16le".into(),
            Enc::Utf16Be => "utf-16be".into(),
            Enc::Legacy(e) => e.name().to_ascii_lowercase(),
        }
    }

    pub fn from_id(id: &str) -> Option<Enc> {
        match id.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => Some(Enc::Utf8),
            "utf-8-bom" | "utf8bom" | "utf-8 with bom" => Some(Enc::Utf8Bom),
            "utf-16le" | "utf-16" => Some(Enc::Utf16Le),
            "utf-16be" => Some(Enc::Utf16Be),
            "ansi" => Some(Enc::Legacy(ansi())),
            other => Encoding::for_label(other.as_bytes()).map(|e| {
                if e == encoding_rs::UTF_8 {
                    Enc::Utf8
                } else {
                    Enc::Legacy(e)
                }
            }),
        }
    }

    pub fn all() -> Vec<Enc> {
        let mut v = vec![Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Le, Enc::Utf16Be];
        v.extend(LEGACY_LABELS.iter().filter_map(|l| Encoding::for_label(l.as_bytes())).map(Enc::Legacy));
        v
    }
}

/// The system ANSI code page as an encoding_rs encoding.
pub fn ansi() -> &'static Encoding {
    let cp = unsafe { windows_sys::Win32::Globalization::GetACP() };
    let label = match cp {
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        1250..=1258 => return Encoding::for_label(format!("windows-{cp}").as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252),
        20866 => "koi8-r",
        21866 => "koi8-u",
        _ => "windows-1252",
    };
    Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
}

/// Best guess at a file's encoding from its bytes.
pub fn detect(bytes: &[u8]) -> Enc {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return Enc::Utf8Bom;
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return Enc::Utf16Le;
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return Enc::Utf16Be;
    }
    // BOM-less UTF-16: mostly-ASCII text has a zero in every other byte.
    let sample = &bytes[..bytes.len().min(4096) & !1];
    if sample.len() >= 4 {
        let pairs = sample.len() / 2;
        let odd_zero = sample.chunks(2).filter(|c| c[1] == 0 && c[0] != 0).count();
        let even_zero = sample.chunks(2).filter(|c| c[0] == 0 && c[1] != 0).count();
        if odd_zero * 10 > pairs * 4 && even_zero * 10 < pairs {
            return Enc::Utf16Le;
        }
        if even_zero * 10 > pairs * 4 && odd_zero * 10 < pairs {
            return Enc::Utf16Be;
        }
    }
    if std::str::from_utf8(bytes).is_ok() || utf8_with_truncated_tail(bytes) {
        return Enc::Utf8;
    }
    Enc::Legacy(ansi())
}

/// Valid UTF-8 apart from a multi-byte sequence cut off at the end.
fn utf8_with_truncated_tail(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(e) => e.error_len().is_none() && bytes.len() - e.valid_up_to() < 4,
    }
}

/// File bytes to the UTF-8 that Scintilla edits.
pub fn decode(bytes: &[u8], enc: Enc) -> Vec<u8> {
    match enc {
        Enc::Utf8 | Enc::Utf8Bom => {
            let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
            match std::str::from_utf8(body) {
                Ok(_) => body.to_vec(),
                Err(_) => String::from_utf8_lossy(body).into_owned().into_bytes(),
            }
        }
        Enc::Utf16Le | Enc::Utf16Be => {
            let le = enc == Enc::Utf16Le;
            let body = if (le && bytes.starts_with(&[0xFF, 0xFE])) || (!le && bytes.starts_with(&[0xFE, 0xFF])) { &bytes[2..] } else { bytes };
            let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
            String::from_utf16_lossy(&units).into_bytes()
        }
        Enc::Legacy(e) => e.decode_without_bom_handling(bytes).0.into_owned().into_bytes(),
    }
}

/// UTF-8 text to file bytes. The flag is true when some characters could not
/// be represented and were replaced.
pub fn encode(text: &[u8], enc: Enc) -> (Vec<u8>, bool) {
    let s = String::from_utf8_lossy(text);
    match enc {
        Enc::Utf8 => (text.to_vec(), false),
        Enc::Utf8Bom => {
            let mut v = Vec::with_capacity(text.len() + 3);
            v.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            v.extend_from_slice(text);
            (v, false)
        }
        Enc::Utf16Le | Enc::Utf16Be => {
            let le = enc == Enc::Utf16Le;
            let mut v = Vec::with_capacity(text.len() * 2 + 2);
            v.extend_from_slice(if le { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
            for u in s.encode_utf16() {
                v.extend_from_slice(&if le { u.to_le_bytes() } else { u.to_be_bytes() });
            }
            (v, false)
        }
        Enc::Legacy(e) => {
            let (out, _, lossy) = e.encode(&s);
            (out.into_owned(), lossy)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Crlf,
    Lf,
    Cr,
}

impl Eol {
    pub fn label(self) -> &'static str {
        match self {
            Eol::Crlf => "CRLF",
            Eol::Lf => "LF",
            Eol::Cr => "CR",
        }
    }
    pub fn from_label(s: &str) -> Option<Eol> {
        match s.to_ascii_lowercase().as_str() {
            "crlf" | "windows" => Some(Eol::Crlf),
            "lf" | "unix" => Some(Eol::Lf),
            "cr" | "mac" => Some(Eol::Cr),
            _ => None,
        }
    }
    pub fn sci_mode(self) -> i64 {
        match self {
            Eol::Crlf => crate::sci_consts::SC_EOL_CRLF,
            Eol::Lf => crate::sci_consts::SC_EOL_LF,
            Eol::Cr => crate::sci_consts::SC_EOL_CR,
        }
    }
}

/// The most common line ending in the first 64 KiB, or `default` if none.
pub fn detect_eol(text: &[u8], default: Eol) -> Eol {
    let (mut crlf, mut lf, mut cr) = (0, 0, 0);
    let t = &text[..text.len().min(65536)];
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            b'\r' if t.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            b'\n' => lf += 1,
            _ => {}
        }
        i += 1;
    }
    if crlf == 0 && lf == 0 && cr == 0 {
        default
    } else if crlf >= lf && crlf >= cr {
        Eol::Crlf
    } else if lf >= cr {
        Eol::Lf
    } else {
        Eol::Cr
    }
}

/// True when the content looks binary: NULs outside UTF-16 text.
pub fn looks_binary(bytes: &[u8], enc: Enc) -> bool {
    if matches!(enc, Enc::Utf16Le | Enc::Utf16Be) {
        return false;
    }
    bytes[..bytes.len().min(8192)].contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_boms_and_utf16() {
        assert_eq!(detect(b"\xEF\xBB\xBFhi"), Enc::Utf8Bom);
        assert_eq!(detect(b"\xFF\xFEh\0i\0"), Enc::Utf16Le);
        assert_eq!(detect(b"\xFE\xFF\0h\0i"), Enc::Utf16Be);
        assert_eq!(detect("héllo wörld".as_bytes()), Enc::Utf8);
        assert_eq!(detect(b"h\0e\0l\0l\0o\0 \0w\0o\0r\0l\0d\0"), Enc::Utf16Le);
        assert!(matches!(detect(b"caf\xE9 cr\xE8me"), Enc::Legacy(_)));
    }

    #[test]
    fn round_trips_every_unicode_encoding() {
        let text = "Grüße, 世界 — ok\r\n".as_bytes();
        for enc in [Enc::Utf8, Enc::Utf8Bom, Enc::Utf16Le, Enc::Utf16Be] {
            let (bytes, lossy) = encode(text, enc);
            assert!(!lossy);
            assert_eq!(detect(&bytes), enc, "{enc:?}");
            assert_eq!(decode(&bytes, enc), text, "{enc:?}");
        }
    }

    #[test]
    fn legacy_reports_unmappable_characters() {
        let w1252 = Enc::Legacy(encoding_rs::WINDOWS_1252);
        let (bytes, lossy) = encode("café".as_bytes(), w1252);
        assert_eq!(bytes, b"caf\xE9");
        assert!(!lossy);
        assert_eq!(decode(&bytes, w1252), "café".as_bytes());
        assert!(encode("世界".as_bytes(), w1252).1);
    }

    #[test]
    fn eol_detection_uses_majority() {
        assert_eq!(detect_eol(b"a\r\nb\r\nc\n", Eol::Lf), Eol::Crlf);
        assert_eq!(detect_eol(b"a\nb\nc\r\n", Eol::Crlf), Eol::Lf);
        assert_eq!(detect_eol(b"abc", Eol::Crlf), Eol::Crlf);
        assert_eq!(detect_eol(b"a\rb\r", Eol::Crlf), Eol::Cr);
    }

    #[test]
    fn ids_round_trip() {
        for e in Enc::all() {
            assert_eq!(Enc::from_id(&e.id()), Some(e), "{}", e.id());
        }
    }
}
