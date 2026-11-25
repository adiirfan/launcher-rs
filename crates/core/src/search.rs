//! Matching what's typed against app names and Android's settings pages.

fn is_break(c: char) -> bool {
    matches!(c, ' ' | '-' | '_' | '.')
}

/// 0: the text starts with `q`; 1: a later word does; 2: `q` is inside it.
fn tier(text: &str, q: &str) -> Option<u8> {
    let text = text.to_lowercase();
    if text.starts_with(q) {
        return Some(0);
    }
    if text.split(is_break).skip(1).any(|w| w.starts_with(q)) {
        return Some(1);
    }
    text.contains(q).then_some(2)
}

fn normalise(query: &str) -> String {
    query.trim().to_lowercase()
}

/// Indexes of `labels` matching `query`, best first; ties keep their order.
pub fn rank(labels: &[&str], query: &str) -> Vec<usize> {
    let q = normalise(query);
    if q.is_empty() {
        return (0..labels.len()).collect();
    }
    let mut hits: Vec<(u8, usize)> = labels.iter().enumerate().filter_map(|(i, l)| tier(l, &q).map(|t| (t, i))).collect();
    hits.sort_by_key(|&(t, i)| (t, i));
    hits.into_iter().map(|(_, i)| i).collect()
}

pub struct SettingsPage {
    pub label: &'static str,
    pub keywords: &'static [&'static str],
    pub action: &'static str,
}

macro_rules! page {
    ($label:expr, [$($k:expr),*], $action:expr) => {
        SettingsPage { label: $label, keywords: &[$($k),*], action: $action }
    };
}

/// Android's own settings pages, opened by their public names.
pub static SETTINGS_PAGES: &[SettingsPage] = &[
    page!("Wi-Fi", ["wifi", "wireless", "internet", "network"], "android.settings.WIFI_SETTINGS"),
    page!("Bluetooth", ["headphones", "pairing"], "android.settings.BLUETOOTH_SETTINGS"),
    page!("Mobile network", ["mobile data", "sim", "carrier", "roaming", "apn"], "android.settings.NETWORK_OPERATOR_SETTINGS"),
    page!("Airplane mode", ["flight", "aeroplane"], "android.settings.AIRPLANE_MODE_SETTINGS"),
    page!("Data usage", ["data saver", "mobile data"], "android.settings.DATA_USAGE_SETTINGS"),
    page!("Display", ["brightness", "screen", "font size", "dark theme", "timeout"], "android.settings.DISPLAY_SETTINGS"),
    page!("Night light", ["blue light", "eye comfort"], "android.settings.NIGHT_DISPLAY_SETTINGS"),
    page!("Sound", ["volume", "ringtone", "vibrate", "do not disturb"], "android.settings.SOUND_SETTINGS"),
    page!("Battery", ["power", "charging"], "android.intent.action.POWER_USAGE_SUMMARY"),
    page!("Battery saver", ["power saving"], "android.settings.BATTERY_SAVER_SETTINGS"),
    page!("Apps", ["applications", "app info", "uninstall", "permissions"], "android.settings.MANAGE_APPLICATIONS_SETTINGS"),
    page!("Default apps", ["browser", "sms", "phone app"], "android.settings.MANAGE_DEFAULT_APPS_SETTINGS"),
    page!("Home app", ["launcher"], "android.settings.HOME_SETTINGS"),
    page!("Storage", ["space", "free up", "memory card"], "android.settings.INTERNAL_STORAGE_SETTINGS"),
    page!("Location", ["gps"], "android.settings.LOCATION_SOURCE_SETTINGS"),
    page!("Security", ["lock screen", "password", "pin", "fingerprint", "face unlock"], "android.settings.SECURITY_SETTINGS"),
    page!("Privacy", ["permissions"], "android.settings.PRIVACY_SETTINGS"),
    page!("Accounts", ["sync", "google account"], "android.settings.SYNC_SETTINGS"),
    page!("Accessibility", ["talkback", "magnification", "captions"], "android.settings.ACCESSIBILITY_SETTINGS"),
    page!("Languages", ["language", "locale", "region"], "android.settings.LOCALE_SETTINGS"),
    page!("Keyboard", ["input", "gboard", "spell check"], "android.settings.INPUT_METHOD_SETTINGS"),
    page!("Date & time", ["clock", "time zone", "timezone"], "android.settings.DATE_SETTINGS"),
    page!("NFC", ["contactless", "tap to pay"], "android.settings.NFC_SETTINGS"),
    page!("Cast", ["screen mirroring", "chromecast"], "android.settings.CAST_SETTINGS"),
    page!("VPN", ["proxy"], "android.settings.VPN_SETTINGS"),
    page!("Developer options", ["developer", "usb debugging", "adb"], "android.settings.APPLICATION_DEVELOPMENT_SETTINGS"),
    page!("About phone", ["model", "android version", "build number", "imei"], "android.settings.DEVICE_INFO_SETTINGS"),
];

