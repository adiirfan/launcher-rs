//! Saving a cropped, turned or straightened copy of a picture. Runs on a
//! worker thread. Only the part of the original the crop needs is read,
//! at full detail, so big photos don't run the phone out of memory.

use android_activity::AndroidApp;
use gallery_core::convert::{Format, SizeChoice};
use gallery_core::crop::{exif_orientation, Crop};
use gallery_core::media::MediaItem;
use holo_platform::jni_util::with_activity;
use jni::objects::{JObject, JValue};
use jni::{jni_sig, jni_str, Env};

use crate::pixels::{resample, Affine, Locked};
use crate::saving;

const JPEG: i32 = 0;
const PNG: i32 = 1;
const WEBP: i32 = 2;
const HEIC: i32 = 3;

/// How a copy is written: SAVE COPY keeps the original's format; SAVE AS…
/// (Convert) picks these.
#[derive(Clone, Copy, Debug)]
pub struct Output {
    pub format: Format,
    pub quality: u8,
    pub size: SizeChoice,
    pub keep_details: bool,
    pub no_location: bool,
}

/// The number Java's `Editing.write` takes for a format.
pub fn java_format(f: Format) -> i32 {
    match f {
        Format::Png => PNG,
        Format::Webp => WEBP,
        Format::Heic => HEIC,
        _ => JPEG,
    }
}

/// Biggest picture we'll make: 100 megapixels.
const MAX_PIXELS: u64 = 100_000_000;

/// The format a copy is saved in: same as the original where Android can
/// write it, JPEG otherwise.
fn format_for(mime: &str) -> (i32, &'static str, &'static str) {
    match mime {
        "image/png" => (PNG, "png", "image/png"),
        "image/webp" => (WEBP, "webp", "image/webp"),
        _ => (JPEG, "jpg", "image/jpeg"),
    }
}

/// Upright picture position → position in the file as stored.
fn to_raw(orientation: u16, raw: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    let (rw, rh) = raw;
    let (ux, uy) = p;
    match orientation {
        90 => (uy, rh - ux),
        180 => (rw - ux, rh - uy),
        270 => (rw - uy, ux),
        _ => (ux, uy),
    }
}

fn recycle(env: &mut Env, b: &JObject) {
    if !b.is_null() {
        let _ = env.call_method(b, jni_str!("recycle"), jni_sig!("()V"), &[]);
    }
}

/// Save the copy. `full` is the picture's upright size. Ok(new file name).
pub fn save(app: &AndroidApp, item: &MediaItem, crop: Crop, full: (u32, u32)) -> Result<String, String> {
    save_as(app, item, crop, full, None)
}

