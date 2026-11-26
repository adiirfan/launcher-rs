//! Video playback. Android's player decodes into a texture we create, so
//! the viewer draws video frames under its bars like any picture. Java
//! tells us when a frame is ready and when the player's state changes.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use android_activity::AndroidApp;
use glow::HasContext;
use holo_platform::jni_util::with_activity;
use jni::objects::{JClass, JFloatArray, JValue};
use jni::sys::jint;
use jni::{jni_sig, jni_str, EnvUnowned};

const TEXTURE_EXTERNAL_OES: u32 = 0x8D65;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// Ready to play; the picture's size before turning.
    Prepared(u32, u32),
    Completed,
    Error,
    /// Paused by Android (the app left the screen, a call came in).
    Paused,
}

static FRAME: AtomicBool = AtomicBool::new(false);
static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onVideoFrame<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>) {
    FRAME.store(true, Ordering::Release);
    holo_platform::waker::wake();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_launcherrs_gallery_MainActivity_onVideoState<'l>(_env: EnvUnowned<'l>, _class: JClass<'l>, state: jint, w: jint, h: jint) {
    let e = match state {
        1 => Event::Prepared(w.max(0) as u32, h.max(0) as u32),
        2 => Event::Completed,
        4 => Event::Paused,
        _ => {
            log::warn!("video error {w} {h}");
            Event::Error
        }
    };
    EVENTS.lock().unwrap().push_back(e);
    // Draw soon, so the viewer hears about it.
    FRAME.store(true, Ordering::Release);
    holo_platform::waker::wake();
}

/// A new frame is waiting (the loop should draw).
pub fn frame_waiting() -> bool {
    FRAME.load(Ordering::Acquire)
}

pub fn take_events() -> Vec<Event> {
    EVENTS.lock().unwrap().drain(..).collect()
}

fn call(app: &AndroidApp, what: &str, f: impl FnOnce(&mut jni::Env, &jni::objects::JObject) -> jni::errors::Result<()>) {
    if let Err(e) = with_activity(app, f) {
        log::warn!("video {what} failed: {e}");
    }
}

pub fn open(app: &AndroidApp, id: i64, tex: glow::Texture) {
    FRAME.store(false, Ordering::Release);
    EVENTS.lock().unwrap().clear();
    call(app, "open", |env, a| {
        env.call_method(a, jni_str!("videoOpen"), jni_sig!("(JI)V"), &[JValue::Long(id), JValue::Int(tex.0.get() as i32)])?;
        Ok(())
    });
}

/// Take the newest frame into the texture (drawing thread only). Returns
/// its texture-coordinate transform.
pub fn update(app: &AndroidApp) -> Option<[f32; 16]> {
    FRAME.store(false, Ordering::Release);
    let r = with_activity(app, |env, a| {
        let obj = env.call_method(a, jni_str!("videoUpdate"), jni_sig!("()[F"), &[])?.l()?;
        if obj.is_null() {
            return Ok(None);
        }
        let arr = env.cast_local::<JFloatArray>(obj)?;
        let mut m = [0f32; 16];
        arr.get_region(env, 0, &mut m)?;
        Ok(Some(m))
    });
    r.unwrap_or_else(|e| {
        log::warn!("video update failed: {e}");
        None
    })
}

pub fn play(app: &AndroidApp) {
    call(app, "play", |env, a| env.call_method(a, jni_str!("videoPlay"), jni_sig!("()V"), &[]).map(|_| ()));
}

pub fn pause(app: &AndroidApp) {
    call(app, "pause", |env, a| env.call_method(a, jni_str!("videoPause"), jni_sig!("()V"), &[]).map(|_| ()));
}

pub fn seek(app: &AndroidApp, ms: u64) {
    call(app, "seek", |env, a| env.call_method(a, jni_str!("videoSeek"), jni_sig!("(I)V"), &[JValue::Int(ms.min(i32::MAX as u64) as i32)]).map(|_| ()));
}

pub fn mute(app: &AndroidApp, muted: bool) {
    call(app, "mute", |env, a| env.call_method(a, jni_str!("videoMute"), jni_sig!("(Z)V"), &[JValue::Bool(muted)]).map(|_| ()));
}

pub fn close(app: &AndroidApp) {
    call(app, "close", |env, a| env.call_method(a, jni_str!("videoClose"), jni_sig!("()V"), &[]).map(|_| ()));
    FRAME.store(false, Ordering::Release);
}

