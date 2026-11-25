//! The phone's own fonts, memory-mapped: only the pages actually read cost memory.

use launcher_draw::font::FontSet;

const DIR: &str = "/system/fonts";
const REGULAR: &[(&str, u32)] = &[
    ("Roboto-Regular.ttf", 0),
    ("NotoSansCJK-Regular.ttc", 0),
    ("DroidSansFallback.ttf", 0),
];
const BOLD: &[(&str, u32)] = &[
    ("Roboto-Medium.ttf", 0),
    ("Roboto-Regular.ttf", 0),
    ("NotoSansCJK-Regular.ttc", 0),
    ("DroidSansFallback.ttf", 0),
];

pub fn load_system_fonts() -> FontSet {
    load(REGULAR, "regular")
}

pub fn load_system_fonts_bold() -> FontSet {
    load(BOLD, "bold")
}

fn load(files: &[(&str, u32)], what: &str) -> FontSet {
    let mut fonts = Vec::new();
    let mut names = Vec::new();
    for &(name, index) in files {
        let Some(bytes) = map(name) else { continue };
        match ab_glyph::FontRef::try_from_slice_and_index(bytes, index) {
            Ok(f) => {
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
