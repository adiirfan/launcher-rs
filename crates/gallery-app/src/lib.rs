//! Holo Gallery's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod bars;
mod chrome;
mod input;
mod system;

use std::time::{Duration, Instant};

use android_activity::{AndroidApp, MainEvent, PollEvent};
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
        if input.back {
            system::finish(&app);
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
                batch.rect(Rect::new(0.0, 0.0, w, h), Rgba::hex(BLACK), 0.0);
                chrome::action_bar(g.gl(), r, &mut batch, w, ins.top, "Gallery", d);
                r.draw(g.gl(), &batch, screen);
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
                again = r.wants_redraw();
            }
            if moving {
                sched.animate_until(now + ANIM_STEP);
            }
            sched.frame_drawn(now);
            if again {
                sched.request_redraw();
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
    }
    log::info!("android_main end");
}
