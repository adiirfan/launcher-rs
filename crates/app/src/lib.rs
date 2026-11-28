//! The launcher's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod apps;
mod cache;
mod calendar;
mod contacts;
mod drawer;
mod folder;
mod home;
mod icons;
mod menu;
mod settings_pages;
mod settings_screen;
mod keyboard;
// Shared with the gallery; re-exported so `crate::render` and friends still work.
pub(crate) use holo_platform::{fonts, gl_surface, holo, jni_util, refresh, render, update, vsync};

holo_platform::updater_jni!(Java_dev_launcherrs_app_Updater_onUpdate, crate::keyboard::wake);
mod listener;
mod lookup;
mod system;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use android_activity::input::{Axis, InputEvent, KeyAction, Keycode, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use launcher_core::frame::FrameScheduler;
use launcher_core::geometry::Insets;
use launcher_core::ime::ImeState;
use launcher_core::refresh::{RateVote, RefreshPolicy};
use launcher_draw::batch::Batch;

use drawer::{Drawer, DrawerAction};
use gl_surface::GlSurface;
use home::{Action, Home, Touch, TouchKind};
use icons::Icons;
use render::Renderer;
use settings_screen::SettingsScreen;

const REPORT_EVERY: Duration = Duration::from_secs(5);
/// Work longer than this risks missing the screen's next refresh.
const SLOW: Duration = Duration::from_millis(8);
/// How long a frame stays "animating" after the pager says it's moving.
const ANIM_STEP: Duration = Duration::from_millis(20);
/// If the screen's refresh call never comes (window going away), draw anyway.
const VSYNC_FALLBACK: Duration = Duration::from_millis(50);

fn slow_step(what: &str, since: Instant) {
    let took = since.elapsed();
    if took > SLOW {
        log::info!("slow step: {what} {:.1} ms", took.as_secs_f64() * 1e3);
    }
}

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    let level = log_level();
    let logger = android_logger::AndroidLogger::new(
        android_logger::Config::default().with_max_level(level).with_tag("launcher"),
    );
    if log::set_logger(Box::leak(Box::new(QuietLogger(logger)))).is_ok() {
        log::set_max_level(level);
    }
    log::info!("android_main start, {:.0} ms after process start", ms_since_process_start());
    return_freed_memory_quickly();
    keyboard::init(&app);
    lookup::start(&app);

    let mut sched = FrameScheduler::new();
    let mut gfx: Option<GlSurface> = None;
    let mut renderer: Option<Renderer> = None;
    let mut batch = Batch::new();
    let store = cache::Store::open(&app);
    let mut home = Home::load(&app, store.clone());
    let mut icons = Icons::new(store);
    icons.start(&app);
    let mut drawer = Drawer::new(app.config().density().unwrap_or(160) as f32 / 160.0);
    let mut settings = SettingsScreen::new(app.config().density().unwrap_or(160) as f32 / 160.0);
    let mut first_frame = true;
    let mut surface_fresh = false;
    let mut last_insets: Option<Insets> = None;
    let mut needs_refresh = true;
    let mut visible = true;
    let mut minute = current_minute();
    let mut timings: Vec<Duration> = Vec::new();
    let mut frames: u32 = 0;
    let mut last_report = Instant::now();
    let mut quit = false;
    let mut policy = RefreshPolicy::new();
    let mut vote = RateVote::Idle;
    let frame_rate = refresh::FrameRate::new(&app);
    icons.set_frame_budget(Duration::from_nanos(frame_rate.period_ns() as u64 / 5));
    let vsync = vsync::Vsync::new();
    if vsync.is_none() {
        log::warn!("no refresh callbacks; drawing on a timer");
    }
    // When the current wait for a refresh began.
    let mut waiting_since: Option<Instant> = None;
    // Fonts and the settings screen's letters are ready (redone after a trim).
    let mut warmed = false;
    // Idle-time loading is still under way: come straight back for more.
    let mut background = false;
    // The refresh the last frame was drawn for, while something was moving.
    let mut moving_tick: Option<i64> = None;
    // Icons are still being put into the atlas ahead of time.
    let mut preloading = true;

