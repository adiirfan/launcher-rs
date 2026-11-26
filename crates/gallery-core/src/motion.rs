//! Motion photos: stills with a short video inside. Samsung writes
//! "MotionPhoto_Data" in the trailer at the end of the file; Google writes
//! MotionPhoto="1" (or the older MicroVideo) in the details at the start.
//! The answer for each photo is worked out once and kept.

use std::collections::HashMap;

/// How much of each end of the file is read.
pub const SCAN: usize = 64 * 1024;

/// Whether `ends` (a file's first and last SCAN bytes, one after the other)
/// show a motion photo. Only JPEG and HEIC files count.
pub fn is_motion(ends: &[u8]) -> bool {
    let jpeg = ends.starts_with(&[0xFF, 0xD8]);
    let heic = ends.get(4..8) == Some(b"ftyp");
    if !jpeg && !heic {
        return false;
    }
    const MARKS: [&[u8]; 5] = [b"MotionPhoto_Data", br#"MotionPhoto="1""#, br#"MicroVideo="1""#, b"MotionPhoto>1<", b"MicroVideo>1<"];
    MARKS.iter().any(|m| find(ends, m))
}

fn find(hay: &[u8], needle: &[u8]) -> bool {
    let first = needle[0];
    hay.iter().enumerate().any(|(i, &b)| b == first && hay[i..].starts_with(needle))
}

/// The kept answers, from motion.txt: one "id 1" or "id 0" a line.
pub fn parse(text: &str) -> HashMap<i64, bool> {
    text.lines()
        .filter_map(|l| {
            let (id, v) = l.trim().split_once(' ')?;
            let v = match v {
                "1" => true,
                "0" => false,
                _ => return None,
            };
            Some((id.parse().ok()?, v))
        })
        .collect()
}

pub fn to_text(known: &HashMap<i64, bool>) -> String {
    known.iter().map(|(id, &m)| format!("{id} {}\n", m as u8)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg(head: &[u8], tail: &[u8]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE1];
        v.extend_from_slice(head);
        v.extend(std::iter::repeat_n(7u8, 1000));
        v.extend_from_slice(tail);
        v
    }

    #[test]
    fn motion_marker_found_in_tail() {
        assert!(is_motion(&jpeg(b"Exif", b"....MotionPhoto_Data\x00SEFT")));
    }

    #[test]
    fn google_marker_found_in_head() {
        assert!(is_motion(&jpeg(br#"<x:xmpmeta GCamera:MotionPhoto="1" "#, b"")));
        assert!(is_motion(&jpeg(br#"GCamera:MicroVideo="1""#, b"")));
    }

    #[test]
    fn plain_and_switched_off_photos_are_not_motion() {
        assert!(!is_motion(&jpeg(b"Exif", b"SEFT")));
        assert!(!is_motion(&jpeg(br#"GCamera:MotionPhoto="0""#, b"")));
    }

    #[test]
    fn heic_counts_and_png_does_not() {
        let mut heic = vec![0, 0, 0, 24];
        heic.extend_from_slice(b"ftypheic....MotionPhoto_Data");
        assert!(is_motion(&heic));
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(b"MotionPhoto_Data");
        assert!(!is_motion(&png));
    }

    #[test]
    fn answers_survive_a_round_trip() {
        let known = HashMap::from([(5, true), (9, false), (-3, true)]);
        assert_eq!(parse(&to_text(&known)), known);
        assert_eq!(parse("junk\n7 1\n8 x\n"), HashMap::from([(7, true)]));
    }
}
