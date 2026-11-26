//! Saving a GIF as an MP4 video, on a worker thread. Rust reads the GIF
//! twice: once for how long each frame shows, then for the pictures, each
//! drawn onto the picture so far. Frames go to the phone's encoder at a
//! steady rate; a frame that shows longer is simply repeated.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use android_activity::AndroidApp;
use gallery_core::gif_video::{bit_rate, delay_ms, timing, video_frame, video_size, Quality};
use gallery_core::media::MediaItem;
use gallery_core::yuv::rgba_to_i420;
use holo_platform::jni_util::with_activity;
use jni::objects::{JByteArray, JObject, JString, JValue};
use jni::{jni_sig, jni_str};

use crate::gif_player;
use crate::gif_maker::Making;
use crate::saving;

/// Tiny GIFs are made up to this many times bigger (smallest first) until
/// the phone's encoder takes the size; many refuse under 96 pixels a side.
const MAX_SCALE: u32 = 16;

pub fn start(app: &AndroidApp, item: MediaItem, q: Quality) -> Arc<Making> {
    let making = Arc::new(Making { done: AtomicUsize::new(0), total: 1000, result: Mutex::new(None) });
    let (app, m) = (app.clone(), making.clone());
    let spawned = std::thread::Builder::new().name("gif-video".into()).spawn(move || {
        let t0 = std::time::Instant::now();
        let r = make(&app, &item, q, &m);
        if r.is_err() {
            let _ = with_activity(&app, |env, a| env.call_method(a, jni_str!("gifVideoAbort"), jni_sig!("()V"), &[]).map(|_| ()));
        }
        log::info!("gif video: {:.1} s: {r:?}", t0.elapsed().as_secs_f64());
        *m.result.lock().unwrap() = Some(r);
        holo_platform::waker::wake();
    });
    if let Err(e) = spawned {
        *making.result.lock().unwrap() = Some(Err(format!("Couldn't start: {e}")));
    }
    making
}

fn read_all(app: &AndroidApp, id: i64) -> Option<Vec<u8>> {
    with_activity(app, |env, a| {
        let o = env.call_method(a, jni_str!("readAll"), jni_sig!("(J)[B"), &[JValue::Long(id)])?.l()?;
        if o.is_null() {
            return Ok(None);
        }
        let b = env.cast_local::<JByteArray>(o)?;
        Ok(Some(env.convert_byte_array(&b)?))
    })
    .ok()
    .flatten()
}

/// How long each frame shows, without decoding the pictures.
fn delays(bytes: &[u8]) -> Result<Vec<u64>, gif::DecodingError> {
    let mut opts = gif::DecodeOptions::new();
    opts.skip_frame_decoding(true);
    let mut dec = opts.read_info(std::io::Cursor::new(bytes))?;
    let mut out = Vec::new();
    while let Some(f) = dec.next_frame_info()? {
        out.push(delay_ms(f.delay));
    }
    Ok(out)
}

fn put(app: &AndroidApp, yuv: Option<&[u8]>, pts_us: i64) -> bool {
    with_activity(app, |env, a| {
        let arr = match yuv {
            Some(y) => JObject::from(env.byte_array_from_slice(y)?),
            None => JObject::null(),
        };
        env.call_method(a, jni_str!("gifVideoPut"), jni_sig!("([BJ)Z"), &[JValue::Object(&arr), JValue::Long(pts_us)])?.z()
    })
    .unwrap_or(false)
}

fn make(app: &AndroidApp, item: &MediaItem, q: Quality, m: &Making) -> Result<String, String> {
    let bytes = read_all(app, item.id).ok_or("Couldn't read the GIF")?;
    let d = delays(&bytes).map_err(|e| format!("Couldn't read the GIF: {e}"))?;
    if d.is_empty() {
        return Err("The GIF has no pictures".into());
    }
    let t = timing(&d);
    let total: usize = t.counts.iter().map(|&c| c as usize).sum();
    m.done.store(0, Ordering::Release);
    let mut dec = gif_player::open(&bytes).map_err(|e| format!("Couldn't read the GIF: {e}"))?;
    let (gw, gh) = (dec.width() as usize, dec.height() as usize);
    if gw == 0 || gh == 0 {
        return Err("Couldn't read the GIF's size".into());
    }
    let fps = (1000 / t.step_ms).max(1) as u32;

    let copy = saving::begin(app, item.id, &item.name, false, "_video", "mp4", "video/mp4", item.when_taken(), true).ok_or("Couldn't make the new file")?;
    let mut chosen = None;
    for k in 1..=MAX_SCALE {
        let (vw, vh) = video_size(gw as u32, gh as u32, k);
        if k > 1 && vw.max(vh) > 1920 {
            break;
        }
        let fd = copy.open_fd("rw").ok_or("Couldn't write the new file")?;
        let rate = bit_rate(vw, vh, fps, q);
        let ok = with_activity(app, |env, a| {
            let args = [JValue::Int(fd), JValue::Int(vw as i32), JValue::Int(vh as i32), JValue::Int(fps as i32), JValue::Int(rate as i32)];
            env.call_method(a, jni_str!("gifVideoStart"), jni_sig!("(IIIII)Z"), &args)?.z()
        })
        .unwrap_or(false);
        if ok {
            log::info!("gif video: {gw}x{gh} GIF as {vw}x{vh}, {fps} fps, {rate} bits/s, {total} frames");
            chosen = Some((k as usize, vw as usize, vh as usize));
            break;
        }
    }
    let (k, vw, vh) = chosen.ok_or("The phone can't make a video this size")?;

    let mut canvas = vec![0u8; gw * gh * 4];
    let mut shown = 0usize;
    let mut i = 0usize;
    while let Some(f) = dec.read_next_frame().map_err(|e| format!("Couldn't read the GIF: {e}"))? {
        let saved = (f.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
        gif_player::draw(&mut canvas, gw as u32, f);
        let n = t.counts.get(i).copied().unwrap_or(0) as usize;
        if n > 0 {
            let yuv = rgba_to_i420(&video_frame(&canvas, gw, gh, k, vw, vh), vw, vh);
            for r in 0..n {
                let pts = (shown as u64 * t.step_ms * 1000) as i64;
                if !put(app, (r == 0).then_some(yuv.as_slice()), pts) {
                    return Err("Couldn't make the video".into());
                }
                shown += 1;
            }
            m.done.store(shown * 1000 / total.max(1), Ordering::Release); // a share of 1000
            holo_platform::waker::wake();
        }
        match f.dispose {
            gif::DisposalMethod::Background => gif_player::clear(&mut canvas, gw as u32, f),
            gif::DisposalMethod::Previous => {
                if let Some(s) = saved {
                    canvas = s;
                }
            }
            _ => {}
        }
        i += 1;
    }
    if shown == 0 {
        return Err("The GIF has no pictures".into());
    }
    let err = with_activity(app, |env, a| -> jni::errors::Result<Option<String>> {
        let o = env.call_method(a, jni_str!("gifVideoFinish"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
        if o.is_null() {
            return Ok(None);
        }
        let s = env.cast_local::<JString>(o)?;
        Ok(Some(s.mutf8_chars(env)?.into()))
    })
    .map_err(|e| format!("Couldn't save: {e}"))?;
    if let Some(msg) = err {
        return Err(msg);
    }
    let name = copy.name.clone();
    if copy.finish() { Ok(name) } else { Err("Couldn't finish the video".into()) }
}