/// Save with `out`'s format and size (SAVE AS…), or as the original's
/// format at full size when `out` is None.
pub fn save_as(app: &AndroidApp, item: &MediaItem, crop: Crop, full: (u32, u32), out: Option<Output>) -> Result<String, String> {
    let (format, ext, mime) = match out {
        Some(o) => (java_format(o.format), o.format.ext(), o.format.mime()),
        None => format_for(&item.mime),
    };
    let suffix = if out.is_some() { "_conv" } else { "_crop" };
    let (quality, keep, no_loc) = out.map_or((95, true, false), |o| (o.quality as i32, o.keep_details, o.no_location));
    let taken = item.when_taken();
    if out.is_none() && crop.is_rotate_only() && format == JPEG {
        if crop.turns == 0 && !crop.flip {
            return Err("Nothing to save yet".into());
        }
        let copy = saving::begin(app, item.id, &item.name, false, "_crop", "jpg", "image/jpeg", taken, false).ok_or("Couldn't make the new file")?;
        let fd = copy.open_fd("rw").ok_or("Couldn't write the new file")?;
        let orientation = exif_orientation(item.orientation, crop.turns, crop.flip) as i32;
        let ok = with_activity(app, |env, a| env.call_method(a, jni_str!("editLossless"), jni_sig!("(JII)Z"), &[JValue::Long(item.id), JValue::Int(fd), JValue::Int(orientation)])?.z()).unwrap_or(false);
        let name = copy.name.clone();
        return if ok && copy.finish() { Ok(name) } else { Err("Couldn't save the copy".into()) };
    }

    let (w, h) = full;
    let (cw, ch) = crop.output_size(w, h);
    let (ow, oh) = out.map_or((cw, ch), |o| o.size.apply(cw, ch));
    // Output pixel → crop pixel.
    let (kx, ky) = (cw as f32 / ow as f32, ch as f32 / oh as f32);
    if ow as u64 * oh as u64 > MAX_PIXELS {
        return Err("Too big to save on this phone".into());
    }
    // The part of the upright picture the crop shows.
    let corners = [(0.0, 0.0), (cw as f32, 0.0), (0.0, ch as f32), (cw as f32, ch as f32)].map(|(x, y)| crop.source_point(x, y, w, h));
    let fold = |f: fn(f32, f32) -> f32, i: usize, start: f32| corners.iter().map(|c| if i == 0 { c.0 } else { c.1 }).fold(start, f);
    let (ux0, uy0) = ((fold(f32::min, 0, f32::MAX) - 2.0).max(0.0), (fold(f32::min, 1, f32::MAX) - 2.0).max(0.0));
    let (ux1, uy1) = ((fold(f32::max, 0, f32::MIN) + 2.0).min(w as f32), (fold(f32::max, 1, f32::MIN) + 2.0).min(h as f32));
    let sideways = item.orientation == 90 || item.orientation == 270;
    let raw = if sideways { (h as f32, w as f32) } else { (w as f32, h as f32) };
    let rc = [(ux0, uy0), (ux1, uy1)].map(|p| to_raw(item.orientation, raw, p));
    let (rx0, ry0) = (rc[0].0.min(rc[1].0).floor().max(0.0), rc[0].1.min(rc[1].1).floor().max(0.0));
    let (rx1, ry1) = (rc[0].0.max(rc[1].0).ceil().min(raw.0), rc[0].1.max(rc[1].1).ceil().min(raw.1));

    let copy_name = std::cell::RefCell::new(None::<saving::Copy>);
    let result = with_activity(app, |env, a| -> jni::errors::Result<Result<String, String>> {
        let args = [JValue::Long(item.id), JValue::Int(rx0 as i32), JValue::Int(ry0 as i32), JValue::Int((rx1 - rx0) as i32), JValue::Int((ry1 - ry0) as i32)];
        let mut src = env.call_method(a, jni_str!("editRegion"), jni_sig!("(JIIII)Landroid/graphics/Bitmap;"), &args)?.l()?;
        // Formats Android can't read in parts: the whole picture, upright.
        let (mut orientation, mut origin) = (item.orientation, (rx0, ry0));
        if src.is_null() {
            log::info!("crop: reading the whole picture");
            src = env.call_method(a, jni_str!("editWhole"), jni_sig!("(J)Landroid/graphics/Bitmap;"), &[JValue::Long(item.id)])?.l()?;
            if crate::actions::sdk(app) >= 28 {
                orientation = 0;
            }
            origin = (0.0, 0.0);
        }
        if src.is_null() {
            return Ok(Err("Couldn't read the picture".into()));
        }
        let dst = env.call_method(a, jni_str!("editBlank"), jni_sig!("(II)Landroid/graphics/Bitmap;"), &[JValue::Int(ow as i32), JValue::Int(oh as i32)])?.l()?;
        if dst.is_null() {
            recycle(env, &src);
            return Ok(Err("Not enough memory for a picture this big".into()));
        }
        {
            let (Some(s), Some(o)) = (Locked::new(env, &src), Locked::new(env, &dst)) else {
                return Ok(Err("Couldn't read the picture".into()));
            };
            let raw_now = if orientation == 90 || orientation == 270 { (h as f32, w as f32) } else { (w as f32, h as f32) };
            let at = |x: f32, y: f32| {
                let p = to_raw(orientation, raw_now, crop.source_point(x, y, w, h));
                (p.0 - origin.0, p.1 - origin.1)
            };
            let p0 = at(0.0, 0.0);
            let (px, py) = (at(1.0, 0.0), at(0.0, 1.0));
            let map = Affine { origin: p0, dx: ((px.0 - p0.0) * kx, (px.1 - p0.1) * kx), dy: ((py.0 - p0.0) * ky, (py.1 - p0.1) * ky) };
            let t0 = std::time::Instant::now();
            resample(&s, &o, map);
            log::info!("crop: {}x{} from {}x{} in {:.0} ms", o.w, o.h, s.w, s.h, t0.elapsed().as_secs_f64() * 1e3);
        }
        recycle(env, &src);
        let Some(copy) = saving::begin(app, item.id, &item.name, false, suffix, ext, mime, taken, false) else {
            recycle(env, &dst);
            return Ok(Err("Couldn't make the new file".into()));
        };
        let Some(fd) = copy.open_fd("rw") else {
            recycle(env, &dst);
            return Ok(Err("Couldn't write the new file".into()));
        };
        let wargs = [JValue::Long(item.id), JValue::Object(&dst), JValue::Int(format), JValue::Int(quality), JValue::Int(fd), JValue::Bool(keep), JValue::Bool(no_loc)];
        let ok = env.call_method(a, jni_str!("editWrite"), jni_sig!("(JLandroid/graphics/Bitmap;IIIZZ)Z"), &wargs)?.z()?;
        recycle(env, &dst);
        if !ok {
            return Ok(Err("Couldn't save the copy".into()));
        }
        let name = copy.name.clone();
        *copy_name.borrow_mut() = Some(copy);
        Ok(Ok(name))
    })
    .map_err(|e| format!("Couldn't save: {e}"))?;
    match (result, copy_name.into_inner()) {
        (Ok(name), Some(copy)) => {
            if copy.finish() { Ok(name) } else { Err("Couldn't finish the copy".into()) }
        }
        (Ok(_), None) => Err("Couldn't save the copy".into()),
        (Err(e), _) => Err(e),
    }
}
