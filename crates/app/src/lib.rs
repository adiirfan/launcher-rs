//! The launcher's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod gl_surface;

use std::time::{Duration, Instant};

use android_activity::input::{InputEvent, KeyAction, Keycode};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use glow::HasContext;
use launcher_core::frame::FrameScheduler;

use gl_surface::GlSurface;

const REPORT_EVERY: Duration = Duration::from_secs(5);

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("launcher"),
    );
    log::info!("android_main start");

    let mut sched = FrameScheduler::new();
    let mut gfx: Option<GlSurface> = None;
    let mut frames: u32 = 0;
    let mut last_report = Instant::now();
    let mut quit = false;

    while !quit {
        let now = Instant::now();
        let to_report = REPORT_EVERY.saturating_sub(now.duration_since(last_report));
        let timeout = sched.next_wake(now).map_or(to_report, |w| w.min(to_report));

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
                        }
                    }
                    MainEvent::TerminateWindow { .. } => {
                        if let Some(g) = gfx.as_mut() {
                            g.suspend();
                        }
                    }
                    MainEvent::WindowResized { .. }
                    | MainEvent::RedrawNeeded { .. }
                    | MainEvent::ContentRectChanged { .. }
                    | MainEvent::ConfigChanged { .. }
                    | MainEvent::Resume { .. } => sched.request_redraw(),
                    MainEvent::Destroy => quit = true,
                    _ => {}
                }
            }
        });

        drain_input(&app);

        let now = Instant::now();
        if sched.should_draw(now) {
            if let Some(g) = gfx.as_ref().filter(|g| g.has_surface()) {
                draw(g);
                frames += 1;
            }
            sched.frame_drawn(now);
        }
        if now.duration_since(last_report) >= REPORT_EVERY {
            log::info!("frames drawn in last 5s: {frames}");
            frames = 0;
            last_report = now;
        }
    }
    log::info!("android_main end");
}

/// Input must always be consumed or Android reports the app as frozen.
fn drain_input(app: &AndroidApp) {
    let Ok(mut iter) = app.input_events_iter() else { return };
    while iter.next(|event| match event {
        // A launcher ignores Back; letting it through would close us.
        InputEvent::KeyEvent(key) if key.key_code() == Keycode::Back => {
            if key.action() == KeyAction::Up {
                log::info!("back ignored");
            }
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }) {}
}

fn draw(g: &GlSurface) {
    let gl = g.gl();
    let (w, h) = g.size();
    unsafe {
        gl.viewport(0, 0, w, h);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    if let Err(e) = g.swap() {
        log::error!("swap failed: {e}");
    }
}
