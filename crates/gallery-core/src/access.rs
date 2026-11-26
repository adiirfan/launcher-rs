//! How much of the phone's photos and videos the gallery may see.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Everything.
    Full,
    /// Android 14+: only the pictures the owner picked.
    Limited,
    /// Nothing yet.
    None,
}

/// From the Android version and which permissions are granted:
/// `images`/`video` are READ_MEDIA_IMAGES/VIDEO (Android 13+),
/// `user_selected` is READ_MEDIA_VISUAL_USER_SELECTED (14+), `legacy_read`
/// is READ_EXTERNAL_STORAGE (up to 12).
pub fn access_from(sdk: u32, images: bool, video: bool, user_selected: bool, legacy_read: bool) -> Access {
    if sdk < 33 {
        return if legacy_read { Access::Full } else { Access::None };
    }
    if images && video {
        Access::Full
    } else if images || video || user_selected {
        Access::Limited
    } else {
        Access::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_states() {
        assert_eq!(access_from(33, true, true, false, false), Access::Full);
        assert_eq!(access_from(34, true, true, true, false), Access::Full);
        assert_eq!(access_from(34, false, false, true, false), Access::Limited);
        assert_eq!(access_from(34, true, false, false, false), Access::Limited);
        assert_eq!(access_from(34, false, false, false, false), Access::None);
        assert_eq!(access_from(28, false, false, false, true), Access::Full);
        assert_eq!(access_from(28, false, false, false, false), Access::None);
        assert_eq!(access_from(32, false, false, false, true), Access::Full);
    }
}
