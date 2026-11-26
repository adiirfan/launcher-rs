//! Converting pictures: the formats, the size choices, a summary of what's
//! chosen, and a rough guess at how big the copies will be.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
    Heic,
    Avif,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SizeChoice {
    Original,
    Fit2048,
    Half,
}

impl Format {
    pub const ALL: [Format; 5] = [Format::Jpeg, Format::Png, Format::Webp, Format::Heic, Format::Avif];

    pub fn ext(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Heic => "heic",
            Format::Avif => "avif",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Webp => "image/webp",
            Format::Heic => "image/heic",
            Format::Avif => "image/avif",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Webp => "WebP",
            Format::Heic => "HEIC",
            Format::Avif => "AVIF",
        }
    }

    /// The line under the name, as the mockup.
    pub fn about(self) -> &'static str {
        match self {
            Format::Jpeg => "Smallest for photos. See-through parts turn white.",
            Format::Png => "Exact copy that keeps see-through parts. Biggest file.",
            Format::Webp => "Small and keeps see-through parts. A few older apps can't open it.",
            Format::Heic => "Smaller than JPEG.",
            Format::Avif => "Smaller still, but few apps open it yet.",
        }
    }

    /// PNG is always exact; the others take a quality.
    pub fn has_quality(self) -> bool {
        self != Format::Png
    }

    /// Size against a JPEG at quality 85, roughly.
    fn ratio(self) -> f64 {
        match self {
            Format::Jpeg => 1.0,
            Format::Png => 3.8,
            Format::Webp => 0.8,
            Format::Heic => 0.66,
            Format::Avif => 0.55,
        }
    }

    /// The format a picture's type is, if it's one of these.
    pub fn of_mime(mime: &str) -> Option<Format> {
        match mime {
            "image/jpeg" | "image/jpg" => Some(Format::Jpeg),
            "image/png" => Some(Format::Png),
            "image/webp" => Some(Format::Webp),
            "image/heic" | "image/heif" => Some(Format::Heic),
            "image/avif" => Some(Format::Avif),
            _ => None,
        }
    }
}

impl SizeChoice {
    pub const ALL: [SizeChoice; 3] = [SizeChoice::Original, SizeChoice::Fit2048, SizeChoice::Half];

    pub fn label(self) -> &'static str {
        match self {
            SizeChoice::Original => "ORIGINAL",
            SizeChoice::Fit2048 => "FIT 2048 PX",
            SizeChoice::Half => "HALF",
        }
    }

    /// The new size for a `w` x `h` picture.
    pub fn apply(self, w: u32, h: u32) -> (u32, u32) {
        match self {
            SizeChoice::Original => (w, h),
            SizeChoice::Half => (w.div_ceil(2).max(1), h.div_ceil(2).max(1)),
            SizeChoice::Fit2048 => {
                let long = w.max(h);
                if long <= 2048 {
                    return (w, h);
                }
                let k = 2048.0 / long as f64;
                (((w as f64 * k).round() as u32).max(1), ((h as f64 * k).round() as u32).max(1))
            }
        }
    }

    /// Share of the pixels kept, for the estimate (Fit 2048 against a 12 MP photo).
    fn pixel_share(self) -> f64 {
        match self {
            SizeChoice::Original => 1.0,
            SizeChoice::Fit2048 => 0.5,
            SizeChoice::Half => 0.25,
        }
    }
}

/// "2 WebP, 1 PNG": how many of each kind, most first.
pub fn summary(mimes: &[&str]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for m in mimes {
        let name = Format::of_mime(m).map_or("other", Format::name);
        match counts.iter_mut().find(|c| c.0 == name) {
            Some(c) => c.1 += 1,
            None => counts.push((name, 1)),
        }
    }
    counts.sort_by_key(|c| (std::cmp::Reverse(c.1), c.0 == "other"));
    counts.iter().map(|(n, c)| format!("{c} {n}")).collect::<Vec<_>>().join(", ")
}

/// A rough guess at the copies' total size, from the originals' total.
pub fn estimate(total_bytes: u64, format: Format, size: SizeChoice, quality: u8) -> u64 {
    let q = if format.has_quality() { (quality as f64 / 85.0).powi(2) } else { 1.0 };
    // Originals are mostly phone JPEGs at about quality 90.
    (total_bytes as f64 * 0.85 * format.ratio() * size.pixel_share() * q).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_2048_keeps_ratio() {
        assert_eq!(SizeChoice::Fit2048.apply(4000, 3000), (2048, 1536));
        assert_eq!(SizeChoice::Fit2048.apply(3000, 4000), (1536, 2048));
        // Already small enough: unchanged.
        assert_eq!(SizeChoice::Fit2048.apply(1200, 800), (1200, 800));
        assert_eq!(SizeChoice::Original.apply(4000, 3000), (4000, 3000));
    }

    #[test]
    fn half_rounds() {
        assert_eq!(SizeChoice::Half.apply(4001, 3001), (2001, 1501));
        assert_eq!(SizeChoice::Half.apply(1, 1), (1, 1));
    }

    #[test]
    fn summary_counts_kinds() {
        assert_eq!(summary(&["image/webp", "image/png", "image/webp"]), "2 WebP, 1 PNG");
        assert_eq!(summary(&["image/jpeg"]), "1 JPEG");
        assert_eq!(summary(&["image/heic", "image/x-weird"]), "1 HEIC, 1 other");
    }

    #[test]
    fn png_has_no_quality() {
        assert!(!Format::Png.has_quality());
        assert!(Format::Jpeg.has_quality() && Format::Webp.has_quality() && Format::Heic.has_quality());
        assert_eq!((Format::Heic.ext(), Format::Heic.mime()), ("heic", "image/heic"));
        assert_eq!((Format::Jpeg.ext(), Format::Webp.mime()), ("jpg", "image/webp"));
    }

    #[test]
    fn estimate_shrinks_with_size_and_quality() {
        let full = estimate(10_000_000, Format::Jpeg, SizeChoice::Original, 85);
        assert!(estimate(10_000_000, Format::Jpeg, SizeChoice::Half, 85) < full / 3);
        assert!(estimate(10_000_000, Format::Jpeg, SizeChoice::Original, 60) < full);
        assert!(estimate(10_000_000, Format::Png, SizeChoice::Original, 60) > full);
    }
}
