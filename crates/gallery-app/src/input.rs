//! Touches from Android, with up to two fingers (for pinching).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use android_activity::input::{Axis, InputEvent, KeyAction, Keycode, MotionAction};
use android_activity::{AndroidApp, InputStatus};
use holo_platform::sys::monotonic_ns;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TouchKind {
    Down,
    Move,
    Up,
    Cancel,
    /// A second finger came down, or one of two lifted.
    SecondDown,
    SecondUp,
}

#[derive(Clone, Copy, Debug)]
pub struct Touch {
    pub kind: TouchKind,
    pub x: f32,
    pub y: f32,
    /// The second finger, while two are down.
    pub second: Option<(f32, f32)>,
    pub t: Instant,
}

#[derive(Default)]
pub struct Input {
    pub touches: Vec<Touch>,
    pub back: bool,
}

/// Android 13+ sends Back (the gesture or the button) to a callback in
/// Java rather than as a key press; it lands here.
static BACK: AtomicBool = AtomicBool::new(false);
static GATE: std::sync::Mutex<gallery_core::back::BackGate> = std::sync::Mutex::new(gallery_core::back::BackGate::new());

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onBack<'l>(_env: jni::EnvUnowned<'l>, _class: jni::objects::JClass<'l>) {
    BACK.store(true, Ordering::Release);
    holo_platform::waker::wake();
}

/// Input must always be consumed or Android reports the app as frozen.
/// Touches keep the time Android stamped on them, so swipe speed is right
/// even when several arrive in one batch.
pub fn drain(app: &AndroidApp) -> Input {
    let mut out = drain_raw(app);
    if out.back {
        let now_ms = (monotonic_ns(libc::CLOCK_MONOTONIC) / 1_000_000).max(0) as u64;
        out.back = GATE.lock().unwrap().pass(now_ms);
        if !out.back {
            log::info!("back: second copy of one press ignored");
        }
    }
    out
}

fn drain_raw(app: &AndroidApp) -> Input {
    let mut out = Input { back: BACK.swap(false, Ordering::AcqRel), ..Input::default() };
    let Ok(mut iter) = app.input_events_iter() else { return out };
    let (now, mono_now) = (Instant::now(), monotonic_ns(libc::CLOCK_MONOTONIC));
    let at = |ns: i64| now.checked_sub(Duration::from_nanos((mono_now - ns).max(0) as u64)).unwrap_or(now);
    while iter.next(|event| match event {
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
                MotionAction::PointerDown => Some(TouchKind::SecondDown),
                MotionAction::PointerUp => Some(TouchKind::SecondUp),
                _ => None,
            };
            if let Some(kind) = kind {
                let p = m.pointer_at_index(0);
                let second = (m.pointer_count() > 1).then(|| {
                    let q = m.pointer_at_index(1);
                    (q.x(), q.y())
                });
                // A one-finger move carries the touches Android gathered
                // since the last one; each counts for finger speed.
                if kind == TouchKind::Move && second.is_none() {
                    for h in p.history() {
                        let (x, y) = (h.axis_value(Axis::X), h.axis_value(Axis::Y));
                        out.touches.push(Touch { kind, x, y, second: None, t: at(h.event_time()) });
                    }
                }
                out.touches.push(Touch { kind, x: p.x(), y: p.y(), second, t: at(m.event_time()) });
            }
            InputStatus::Handled
        }
        _ => InputStatus::Unhandled,
    }) {}
    out
}
