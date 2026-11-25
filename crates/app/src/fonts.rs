//! The phone's own fonts, memory-mapped: only the pages actually read cost memory.

use launcher_draw::font::FontSet;

const DIR: &str = "/system/fonts";
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Light = 0,
    Regular = 1,
    Medium = 2,
    Bold = 3,
}

const FALLBACK: &[(&str, u32)] = &[("NotoSansCJK-Regular.ttc", 0), ("DroidSansFallback.ttf", 0)];

/// The phone's fonts for one weight, with lighter or heavier Roboto and then
/// the wide-coverage fonts behind it for anything missing.
pub fn load_weight(weight: Weight) -> FontSet {
    let first: &[&str] = match weight {
        Weight::Light => &["Roboto-Light.ttf", "Roboto-Regular.ttf"],
        Weight::Regular => &["Roboto-Regular.ttf"],
        Weight::Medium => &["Roboto-Medium.ttf", "Roboto-Regular.ttf"],
        Weight::Bold => &["Roboto-Bold.ttf", "Roboto-Medium.ttf", "Roboto-Regular.ttf"],
    };
    let files: Vec<(&str, u32)> = first.iter().map(|&n| (n, 0)).chain(FALLBACK.iter().copied()).collect();
    let wght = match weight {
        Weight::Light => 300.0,
        Weight::Regular => 400.0,
        Weight::Medium => 500.0,
        Weight::Bold => 700.0,
    };
    load(&files, &format!("{weight:?}"), wght)
}

/// `wght` picks the weight inside fonts that hold them all in one file
/// (Roboto from Android 12 on); single-weight files ignore it.
fn load(files: &[(&str, u32)], what: &str, wght: f32) -> FontSet {
    let mut fonts = Vec::new();
    let mut names = Vec::new();
    for &(name, index) in files {
        let Some(bytes) = map(name) else { continue };
        match ab_glyph::FontRef::try_from_slice_and_index(bytes, index) {
            Ok(mut f) => {
                use ab_glyph::VariableFont;
                f.set_variation(b"wght", wght);
                fonts.push(f);
                names.push(name);
            }
            Err(e) => log::warn!("fonts: {name} unreadable: {e}"),
        }
    }
    if names.is_empty() {
        log::warn!("fonts ({what}): none found");
    } else {
        log::info!("fonts ({what}): loaded {names:?}");
    }
    FontSet::new(fonts)
}

/// Map a font file for the life of the app (fonts are never unloaded).
fn map(name: &str) -> Option<&'static [u8]> {
    static MAPS: std::sync::Mutex<Vec<(&'static str, &'static [u8])>> = std::sync::Mutex::new(Vec::new());
    let mut maps = MAPS.lock().unwrap();
    if let Some(&(_, b)) = maps.iter().find(|(n, _)| *n == name) {
        return Some(b);
    }
    let file = std::fs::File::open(format!("{DIR}/{name}")).ok()?;
    // Safety: system font files are read-only and never change while we run.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.ok()?;
    let bytes: &'static [u8] = &Box::leak(Box::new(mmap))[..];
    let name: &'static str = Box::leak(name.to_string().into_boxed_str());
    maps.push((name, bytes));
    Some(bytes)
}
