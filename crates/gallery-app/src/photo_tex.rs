//! Big pictures for the viewer. They are far too large for the shared
//! picture atlas, so each gets its own texture, drawn with a one-quad
//! program before the bars and buttons go on top.

use glow::HasContext;

const VS: &str = r#"#version 300 es
uniform vec2 u_screen;
uniform vec4 u_dest;
uniform vec4 u_uv;
out vec2 v_uv;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 p = u_dest.xy + c * u_dest.zw;
    v_uv = mix(u_uv.xy, u_uv.zw, c);
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS: &str = r#"#version 300 es
precision mediump float;
uniform sampler2D u_tex;
in vec2 v_uv;
out vec4 o;
void main() { o = texture(u_tex, v_uv); }"#;

/// One picture on the graphics chip.
pub struct Tex {
    tex: glow::Texture,
    pub w: u32,
    pub h: u32,
}

impl Tex {
    /// Upload premultiplied RGBA, `w`×`h`.
    pub fn upload(gl: &glow::Context, w: u32, h: u32, rgba: &[u8]) -> Option<Tex> {
        if rgba.len() < (w * h * 4) as usize || w == 0 || h == 0 {
            return None;
        }
        unsafe {
            let tex = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w as i32, h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba[..(w * h * 4) as usize])));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            Some(Tex { tex, w, h })
        }
    }

    /// Replace the pixels of a texture the same size (the next GIF frame).
    pub fn update(&self, gl: &glow::Context, rgba: &[u8]) {
        if rgba.len() < (self.w * self.h * 4) as usize {
            return;
        }
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, self.w as i32, self.h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&rgba[..(self.w * self.h * 4) as usize])));
        }
    }

    pub fn free(self, gl: &glow::Context) {
        unsafe { gl.delete_texture(self.tex) };
    }

}

/// A picture going onto the graphics chip a few rows per frame, so a big
/// one never freezes the screen (a 20 MB copy took 77 ms on the XZ2).
pub struct Upload {
    tex: Tex,
    rgba: Vec<u8>,
    row: u32,
}

/// Rows copied between clock checks.
const ROWS: u32 = 32;

impl Upload {
    pub fn start(gl: &glow::Context, w: u32, h: u32, rgba: Vec<u8>) -> Option<Upload> {
        if rgba.len() < (w * h * 4) as usize || w == 0 || h == 0 {
            return None;
        }
        unsafe {
            let tex = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w as i32, h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            Some(Upload { tex: Tex { tex, w, h }, rgba, row: 0 })
        }
    }

    /// Copy rows until `until`. The finished picture when all are in.
    pub fn step(mut self, gl: &glow::Context, until: std::time::Instant) -> Result<Tex, Upload> {
        let (w, h) = (self.tex.w, self.tex.h);
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            while self.row < h {
                let n = ROWS.min(h - self.row);
                let from = (self.row * w * 4) as usize;
                let to = from + (n * w * 4) as usize;
                gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, self.row as i32, w as i32, n as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&self.rgba[from..to])));
                self.row += n;
                if std::time::Instant::now() >= until {
                    break;
                }
            }
        }
        if self.row >= h { Ok(self.tex) } else { Err(self) }
    }

    pub fn cancel(self, gl: &glow::Context) {
        self.tex.free(gl);
    }
}

pub struct PhotoDraw {
    program: glow::Program,
    vao: glow::VertexArray,
    u_screen: Option<glow::UniformLocation>,
    u_dest: Option<glow::UniformLocation>,
    u_uv: Option<glow::UniformLocation>,
}

impl PhotoDraw {
    pub fn new(gl: &glow::Context) -> PhotoDraw {
        unsafe {
            let program = gl.create_program().expect("program");
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(kind).expect("shader");
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    log::error!("photo shader: {}", gl.get_shader_info_log(sh));
                }
                gl.attach_shader(program, sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                log::error!("photo link: {}", gl.get_program_info_log(program));
            }
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 0);
            PhotoDraw {
                u_screen: gl.get_uniform_location(program, "u_screen"),
                u_dest: gl.get_uniform_location(program, "u_dest"),
                u_uv: gl.get_uniform_location(program, "u_uv"),
                vao: gl.create_vertex_array().expect("vao"),
                program,
            }
        }
    }

    /// Clear the screen to black, ready for pictures.
    pub fn clear(gl: &glow::Context, screen: (i32, i32)) {
        unsafe {
            gl.viewport(0, 0, screen.0, screen.1);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    /// Draw `t`'s part `uv` (u0, v0, u1, v1 in 0..1) at screen `dest` (x, y, w, h).
    pub fn draw(&self, gl: &glow::Context, t: &Tex, dest: (f32, f32, f32, f32), uv: (f32, f32, f32, f32), screen: (i32, i32)) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.uniform_2_f32(self.u_screen.as_ref(), screen.0 as f32, screen.1 as f32);
            gl.uniform_4_f32(self.u_dest.as_ref(), dest.0, dest.1, dest.2, dest.3);
            gl.uniform_4_f32(self.u_uv.as_ref(), uv.0, uv.1, uv.2, uv.3);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
            gl.disable(glow::BLEND);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }
}