    while !quit {
        let now = Instant::now();
        let mut timeout = REPORT_EVERY.saturating_sub(now.duration_since(last_report));
        if visible {
            timeout = timeout.min(until_next_minute());
        }
        let has_surface = gfx.as_ref().is_some_and(GlSurface::has_surface);
        let frame_wake = match &vsync {
            Some(v) if has_surface && sched.should_draw(now) => {
                v.request();
                let since = *waiting_since.get_or_insert(now);
                Some(VSYNC_FALLBACK.saturating_sub(now.duration_since(since)))
            }
            _ if has_surface => sched.next_wake(now),
            _ => None,
        };
        let timeout = [frame_wake, background.then_some(Duration::ZERO), policy.next_check(now), home.next_wake(now), drawer.next_wake(now)]
            .into_iter()
            .flatten()
            .fold(timeout, Duration::min);

        app.poll_events(Some(timeout), |event| {
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
                            surface_fresh = true;
                            sched.request_redraw();
                            // A new surface starts with no vote: re-apply ours.
                            frame_rate.apply(&window, vote);
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        if let Some(g) = gfx.as_mut() {
                            g.suspend();
                        }
                        surface_fresh = false;
                    }
                    MainEvent::Resume { .. } => {
                        visible = true;
                        drawer.shown_again();
                        needs_refresh = true;
                        drawer.recheck_permission();
                        let m = current_minute();
                        if m != minute {
                            minute = m;
                            home.update_clock(&app);
                        }
                        sched.request_redraw();
                    }
                    MainEvent::Pause => visible = false,
                    MainEvent::Stop => {
                        visible = false;
                        // Whatever app opened now covers us: put Home back
                        // behind it, ready for when the user returns.
                        if drawer.hidden(Instant::now()) {
                            keyboard::clear_text(&app);
                        }
                    }
                    MainEvent::RedrawNeeded { .. } => {
                        // Draw synchronously before returning to NativeActivity so
                        // WindowManager never transitions onto an empty surface.
                        let now = Instant::now();
                        let style = home.settings();
                        drawer.set_style(style.animations.factor(), style.glow);
                        if keyboard::take_home() {
                            handle_home_intent(&app, &mut home, &mut drawer, &mut settings, style.animations.factor(), surface_fresh, now);
                        }
                        let m = current_minute();
                        if m != minute {
                            minute = m;
                            home.update_clock(&app);
                        }
                        if gfx.as_ref().is_some_and(GlSurface::has_surface) {
                            waiting_since = None;
                            draw_frame(
                                &app, &mut gfx, &mut renderer, &mut batch, &mut home, &mut icons,
                                &mut drawer, &mut settings, &frame_rate, vote, &mut last_insets,
                                &mut first_frame, &mut surface_fresh, &mut timings, &mut frames,
                                None, &mut moving_tick, &mut sched, now,
                            );
                        } else {
                            sched.request_redraw();
                        }
                    }
                    MainEvent::WindowResized { .. }
                    | MainEvent::ContentRectChanged { .. }
                    | MainEvent::ConfigChanged { .. } => sched.request_redraw(),
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        let style = home.settings();
        drawer.set_style(style.animations.factor(), style.glow);
        let input = drain_input(&app);
        if !input.touches.is_empty() {
            policy.note_activity(Instant::now());
            sched.request_redraw();
        }
        let now = Instant::now();
        let speed = style.animations.factor();
        for t in input.touches {
            if settings.is_open() {
                settings.touch(t, &app, &mut home);
                continue;
            }
            if drawer.is_open() {
                let action = drawer.touch(t, home.apps().0);
                drawer_action(&app, &mut drawer, action, now);
                continue;
            }
            let action = home.touch(t);
            home_action(&app, &mut drawer, &mut settings, speed, action, now);
        }
        if let Some((key, icon, at)) = drawer.tick(now, home.apps().0) {
            // The list steps aside at once; the same finger carries the app on Home.
            drawer.close(now, false);
            keyboard::show(&app, false);
            keyboard::clear_text(&app);
            home.lift_from_list(&app, key, icon, at);
            sched.request_redraw();
        }
        if home.tick(&app, now) {
            sched.request_redraw();
        }
        if input.back {
            if settings.is_open() {
                settings.back(now, speed);
            } else if drawer.is_open() {
                drawer_action(&app, &mut drawer, DrawerAction::Close, now);
            } else {
                let action = home.back(now);
                home_action(&app, &mut drawer, &mut settings, speed, action, now);
            }
            sched.request_redraw();
        }
        if keyboard::take_home() {
            handle_home_intent(&app, &mut home, &mut drawer, &mut settings, speed, surface_fresh, now);
            sched.request_redraw();
        }
        if let (Some(g), Some(r)) = (gfx.as_ref().filter(|g| g.has_surface()), renderer.as_mut())
            && icons.drain_ready(g.gl(), r)
        {
            sched.request_redraw();
        }
        if update::take_changed() {
            sched.request_redraw();
        }
        if let Some(state) = keyboard::take() {
            on_text(&state);
            if drawer.is_open() {
                drawer.set_query(&state.text);
                sched.request_redraw();
            } else if home.renaming() {
                home.set_rename_text(&state.text);
                sched.request_redraw();
            }
        }
        if keyboard::take_action() {
            if drawer.is_open() {
                let action = drawer.first_match(home.apps().0);
                drawer_action(&app, &mut drawer, action, now);
            } else if home.finish_rename() {
                home_action(&app, &mut drawer, &mut settings, speed, home::Action::EndRename, now);
                sched.request_redraw();
            }
        }
        if let Some((code, granted)) = keyboard::take_permission() {
            log::info!("permission {code}: {granted}");
            match code {
                keyboard::CONTACTS => drawer.permission_answer(&app, granted),
                keyboard::CALENDAR => home.calendar_answer(&app, granted),
                _ => {}
            }
            sched.request_redraw();
        }

        // Android sends TRIM_MEMORY_UI_HIDDEN (20) on every normal app switch.
        // Only drop glyphs and warm caches under real background pressure (40+),
        // so returning to Home never has to re-rasterise text or re-read icons.
        if let Some(level) = keyboard::take_trim() {
            log::info!("trim memory level {level}, visible {visible}");
            if level >= 40 && !visible {
                drawer.trim();
                icons.trim();
                warmed = false;
                if let Some(r) = renderer.as_mut() {
                    r.trim_glyphs();
                }
                purge_freed_memory();
            } else if level >= 20 && !visible {
                if let Some(r) = renderer.as_mut() {
                    icons.trim_uploaded(r);
                }
                purge_freed_memory();
            }
        }

        let m = current_minute();
        if m != minute {
            minute = m;
            if home.update_clock(&app) {
                sched.request_redraw();
            }
            if m % 5 == 0 && visible && home.update_event(&app) {
                sched.request_redraw();
            }
        }
        if home.take_listener() {
            sched.request_redraw();
        }

        let now = Instant::now();
        // A refresh that came while nothing needed drawing is simply dropped.
        let tick = vsync.as_ref().and_then(|v| v.take_tick());
        let on_beat = surface_fresh
            || vsync.is_none()
            || tick.is_some()
            || waiting_since.is_some_and(|t| now.duration_since(t) >= VSYNC_FALLBACK);
        if sched.should_draw(now) && on_beat {
            if let Some(w) = waiting_since.map(|t| now.duration_since(t)).filter(|w| *w > Duration::from_millis(250)) {
                log::info!("slow wait: wanted a frame for {:.0} ms", w.as_secs_f64() * 1e3);
            }
            waiting_since = None;
            // Ask for the next refresh now, so a long frame can't lose it.
            if let Some(v) = vsync.as_ref().filter(|_| sched.is_animating()) {
                v.request();
            }
            draw_frame(
                &app, &mut gfx, &mut renderer, &mut batch, &mut home, &mut icons,
                &mut drawer, &mut settings, &frame_rate, vote, &mut last_insets,
                &mut first_frame, &mut surface_fresh, &mut timings, &mut frames,
                tick, &mut moving_tick, &mut sched, now,
            );
        }

        // Slow work goes after a frame is on screen, never before (including
        // while resuming from another app before InitWindow/RedrawNeeded draw).
        let frame_shown = !first_frame && !surface_fresh && gfx.as_ref().is_some_and(GlSurface::has_surface);
        if frame_shown {
            // Loading ahead, only while nothing moves and the launcher is in front.
            background = false;
            if visible && !sched.should_draw(now) && !sched.is_animating() {
                if !warmed
                    && let (Some(g), Some(r)) = (gfx.as_ref().filter(|g| g.has_surface()), renderer.as_mut())
                {
                    warmed = true;
                    let t = Instant::now();
                    r.load_fonts();
                    let screen = g.size();
                    let ins = insets(&app, screen, &mut last_insets);
                    let budget = r.set_glyph_budget(Duration::MAX);
                    settings.warm(g.gl(), r, &mut icons, &home, screen, ins, now);
                    r.set_glyph_budget(budget);
                    log::info!("fonts and settings text ready in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
                }
                let keys = home.apps().0.iter().map(|a| &a.key);
                background = icons.warm(keys, Duration::from_millis(4));
                icons.prefetch_missing(home.apps().0.iter().map(|a| &a.key));
                if !background
                    && let (Some(g), Some(r)) = (gfx.as_ref().filter(|g| g.has_surface()), renderer.as_mut())
                {
                    let keys = home.apps().0.iter().map(|a| &a.key);
                    let more = icons.preload(g.gl(), r, keys, Duration::from_millis(4));
                    icons::Icons::settle(g.gl());
                    // Done filling: hand back the memory the unpacking used.
                    if preloading && !more {
                        purge_freed_memory();
                    }
                    preloading = more;
                    background = more;
                }
            }
            let t = Instant::now();
            if drawer.after_frame(&app) {
                sched.request_redraw();
            }
            slow_step("list lookups", t);
            if needs_refresh {
                needs_refresh = false;
                if home.refresh(&app, &mut icons) {
                    sched.request_redraw();
                }
                if home.update_listener(&app) | home.update_event(&app) {
                    sched.request_redraw();
                }
                if home.update_clock(&app) {
                    sched.request_redraw();
                }
            }
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
        if now.duration_since(last_report) >= REPORT_EVERY {
            if frames > 0 {
                log::info!("frames drawn in last 5s: {frames}");
            }
            frames = 0;
            last_report = now;
        }
    }
    log::info!("android_main end");
}

#[allow(clippy::too_many_arguments)]
fn handle_home_intent(
    app: &AndroidApp,
    home: &mut Home,
    drawer: &mut Drawer,
    settings: &mut SettingsScreen,
    speed: f32,
    instant: bool,
    now: Instant,
) {
    if settings.is_open() {
        settings.close(now, if instant { 0.0 } else { speed });
    } else if drawer.is_open() {
        if instant {
            drawer.close(now, false);
            keyboard::show(app, false);
            keyboard::clear_text(app);
        } else {
            drawer_action(app, drawer, DrawerAction::Close, now);
        }
    } else {
        if home.renaming() {
            home_action(app, drawer, settings, speed, home::Action::EndRename, now);
        }
        if instant {
            home.jump_to_first_page(now);
        } else {
            home.go_to_first_page(now);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_frame(
    app: &AndroidApp,
    gfx: &mut Option<GlSurface>,
    renderer: &mut Option<Renderer>,
    batch: &mut Batch,
    home: &mut Home,
    icons: &mut Icons,
    drawer: &mut Drawer,
    settings: &mut SettingsScreen,
    frame_rate: &refresh::FrameRate,
    vote: RateVote,
    last_insets: &mut Option<Insets>,
    first_frame: &mut bool,
    surface_fresh: &mut bool,
    timings: &mut Vec<Duration>,
    frames: &mut u32,
    tick: Option<i64>,
    moving_tick: &mut Option<i64>,
    sched: &mut FrameScheduler,
    now: Instant,
) {
    let mut again = false;
    // The window changed size (folding, unfolding, rotating): build
    // the surface afresh. Drawing on into the old one leaves the
    // driver holding buffers of the old size as well as the new.
    if let (Some(g), Some(w)) = (gfx.as_mut(), app.native_window()) {
        if g.outgrown(&w) {
            if let Err(e) = g.resume(&w) {
                log::error!("GL surface rebuild failed: {e}");
            }
            *surface_fresh = true;
            frame_rate.apply(&w, vote);
        }
    }
    if let Some(g) = gfx.as_ref().filter(|g| g.has_surface()) {
        let r = renderer.get_or_insert_with(|| {
            let mut r = Renderer::new(g.gl(), render::RendererConfig::LAUNCHER);
            r.set_glyph_budget(Duration::from_nanos(frame_rate.period_ns() as u64 / 5));
            r
        });
        r.reserve_icons(g.gl(), home.apps().0.len() + 12, icons.size());
        icons.drain_ready(g.gl(), r);
        // On a brand-new or resumed surface, render every visible glyph and
        // cached icon in one go so Frame 0 never pops in text or grey boxes.
        let complete = *first_frame || *surface_fresh;
        let saved_glyph_budget = complete.then(|| r.set_glyph_budget(Duration::MAX));
        let saved_icon_budget = complete.then(|| icons.set_frame_budget(Duration::MAX));

        let t0 = Instant::now();
        r.begin_frame();
        batch.clear();
        let screen = g.size();
        let ins = insets(app, screen, last_insets);
        icons.begin_frame(r);
        let mut moving = false;
        if !drawer.covers_home(now) && !settings.covers_home(now) {
            moving |= home.draw(g.gl(), r, batch, icons, screen, ins, now);
        }
        moving |= drawer.draw(g.gl(), r, batch, icons, home.apps(), home.dots(), screen, ins, now);
        moving |= settings.draw(g.gl(), r, batch, icons, home, screen, ins, now);
        let built = t0.elapsed();
        r.draw(g.gl(), batch, screen);
        let submitted = t0.elapsed();

        if let Some(b) = saved_glyph_budget {
            r.set_glyph_budget(b);
        }
        if let Some(b) = saved_icon_budget {
            icons.set_frame_budget(b);
        }

        if submitted > SLOW {
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            log::info!(
                "slow frame: build {:.1} ms (icons {} in {:.1}, glyphs {:.1}, fonts {:.1}), submit {:.1} ms",
                ms(built), icons.decoded, ms(icons.spent), ms(r.spent_glyphs), ms(r.spent_fonts), ms(submitted - built)
            );
        }
        let swap_t = Instant::now();
        if let Err(e) = g.swap() {
            log::error!("swap failed: {e}");
        }
        if swap_t.elapsed() > Duration::from_millis(100) {
            log::info!("slow swap: {:.0} ms handing the frame to the screen", swap_t.elapsed().as_secs_f64() * 1e3);
        }
        // Something kept moving, yet a refresh went by with no frame.
        if let (Some(t), Some(prev)) = (tick, *moving_tick)
            && t - prev > frame_rate.period_ns() * 3 / 2
        {
            log::info!("slow beat: {:.1} ms between moving frames", (t - prev) as f64 / 1e6);
        }
        // Mid-movement and done later than one refresh after its
        // own: shown a refresh late. (The first frame after idle is
        // left out: on screens that slow right down when still, its
        // refresh time says little.)
        if let Some(t) = tick.filter(|_| moving_tick.is_some()) {
            let late = monotonic_ns(libc::CLOCK_MONOTONIC) - t;
            if late > frame_rate.period_ns() {
                log::info!("slow beat: frame done {:.1} ms after its refresh", late as f64 / 1e6);
            }
        }
        timings.push(t0.elapsed());
        if timings.len() == 10 {
            let worst = timings.iter().max().unwrap();
            let avg = timings.iter().sum::<Duration>() / 10;
            log::info!("frame time over 10: avg {:.2} ms, worst {:.2} ms",
                avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
            timings.clear();
        }
        if *first_frame {
            *first_frame = false;
            log::info!("first frame: {:.0} ms after process start", ms_since_process_start());
            purge_freed_memory();
        }
        *surface_fresh = false;
        if moving {
            sched.animate_until(now + ANIM_STEP);
        }
        again = r.wants_redraw() || icons.deferred();
        *frames += 1;
    }
    sched.frame_drawn(now);
    // After frame_drawn, which clears requests made before it.
    if again {
        sched.request_redraw();
    }
    // Still moving after this frame: the next one is due next refresh.
    *moving_tick = tick.filter(|_| sched.is_animating());
}

/// Space the status and navigation bars take, from the window's content
/// area. Before Android reports it, assume the usual 24 dp and 48 dp.
/// The content area also shrinks when the keyboard opens; a bottom edge
/// taller than any navigation bar is the keyboard, so the last real value
/// is kept instead and nothing on screen changes size.
fn insets(app: &AndroidApp, (w, h): (i32, i32), last: &mut Option<Insets>) -> Insets {
    let d = app.config().density().unwrap_or(160) as f32 / 160.0;
    let r = app.content_rect();
    let fallback = Insets { top: 24.0 * d, bottom: 48.0 * d, left: 0.0, right: 0.0 };
    if let Some([top, bottom, left, right]) = keyboard::bar_insets() {
        let mut ins = Insets { top: top as f32, bottom: bottom as f32, left: left as f32, right: right as f32 };
        // A bottom this tall is the on-screen keyboard, not a bar: the
        // layout must not shrink around it.
        if ins.bottom > 100.0 * d {
            ins.bottom = last.map_or(fallback.bottom, |l| l.bottom);
        }
        *last = Some(ins);
        return ins;
    }
    if r.right <= r.left || r.bottom <= r.top {
        return last.unwrap_or(fallback);
    }
    let mut ins = Insets {
        top: r.top as f32,
        bottom: (h - r.bottom).max(0) as f32,
        left: r.left as f32,
        right: (w - r.right).max(0) as f32,
    };
    if ins.bottom > 100.0 * d {
        ins.bottom = last.map_or(fallback.bottom, |l| l.bottom);
    }
    *last = Some(ins);
    ins
}

unsafe extern "C" {
    fn __system_property_get(name: *const libc::c_char, value: *mut libc::c_char) -> libc::c_int;
}

/// Drops one message: android-activity reports every refresh call as
/// "spurious", which would fill the log 60 times a second while moving.
struct QuietLogger(android_logger::AndroidLogger);

impl log::Log for QuietLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        self.0.enabled(m)
    }
    fn log(&self, r: &log::Record) {
        let spurious = r.target().starts_with("android_activity")
            && r.args().as_str().is_some_and(|s| s.starts_with("Spurious ALOOPER_POLL_CALLBACK"));
        if !spurious {
            self.0.log(r);
        }
    }
    fn flush(&self) {
        self.0.flush();
    }
}

/// Warnings only, unless `adb shell setprop debug.launcherrs.log info`
/// (or `debug`) asks for more. Read once at start.
fn log_level() -> log::LevelFilter {
    let mut value = [0u8; 92];
    let n = unsafe { __system_property_get(c"debug.launcherrs.log".as_ptr(), value.as_mut_ptr().cast()) };
    match &value[..n.clamp(0, 91) as usize] {
        b"info" => log::LevelFilter::Info,
        b"debug" => log::LevelFilter::Debug,
        _ => log::LevelFilter::Warn,
    }
}

fn current_minute() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 60)
}

fn until_next_minute() -> Duration {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    Duration::from_millis(60_000 - ms % 60_000 + 50)
}

fn monotonic_ns(clock: libc::clockid_t) -> i64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(clock, &mut ts) };
    ts.tv_sec * 1_000_000_000 + ts.tv_nsec
}

/// Time since this process started, from /proc (for start-up timing logs).
fn ms_since_process_start() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else { return -1.0 };
    // Field 22 (starttime, clock ticks since boot) comes after the ")" of the name.
    let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { return -1.0 };
    let Some(ticks) = rest.split_whitespace().nth(19).and_then(|v| v.parse::<f64>().ok()) else { return -1.0 };
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let now = monotonic_ns(libc::CLOCK_BOOTTIME) as f64 / 1e6;
    now - ticks / hz * 1e3
}

