//! Writing a GIF from part of a video, on a worker thread. The phone's
//! decoder reads the frames once, in order; Rust turns each upright at the
//! GIF's size, brings it down to 256 colours and writes only the part that
//! changed. Backwards and bounce play need the frames in reverse: the step
//! back from each frame is made during the same pass and kept compressed.

use std::io::{BufWriter, Write};
use std::os::fd::FromRawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use android_activity::AndroidApp;
use gallery_core::gif_plan::{GifPlan, Plays};
use gallery_core::media::MediaItem;
use gallery_core::quant::{changed_rect, reduce};
use gallery_core::yuv::Yuv420;
use holo_platform::jni_util::with_activity;
use jni::objects::{JByteArray, JIntArray, JValue};
use jni::{jni_sig, jni_str};

use crate::saving;

/// A GIF being made: how many frames are read, and how it ended.
pub struct Making {
    pub done: AtomicUsize,
    pub total: usize,
    pub result: Mutex<Option<Result<String, String>>>,
}

/// The video's size standing upright.
pub fn upright(item: &MediaItem) -> (u32, u32) {
    if item.orientation == 90 || item.orientation == 270 { (item.h, item.w) } else { (item.w, item.h) }
}

pub fn start(app: &AndroidApp, item: MediaItem, plan: GifPlan) -> Arc<Making> {
    let total = plan.frame_times().len();
    let making = Arc::new(Making { done: AtomicUsize::new(0), total, result: Mutex::new(None) });
    let (app, m) = (app.clone(), making.clone());
    let spawned = std::thread::Builder::new().name("gif".into()).spawn(move || {
        let t0 = std::time::Instant::now();
        let r = make(&app, &item, plan, &m);
        let _ = with_activity(&app, |env, a| env.call_method(a, jni_str!("gifDone"), jni_sig!("()V"), &[]).map(|_| ()));
        log::info!("gif: {} frames in {:.1} s: {r:?}", m.done.load(Ordering::Acquire), t0.elapsed().as_secs_f64());
        *m.result.lock().unwrap() = Some(r);
        holo_platform::waker::wake();
    });
    if let Err(e) = spawned {
        *making.result.lock().unwrap() = Some(Err(format!("Couldn't start: {e}")));
    }
    making
}

/// The decoder's frame showing at `ms`, upright, `w` x `h`, as RGBA.
fn read(app: &AndroidApp, ms: u64, w: u32, h: u32) -> Option<Vec<u8>> {
    with_activity(app, |env, a| {
        if !env.call_method(a, jni_str!("readerAdvance"), jni_sig!("(J)Z"), &[JValue::Long(ms as i64 * 1000)])?.z()? {
            return Ok(None);
        }
        let o = env.call_method(a, jni_str!("readerInfo"), jni_sig!("()[I"), &[])?.l()?;
        let arr = env.cast_local::<JIntArray>(o)?;
        let mut info = [0i32; 8];
        arr.get_region(env, 0, &mut info)?;
        let mut planes: [Vec<u8>; 3] = Default::default();
        for (i, p) in planes.iter_mut().enumerate() {
            let o = env.call_method(a, jni_str!("readerPlane"), jni_sig!("(I)[B"), &[JValue::Int(i as i32)])?.l()?;
            if o.is_null() {
                return Ok(None);
            }
            let b = env.cast_local::<JByteArray>(o)?;
            *p = env.convert_byte_array(&b)?;
        }
        let [fw, fh, left, top, y_row, uv_row, uv_pix, rotation] = info.map(|v| v.max(0) as usize);
        if fw == 0 || fh == 0 {
            return Ok(None);
        }
        let frame = Yuv420 { w: fw, h: fh, left, top, y: &planes[0], y_row, u: &planes[1], v: &planes[2], uv_row, uv_pix: uv_pix.max(1) };
        Ok(Some(frame.to_rgba(rotation as u32, w as usize, h as usize)))
    })
    .unwrap_or_else(|e| {
        log::warn!("gif: reading a frame failed: {e}");
        None
    })
}

/// The pixels inside `(x, y, w, h)` of a `stride`-wide RGBA frame.
fn sub_image(rgba: &[u8], stride: usize, (x, y, w, h): (usize, usize, usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        out.extend_from_slice(&rgba[(row * stride + x) * 4..(row * stride + x + w) * 4]);
    }
    out
}

