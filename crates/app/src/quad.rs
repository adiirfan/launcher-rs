//! Throwaway rectangle drawing for the risk check. Plan 2 replaces it.

use glow::HasContext;

const VS: &str = r#"#version 300 es
uniform vec4 u_rect;
uniform vec2 u_screen;
out vec2 v_uv;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    v_uv = c;
    vec2 p = u_rect.xy + c * u_rect.zw;
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS: &str = r#"#version 300 es
precision mediump float;
in vec2 v_uv;
uniform vec4 u_color;
uniform sampler2D u_tex;
uniform int u_use_tex;
out vec4 o;
void main() {
    // Colours and Android bitmaps are both premultiplied by alpha.
    o = u_use_tex == 1 ? texture(u_tex, v_uv) : vec4(u_color.rgb * u_color.a, u_color.a);
}"#;

pub struct Quads {
    program: glow::Program,
    vao: glow::VertexArray,
    u_rect: Option<glow::UniformLocation>,
    u_screen: Option<glow::UniformLocation>,
    u_color: Option<glow::UniformLocation>,
    u_use_tex: Option<glow::UniformLocation>,
}

impl Quads {
    pub fn new(gl: &glow::Context) -> Quads {
        unsafe {
            let program = gl.create_program().expect("program");
            for (kind, src) in [(glow::VERTEX_SHADER, VS), (glow::FRAGMENT_SHADER, FS)] {
                let sh = gl.create_shader(kind).expect("shader");
                gl.shader_source(sh, src);
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    log::error!("shader: {}", gl.get_shader_info_log(sh));
                }
                gl.attach_shader(program, sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                log::error!("link: {}", gl.get_program_info_log(program));
            }
            Quads {
                vao: gl.create_vertex_array().expect("vao"),
                u_rect: gl.get_uniform_location(program, "u_rect"),
                u_screen: gl.get_uniform_location(program, "u_screen"),
                u_color: gl.get_uniform_location(program, "u_color"),
                u_use_tex: gl.get_uniform_location(program, "u_use_tex"),
                program,
            }
        }
    }

    fn draw(&self, gl: &glow::Context, r: [f32; 4], screen: (i32, i32), color: [f32; 4], tex: Option<glow::Texture>) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.uniform_4_f32(self.u_rect.as_ref(), r[0], r[1], r[2], r[3]);
            gl.uniform_2_f32(self.u_screen.as_ref(), screen.0 as f32, screen.1 as f32);
            gl.uniform_4_f32(self.u_color.as_ref(), color[0], color[1], color[2], color[3]);
            gl.uniform_1_i32(self.u_use_tex.as_ref(), tex.is_some() as i32);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, tex);
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn solid(&mut self, gl: &glow::Context, x: f32, y: f32, w: f32, h: f32, rgba: [f32; 4], screen: (i32, i32)) {
        self.draw(gl, [x, y, w, h], screen, rgba, None);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn textured(&mut self, gl: &glow::Context, x: f32, y: f32, w: f32, h: f32, tex: glow::Texture, screen: (i32, i32)) {
        self.draw(gl, [x, y, w, h], screen, [0.0; 4], Some(tex));
    }
}

/// Upload premultiplied RGBA pixels as a texture with smooth scaling.
pub fn upload_rgba(gl: &glow::Context, w: i32, h: i32, pixels: &[u8]) -> glow::Texture {
    unsafe {
        let tex = gl.create_texture().expect("texture");
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D, 0, glow::RGBA as i32, w, h, 0, glow::RGBA, glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(pixels)),
        );
        tex
    }
}
