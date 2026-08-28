//! Reject-condition metrics (C-19.3): over/underexposure from pixel
//! statistics, eyes-closed via semantic similarity (done in main.rs where
//! the engine lives). Blur detection is NOT implemented yet — the UI keeps
//! the checkbox, the filter simply ignores it.
//!
//! Exposure rules (user-specified):
//! - Overexposed: pixels with luma 252-255 cover > 20% of the area.
//! - Underexposed: luma < 20 covers > 20% AND the brightest pixel < 160.
//! Never fails: unreadable images count as neither (so they don't get
//! re-analyzed on every run).
//!
//! When these rules change, bump EXPOSURE_RULE_VERSION in main.rs — the
//! startup check then resets the cached metrics so the library re-analyzes.

use std::path::Path;

/// Prefer the cached thumbnail for statistics (far fewer pixels = much
/// faster); fall back to the original when the thumbnail is missing or the
/// file's mtime no longer matches (stale cache). Exposure is a global
/// property — thumbnail quality is plenty for the 20%/30% thresholds.
pub fn thumbnail_or_original(cache_dir: &Path, path: &Path) -> std::path::PathBuf {
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mt) = meta.modified() {
            if let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH) {
                let thumb = crate::utils::thumbnail_path(
                    cache_dir,
                    &path.to_string_lossy(),
                    d.as_secs() as i64,
                );
                if thumb.exists() {
                    return thumb;
                }
            }
        }
    }
    path.to_path_buf()
}

/// (overexposed, underexposed) from the image's luma histogram.
/// Downscaled to 256px first — exposure is a global property, and this keeps
/// large JPEGs fast (the check runs over the whole library once).
pub fn analyze_exposure(path: &Path) -> (bool, bool) {
    let img = match image::open(path) {
        Ok(i) => i,
        Err(_) => return (false, false),
    };
    let img = img.resize(256, 256, image::imageops::FilterType::Triangle);
    let luma = img.to_luma8();
    let total = (luma.width() * luma.height()) as f32;
    let mut over = 0u32;
    let mut under = 0u32;
    let mut max_l = 0u8;
    for p in luma.pixels() {
        let v = p[0];
        if v >= 252 {
            over += 1;
        }
        if v < 20 {
            under += 1;
        }
        if v > max_l {
            max_l = v;
        }
    }
    let over_ratio = over as f32 / total;
    let under_ratio = under as f32 / total;
    (over_ratio > 0.20, under_ratio > 0.20 && max_l < 160)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_is_overexposed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("white.jpg");
        image::RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]))
            .save(&p)
            .unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(over && !under);
    }

    #[test]
    fn black_is_underexposed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("black.jpg");
        image::RgbImage::from_pixel(64, 64, image::Rgb([0, 0, 0]))
            .save(&p)
            .unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(!over && under);
    }

    #[test]
    fn mid_gray_is_neither() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gray.jpg");
        image::RgbImage::from_pixel(64, 64, image::Rgb([128, 128, 128]))
            .save(&p)
            .unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(!over && !under);
    }

    #[test]
    fn dark_with_bright_spots_is_not_underexposed() {
        // ~91% black, ~9% white → under-ratio 0.91 > 0.3 BUT max luma 255
        // (>= 150), so NOT underexposed; the white strip is 9.4% < the
        // 10% overexposure threshold, so NOT overexposed either.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mixed.jpg");
        let mut img = image::RgbImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let px = if x >= 58 { [255, 255, 255] } else { [0, 0, 0] };
                img.put_pixel(x, y, image::Rgb(px));
            }
        }
        img.save(&p).unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(!over);
        assert!(!under);
    }

    #[test]
    fn thirty_percent_bright_area_is_overexposed() {
        // ~69% black, ~31% white → over (well above the 20% threshold);
        // under fails because the brightest pixel is 255 (>= 140).
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mixed2.jpg");
        let mut img = image::RgbImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let px = if x >= 44 { [255, 255, 255] } else { [0, 0, 0] };
                img.put_pixel(x, y, image::Rgb(px));
            }
        }
        img.save(&p).unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(over);
        assert!(!under);
    }

    #[test]
    fn dark_with_dim_highlights_is_underexposed() {
        // ~80% black + ~20% dim gray (luma 150 — between the old 140 and the
        // current 160 limit, so this test pins the <160 rule): under-ratio
        // 0.8 > 0.2 AND the brightest pixel is 150 < 160 → underexposed;
        // nothing >= 252 → not overexposed.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("dim.jpg");
        let mut img = image::RgbImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                let px = if x >= 51 { [150, 150, 150] } else { [0, 0, 0] };
                img.put_pixel(x, y, image::Rgb(px));
            }
        }
        img.save(&p).unwrap();
        let (over, under) = analyze_exposure(&p);
        assert!(!over);
        assert!(under);
    }
}
