//! The launcher's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod gl_surface;
mod quad;
mod jni_util;
mod apps;
mod fonts;

use std::time::{Duration, Instant};

use android_activity::input::{
    ImeOptions, InputEvent, InputType, KeyAction, Keycode, MotionAction, TextInputAction,
    TextInputState,
};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use glow::HasContext;
use launcher_core::frame::FrameScheduler;

use gl_surface::GlSurface;
use apps::AppEntry;
use quad::Quads;

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
    let mut quads: Option<Quads> = None;
    let mut shown: Option<Shown> = None;
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

        let input = drain_input(&app);
        if let Some((x, y)) = input.tap {
            log::info!("tap at {x:.0},{y:.0}");
            if let Some(s) = shown.as_ref().filter(|s| s.contains(x, y)) {
                let ok = apps::launch(&app, &s.entry);
                log::info!("launch {} -> {ok}", s.entry.package);
            } else if y < KEYBOARD_TAP_ZONE {
                app.set_ime_editor_info(
                    InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_VARIATION_NORMAL,
                    TextInputAction::Search,
                    ImeOptions::IME_FLAG_NO_FULLSCREEN,
                );
                app.show_soft_input(true);
            }
        }
        if input.back {
            app.hide_soft_input(true);
        }
        if let Some(state) = input.text {
            on_text(&state);
            sched.request_redraw();
        }

        let now = Instant::now();
        if sched.should_draw(now) {
            if let Some(g) = gfx.as_ref().filter(|g| g.has_surface()) {
                let q = quads.get_or_insert_with(|| Quads::new(g.gl()));
                let s = shown.get_or_insert_with(|| Shown::load(&app, g.gl()));
                draw(g, q, s);
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

/// Taps in this many pixels from the top open the keyboard (risk check only).
const KEYBOARD_TAP_ZONE: f32 = 300.0;

#[derive(Default)]
struct InputOut {
    tap: Option<(f32, f32)>,
    back: bool,
    text: Option<TextInputState>,
}

/// Input must always be consumed or Android reports the app as frozen.
fn drain_input(app: &AndroidApp) -> InputOut {
    let mut out = InputOut::default();
    let Ok(mut iter) = app.input_events_iter() else { return out };
    while iter.next(|event| match event {
        // A launcher ignores Back; letting it through would close us.
        InputEvent::KeyEvent(key) if key.key_code() == Keycode::Back => {
            if key.action() == KeyAction::Up {
                log::info!("back ignored");
                out.back = true;
            }
            InputStatus::Handled
        }
        InputEvent::MotionEvent(m) => {
            if m.action() == MotionAction::Up {
                let p = m.pointer_at_index(0);
                out.tap = Some((p.x(), p.y()));
            }
            InputStatus::Handled
        }
        InputEvent::TextEvent(state) => {
            out.text = Some(state.clone());
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }) {}
    out
}

fn on_text(state: &TextInputState) {
    let compose = state
        .compose_region
        .map_or("none".to_string(), |c| format!("{}..{}", c.start, c.end));
    log::info!(
        "text: {:?} sel={}..{} compose={}",
        state.text, state.selection.start, state.selection.end, compose
    );
}

/// The one app shown during the risk check: its entry, icon and where it sits.
struct Shown {
    entry: AppEntry,
    icon: Option<glow::Texture>,
    rect: std::cell::Cell<[f32; 4]>,
}

impl Shown {
    fn load(app: &AndroidApp, gl: &glow::Context) -> Shown {
        let apps = apps::list_apps(app);
        log::info!("apps: {}", apps.len());
        for a in apps.iter().take(5) {
            log::info!("  {} ({}/{})", a.label, a.package, a.activity);
        }
        let entry = apps.into_iter().next().unwrap_or(AppEntry {
            label: "none".into(),
            package: "none".into(),
            activity: "none".into(),
        });
        let size = icon_px(app);
        let icon = apps::load_icon_rgba(app, &entry, size)
            .map(|(px, w, h)| quad::upload_rgba(gl, w, h, &px));
        Shown { entry, icon, rect: std::cell::Cell::new([0.0; 4]) }
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        let [rx, ry, rw, rh] = self.rect.get();
        x >= rx && x < rx + rw && y >= ry && y < ry + rh
    }
}

/// 56dp in pixels for this screen.
fn icon_px(app: &AndroidApp) -> i32 {
    let dpi = app.config().density().unwrap_or(160) as i32;
    56 * dpi / 160
}

fn draw(g: &GlSurface, q: &mut Quads, shown: &Shown) {
    let gl = g.gl();
    let (w, h) = g.size();
    unsafe {
        gl.viewport(0, 0, w, h);
        // Fully transparent: the phone's wallpaper shows through.
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
    }
    let side = 168.0_f32.max(1.0);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let rect = [cx - side / 2.0, cy - side / 2.0, side, side];
    shown.rect.set(rect);
    match shown.icon {
        Some(tex) => q.textured(gl, rect[0], rect[1], side, side, tex, (w, h)),
        // Placeholder when the icon can't be loaded.
        None => q.solid(gl, rect[0], rect[1], side, side, [0.45, 0.45, 0.45, 1.0], (w, h)),
    }
    if let Err(e) = g.swap() {
        log::error!("swap failed: {e}");
    }
}
