//! The launcher's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod gl_surface;
mod render;
mod jni_util;
mod apps;
mod cache;
mod fonts;
mod home;
mod refresh;
mod keyboard;
mod system;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use android_activity::input::{InputEvent, KeyAction, Keycode, MotionAction};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use launcher_core::frame::FrameScheduler;
use launcher_core::geometry::Insets;
use launcher_core::ime::ImeState;
use launcher_core::refresh::{RateVote, RefreshPolicy};
use launcher_draw::batch::Batch;

use gl_surface::GlSurface;
use home::{Action, Home, Touch, TouchKind};
use render::Renderer;

const REPORT_EVERY: Duration = Duration::from_secs(5);
/// How long a frame stays "animating" after the pager says it's moving.
const ANIM_STEP: Duration = Duration::from_millis(20);

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("launcher"),
    );
    log::info!("android_main start, {:.0} ms after process start", ms_since_process_start());
    return_freed_memory_quickly();
    keyboard::init(&app);

    let mut sched = FrameScheduler::new();
    let mut gfx: Option<GlSurface> = None;
    let mut renderer: Option<Renderer> = None;
    let mut batch = Batch::new();
    let mut home = Home::load(&app);
    let mut first_frame = true;
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

    while !quit {
        let now = Instant::now();
        let mut timeout = REPORT_EVERY.saturating_sub(now.duration_since(last_report));
        if visible {
            timeout = timeout.min(until_next_minute());
        }
        let timeout = [sched.next_wake(now), policy.next_check(now)]
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
                            sched.request_redraw();
                            // A new surface starts with no vote: re-apply ours.
                            frame_rate.apply(&window, vote);
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        if let Some(g) = gfx.as_mut() {
                            g.suspend();
                        }
                    }
                    MainEvent::Resume { .. } => {
                        visible = true;
                        needs_refresh = true;
                        sched.request_redraw();
                    }
                    MainEvent::Pause => visible = false,
                    MainEvent::WindowResized { .. }
                    | MainEvent::RedrawNeeded { .. }
                    | MainEvent::ContentRectChanged { .. }
                    | MainEvent::ConfigChanged { .. } => sched.request_redraw(),
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        let input = drain_input(&app);
        if !input.touches.is_empty() {
            policy.note_activity(Instant::now());
            sched.request_redraw();
        }
        for t in input.touches {
            match home.touch(t) {
                Action::None => {}
                Action::Launch(key) => {
                    let ok = apps::launch(&app, &key);
                    log::info!("launch {} -> {ok}", key.package);
                }
                Action::OpenNotifications => system::expand_notifications(&app),
                Action::OpenAppList => log::info!("open app list (plan 4)"),
                Action::OpenSearch => log::info!("open search (plan 4)"),
            }
        }
        if input.back {
            keyboard::show(&app, false);
        }
        if let Some(state) = keyboard::take() {
            on_text(&state);
        }
        if keyboard::take_action() {
            log::info!("action");
        }

        let m = current_minute();
        if m != minute {
            minute = m;
            if home.update_date(&app) {
                sched.request_redraw();
            }
        }

        let now = Instant::now();
        if sched.should_draw(now) {
            if let Some(g) = gfx.as_ref().filter(|g| g.has_surface()) {
                let r = renderer.get_or_insert_with(|| {
                    Renderer::new(g.gl(), fonts::load_system_fonts(), fonts::load_system_fonts_bold())
                });
                let t0 = Instant::now();
                r.begin_frame();
                batch.clear();
                let screen = g.size();
                let moving = home.draw(g.gl(), r, &mut batch, screen, insets(&app, screen), now);
                r.draw(g.gl(), &batch, screen);
                if let Err(e) = g.swap() {
                    log::error!("swap failed: {e}");
                }
                timings.push(t0.elapsed());
                if timings.len() == 10 {
                    let worst = timings.iter().max().unwrap();
                    let avg = timings.iter().sum::<Duration>() / 10;
                    log::info!("frame time over 10: avg {:.2} ms, worst {:.2} ms",
                        avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
                    timings.clear();
                }
                if first_frame {
                    first_frame = false;
                    log::info!("first frame: {:.0} ms after process start", ms_since_process_start());
                    purge_freed_memory();
                }
                if moving {
                    sched.animate_until(now + ANIM_STEP);
                }
                if r.wants_redraw() {
                    sched.request_redraw();
                }
                frames += 1;
            }
            sched.frame_drawn(now);
        }

        // Slow work goes after a frame is on screen, never before.
        if !first_frame {
            if home.has_missing_icons() && home.load_missing_icons(&app) {
                sched.request_redraw();
            }
            if needs_refresh {
                needs_refresh = false;
                if home.refresh(&app) {
                    sched.request_redraw();
                }
                if home.update_date(&app) {
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

/// Space the status and navigation bars take, from the window's content
/// area. Before Android reports it, assume the usual 24 dp and 48 dp.
fn insets(app: &AndroidApp, (w, h): (i32, i32)) -> Insets {
    let r = app.content_rect();
    if r.right <= r.left || r.bottom <= r.top {
        let d = app.config().density().unwrap_or(160) as f32 / 160.0;
        return Insets { top: 24.0 * d, bottom: 48.0 * d, left: 0.0, right: 0.0 };
    }
    Insets {
        top: r.top as f32,
        bottom: (h - r.bottom).max(0) as f32,
        left: r.left as f32,
        right: (w - r.right).max(0) as f32,
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
                let p = m.pointer_at_index(0);
                let age = Duration::from_nanos((mono_now - m.event_time()).max(0) as u64);
                out.touches.push(Touch { kind, x: p.x(), y: p.y(), t: now.checked_sub(age).unwrap_or(now) });
            }
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }) {}
    out
}

fn on_text(state: &ImeState) {
    let compose = state.compose.map_or("none".to_string(), |c| format!("{}..{}", c.0, c.1));
    log::info!("text: {:?} sel={}..{} compose={}", state.text, state.selection.0, state.selection.1, compose);
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