/// Where playback is and how long the video is, in ms.
pub fn position(app: &AndroidApp) -> (u64, u64) {
    with_activity(app, |env, a| {
        let p = env.call_method(a, jni_str!("videoPosition"), jni_sig!("()I"), &[])?.i()?;
        let d = env.call_method(a, jni_str!("videoDuration"), jni_sig!("()I"), &[])?.i()?;
        Ok((p.max(0) as u64, d.max(0) as u64))
    })
    .unwrap_or((0, 0))
}

pub fn new_texture(gl: &glow::Context) -> Option<glow::Texture> {
    unsafe {
        let t = gl.create_texture().ok()?;
        gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(t));
        gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(TEXTURE_EXTERNAL_OES, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        Some(t)
    }
}

const VS3: &str = r#"#version 300 es
uniform vec2 u_screen;
uniform vec4 u_dest;
uniform mat4 u_tm;
out vec2 v_uv;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 p = u_dest.xy + c * u_dest.zw;
    v_uv = (u_tm * vec4(c.x, 1.0 - c.y, 0.0, 1.0)).xy;
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS3: &str = r#"#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision mediump float;
uniform samplerExternalOES u_tex;
in vec2 v_uv;
out vec4 o;
void main() { o = texture(u_tex, v_uv); }"#;

/// Older drivers only know the first version of the extension.
const VS1: &str = r#"#version 100
uniform vec2 u_screen;
uniform vec4 u_dest;
uniform mat4 u_tm;
attribute vec2 a_c;
varying vec2 v_uv;
void main() {
    vec2 p = u_dest.xy + a_c * u_dest.zw;
    v_uv = (u_tm * vec4(a_c.x, 1.0 - a_c.y, 0.0, 1.0)).xy;
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS1: &str = r#"#version 100
#extension GL_OES_EGL_image_external : require
precision mediump float;
uniform samplerExternalOES u_tex;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(u_tex, v_uv); }"#;

pub struct VideoDraw {
    program: glow::Program,
    vao: glow::VertexArray,
    /// The old-style program reads its corners from this buffer.
    corners: Option<glow::Buffer>,
    u_screen: Option<glow::UniformLocation>,
    u_dest: Option<glow::UniformLocation>,
    u_tm: Option<glow::UniformLocation>,
}

unsafe fn build(gl: &glow::Context, vs: &str, fs: &str) -> Option<glow::Program> {
    unsafe {
        let program = gl.create_program().ok()?;
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let sh = gl.create_shader(kind).ok()?;
            gl.shader_source(sh, src);
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                log::info!("video shader: {}", gl.get_shader_info_log(sh));
                gl.delete_program(program);
                return None;
            }
            gl.attach_shader(program, sh);
        }
        gl.bind_attrib_location(program, 0, "a_c");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            log::info!("video link: {}", gl.get_program_info_log(program));
            gl.delete_program(program);
            return None;
        }
        Some(program)
    }
}

impl VideoDraw {
    pub fn free(self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            if let Some(b) = self.corners {
                gl.delete_buffer(b);
            }
        }
    }

    pub fn new(gl: &glow::Context) -> Option<VideoDraw> {
        unsafe {
            let (program, old) = match build(gl, VS3, FS3) {
                Some(p) => (p, false),
                None => (build(gl, VS1, FS1)?, true),
            };
            let vao = gl.create_vertex_array().ok()?;
            let corners = if old {
                gl.bind_vertex_array(Some(vao));
                let b = gl.create_buffer().ok()?;
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(b));
                let pts: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck_f32(&pts), glow::STATIC_DRAW);
                gl.enable_vertex_attrib_array(0);
                gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
                gl.vertex_attrib_divisor(0, 0);
                Some(b)
            } else {
                None
            };
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 0);
            Some(VideoDraw {
                u_screen: gl.get_uniform_location(program, "u_screen"),
                u_dest: gl.get_uniform_location(program, "u_dest"),
                u_tm: gl.get_uniform_location(program, "u_tm"),
                program,
                vao,
                corners,
            })
        }
    }

    pub fn draw(&self, gl: &glow::Context, tex: glow::Texture, dest: (f32, f32, f32, f32), tm: &[f32; 16], screen: (i32, i32)) {
        let _ = self.corners;
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.uniform_2_f32(self.u_screen.as_ref(), screen.0 as f32, screen.1 as f32);
            gl.uniform_4_f32(self.u_dest.as_ref(), dest.0, dest.1, dest.2, dest.3);
            gl.uniform_matrix_4_f32_slice(self.u_tm.as_ref(), false, tm);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(TEXTURE_EXTERNAL_OES, Some(tex));
            gl.disable(glow::BLEND);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_texture(TEXTURE_EXTERNAL_OES, None);
        }
    }
}

fn bytemuck_f32(v: &[f32]) -> &[u8] {
    // Safety: f32 has no padding and u8 has no alignment needs.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(v)) }
}