#[derive(Default)]
struct InputOut {
    touches: Vec<Touch>,
    back: bool,
}

/// Input must always be consumed or Android reports the app as frozen.
/// Touches keep the time Android stamped on them, so swipe speed is right
/// even when several arrive in one batch.
fn drain_input(app: &AndroidApp) -> InputOut {
    let mut out = InputOut::default();
    let Ok(mut iter) = app.input_events_iter() else { return out };
    let (now, mono_now) = (Instant::now(), monotonic_ns(libc::CLOCK_MONOTONIC));
    while iter.next(|event| match event {
        // A launcher ignores Back; letting it through would close us.
        InputEvent::KeyEvent(key) if key.key_code() == Keycode::Back => {
            if key.action() == KeyAction::Up {
                out.back = true;
            }
            InputStatus::Handled
        }
        InputEvent::MotionEvent(m) => {
            let kind = match m.action() {
                MotionAction::Down => Some(TouchKind::Down),
                MotionAction::Move => Some(TouchKind::Move),
                MotionAction::Up => Some(TouchKind::Up),
                MotionAction::Cancel => Some(TouchKind::Cancel),
                _ => None,
            };
            if let Some(kind) = kind {
                let at = |ns: i64| now.checked_sub(Duration::from_nanos((mono_now - ns).max(0) as u64)).unwrap_or(now);
                let p = m.pointer_at_index(0);
                // A move carries the touches Android gathered since the last
                // one; each counts for finger speed and smooth dragging.
                if kind == TouchKind::Move {
                    for h in p.history() {
                        let (x, y) = (h.axis_value(Axis::X), h.axis_value(Axis::Y));
                        out.touches.push(Touch { kind, x, y, t: at(h.event_time()) });
                    }
                }
                out.touches.push(Touch { kind, x: p.x(), y: p.y(), t: at(m.event_time()) });
            }
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }) {}
    out
}

/// Logs the shape of the typed text, never the text itself.
fn on_text(state: &ImeState) {
    let compose = state.compose.map_or("none".to_string(), |c| format!("{}..{}", c.0, c.1));
    log::info!("text: {} chars sel={}..{} compose={}", state.text.chars().count(), state.selection.0, state.selection.1, compose);
}

fn home_action(app: &AndroidApp, drawer: &mut Drawer, settings: &mut SettingsScreen, speed: f32, action: Action, now: Instant) {
    match action {
        Action::None => {}
        Action::Launch(key, at) => {
            let ok = apps::launch(app, &key, at);
            log::info!("launch {} -> {ok}", key.package);
        }
        Action::OpenNotifications => system::expand_notifications(app),
        Action::OpenAppList => {
            // Start from an empty box, whatever the keyboard last held.
            keyboard::clear_text(app);
            drawer.open(now, false);
        }
        Action::OpenSearch => {
            keyboard::clear_text(app);
            drawer.open(now, true);
            keyboard::show(app, true);
        }
        Action::Media(b) => listener::media_button(app, b),
        Action::OpenEvent(begin) => {
            let ok = system::start(app, "android.intent.action.VIEW", Some(&format!("content://com.android.calendar/time/{begin}")));
            log::info!("open calendar -> {ok}");
        }
        Action::StartShortcut(s) => {
            let ok = apps::start_shortcut(app, &s);
            log::info!("shortcut of {} -> {ok}", s.package);
        }
        Action::AppInfo(key) => {
            let ok = apps::app_info(app, &key);
            log::info!("app info {} -> {ok}", key.package);
        }
        Action::Uninstall(key) => {
            let ok = apps::uninstall(app, &key);
            log::info!("uninstall prompt {} -> {ok}", key.package);
        }
        Action::PickWallpaper => system::pick_wallpaper(app),
        Action::HomeSettings => settings.open(now, speed),
        Action::StartRename => {
            keyboard::clear_text(app);
            keyboard::show(app, true);
        }
        Action::EndRename => {
            keyboard::show(app, false);
            keyboard::clear_text(app);
        }
        Action::ShowKeyboard => keyboard::show(app, true),
    }
}

fn drawer_action(app: &AndroidApp, drawer: &mut Drawer, action: DrawerAction, now: Instant) {
    match action {
        DrawerAction::None => {}
        DrawerAction::Launch(key, at) => {
            let ok = apps::launch(app, &key, at);
            log::info!("launch {} -> {ok}", key.package);
            if ok {
                drawer.launched();
                keyboard::show(app, false);
            }
        }
        DrawerAction::Close => {
            drawer.close(now, true);
            keyboard::show(app, false);
            keyboard::clear_text(app);
        }
        DrawerAction::ShowKeyboard => keyboard::show(app, true),
        DrawerAction::ClearText => keyboard::clear_text(app),
        DrawerAction::OpenSettings(action) => {
            let ok = system::start(app, action, None);
            log::info!("settings page {action} -> {ok}");
        }
        DrawerAction::OpenContact(c) => {
            let ok = system::start(app, "android.intent.action.VIEW", Some(&c.lookup_uri));
            log::info!("open contact -> {ok}");
        }
        DrawerAction::AskContacts => keyboard::ask_contacts(app),
        DrawerAction::OpenAppDetails => {
            system::open_own_app_details(app);
        }
    }
}

unsafe extern "C" {
    fn mallopt(param: i32, value: i32) -> i32;
}

/// Ask the allocator to hand freed pages back about a second after they are
/// freed (bionic's M_DECAY_TIME, Android 9+), instead of keeping them.
fn return_freed_memory_quickly() {
    const M_DECAY_TIME: i32 = -100;
    let rc = unsafe { mallopt(M_DECAY_TIME, 1) };
    log::info!("allocator decay time 1 s -> {rc}");
}

/// Give memory freed during startup back to the system now (bionic's
/// M_PURGE; returns 1 where supported).
fn purge_freed_memory() {
    const M_PURGE: i32 = -101;
    let rc = unsafe { mallopt(M_PURGE, 0) };
    log::info!("purge freed memory -> {rc}");
}
