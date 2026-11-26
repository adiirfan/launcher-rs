//! Holo Gallery's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod actions;
mod albums_screen;
mod bin_screen;
mod bars;
mod gif_maker;
mod gif_player;
mod gif_screen;
mod gif_to_video;
mod motion;
mod save_video;
mod photo_tex;
mod video;
mod viewer;
mod viewer_jobs;
mod chrome;
mod input;
mod library;
mod mover;
mod open_with;
mod clip_screen;
mod convert_screen;
mod crop_save;
mod crop_screen;
mod photos;
mod pixels;
mod saving;
mod settings_screen;
mod system;
mod thumbs;
mod widgets;

use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent, PollEvent};
use gallery_core::access::Access;
use gallery_core::arrange::Filter;
use gallery_core::settings::GallerySettings;
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
    motion::load(&data_dir);
    if !system_bin {
        actions::purge_expired(&app, actions::now_ms());
    }
    let mut bin: Option<bin_screen::BinScreen> = None;
    let thumbs = Thumbs::new(&app);
    let d0 = app.config().density().unwrap_or(160) as f32 / 160.0;
    let mut photos = Photos::new(d0, Filter::All, None);
    let mut albums = AlbumsScreen::new(d0);
    let mut gs = std::fs::read_to_string(settings_path(&data_dir)).map(|t| GallerySettings::from_text(&t)).unwrap_or_default();
    apply_settings(&gs, &mut photos, None, &mut albums);
    let mut settings: Option<settings_screen::SettingsScreen> = None;
    // Android 12+: whether "Media management" is allowed (None: not offered).
    let mut manage: Option<bool> = None;
    let mut manage_stale = true;
    // An open album: the same grid with a filter and a back arrow.
    let mut album: Option<Photos> = None;
    let mut on_albums = false;
    let jobs = viewer_jobs::Jobs::new(&app);
    let mut viewer: Option<Viewer> = None;
    // Closed viewers whose pictures still need freeing (needs the GL context).
    let mut closed: Vec<Viewer> = Vec::new();
    // The crop screen sits over the viewer; closed ones wait for the GL context.
    let mut crop: Option<crop_screen::CropScreen> = None;
    let mut closed_crops: Vec<crop_screen::CropScreen> = Vec::new();
    // The clip screen, the same way.
    let mut clip: Option<clip_screen::ClipScreen> = None;
    let mut closed_clips: Vec<clip_screen::ClipScreen> = Vec::new();
    // Convert sits over everything (over Crop for SAVE AS…).
    let mut convert: Option<convert_screen::ConvertScreen> = None;
    // Make a GIF; its "choose the part" step borrows the clip screen.
    let mut gif: Option<gif_screen::GifScreen> = None;
    // "Save as video" for a GIF: a box over the viewer.
    let mut save_vid: Option<save_video::SaveVideo> = None;
    let mut viewer_list_stale = false;
    // MOVE: the folder chooser and the items it's for; then the move
    // waiting for Android's yes; then a message for the grid.
    let mut move_pick: Option<(AlbumsScreen, Vec<gallery_core::media::MediaItem>)> = None;
    let mut pending_move: Option<mover::Pending> = None;
    let mut grid_toast: Option<(Instant, String)> = None;
    let mut photo_draw: Option<photo_tex::PhotoDraw> = None;
    let mut screen_w = 1080.0f32;
    let mut asked_access = false;
    // "Open with" from another app: the link waits until the library is in.
    let mut want_open: Option<(open_with::Handed, Instant)> = None;
    let mut library_seen = false;
    // Opened from another app: Back returns there.
    let mut outside = false;
    let mut viewer_source = ViewerSource::Grid;

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
                        motion::save();
                        // Left while showing another app's photo (Home, not
                        // Back): that app is no longer waiting, so the next
                        // Back goes to the grid instead of leaving the gallery.
                        outside = false;
                        if let Some(g) = gfx.as_mut() {
                            g.suspend();
                        }
                    }
                    MainEvent::Resume { .. } => {
                        // Back from Android's settings: it may have changed.
                        manage_stale = true;
                        sched.request_redraw();
                    }
                    MainEvent::WindowResized { .. }
                    | MainEvent::RedrawNeeded { .. }
                    | MainEvent::ContentRectChanged { .. }
                    | MainEvent::ConfigChanged { .. } => sched.request_redraw(),
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        if let Some(h) = open_with::take() {
            want_open = Some((h, Instant::now()));
        }
        if want_open.is_some() {
            sched.request_redraw();
        }
        let input = input::drain(&app);
        if !input.touches.is_empty() {
            policy.note_activity(Instant::now());
            sched.request_redraw();
        }
        for t in input.touches {
            if let Some(c) = convert.as_mut() {
                if c.touch(t) == convert_screen::Action::Close {
                    convert = None;
                }
                continue;
            }
            if let Some(c) = clip.as_mut() {
                match c.touch(t) {
                    clip_screen::Action::None => {}
                    clip_screen::Action::Close => closed_clips.extend(clip.take()),
                    clip_screen::Action::Saved(name) => {
                        closed_clips.extend(clip.take());
                        if let Some(v) = viewer.as_mut() {
                            v.toast(format!("Saved as {name}"));
                        }
                    }
                    clip_screen::Action::Picked(range) => {
                        closed_clips.extend(clip.take());
                        if let Some(g) = gif.as_mut() {
                            g.set_range(range);
                        }
                    }
                }
                continue;
            }
            if let Some(g) = gif.as_mut() {
                match g.touch(t) {
                    gif_screen::Action::None => {}
                    gif_screen::Action::Close => gif = None,
                    gif_screen::Action::ChoosePart(range) => {
                        if let Some(v) = viewer.as_mut() {
                            v.release_video();
                            if let Some(it) = v.item_by_id(g.item_id()).cloned() {
                                clip = Some(clip_screen::ClipScreen::picker(&app, photos_density(&app), it, range));
                            }
                        }
                    }
                    gif_screen::Action::Done(name) => {
                        gif = None;
                        if let Some(v) = viewer.as_mut() {
                            v.toast(format!("Saved as {name}"));
                        }
                    }
                }
                continue;
            }
            if let Some(c) = crop.as_mut() {
                match c.touch(t) {
                    crop_screen::Action::None => {}
                    crop_screen::Action::Close => closed_crops.extend(crop.take()),
                    crop_screen::Action::Saved(name) => {
                        closed_crops.extend(crop.take());
                        if let Some(v) = viewer.as_mut() {
                            v.toast(format!("Saved as {name}"));
                        }
                    }
                    crop_screen::Action::SaveAs(cr) => {
                        let (it, full) = (c.item().clone(), c.full());
                        convert = Some(convert_screen::ConvertScreen::new(&app, photos_density(&app), vec![it], Some((cr, full)), 0));
                    }
                }
                continue;
            }
            if let Some(sv) = save_vid.as_mut() {
                match sv.touch(t) {
                    save_video::Action::None => {}
                    save_video::Action::Close => save_vid = None,
                    save_video::Action::Done(name) => {
                        save_vid = None;
                        if let Some(v) = viewer.as_mut() {
                            v.toast(format!("Saved as {name}"));
                        }
                    }
                }
                continue;
            }
            if let Some(v) = viewer.as_mut() {
                let act = v.touch(t);
                let item = |id: i64| photos.items().iter().find(|i| i.id == id).cloned();
                match act {
                    viewer::Action::None => {}
                    viewer::Action::Back => {
                        closed.extend(viewer.take());
                        if std::mem::take(&mut outside) {
                            open_with::leave(&app);
                        }
                    }
                    viewer::Action::Share(id) => {
                        if id == open_with::FOREIGN_ID {
                            actions::share(&app, &[(id, v.is_video())]);
                        } else if let Some(it) = item(id) {
                            actions::share(&app, &[(id, it.kind == gallery_core::media::Kind::Video)]);
                        }
                    }
                    viewer::Action::Delete(id) => {
                        if let Some(it) = item(id) {
                            trash(&app, system_bin, &[(id, it.kind == gallery_core::media::Kind::Video)]);
                        }
                    }
                    viewer::Action::Edit(id, kind) => {
                        log::info!("edit {kind:?}");
                        let found = v.item_by_id(id).cloned();
                        match (kind, found) {
                            (viewer::EditKind::Crop, Some(it)) => {
                                video::pause(&app);
                                crop = Some(crop_screen::CropScreen::new(&app, photos_density(&app), it));
                            }
                            (viewer::EditKind::MakeGif, Some(it)) => {
                                let at = v.video_pos();
                                video::pause(&app);
                                gif = Some(gif_screen::GifScreen::new(&app, photos_density(&app), it, at));
                            }
                            (viewer::EditKind::Clip, Some(it)) => {
                                v.release_video();
                                clip = Some(clip_screen::ClipScreen::new(&app, photos_density(&app), it));
                            }
                            (viewer::EditKind::GifToVideo, Some(it)) => {
                                save_vid = Some(save_video::SaveVideo::new(&app, photos_density(&app), it));
                            }
                            _ => v.toast("Couldn't find this item".into()),
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
                }
                continue;
            }
            if let Some((picker, chosen)) = move_pick.as_mut() {
                match picker.touch(t) {
                    albums_screen::Action::Photos => move_pick = None,
                    albums_screen::Action::Open(target) => {
                        let chosen = std::mem::take(chosen);
                        move_pick = None;
                        photos.end_selection();
                        if let Some(a) = album.as_mut() {
                            a.end_selection();
                        }
                        match begin_move(&app, system_bin, photos.items(), chosen, &target) {
                            Ok(Some(p)) => pending_move = Some(p),
                            Ok(None) => {}
                            Err(msg) => grid_toast = Some((Instant::now(), msg)),
                        }
                    }
                    _ => {}
                }
                continue;
            }
            if let Some(st) = settings.as_mut() {
                let mut s = gs.clone();
                match st.touch(t, &mut s) {
                    settings_screen::Action::None => {}
                    settings_screen::Action::Back => settings = None,
                    settings_screen::Action::Changed => {
                        log::info!("settings changed");
                        gs = s;
                        save_settings(&data_dir, &gs);
                        apply_settings(&gs, &mut photos, album.as_mut(), &mut albums);
                    }
                    settings_screen::Action::ManageMedia => {
                        actions::open_manage_media(&app);
                    }
                    settings_screen::Action::Update => holo_platform::update::tapped(&app),
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
                            grid.set_cols(gs.cols);
                            grid.set_hidden(gs.hidden.iter().copied().collect());
                            album = Some(grid);
                        }
                        albums_screen::Action::Settings => settings = Some(settings_screen::SettingsScreen::new(photos_density(&app), photos.items())),
                        albums_screen::Action::Hidden => {
                            let mut st = settings_screen::SettingsScreen::new(photos_density(&app), photos.items());
                            st.open_hidden();
                            settings = Some(st);
                        }
                    }
                    Action::None
                }
                (None, false) => photos.touch(t, library::access()),
            };
            match action {
                Action::None => {}
                Action::Open(i) => {
                    outside = false;
                    let grid = album.as_ref().unwrap_or(&photos);
                    viewer = Some(Viewer::open(&app, photos_density(&app), grid.items().clone(), grid.arranged().order.clone(), i as usize, screen_w));
                    viewer_source = ViewerSource::Grid;
                }
                Action::Albums => on_albums = true,
                Action::Settings => settings = Some(settings_screen::SettingsScreen::new(photos_density(&app), photos.items())),
                Action::Bin => bin = Some(open_bin(&app, system_bin, photos_density(&app))),
                Action::Share => {
                    let grid = album.as_ref().unwrap_or(&photos);
                    actions::share(&app, &grid.chosen());
                }
                Action::Convert => {
                    let grid = album.as_ref().unwrap_or(&photos);
                    let ids: std::collections::HashSet<i64> = grid.chosen().iter().map(|c| c.0).collect();
                    let (pictures, others): (Vec<_>, Vec<_>) = grid.items().iter().filter(|it| ids.contains(&it.id)).cloned().partition(|it| it.kind == gallery_core::media::Kind::Image);
                    if pictures.is_empty() {
                        grid_toast = Some((Instant::now(), "Only pictures can be converted".into()));
                    } else {
                        convert = Some(convert_screen::ConvertScreen::new(&app, photos_density(&app), pictures, None, others.len()));
                    }
                }
                Action::Move => {
                    let grid = album.as_ref().unwrap_or(&photos);
                    let ids: std::collections::HashSet<i64> = grid.chosen().iter().map(|c| c.0).collect();
                    let chosen: Vec<_> = grid.items().iter().filter(|it| ids.contains(&it.id)).cloned().collect();
                    let title = "Move to".to_string();
                    move_pick = Some((AlbumsScreen::picker(photos_density(&app), photos.items().clone(), title), chosen));
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
            }
        }
        if let Some(c) = convert.as_mut()
            && let convert_screen::Action::Done(msg) = c.poll()
        {
            convert = None;
            if let Some(cr) = crop.take() {
                closed_crops.push(cr);
            }
            photos.end_selection();
            if let Some(a) = album.as_mut() {
                a.end_selection();
            }
            match viewer.as_mut() {
                Some(v) => v.toast(msg),
                None => grid_toast = Some((Instant::now(), msg)),
            }
            sched.request_redraw();
        }
        if let Some(g) = gif.as_mut()
            && let gif_screen::Action::Done(name) = g.poll()
        {
            gif = None;
            if let Some(v) = viewer.as_mut() {
                v.toast(format!("Saved as {name}"));
            }
            sched.request_redraw();
        }
        if let Some(sv) = save_vid.as_mut()
            && let save_video::Action::Done(name) = sv.poll()
        {
            save_vid = None;
            if let Some(v) = viewer.as_mut() {
                v.toast(format!("Saved as {name}"));
            }
            sched.request_redraw();
        }
        if let Some(c) = clip.as_mut()
            && let clip_screen::Action::Saved(name) = c.poll()
        {
            closed_clips.extend(clip.take());
            if let Some(v) = viewer.as_mut() {
                v.toast(format!("Saved as {name}"));
            }
            sched.request_redraw();
        }
        if let Some(c) = crop.as_mut() {
            match c.poll() {
                crop_screen::Action::Saved(name) => {
                    closed_crops.extend(crop.take());
                    if let Some(v) = viewer.as_mut() {
                        v.toast(format!("Saved as {name}"));
                    }
                    sched.request_redraw();
                }
                crop_screen::Action::None => {}
                _ => {}
            }
        }
        if input.back {
            sched.request_redraw();
            if let Some(c) = convert.as_mut() {
                if !c.back() {
                    convert = None;
                }
            } else if let Some(c) = clip.as_mut() {
                if !c.back() {
                    closed_clips.extend(clip.take());
                }
            } else if let Some(g) = gif.as_mut() {
                if !g.back() {
                    gif = None;
                }
            } else if let Some(c) = crop.as_mut() {
                if !c.back() {
                    closed_crops.extend(crop.take());
                }
            } else if let Some(sv) = save_vid.as_mut() {
                if !sv.back() {
                    save_vid = None;
                }
            } else if move_pick.is_some() {
                move_pick = None;
            } else if let Some(v) = viewer.as_mut() {
                if !v.back() {
                    closed.extend(viewer.take());
                    if std::mem::take(&mut outside) {
                        open_with::leave(&app);
                    }
                }
            } else if let Some(st) = settings.as_mut() {
                if !st.back() {
                    settings = None;
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
                // Nothing left to close: step aside like Android 12+ apps
                // do, keeping the gallery as it was for next time.
                open_with::leave(&app);
            }
        }
        for (op, ok) in actions::take_done() {
            log::info!("media action {op:?}: {}", if ok { "done" } else { "not done" });
            sched.request_redraw();
            if op == actions::Op::Move {
                match pending_move.take() {
                    Some(p) if ok => mover::start(&app, p),
                    _ => {}
                }
                continue;
            }
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
        if let Some(msg) = mover::take_result() {
            grid_toast = Some((Instant::now(), msg));
            sched.request_redraw();
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
            if let Some(st) = settings.as_mut() {
                st.set_items(&fresh);
            }
            photos.set_items(fresh);
            viewer_list_stale = viewer.is_some() && viewer_source != ViewerSource::Foreign;
            library_seen = true;
            thumbs.clear_failures();
            sched.request_redraw();
        }
        if library::take_asked() {
            sched.request_redraw();
        }
        // The grid's sort menu changed the sort: remember it.
        if (photos.sort, photos.order) != (gs.sort, gs.order) {
            gs.sort = photos.sort;
            gs.order = photos.order;
            save_settings(&data_dir, &gs);
        }
        if std::mem::take(&mut manage_stale) && actions::sdk(&app) >= 31 {
            manage = Some(actions::can_manage_media(&app));
        }
        if holo_platform::update::take_changed() {
            sched.request_redraw();
        }
        if crop.as_ref().is_some_and(|c| c.wants_frames()) || clip.as_ref().is_some_and(|c| c.wants_frames()) || convert.as_ref().is_some_and(|c| c.wants_frames()) || gif.as_ref().is_some_and(|g| g.wants_frames()) || save_vid.as_ref().is_some_and(|s| s.wants_frames()) {
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
                for c in closed_crops.drain(..) {
                    c.free(g.gl());
                }
                for c in closed_clips.drain(..) {
                    c.free(g.gl());
                }
                if viewer.is_none() && crop.is_none() && clip.is_none() {
                    batch.rect(Rect::new(0.0, 0.0, w, h), Rgba::hex(BLACK), 0.0);
                }
                photos.set_density(d);
                albums.set_density(d);
                let (today, utc_offset_s) = system::local_today();
                let frame = photos::Frame { w, h, ins, today, utc_offset_s };
                let access = library::access();
                if let Some((h, since)) = want_open.take() {
                    let in_library = gallery_core::open_with::media_id(&h.uri).map(|(id, _)| id);
                    if in_library.is_some() && !library_seen && since.elapsed() < Duration::from_secs(2) {
                        want_open = Some((h, since));
                    } else {
                        closed.extend(viewer.take());
                        bin = None;
                        photos.refresh(&frame);
                        let order = &photos.arranged().order;
                        let at = in_library.and_then(|id| order.iter().position(|&i| photos.items().get(i as usize).is_some_and(|it| it.id == id)));
                        let pd = photos_density(&app);
                        viewer = Some(match at {
                            Some(at) => {
                                viewer_source = ViewerSource::All;
                                Viewer::open(&app, pd, photos.items().clone(), order.clone(), at, w)
                            }
                            None => {
                                viewer_source = ViewerSource::Foreign;
                                let mut v = Viewer::open(&app, pd, std::sync::Arc::new(vec![open_with::foreign_item(&h)]), vec![0], 0, w);
                                v.set_outside();
                                v
                            }
                        });
                        outside = true;
                    }
                }
                if std::mem::take(&mut viewer_list_stale) && let Some(v) = viewer.as_mut() {
                    let grid = match viewer_source {
                        ViewerSource::Grid => album.as_mut().unwrap_or(&mut photos),
                        _ => &mut photos,
                    };
                    grid.refresh(&frame);
                    if !v.set_list(grid.items().clone(), grid.arranged().order.clone()) {
                        closed.extend(viewer.take());
                    }
                }
                if viewer.is_none() {
                    // The box belongs to the GIF on screen; a save already
                    // running still finishes.
                    save_vid = None;
                }
                let (anim, missing) = if let Some(c) = convert.as_mut() {
                    c.draw(g.gl(), r, &mut batch, frame, now)
                } else if let Some(c) = clip.as_mut() {
                    c.draw_preview(g.gl(), frame, screen, now);
                    (c.draw(g.gl(), r, &mut batch, frame, now), Vec::new())
                } else if let Some(gs) = gif.as_mut() {
                    (gs.draw(g.gl(), r, &mut batch, frame, now), Vec::new())
                } else if let Some(c) = crop.as_mut() {
                    c.draw_picture(g.gl(), &jobs, frame, screen, now);
                    (c.draw(g.gl(), r, &mut batch, frame, now), Vec::new())
                } else if let Some(v) = viewer.as_mut() {
                    let pd = photo_draw.get_or_insert_with(|| photo_tex::PhotoDraw::new(g.gl()));
                    let mut anim = v.draw(g.gl(), r, &mut batch, pd, &thumbs, &jobs, frame, now);
                    if let Some(sv) = save_vid.as_mut() {
                        anim |= sv.draw(g.gl(), r, &mut batch, frame, now);
                    }
                    (anim, Vec::new())
                } else if let Some((picker, _)) = move_pick.as_mut() {
                    picker.set_items(photos.items().clone());
                    picker.draw(g.gl(), r, &mut batch, &thumbs, frame, now)
                } else if let Some(st) = settings.as_mut() {
                    st.draw(g.gl(), r, &mut batch, &thumbs, frame, &gs, manage, now)
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
                if viewer.is_none() && crop.is_none() && clip.is_none() && convert.is_none() && let Some((at, msg)) = grid_toast.as_ref() {
                    if now < *at + Duration::from_millis(2500) {
                        widgets::toast(g.gl(), r, &mut batch, msg, frame.w, frame.h - frame.ins.bottom - 64.0 * d, d);
                        moving = true;
                    } else {
                        grid_toast = None;
                    }
                }
                r.draw_layer(g.gl(), &batch, screen, viewer.is_none() && crop.is_none() && clip.is_none());
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

holo_platform::updater_jni!(Java_dev_launcherrs_gallery_Updater_onUpdate, holo_platform::waker::wake);

fn settings_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("settings.txt")
}

fn save_settings(data_dir: &std::path::Path, s: &GallerySettings) {
    let tmp = data_dir.join("settings.tmp");
    if std::fs::write(&tmp, s.to_text()).is_ok() {
        let _ = std::fs::rename(&tmp, settings_path(data_dir));
    }
}

/// Put the settings into effect on every screen that shows them.
fn apply_settings(s: &GallerySettings, photos: &mut Photos, album: Option<&mut Photos>, albums: &mut AlbumsScreen) {
    holo_platform::holo::set_accent(s.accent.rgb());
    let hidden: std::collections::HashSet<i64> = s.hidden.iter().copied().collect();
    photos.set_sort(s.sort, s.order);
    photos.set_cols(s.cols);
    photos.set_hidden(hidden.clone());
    if let Some(a) = album {
        a.set_cols(s.cols);
        a.set_hidden(hidden.clone());
    }
    albums.set_hidden(hidden);
}

/// Where the open viewer's list came from.
#[derive(Clone, Copy, PartialEq)]
enum ViewerSource {
    /// The grid on screen (all photos or an album).
    Grid,
    /// All photos, opened from another app.
    All,
    /// One picture from another app, outside the library.
    Foreign,
}

fn photos_density(app: &AndroidApp) -> f32 {
    app.config().density().unwrap_or(160) as f32 / 160.0
}

/// MOVE into `target`: items already there are left alone. Android 11+
/// asks the owner first (answer through `take_done`), so the move waits as
/// Ok(Some); older versions move straight away.
fn begin_move(app: &AndroidApp, system_bin: bool, all: &[gallery_core::media::MediaItem], chosen: Vec<gallery_core::media::MediaItem>, target: &gallery_core::albums::Album) -> Result<Option<mover::Pending>, String> {
    let gallery_core::arrange::Filter::Bucket(bucket) = target.filter else { return Ok(None) };
    let items: Vec<_> = chosen.into_iter().filter(|it| it.bucket_id != bucket).collect();
    if items.is_empty() {
        return Err(format!("Already in {}", target.name));
    }
    let cover_video = all.iter().find(|it| it.id == target.cover).is_some_and(|it| it.kind == gallery_core::media::Kind::Video);
    let rel = saving::rel_path(app, target.cover, cover_video);
    if rel.is_empty() {
        return Err("Couldn't find that folder".into());
    }
    let p = mover::Pending { items, rel, folder: target.name.clone() };
    if system_bin {
        let ids: Vec<(i64, bool)> = p.items.iter().map(|it| (it.id, it.kind == gallery_core::media::Kind::Video)).collect();
        actions::request(app, actions::Op::Move, &ids, false);
        Ok(Some(p))
    } else {
        mover::start(app, p);
        Ok(None)
    }
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
