//! Files in the app's private folder: the app list, finished icons, the
//! home layout. Each write goes to a temp file first and is then renamed,
//! so a crash mid-write never leaves a half file behind.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use android_activity::AndroidApp;
use launcher_core::applist::{self, AppRecord};
use launcher_core::home::{AppKey, HomeLayout};
use launcher_core::settings::Settings;

/// Holo icons (two-layer icons cut to a rounded square, padded pictures
/// enlarged).
const ICONS: &str = "icons-holo-2";
/// Earlier icon folders, removed on start.
const OLD_ICONS: [&str; 2] = ["icons", "icons-holo"];

#[derive(Clone)]
pub struct Store {
    /// The app's private folder; the home layout lives here.
    data: PathBuf,
    /// `cache/` inside it: things that can be rebuilt from Android.
    dir: PathBuf,
}

fn key_hash(key: &AppKey) -> u64 {
    // FNV-1a: stable across builds, unlike std's hasher.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.package.bytes().chain([b'/']).chain(key.activity.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

impl Store {
    pub fn open(app: &AndroidApp) -> Store {
        let base = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp"));
        let dir = base.join("cache");
        if let Err(e) = fs::create_dir_all(dir.join(ICONS)) {
            log::warn!("cache folder: {e}");
        }
        // Icons made the old way; never mixed with the current ones.
        for old in OLD_ICONS {
            if dir.join(old).exists() {
                let _ = fs::remove_dir_all(dir.join(old));
                log::info!("cache: removed old icon folder");
            }
        }
        Store { data: base, dir }
    }

    fn icon_path(&self, key: &AppKey) -> PathBuf {
        self.dir.join(ICONS).join(format!("{:016x}.qoi", key_hash(key)))
    }

    pub fn load_apps(&self) -> Option<(Vec<AppRecord>, i32)> {
        let text = fs::read_to_string(self.dir.join("apps.txt")).ok()?;
        let parsed = applist::from_text(&text);
        if parsed.is_none() {
            log::warn!("cache: apps.txt is broken, listing live");
        }
        parsed
    }

    pub fn save_apps(&self, records: &[AppRecord], seq: i32) {
        if let Err(e) = write_atomic(&self.dir.join("apps.txt"), applist::to_text(records, seq).as_bytes()) {
            log::warn!("cache: saving apps failed: {e}");
        }
    }

    /// Boot count the app list was checked under. Android restarts its
    /// change counter on every boot, so a different boot means "list again".
    pub fn load_boot(&self) -> Option<i32> {
        fs::read_to_string(self.dir.join("boot.txt")).ok()?.trim().parse().ok()
    }

    pub fn save_boot(&self, boot: i32) {
        let _ = write_atomic(&self.dir.join("boot.txt"), boot.to_string().as_bytes());
    }

    /// The cached icon file's bytes, still compressed. Opening a file costs
    /// 1.5-3 ms on the XZ3, ten times the decode, so callers keep these.
    pub fn read_icon(&self, key: &AppKey) -> Option<Vec<u8>> {
        fs::read(self.icon_path(key)).ok()
    }

    /// Unpack bytes from `read_icon`; `None` if they're for another size.
    pub fn decode_icon(bytes: &[u8], size: u32) -> Option<(Vec<u8>, u32, u32)> {
        let (header, rgba) = qoi::decode_to_vec(bytes).ok()?;
        if header.width != size || header.height != size || header.channels != qoi::Channels::Rgba {
            return None;
        }
        Some((rgba, size, size))
    }

    pub fn encode_icon(size: u32, rgba: &[u8]) -> Option<Vec<u8>> {
        match qoi::encode_to_vec(rgba, size, size) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                log::warn!("cache: encoding icon failed: {e}");
                None
            }
        }
    }

    pub fn save_icon_bytes(&self, key: &AppKey, bytes: &[u8]) {
        if let Err(e) = write_atomic(&self.icon_path(key), bytes) {
            log::warn!("cache: saving icon failed: {e}");
        }
    }

    pub fn forget_icon(&self, key: &AppKey) {
        let _ = fs::remove_file(self.icon_path(key));
    }

    /// The home layout and the settings, which share one file.
    pub fn load_layout(&self) -> Option<(HomeLayout, Settings)> {
        let text = fs::read_to_string(self.data.join("layout.txt")).ok()?;
        let parsed = HomeLayout::from_text(&text);
        if parsed.is_none() {
            log::warn!("cache: layout.txt is broken, starting fresh");
        }
        Some((parsed?, Settings::from_text(&text)))
    }

    pub fn save_layout(&self, layout: &HomeLayout, settings: &Settings) {
        let text = layout.to_text() + &settings.to_text();
        if let Err(e) = write_atomic(&self.data.join("layout.txt"), text.as_bytes()) {
            log::warn!("cache: saving layout failed: {e}");
        }
    }
}
