//! EXIF lens / focal-length extraction (C-15).
//!
//! kamadak-exif parses EXIF from JPEG (and TIFF/PNG/WebP/HEIF containers
//! that carry it). Anything else — or files without EXIF — yields
//! `(None, None)`; the caller marks the file as checked either way, so the
//! startup backfill never re-reads files that simply have no EXIF.

/// Read the lens model (LensModel, 0xA434) and focal length (FocalLength,
/// 0x920A, mm) from a file's EXIF. Never fails — any parse problem just
/// means "no EXIF data".
pub fn read_lens_focal(path: &std::path::Path) -> (Option<String>, Option<f64>) {
    use exif::{In, Reader, Tag, Value};

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None),
    };
    let exif = match Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
        Ok(e) => e,
        Err(_) => return (None, None),
    };

    // kamadak's display_value() wraps ASCII strings in quotes, e.g.
    // `"EF50mm f/1.8 STM"` — strip them for storage/display. The "----"
    // placeholder (cameras with no lens mounted) is treated as absent.
    let lens = exif
        .get_field(Tag::LensModel, In::PRIMARY)
        .map(|f| f.display_value().to_string())
        .map(|s| {
            let s = s.trim();
            if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
                s[1..s.len() - 1].to_string()
            } else {
                s.to_string()
            }
        })
        .filter(|s| !s.is_empty() && s != "----");

    let focal = exif
        .get_field(Tag::FocalLength, In::PRIMARY)
        .and_then(|f| match f.value {
            Value::Rational(ref v) if !v.is_empty() => {
                let r = v[0].to_f64();
                (r.is_finite() && r > 0.0).then_some(r)
            }
            Value::SRational(ref v) if !v.is_empty() => {
                let r = v[0].to_f64();
                (r.is_finite() && r > 0.0).then_some(r)
            }
            _ => None,
        });

    (lens, focal)
}

#[cfg(test)]
mod tests {
    /// A tiny hand-built JPEG with an APP1 EXIF segment. FocalLength and
    /// LensModel live in the Exif sub-IFD (Context::Exif) per the EXIF
    /// standard, reached via the ExifIFDPointer (0x8769) in IFD0.
    fn make_exif_jpeg() -> Vec<u8> {
        // TIFF header: little-endian "II", magic 42, IFD0 at offset 8.
        let mut tiff: Vec<u8> = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        // IFD0: 1 entry — ExifIFDPointer (0x8769, LONG=4, count 1, value 26
        // inline since 4 bytes fit in the value field).
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes());
        // Next IFD offset = 0.
        tiff.extend_from_slice(&0u32.to_le_bytes());
        // Exif IFD at offset 26: 2 entries (FocalLength RATIONAL,
        // LensModel ASCII), values at 64 / 72.
        assert_eq!(tiff.len(), 26);
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&0x920Au16.to_le_bytes());
        tiff.extend_from_slice(&5u16.to_le_bytes()); // RATIONAL
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&64u32.to_le_bytes()); // 8-byte aligned
        tiff.extend_from_slice(&0xA434u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
        tiff.extend_from_slice(&11u32.to_le_bytes()); // "50mm f/1.8" + NUL
        tiff.extend_from_slice(&72u32.to_le_bytes());
        // Next IFD offset = 0.
        tiff.extend_from_slice(&0u32.to_le_bytes());
        // Pad to 64, then the rational 50/1.
        assert_eq!(tiff.len(), 56);
        tiff.extend_from_slice(&[0u8; 8]);
        assert_eq!(tiff.len(), 64);
        tiff.extend_from_slice(&50u32.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        // ASCII value at offset 72: "50mm f/1.8\0".
        assert_eq!(tiff.len(), 72);
        tiff.extend_from_slice(b"50mm f/1.8\0");

        // JPEG: SOI + APP1 (segment length counts itself + "Exif\0\0" +
        // tiff) + minimal EOI.
        let mut jpeg: Vec<u8> = Vec::new();
        jpeg.extend_from_slice(&[0xFF, 0xD8]);
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        let seg_len = (tiff.len() + 6 + 2) as u16; // len field + Exif\0\0 + tiff
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    #[test]
    fn reads_lens_and_focal_from_exif() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.jpg");
        std::fs::write(&path, make_exif_jpeg()).unwrap();
        // Debug: see what the parser actually finds.
        if let Ok(file) = std::fs::File::open(&path) {
            match exif::Reader::new().read_from_container(&mut std::io::BufReader::new(file)) {
                Ok(e) => {
                    for f in e.fields() {
                        eprintln!("DBG field {:?} = {}", f.tag, f.display_value());
                    }
                }
                Err(err) => eprintln!("DBG parse error: {err:?}"),
            }
        }
        let (lens, focal) = super::read_lens_focal(&path);
        assert_eq!(lens.as_deref(), Some("50mm f/1.8"));
        assert_eq!(focal, Some(50.0));
    }

    #[test]
    fn no_exif_yields_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.jpg");
        std::fs::write(&path, b"\xff\xd8\xff\xe0\x00\x10JFIF\0\x01\x02\xff\xd9").unwrap();
        let (lens, focal) = super::read_lens_focal(&path);
        assert_eq!(lens, None);
        assert_eq!(focal, None);
    }
}
