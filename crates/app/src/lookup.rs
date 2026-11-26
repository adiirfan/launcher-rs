//! Slow searches (contacts, which settings pages exist) on their own thread.
//!
//! Each asks another app over Android's binder, which can take tens of ms per
//! letter and over half a second the first time. Run on the main loop, that
//! held up typing and drawing: letters showed late and in bunches. Here the
//! main loop only leaves a request and picks up the answer when it's ready.

use std::collections::HashSet;
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;

use android_activity::AndroidApp;

use crate::contacts::{self, Contact};
use crate::{keyboard, settings_pages};

struct Shared {
    /// The activity to ask through; replaced when Android rebuilds it.
    app: Option<AndroidApp>,
    /// The newest contacts search wanted, not yet started.
    wanted: Option<(String, usize)>,
    /// The newest finished search: the query and its matches.
    found: Option<(String, Vec<Contact>)>,
    /// Settings pages this phone has, once known.
    pages: Option<HashSet<&'static str>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared { app: None, wanted: None, found: None, pages: None });
static WORK: Condvar = Condvar::new();
static STARTED: OnceLock<()> = OnceLock::new();

/// Call at the start of every main loop: hands over the current activity,
/// starts the thread the first time, and has it check the settings pages.
pub fn start(app: &AndroidApp) {
    SHARED.lock().unwrap().app = Some(app.clone());
    STARTED.get_or_init(|| {
        thread::Builder::new().name("lookups".into()).spawn(run).expect("lookup thread");
    });
    WORK.notify_one();
}

/// Ask for the contacts matching `query`. Only the newest request runs; an
/// older one still waiting is dropped.
pub fn want_contacts(query: &str, limit: usize) {
    let mut s = SHARED.lock().unwrap();
    s.wanted = Some((query.to_string(), limit));
    drop(s);
    WORK.notify_one();
}

/// The newest finished contacts search, if one came in since the last call.
pub fn take_contacts() -> Option<(String, Vec<Contact>)> {
    SHARED.lock().unwrap().found.take()
}

/// The settings pages this phone has, once the thread has checked.
pub fn settings_pages() -> Option<HashSet<&'static str>> {
    SHARED.lock().unwrap().pages.clone()
}

fn run() {
    loop {
        let (app, job, check_pages) = {
            let mut s = SHARED.lock().unwrap();
            loop {
                let check_pages = s.pages.is_none() && s.app.is_some();
                if s.wanted.is_some() || check_pages {
                    break (s.app.clone(), s.wanted.take(), check_pages);
                }
                s = WORK.wait(s).unwrap();
            }
        };
        let Some(app) = app else { continue };
        if check_pages {
            let pages = settings_pages::available(&app);
            SHARED.lock().unwrap().pages = Some(pages);
            keyboard::wake();
        }
        if let Some((query, limit)) = job {
            let found = contacts::search(&app, &query, limit);
            SHARED.lock().unwrap().found = Some((query, found));
            keyboard::wake();
        }
    }
}