/// One GIF frame showing `rgba`'s part `rect` (of a `stride`-wide frame).
fn frame(rgba: &[u8], stride: usize, rect: (usize, usize, usize, usize)) -> gif::Frame<'static> {
    let part = sub_image(rgba, stride, rect);
    let (palette, idx) = reduce(&part, rect.2, rect.3);
    gif::Frame {
        left: rect.0 as u16,
        top: rect.1 as u16,
        width: rect.2 as u16,
        height: rect.3 as u16,
        dispose: gif::DisposalMethod::Keep,
        palette: Some(palette),
        buffer: idx.into(),
        ..Default::default()
    }
}

fn make(app: &AndroidApp, item: &MediaItem, plan: GifPlan, m: &Making) -> Result<String, String> {
    let (vw, vh) = upright(item);
    if vw == 0 || vh == 0 {
        return Err("Couldn't read the video's size".into());
    }
    let (gw, gh) = plan.size(vw, vh);
    let (gw, gh) = (gw.min(u16::MAX as u32), gh.min(u16::MAX as u32));
    let (w, h) = (gw as usize, gh as usize);
    let times = plan.frame_times();
    let opened = with_activity(app, |env, a| env.call_method(a, jni_str!("readerOpen"), jni_sig!("(JJ)Z"), &[JValue::Long(item.id), JValue::Long(times[0] as i64 * 1000)])?.z()).unwrap_or(false);
    if !opened {
        return Err("Couldn't read the video".into());
    }
    let copy = saving::begin(app, item.id, &item.name, true, "_gif", "gif", "image/gif", item.when_taken(), false).ok_or("Couldn't make the new file")?;
    let fd = copy.open_fd("rw").ok_or("Couldn't write the new file")?;
    // Safety: `open_fd` hands us a descriptor nobody else owns.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut out = BufWriter::with_capacity(1 << 20, file);
    {
        let mut enc = gif::Encoder::new(&mut out, gw as u16, gh as u16, &[]).map_err(|e| format!("Couldn't start the GIF: {e}"))?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| e.to_string())?;
        let mut shown = 0usize;
        let mut put = |enc: &mut gif::Encoder<&mut BufWriter<std::fs::File>>, mut f: gif::Frame<'static>, pre: bool| -> Result<(), String> {
            f.delay = plan.delay_cs(shown);
            shown += 1;
            let r = if pre { enc.write_lzw_pre_encoded_frame(&f) } else { enc.write_frame(&f) };
            r.map_err(|e| format!("Couldn't write the GIF: {e}"))
        };
        let backwards = plan.plays != Plays::Loop;
        // backs[i]: the step from frame i back to frame i-1, compressed.
        let mut backs: Vec<Option<gif::Frame<'static>>> = Vec::new();
        let mut prev: Option<Vec<u8>> = None;
        let (mut t_read, mut t_rest) = (0f64, 0f64);
        for (i, &ms) in times.iter().enumerate() {
            let t0 = std::time::Instant::now();
            let rgba = read(app, ms, gw, gh).ok_or("Couldn't read the video")?;
            let t1 = std::time::Instant::now();
            t_read += (t1 - t0).as_secs_f64();
            let rect = match &prev {
                None => (0, 0, w, h),
                Some(p) => changed_rect(p, &rgba, w, h).unwrap_or((0, 0, 1, 1)),
            };
            if backwards {
                backs.push(prev.as_ref().map(|p| {
                    let mut f = frame(p, w, rect);
                    f.make_lzw_pre_encoded();
                    f
                }));
            }
            if plan.plays != Plays::Backwards {
                put(&mut enc, frame(&rgba, w, rect), false)?;
            }
            prev = Some(rgba);
            t_rest += t1.elapsed().as_secs_f64();
            m.done.store(i + 1, Ordering::Release);
            holo_platform::waker::wake();
        }
        let n = times.len();
        match plan.plays {
            Plays::Loop => {}
            // Back down to the second frame; the loop starts again at the first.
            Plays::Bounce => {
                for i in (2..n).rev() {
                    put(&mut enc, backs[i].take().expect("made above"), true)?;
                }
            }
            Plays::Backwards => {
                let last = prev.as_ref().expect("at least one frame");
                put(&mut enc, frame(last, w, (0, 0, w, h)), false)?;
                for i in (1..n).rev() {
                    put(&mut enc, backs[i].take().expect("made above"), true)?;
                }
            }
        }
        log::info!("gif: {gw}x{gh}, {n} frames read in {t_read:.1} s, colours and writing {t_rest:.1} s");
    }
    out.flush().map_err(|e| e.to_string())?;
    let file = out.into_inner().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    let name = copy.name.clone();
    if copy.finish() { Ok(name) } else { Err("Couldn't finish the GIF".into()) }
}
