//! A picture or video handed over by another app. Links into Android's
//! media library open inside our own list; anything else opens on its own.

use crate::media::Kind;

/// The library item a link points at: its id, and whether it is a video
/// when the link says (`None` for the general "file" list).
pub fn media_id(uri: &str) -> Option<(i64, Option<bool>)> {
    let uri = uri.split(['?', '#']).next()?;
    let number = |s: &str| (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten();
    if let Some(rest) = uri.strip_prefix("content://media/") {
        // <volume>/images/media/<id>, <volume>/video/media/<id>, <volume>/file/<id>
        let parts: Vec<&str> = rest.split('/').collect();
        return match parts.as_slice() {
            [_, "images", "media", id] => Some((number(id)?, Some(false))),
            [_, "video", "media", id] => Some((number(id)?, Some(true))),
            [_, "file", id] => Some((number(id)?, None)),
            _ => None,
        };
    }
    if let Some(doc) = uri.strip_prefix("content://com.android.providers.media.documents/document/") {
        let doc = doc.replace("%3A", ":").replace("%3a", ":");
        let (kind, id) = doc.split_once(':')?;
        let video = match kind {
            "image" => false,
            "video" => true,
            _ => return None,
        };
        return Some((number(id)?, Some(video)));
    }
    None
}

/// What kind of thing to show, from the type the other app gave.
pub fn kind_of(mime: &str) -> Kind {
    if mime.starts_with("video/") {
        Kind::Video
    } else if mime == "image/gif" {
        Kind::Gif
    } else {
        Kind::Image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_store_image_link() {
        assert_eq!(media_id("content://media/external/images/media/123"), Some((123, Some(false))));
    }

    #[test]
    fn media_store_video_link_with_volume_and_query() {
        assert_eq!(media_id("content://media/external_primary/video/media/77?requireOriginal=1"), Some((77, Some(true))));
    }

    #[test]
    fn media_store_file_link() {
        assert_eq!(media_id("content://media/external/file/9"), Some((9, None)));
    }

    #[test]
    fn documents_provider_link() {
        assert_eq!(media_id("content://com.android.providers.media.documents/document/image%3A42"), Some((42, Some(false))));
        assert_eq!(media_id("content://com.android.providers.media.documents/document/video%3A5"), Some((5, Some(true))));
    }

    #[test]
    fn other_links_are_not_library_items() {
        assert_eq!(media_id("content://com.whatsapp.provider.media/item/123"), None);
        assert_eq!(media_id("file:///sdcard/DCIM/a.jpg"), None);
        assert_eq!(media_id("content://media/external/images/media/"), None);
        assert_eq!(media_id("content://media/external/images/media/12abc"), None);
        assert_eq!(media_id("content://com.android.providers.media.documents/document/audio%3A3"), None);
    }

    #[test]
    fn kind_from_type() {
        assert_eq!(kind_of("video/mp4"), Kind::Video);
        assert_eq!(kind_of("image/gif"), Kind::Gif);
        assert_eq!(kind_of("image/jpeg"), Kind::Image);
        assert_eq!(kind_of(""), Kind::Image);
    }
}
