//! MOVE: chosen items go into another folder, on a worker thread. A name
//! already used there gets a number (`IMG_1_2.jpg`). The result comes back
//! as a short message for the grid.

use std::sync::Mutex;

use android_activity::AndroidApp;
use gallery_core::copies::free_name;
use gallery_core::media::{Kind, MediaItem};

use crate::{actions, library, saving};

/// A move waiting for Android's yes (11+) or about to run.
pub struct Pending {
    pub items: Vec<MediaItem>,
    /// The folder, relative to the storage root, e.g. `DCIM/Camera/`.
    pub rel: String,
    /// Its name for the message.
    pub folder: String,
}

static RESULT: Mutex<Option<String>> = Mutex::new(None);

pub fn take_result() -> Option<String> {
    RESULT.lock().unwrap().take()
}

pub fn start(app: &AndroidApp, p: Pending) {
    let app = app.clone();
    let spawned = std::thread::Builder::new().name("move".into()).spawn(move || {
        let mut moved = 0;
        for it in &p.items {
            let name = free_name(&it.name, &|n| saving::exists(&app, &p.rel, n));
            if actions::move_one(&app, it.id, it.kind == Kind::Video, &p.rel, &name) {
                moved += 1;
            } else {
                log::warn!("move: {} could not go to {}", it.name, p.rel);
            }
        }
        log::info!("move: {moved} of {} into {}", p.items.len(), p.rel);
        let msg = match (moved, p.items.len()) {
            (0, _) => "Couldn't move them".to_string(),
            (m, n) if m == n => format!("Moved {} to {}", gallery_core::format::count(m), p.folder),
            (m, n) => format!("Moved {m} of {n} to {}", p.folder),
        };
        *RESULT.lock().unwrap() = Some(msg);
        library::refresh();
        holo_platform::waker::wake();
    });
    if let Err(e) = spawned {
        log::warn!("move thread: {e}");
    }
}
