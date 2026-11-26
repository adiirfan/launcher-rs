//! Where an edited or converted copy goes and what it is called. The
//! original is never touched; the copy sits next to it when Android lets us.

/// `IMG_1.jpg` + `_crop` + `jpg` → `IMG_1_crop.jpg`, or `IMG_1_crop_2.jpg`
/// and so on when `taken` says a name is in use.
pub fn copy_name(original: &str, suffix: &str, ext: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let base = match original.rfind('.') {
        Some(i) if i > 0 => &original[..i],
        _ => original,
    };
    let base = strip_ours(base);
    let first = format!("{base}{suffix}.{ext}");
    if !taken(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{base}{suffix}_{n}.{ext}"))
        .find(|name| !taken(name))
        .expect("some number is free")
}

/// `IMG_1.jpg` as it is, or `IMG_1_2.jpg` and so on when `taken` says
/// the name is in use (moving into a folder that has one already).
pub fn free_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (base, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..).map(|n| format!("{base}_{n}{ext}")).find(|n| !taken(n)).expect("some number is free")
}

const SUFFIXES: [&str; 5] = ["_crop", "_clip", "_gif", "_video", "_conv"];

/// `IMG_1_crop_2` → `IMG_1`; anything else unchanged.
fn strip_ours(base: &str) -> &str {
    let no_number = match base.rfind('_') {
        Some(i) if i + 1 < base.len() && base[i + 1..].bytes().all(|b| b.is_ascii_digit()) => &base[..i],
        _ => base,
    };
    for s in SUFFIXES {
        if let Some(b) = no_number.strip_suffix(s).filter(|b| !b.is_empty()) {
            return b;
        }
    }
    base
}

/// The folder (relative to the storage root, ending in `/`) a copy of a
/// file in `original_rel` goes to. From Android 11 apps may only add files
/// under the standard folders.
pub fn copy_folder(original_rel: &str, video: bool, sdk: u32) -> String {
    let rel = original_rel.trim_matches('/');
    let fallback = if video { "Movies/Edited/" } else { "Pictures/Edited/" };
    if rel.is_empty() {
        return fallback.into();
    }
    const ROOTS: [&str; 12] = ["DCIM", "Pictures", "Movies", "Download", "Documents", "Music", "Podcasts", "Alarms", "Ringtones", "Notifications", "Audiobooks", "Recordings"];
    let top = rel.split('/').next().unwrap_or_default();
    if sdk < 30 || ROOTS.iter().any(|r| r.eq_ignore_ascii_case(top)) {
        format!("{rel}/")
    } else {
        fallback.into()
    }
}

/// A copy this app made (name ends in one of our suffixes).
pub fn is_edited_name(name: &str) -> bool {
    let base = match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    };
    strip_ours(base) != base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_name_adds_suffix_and_number() {
        let none = |_: &str| false;
        assert_eq!(copy_name("IMG_1.jpg", "_crop", "jpg", &none), "IMG_1_crop.jpg");
        assert_eq!(copy_name("VID 2.MP4", "_clip", "mp4", &none), "VID 2_clip.mp4");
        assert_eq!(copy_name("noext", "_conv", "png", &none), "noext_conv.png");
        let used = |n: &str| n == "IMG_1_crop.jpg" || n == "IMG_1_crop_2.jpg";
        assert_eq!(copy_name("IMG_1.jpg", "_crop", "jpg", &used), "IMG_1_crop_3.jpg");
        // Editing a copy again doesn't pile up suffixes.
        let first = |n: &str| n == "IMG_1_crop.jpg";
        assert_eq!(copy_name("IMG_1_crop.jpg", "_crop", "jpg", &first), "IMG_1_crop_2.jpg");
        assert_eq!(copy_name("IMG_1_crop_2.jpg", "_conv", "png", &none), "IMG_1_conv.png");
    }

    #[test]
    fn free_name_keeps_name_unless_taken() {
        let none = |_: &str| false;
        assert_eq!(free_name("IMG_1.jpg", &none), "IMG_1.jpg");
        let used = |n: &str| n == "IMG_1.jpg" || n == "IMG_1_2.jpg";
        assert_eq!(free_name("IMG_1.jpg", &used), "IMG_1_3.jpg");
        let bare = |n: &str| n == "noext";
        assert_eq!(free_name("noext", &bare), "noext_2");
    }

    #[test]
    fn copy_folder_keeps_standard_and_redirects_others() {
        assert_eq!(copy_folder("DCIM/Camera/", false, 34), "DCIM/Camera/");
        assert_eq!(copy_folder("Pictures/Screenshots", false, 34), "Pictures/Screenshots/");
        assert_eq!(copy_folder("Download/", true, 34), "Download/");
        assert_eq!(copy_folder("Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Images/", false, 34), "Pictures/Edited/");
        assert_eq!(copy_folder("Android/media/com.whatsapp/WhatsApp/Media/WhatsApp Video/", true, 34), "Movies/Edited/");
        assert_eq!(copy_folder("", false, 34), "Pictures/Edited/");
        // Android 8-10 can write anywhere it can read.
        assert_eq!(copy_folder("WhatsApp/Media/WhatsApp Images/", false, 28), "WhatsApp/Media/WhatsApp Images/");
    }

    #[test]
    fn edited_names_are_recognised() {
        for n in ["IMG_1_crop.jpg", "VID_clip.mp4", "a_gif.gif", "b_video.mp4", "c_conv.webp", "IMG_1_crop_2.jpg", "x_crop_12.png"] {
            assert!(is_edited_name(n), "{n}");
        }
        for n in ["IMG_1.jpg", "crop.jpg", "my_cropped.jpg", "IMG_1_crop_.jpg", "photo_2.jpg"] {
            assert!(!is_edited_name(n), "{n}");
        }
    }
}
