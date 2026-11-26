//! Holo Gallery's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod actions;
mod albums_screen;
mod bin_screen;
mod bars;
mod gif_player;
mod photo_tex;
mod video;
mod viewer;
mod viewer_jobs;
mod chrome;
mod input;
mod library;
mod photos;
mod system;
mod thumbs;
mod widgets;

use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent, PollEvent};
use gallery_core::access::Access;
use gallery_core::arrange::Filter;
use holo_platform::gl_surface::GlSurface;
use holo_platform::holo::BLACK;
use holo_platform::refresh::FrameRate;
use holo_platform::render::{Renderer, RendererConfig};
use holo_platform::sys;
use holo_platform::vsync::Vsync;
use holo_platform::waker;
use launcher_core::frame::FrameScheduler;
use launcher_core::refresh::{RateVote, RefreshPolicy};
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};
use albums_screen::AlbumsScreen;
use photos::{Action, Photos};
use thumbs::Thumbs;
use viewer::Viewer;

/// Work longer than this risks missing the screen's next refresh.
const SLOW: Duration = Duration::from_millis(8);
/// If the screen's refresh call never comes (window going away), draw anyway.
const VSYNC_FALLBACK: Duration = Duration::from_millis(50);
const ANIM_STEP: Duration = Duration::from_millis(20);

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    sys::init_logging("gallery", c"debug.gallery.log");
    log::info!("android_main start, {:.0} ms after process start", sys::ms_since_process_start());
    sys::return_freed_memory_quickly();
    // A rebuilt activity runs a new loop in the same process: wake-ups and
    // the refresh callback must point at this one.
    waker::init(&app);
    let data_dir = app.internal_data_path().unwrap_or_else(|| std::path::PathBuf::from("/data/local/tmp"));
    library::start(&app, data_dir.clone());
    // Android 8-10: our own bin, and our own favourites list.
    let system_bin = actions::system_bin(&app);
    let mut favs = actions::Favourites::load(&data_dir);
    if !system_bin {
        actions::purge_expired(&app, actions::now_ms());
    }
    let mut bin: Option<bin_screen::BinScreen> = None;
    let thumbs = Thumbs::new(&app);
    let d0 = app.config().density().unwrap_or(160) as f32 / 160.0;
    let mut photos = Photos::new(d0, Filter::All, None);
    let mut albums = AlbumsScreen::new(d0);
    // An open album: the same grid with a filter and a back arrow.
    let mut album: Option<Photos> = None;
    let mut on_albums = false;
    let jobs = viewer_jobs::Jobs::new(&app);
    let mut viewer: Option<Viewer> = None;
    // Closed viewers whose pictures still need freeing (needs the GL context).
    let mut closed: Vec<Viewer> = Vec::new();
    let mut viewer_list_stale = false;
    let mut photo_draw: Option<photo_tex::PhotoDraw> = None;
    let mut screen_w = 1080.0f32;
    let mut asked_access = false;

    let mut sched = FrameScheduler::new();
    let mut gfx: Option<GlSurface> = None;
    let mut renderer: Option<Renderer> = None;
    let mut batch = Batch::new();
    let mut policy = RefreshPolicy::new();
    let mut vote = RateVote::Idle;
    let frame_rate = FrameRate::new(&app);
    let vsync = Vsync::new();
    let mut waiting_since: Option<Instant> = None;
    let mut first_frame = true;
    let mut quit = false;

    while !quit {
        let now = Instant::now();
        let frame_wake = match &vsync {
            Some(v) if sched.should_draw(now) => {
                v.request();
                let since = *waiting_since.get_or_insert(now);
                Some(VSYNC_FALLBACK.saturating_sub(now.duration_since(since)))
            }
            _ => sched.next_wake(now),
        };
        let timeout = [frame_wake, policy.next_check(now)].into_iter().flatten().min();

        app.poll_events(timeout, |event| {
            if let PollEvent::Main(main) = event {
                match main {
                    MainEvent::InitWindow { .. } => {
                        if let Some(window) = app.native_window() {
                            let result = match gfx.as_mut() {
                                Some(g) => g.resume(&window),
                                None => GlSurface::new(&window).map(|g| gfx = Some(g)),
                            };
                            if let Err(e) = result {
                                log::error!("GL setup failed: {e}");
                            }
                            sched.request_redraw();
                            frame_rate.apply(&window, vote);
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        if let Some(g) = gfx.as_mut() {
                            g.suspend();
                        }
                    }
                    MainEvent::Resume { .. }
                    | MainEvent::WindowResized { .. }
                    | MainEvent::RedrawNeeded { .. }
                    | MainEvent::ContentRectChanged { .. }
                    | MainEvent::ConfigChanged { .. } => sched.request_redraw(),
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        let input = input::drain(&app);
        if !input.touches.is_empty() {
            policy.note_activity(Instant::now());
            sched.request_redraw();
        }
        for t in input.touches {
            if let Some(v) = viewer.as_mut() {
                let act = v.touch(t);
                let item = |id: i64| photos.items().iter().find(|i| i.id == id).cloned();
                match act {
                    viewer::Action::None => {}
                    viewer::Action::Back => closed.extend(viewer.take()),
                    viewer::Action::Share(id) => {
                        if let Some(it) = item(id) {
                            actions::share(&app, &[(id, it.kind == gallery_core::media::Kind::Video)]);
                        }
                    }
                    viewer::Action::Delete(id) => {
                        if let Some(it) = item(id) {
                            trash(&app, system_bin, &[(id, it.kind == gallery_core::media::Kind::Video)]);
                        }
                    }
                    viewer::Action::Favourite(id) => {
                        if let Some(it) = item(id) {
                            if system_bin {
                                actions::request(&app, actions::Op::Favourite, &[(id, it.kind == gallery_core::media::Kind::Video)], !it.favourite);
                            } else {
                                favs.set(id, !it.favourite);
                                let fresh = with_favourites(photos.items(), &favs);
                                albums.set_items(fresh.clone());
                                if let Some(a) = album.as_mut() {
                                    a.set_items(fresh.clone());
                                }
                                photos.set_items(fresh);
                                viewer_list_stale = true;
                            }
                        }
                    }
                    other => log::info!("not built yet: {other:?}"),
                }
                continue;
            }
            if let Some(b) = bin.as_mut() {
                match b.touch(t) {
                    bin_screen::Action::None => {}
                    bin_screen::Action::Back => bin = None,
                    bin_screen::Action::Restore => bin_op(&app, system_bin, b, actions::Op::Restore, false),
                    bin_screen::Action::DeleteForever => bin_op(&app, system_bin, b, actions::Op::Delete, false),
                    bin_screen::Action::Empty => bin_op(&app, system_bin, b, actions::Op::Delete, true),
                }
                continue;
            }
            let action = match (album.as_mut(), on_albums) {
                (Some(a), _) => a.touch(t, library::access()),
                (None, true) => {
                    match albums.touch(t) {
                        albums_screen::Action::None => {}
                        albums_screen::Action::Photos => on_albums = false,
                        albums_screen::Action::Bin => bin = Some(open_bin(&app, system_bin, photos_density(&app))),
                        albums_screen::Action::Open(a) => {
                            let mut grid = Photos::new(photos_density(&app), a.filter, Some(a.name));
                            grid.set_items(photos.items().clone());
                            grid.set_sort(photos.sort, photos.order);
                            album = Some(grid);
                        }
                        other => log::info!("not built yet: {other:?}"),
                    }
                    Action::None
                }
                (None, false) => photos.touch(t, library::access()),
            };
            match action {
                Action::None => {}
                Action::Open(i) => {
                    let grid = album.as_ref().unwrap_or(&photos);
                    viewer = Some(Viewer::open(&app, photos_density(&app), grid.items().clone(), grid.arranged().order.clone(), i as usize, screen_w));
                }
                Action::Albums => on_albums = true,
                Action::Bin => bin = Some(open_bin(&app, system_bin, photos_density(&app))),
                Action::Share => {
                    let grid = album.as_ref().unwrap_or(&photos);
                    actions::share(&app, &grid.chosen());
                }
                Action::Delete => {
                    let grid = album.as_mut().unwrap_or(&mut photos);
                    trash(&app, system_bin, &grid.chosen());
                    // Android 11+ asks first; the choice ends when it's done.
                    if !system_bin {
                        grid.end_selection();
                    }
                }
                Action::Back => {
                    if let Some(a) = album.take() {
                        photos.set_sort(a.sort, a.order);
                    }
                }
                Action::AskAccess => {
                    if library::can_ask(&app) {
                        library::request_access(&app);
                    } else {
                        library::open_settings(&app);
                    }
                }
                other => log::info!("not built yet: {other:?}"),
            }
        }
        if input.back {
            sched.request_redraw();
            if let Some(v) = viewer.as_mut() {
                if !v.back() {
                    closed.extend(viewer.take());
                }
            } else if let Some(b) = bin.as_mut() {
                if !b.back() {
                    bin = None;
                }
            } else if let Some(a) = album.as_mut() {
                if !a.back() {
                    let a = album.take().unwrap();
                    photos.set_sort(a.sort, a.order);
                }
            } else if on_albums {
                on_albums = false;
            } else if !photos.back() {
                system::finish(&app);
            }
        }
        for (op, ok) in actions::take_done() {
            log::info!("media action {op:?}: {}", if ok { "done" } else { "not done" });
            sched.request_redraw();
            if !ok {
                continue;
            }
            if op == actions::Op::Trash {
                photos.end_selection();
                if let Some(a) = album.as_mut() {
                    a.end_selection();
                }
            }
            if let Some(b) = bin.as_mut() {
                b.clear_selection();
                b.set_items(bin_items(&app, system_bin));
            }
        }
        if let Some(fresh) = library::take_update() {
            log::info!("showing {} items", fresh.len());
            let fresh = if system_bin { fresh } else { with_favourites(&fresh, &favs) };
            if let Some(b) = bin.as_mut() {
                b.set_items(bin_items(&app, system_bin));
            }
            albums.set_items(fresh.clone());
            if let Some(a) = album.as_mut() {
                a.set_items(fresh.clone());
            }
            photos.set_items(fresh);
            viewer_list_stale = viewer.is_some();
            thumbs.clear_failures();
            sched.request_redraw();
        }
        if library::take_asked() {
            sched.request_redraw();
        }
        if thumbs.has_ready() || jobs.has_done() || video::frame_waiting() || actions::has_done() {
            sched.request_redraw();
        }

        let now = Instant::now();
        let tick = vsync.as_ref().and_then(|v| v.take_tick());
        let on_beat = vsync.is_none()
            || tick.is_some()
            || waiting_since.is_some_and(|t| now.duration_since(t) >= VSYNC_FALLBACK);
        if sched.should_draw(now) && on_beat {
            if let Some(w) = waiting_since.map(|t| now.duration_since(t)).filter(|w| *w > Duration::from_millis(250)) {
                log::info!("slow wait: wanted a frame for {:.0} ms", w.as_secs_f64() * 1e3);
            }
            waiting_since = None;
            if let Some(v) = vsync.as_ref().filter(|_| sched.is_animating()) {
                v.request();
            }
            // Folding or unfolding: build the surface afresh at the new size.
            if let (Some(g), Some(w)) = (gfx.as_mut(), app.native_window()) {
                if g.outgrown(&w) {
                    if let Err(e) = g.resume(&w) {
                        log::error!("GL surface rebuild failed: {e}");
                    }
                    frame_rate.apply(&w, vote);
                }
            }
            let mut moving = false;
            let mut again = false;
            if let Some(g) = gfx.as_ref().filter(|g| g.has_surface()) {
                let r = renderer.get_or_insert_with(|| {
                    let mut r = Renderer::new(g.gl(), RendererConfig::GALLERY);
                    r.set_glyph_budget(Duration::from_nanos(frame_rate.period_ns() as u64 / 5));
                    r
                });
                let t0 = Instant::now();
                r.begin_frame();
                batch.clear();
                let screen = g.size();
                let d = app.config().density().unwrap_or(160) as f32 / 160.0;
                let ins = bars::insets(d);
                let (w, h) = (screen.0 as f32, screen.1 as f32);
                // New thumbnails go in before drawing, a slice of the frame at most.
                let had_ready = thumbs.has_ready();
                let more = thumbs.upload(g.gl(), r, Duration::from_nanos(frame_rate.period_ns() as u64 / 5));
                if had_ready && !more {
                    Thumbs::settle(g.gl());
                }
                screen_w = w;
                for v in closed.drain(..) {
                    v.free(g.gl());
                }
                if viewer.is_none() {
                    batch.rect(Rect::new(0.0, 0.0, w, h), Rgba::hex(BLACK), 0.0);
                }
                photos.set_density(d);
                albums.set_density(d);
                let (today, utc_offset_s) = system::local_today();
                let frame = photos::Frame { w, h, ins, today, utc_offset_s };
                let access = library::access();
                if std::mem::take(&mut viewer_list_stale) && let Some(v) = viewer.as_mut() {
                    let grid = album.as_mut().unwrap_or(&mut photos);
                    grid.refresh(&frame);
                    if !v.set_list(grid.items().clone(), grid.arranged().order.clone()) {
                        closed.extend(viewer.take());
                    }
                }
                let (anim, missing) = if let Some(v) = viewer.as_mut() {
                    let pd = photo_draw.get_or_insert_with(|| photo_tex::PhotoDraw::new(g.gl()));
                    (v.draw(g.gl(), r, &mut batch, pd, &thumbs, &jobs, frame, now), Vec::new())
                } else if let Some(b) = bin.as_mut() {
                    b.draw(g.gl(), r, &mut batch, &thumbs, frame, now)
                } else { match (album.as_mut(), on_albums) {
                    (Some(a), _) => {
                        a.set_density(d);
                        a.draw(g.gl(), r, &mut batch, &thumbs, frame, access, now)
                    }
                    (None, true) => albums.draw(g.gl(), r, &mut batch, &thumbs, frame, now),
                    (None, false) => photos.draw(g.gl(), r, &mut batch, &thumbs, frame, access, now),
                } };
                thumbs.request(&missing);
                moving = anim;
                r.draw_layer(g.gl(), &batch, screen, viewer.is_none());
                if t0.elapsed() > SLOW {
                    log::info!("slow frame: {:.1} ms", t0.elapsed().as_secs_f64() * 1e3);
                }
                let swap_t = Instant::now();
                if let Err(e) = g.swap() {
                    log::error!("swap failed: {e}");
                }
                if swap_t.elapsed() > Duration::from_millis(100) {
                    log::info!("slow swap: {:.0} ms", swap_t.elapsed().as_secs_f64() * 1e3);
                }
                if first_frame {
                    first_frame = false;
                    log::info!("first frame: {:.0} ms after process start", sys::ms_since_process_start());
                    sys::purge_freed_memory();
                }
                again = r.wants_redraw() || more;
            }
            if moving {
                sched.animate_until(now + ANIM_STEP);
            }

            sched.frame_drawn(now);
            if again {
                sched.request_redraw();
            }
        }

        // First run: ask for the library once the first frame is up.
        if !first_frame && !asked_access && library::access_known() && library::access() == Access::None {
            asked_access = true;
            library::request_access(&app);
        }

        if sched.is_animating() {
            policy.note_activity(now);
        }
        if let Some(v) = policy.update(now) {
            vote = v;
            if let Some(window) = app.native_window() {
                frame_rate.apply(&window, vote);
            }
        }
    }
    log::info!("android_main end");
}

fn photos_density(app: &AndroidApp) -> f32 {
    app.config().density().unwrap_or(160) as f32 / 160.0
}

/// Into the Bin: Android's on 11+ (it may ask first), ours on 8-10.
fn trash(app: &AndroidApp, system_bin: bool, items: &[(i64, bool)]) {
    if items.is_empty() {
        return;
    }
    if system_bin {
        actions::request(app, actions::Op::Trash, items, true);
    } else {
        let n = actions::trash_legacy(app, items);
        log::info!("bin: moved {n} of {}", items.len());
        library::refresh();
    }
}

fn open_bin(app: &AndroidApp, system_bin: bool, d: f32) -> bin_screen::BinScreen {
    let mut b = bin_screen::BinScreen::new(d);
    b.set_items(bin_items(app, system_bin));
    b
}

/// What the Bin holds. On Android 8-10 our own list, as items with
/// negative ids whose thumbnails come from the stored files.
fn bin_items(app: &AndroidApp, system_bin: bool) -> Vec<gallery_core::media::MediaItem> {
    use gallery_core::media::{Kind, MediaItem};
    if system_bin {
        return library::query_trashed(app);
    }
    let entries = actions::own_bin(app);
    let mut paths = std::collections::HashMap::new();
    let items = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let id = -(i as i64) - 1;
            paths.insert(id, e.stored.clone());
            let orig = std::path::Path::new(&e.original_path);
            let name = orig.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = orig.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
            let kind = match ext.as_str() {
                "mp4" | "3gp" | "mkv" | "webm" | "mov" => Kind::Video,
                "gif" => Kind::Gif,
                _ => Kind::Image,
            };
            MediaItem {
                id,
                kind,
                w: 0,
                h: 0,
                orientation: 0,
                duration_ms: 0,
                size: std::fs::metadata(&e.stored).map_or(0, |m| m.len()),
                taken_ms: e.deleted_ms,
                modified_ms: e.deleted_ms,
                added_ms: e.deleted_ms,
                favourite: false,
                bucket_id: 0,
                name,
                folder: orig.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                mime: String::new(),
                expires_ms: e.deleted_ms + gallery_core::own_bin::KEEP_MS,
            }
        })
        .collect();
    thumbs::set_paths(paths);
    items
}

/// RESTORE, DELETE FOREVER, or (`everything`) EMPTY BIN.
fn bin_op(app: &AndroidApp, system_bin: bool, b: &mut bin_screen::BinScreen, op: actions::Op, everything: bool) {
    let chosen = if everything { b.all() } else { b.chosen() };
    if chosen.is_empty() {
        return;
    }
    if system_bin {
        actions::request(app, op, &chosen, false);
        return;
    }
    let entries = actions::own_bin(app);
    let which: Vec<gallery_core::own_bin::BinEntry> = chosen.iter().filter_map(|&(id, _)| entries.get((-id - 1) as usize).cloned()).collect();
    if op == actions::Op::Restore {
        actions::restore_legacy(app, &which);
        library::refresh();
    } else {
        actions::delete_legacy(app, &which);
    }
    b.clear_selection();
    b.set_items(bin_items(app, system_bin));
}

/// Android 8-10: our favourites list laid over Android's list.
fn with_favourites(items: &std::sync::Arc<Vec<gallery_core::media::MediaItem>>, favs: &actions::Favourites) -> std::sync::Arc<Vec<gallery_core::media::MediaItem>> {
    let mut v = (**items).clone();
    for it in &mut v {
        it.favourite = favs.ids.contains(&it.id);
    }
    std::sync::Arc::new(v)
}