pub const MAX_SETTINGS: usize = 4;

/// Settings pages matching `query` that exist on this phone, best first.
pub fn match_settings(query: &str, available: &dyn Fn(&str) -> bool) -> Vec<&'static SettingsPage> {
    let q = normalise(query);
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, usize)> = SETTINGS_PAGES
        .iter()
        .enumerate()
        .filter(|(_, p)| available(p.action))
        .filter_map(|(i, p)| {
            let label = tier(p.label, &q);
            // A keyword counts as a later word: never ahead of a name match.
            let keyword = p.keywords.iter().filter_map(|k| tier(k, &q).map(|t| t.max(1))).min();
            label.into_iter().chain(keyword).min().map(|t| (t, i))
        })
        .collect();
    hits.sort_by_key(|&(t, i)| (t, i));
    hits.into_iter().take(MAX_SETTINGS).map(|(_, i)| &SETTINGS_PAGES[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPS: [&str; 4] = ["Camera", "Calendar", "Google Camera", "Scanner"];

    #[test]
    fn start_of_name_then_word_then_anywhere() {
        assert_eq!(rank(&APPS, "cam"), vec![0, 2]);
        assert_eq!(rank(&APPS, "can"), vec![3]);
        assert_eq!(rank(&["Files by Google", "Google"], "goo"), vec![1, 0]);
        assert_eq!(rank(&["Xcalc", "My Calc", "Calc"], "calc"), vec![2, 1, 0]);
    }

    #[test]
    fn case_and_spaces_do_not_matter() {
        assert_eq!(rank(&APPS, "  CAM "), rank(&APPS, "cam"));
    }

    #[test]
    fn word_breaks_include_punctuation() {
        assert_eq!(rank(&["Zed", "1.1.1.1", "my-app"], "app"), vec![2]);
        assert_eq!(rank(&["Zed", "1.1.1.1"], "1"), vec![1]);
    }

    #[test]
    fn empty_query_is_everything() {
        assert_eq!(rank(&APPS, ""), vec![0, 1, 2, 3]);
        assert_eq!(rank(&APPS, "   "), vec![0, 1, 2, 3]);
    }

    fn all(_: &str) -> bool {
        true
    }

    fn labels(v: Vec<&SettingsPage>) -> Vec<&str> {
        v.into_iter().map(|p| p.label).collect()
    }

    #[test]
    fn settings_by_name_and_keyword() {
        assert_eq!(labels(match_settings("wifi", &all))[0], "Wi-Fi");
        assert_eq!(labels(match_settings("bright", &all)), vec!["Display"]);
        assert_eq!(labels(match_settings("bat", &all))[..2], ["Battery", "Battery saver"]);
    }

    #[test]
    fn settings_missing_on_this_phone_are_left_out() {
        assert!(match_settings("wifi", &|_| false).is_empty());
        let no_nfc = |a: &str| a != "android.settings.NFC_SETTINGS";
        assert!(labels(match_settings("nfc", &no_nfc)).is_empty());
    }

    #[test]
    fn at_most_four_settings() {
        assert_eq!(match_settings("s", &all).len(), 4);
        assert!(match_settings("", &all).is_empty());
    }
}
