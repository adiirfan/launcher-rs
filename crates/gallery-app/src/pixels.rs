//! Pixel work on Android bitmaps, in place: the big pictures never get
//! copied between Java and Rust.

use std::ffi::c_void;

use jni::objects::JObject;
use jni::Env;

#[repr(C)]
#[derive(Default)]
struct BitmapInfo {
    width: u32,
    height: u32,
    stride: u32,
    format: i32,
    flags: u32,
}

const FORMAT_RGBA_8888: i32 = 1;

#[link(name = "jnigraphics")]
unsafe extern "C" {
    fn AndroidBitmap_getInfo(env: *mut c_void, bitmap: *mut c_void, info: *mut BitmapInfo) -> i32;
    fn AndroidBitmap_lockPixels(env: *mut c_void, bitmap: *mut c_void, addr: *mut *mut c_void) -> i32;
    fn AndroidBitmap_unlockPixels(env: *mut c_void, bitmap: *mut c_void) -> i32;
}

/// A bitmap's pixels, locked until dropped.
pub struct Locked<'a> {
    env: *mut c_void,
    obj: *mut c_void,
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    pub ptr: *mut u8,
    _life: std::marker::PhantomData<&'a ()>,
}

impl<'a> Locked<'a> {
    /// Lock an ARGB_8888 bitmap. None for any other kind.
    pub fn new(env: &mut Env, bitmap: &'a JObject) -> Option<Locked<'a>> {
        let raw_env = env.get_raw() as *mut c_void;
        let obj = bitmap.as_raw() as *mut c_void;
        let mut info = BitmapInfo::default();
        // Safety: a live local reference to a Bitmap and this thread's env.
        unsafe {
            if AndroidBitmap_getInfo(raw_env, obj, &mut info) != 0 || info.format != FORMAT_RGBA_8888 {
                return None;
            }
            let mut ptr = std::ptr::null_mut();
            if AndroidBitmap_lockPixels(raw_env, obj, &mut ptr) != 0 || ptr.is_null() {
                return None;
            }
            Some(Locked { env: raw_env, obj, w: info.width, h: info.height, stride: info.stride, ptr: ptr as *mut u8, _life: std::marker::PhantomData })
        }
    }
}

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        // Safety: locked in `new` on this thread.
        unsafe { AndroidBitmap_unlockPixels(self.env, self.obj) };
    }
}

/// An affine map from output pixel centres to source positions:
/// source = origin + ox * dx + oy * dy.
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    pub origin: (f32, f32),
    pub dx: (f32, f32),
    pub dy: (f32, f32),
}

/// Fill `out` by sampling `src` through `map`, smoothly (bilinear), on
/// several threads. Positions outside the source take the nearest edge.
pub fn resample(src: &Locked, out: &Locked, map: Affine) {
    let (sw, sh, ss) = (src.w as usize, src.h as usize, src.stride as usize);
    let (ow, oh, os) = (out.w as usize, out.h as usize, out.stride as usize);
    // Safety: both are locked for the whole call; rows are written by one thread each.
    let src_px = unsafe { std::slice::from_raw_parts(src.ptr as *const u8, ss * sh) };
    let out_addr = out.ptr as usize;
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let band = oh.div_ceil(threads);
    std::thread::scope(|scope| {
        for t in 0..threads {
            let rows = (t * band)..((t + 1) * band).min(oh);
            scope.spawn(move || {
                for oy in rows {
                    // Safety: row `oy` belongs to this thread only.
                    let row = unsafe { std::slice::from_raw_parts_mut((out_addr + oy * os) as *mut u8, ow * 4) };
                    let fy = oy as f32 + 0.5;
                    let mut sx = map.origin.0 + 0.5 * map.dx.0 + fy * map.dy.0;
                    let mut sy = map.origin.1 + 0.5 * map.dx.1 + fy * map.dy.1;
                    for px in row.chunks_exact_mut(4) {
                        sample(src_px, sw, sh, ss, sx - 0.5, sy - 0.5, px);
                        sx += map.dx.0;
                        sy += map.dx.1;
                    }
                }
            });
        }
    });
}

#[inline]
fn sample(src: &[u8], w: usize, h: usize, stride: usize, x: f32, y: f32, out: &mut [u8]) {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |xx: usize, yy: usize| &src[yy * stride + xx * 4..yy * stride + xx * 4 + 4];
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    for i in 0..4 {
        let top = a[i] as f32 + (b[i] as f32 - a[i] as f32) * fx;
        let bot = c[i] as f32 + (d[i] as f32 - c[i] as f32) * fx;
        out[i] = (top + (bot - top) * fy + 0.5) as u8;
    }
}
