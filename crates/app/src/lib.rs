//! The launcher's Android entry point. Everything here is Android-only.
#![cfg(target_os = "android")]

mod gl_surface;
mod render;
mod jni_util;
mod apps;
mod fonts;
mod refresh;

use std::time::{Duration, Instant};

use android_activity::input::{
    ImeOptions, InputEvent, InputType, KeyAction, Keycode, MotionAction, TextInputAction,
    TextInputState,
};
use android_activity::{AndroidApp, InputStatus, MainEvent, PollEvent};
use launcher_core::frame::FrameScheduler;
use launcher_core::refresh::{RateVote, RefreshPolicy};

use gl_surface::GlSurface;
use apps::AppEntry;
use launcher_draw::batch::Batch;
use launcher_draw::{Rect, Rgba};
use render::Renderer;

const REPORT_EVERY: Duration = Duration::from_secs(5);

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("launcher"),
    );
    log::info!("android_main start");
    return_freed_memory_quickly();

    let mut sched = FrameScheduler::new();
    let mut gfx: Option<GlSurface> = None;
    let mut renderer: Option<Renderer> = None;
    let mut batch = Batch::new();
    let mut scene: Option<Scene> = None;
    let mut timings: Vec<Duration> = Vec::new();
    let mut frames: u32 = 0;
    let mut last_report = Instant::now();
    let mut quit = false;
    let mut policy = RefreshPolicy::new();
    let mut vote = RateVote::Idle;
    let frame_rate = refresh::FrameRate::new(&app);

    while !quit {
        let now = Instant::now();
        let to_report = REPORT_EVERY.saturating_sub(now.duration_since(last_report));
        let timeout = [sched.next_wake(now), policy.next_check(now)]
            .into_iter()
            .flatten()
            .fold(to_report, Duration::min);

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
        if input.touched {
            policy.note_activity(Instant::now());
        }
        if let Some((x, y)) = input.tap {
            log::info!("tap at {x:.0},{y:.0}");
            if let Some(entry) = scene.as_ref().and_then(|s| s.hit(x, y)) {
                let ok = apps::launch(&app, entry);
                log::info!("launch {} -> {ok}", entry.package);
            } else if y > KEYBOARD_TAP_ZONE {
                // Empty space: force a redraw (used to time frames).
                sched.request_redraw();
            } else {
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
                let r = renderer.get_or_insert_with(|| {
                    Renderer::new(g.gl(), fonts::load_system_fonts(), fonts::load_system_fonts_bold())
                });
                let first = scene.is_none();
                let s = scene.get_or_insert_with(|| Scene::load(&app));
                let t0 = Instant::now();
                draw(g, r, &mut batch, s);
                timings.push(t0.elapsed());
                if timings.len() == 10 {
                    let worst = timings.iter().max().unwrap();
                    let avg = timings.iter().sum::<Duration>() / 10;
                    log::info!("frame time over 10: avg {:.2} ms, worst {:.2} ms",
                        avg.as_secs_f64() * 1e3, worst.as_secs_f64() * 1e3);
                    timings.clear();
                }
                if first {
                    purge_freed_memory();
                }
                if r.wants_redraw() {
                    sched.request_redraw();
                }
                frames += 1;
            }
            sched.frame_drawn(now);
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
    /// Any finger down, moving or lifted.
    touched: bool,
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
            out.touched = true;
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

/// The drawing-kit check scene: shapes, text and the first 16 apps.
struct Scene {
    apps: Vec<(AppEntry, Option<(Vec<u8>, i32, i32)>)>,
    density: f32,
    hits: std::cell::RefCell<Vec<(Rect, usize)>>,
}

impl Scene {
    fn load(app: &AndroidApp) -> Scene {
        let all = apps::list_apps(app);
        log::info!("apps: {}", all.len());
        let size = icon_px(app);
        let apps = all
            .into_iter()
            .take(16)
            .map(|e| {
                let icon = apps::load_icon_rgba(app, &e, size);
                (e, icon)
            })
            .collect();
        let density = app.config().density().unwrap_or(160) as f32 / 160.0;
        Scene { apps, density, hits: Default::default() }
    }

    fn hit(&self, x: f32, y: f32) -> Option<&AppEntry> {
        let hits = self.hits.borrow();
        hits.iter().find(|(r, _)| r.contains(x, y)).map(|&(_, i)| &self.apps[i].0)
    }
}

/// 56dp in pixels for this screen.
fn icon_px(app: &AndroidApp) -> i32 {
    let dpi = app.config().density().unwrap_or(160) as i32;
    56 * dpi / 160
}

fn draw(g: &GlSurface, r: &mut Renderer, batch: &mut Batch, scene: &Scene) {
    let gl = g.gl();
    let (w, h) = g.size();
    let (wf, d) = (w as f32, scene.density);
    let dp = |v: f32| v * d;
    r.begin_frame();
    batch.clear();

    let white = Rgba::hex(0xFFFFFF);
    let holo = Rgba::hex(0x33B5E5);

    // Panel with title, divider, outline pill, circle and glow.
    let panel = Rect::new(dp(16.0), dp(40.0), wf - dp(32.0), dp(190.0));
    batch.rect(panel, Rgba::hex(0x101917).with_alpha(0.92), dp(24.0));
    r.text(gl, batch, "Drawing kit ✓", panel.x + dp(20.0), panel.y + dp(40.0), dp(20.0), panel.w - dp(40.0), white, true);
    batch.line(panel.x + dp(20.0), panel.y + dp(58.0), panel.x + panel.w - dp(20.0), panel.y + dp(58.0), 1.0, white.with_alpha(0.2));
    let pill = Rect::new(panel.x + dp(20.0), panel.y + dp(80.0), dp(150.0), dp(52.0));
    batch.outline(pill, Rgba::hex(0x9CC9B8), dp(26.0), dp(2.0));
    r.text(gl, batch, "Outline", pill.x + dp(24.0), pill.y + dp(32.0), dp(16.0), dp(110.0), Rgba::hex(0x9CC9B8), false);
    batch.circle(pill.x + pill.w + dp(44.0), pill.y + pill.h / 2.0, dp(22.0), Rgba::hex(0xFF8800));
    let gc = (panel.x + panel.w - dp(60.0), pill.y + pill.h / 2.0);
    let gr = Rect::new(gc.0 - dp(22.0), gc.1 - dp(22.0), dp(44.0), dp(44.0));
    batch.glow(gr, holo.with_alpha(0.8), dp(22.0), dp(14.0));
    batch.circle(gc.0, gc.1, dp(22.0), holo);

    // A long mixed-script line that has to be cut.
    r.text(gl, batch, "設定 Settings 日本語テスト — a very long label that must be cut",
        dp(16.0), dp(262.0), dp(14.0), wf - dp(32.0), white, false);

    // 4-column grid of real apps, round icons, labels underneath.
    let cell_w = (wf - dp(16.0)) / 4.0;
    let icon = dp(56.0);
    let mut hits = scene.hits.borrow_mut();
    hits.clear();
    for (i, (entry, px)) in scene.apps.iter().enumerate() {
        let (col, row) = ((i % 4) as f32, (i / 4) as f32);
        let x = dp(8.0) + col * cell_w;
        let y = dp(290.0) + row * dp(100.0);
        let ir = Rect::new(x + (cell_w - icon) / 2.0, y, icon, icon);
        let uv = px.as_ref().and_then(|(p, iw, ih)| r.icon(gl, &entry.package, p, *iw as u32, *ih as u32));
        match uv {
            Some(uv) => batch.image(ir, uv, icon / 2.0, 1.0),
            None => batch.circle(ir.x + icon / 2.0, ir.y + icon / 2.0, icon / 2.0, Rgba::hex(0x737373)),
        }
        // Centre the label: measure first, then place.
        let max = cell_w - dp(8.0);
        let mut probe = Batch::new();
        let lw = r.text(gl, &mut probe, &entry.label, 0.0, 0.0, dp(12.0), max, white, false);
        r.text(gl, batch, &entry.label, x + (cell_w - lw) / 2.0, y + icon + dp(20.0), dp(12.0), max, white, false);
        hits.push((ir, i));
    }

    r.draw(gl, batch, (w, h));
    if let Err(e) = g.swap() {
        log::error!("swap failed: {e}");
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
