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

/// Holo icons (two-layer icons cut to a rounded square).
const ICONS: &str = "icons-holo";

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
        // Icons cut by the old look; never mixed with the Holo ones.
        if dir.join("icons").exists() {
            let _ = fs::remove_dir_all(dir.join("icons"));
            log::info!("cache: removed old icon folder");
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

    pub fn load_icon(&self, key: &AppKey, size: u32) -> Option<(Vec<u8>, u32, u32)> {
        let bytes = fs::read(self.icon_path(key)).ok()?;
        let (header, rgba) = qoi::decode_to_vec(&bytes).ok()?;
        if header.width != size || header.height != size || header.channels != qoi::Channels::Rgba {
            return None;
        }
        Some((rgba, size, size))
    }

    pub fn save_icon(&self, key: &AppKey, size: u32, rgba: &[u8]) {
        match qoi::encode_to_vec(rgba, size, size) {
            Ok(bytes) => {
                if let Err(e) = write_atomic(&self.icon_path(key), &bytes) {
                    log::warn!("cache: saving icon failed: {e}");
                }
            }
            Err(e) => log::warn!("cache: encoding icon failed: {e}"),
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
