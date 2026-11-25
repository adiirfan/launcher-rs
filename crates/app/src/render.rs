//! The one renderer: every shape, icon and glyph in a single instanced draw call.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use glow::HasContext;
use launcher_draw::atlas::ShelfPacker;
use launcher_draw::batch::{Batch, Instance};
use launcher_draw::font::FontSet;
use launcher_draw::slots::SlotPool;
use launcher_draw::text::layout_line;
use launcher_draw::{Rect, Rgba};

/// Icon atlas: 2048×1024 holds about 75 home-size icons, enough for two
/// full pages plus the dock while swiping between them.
const ICON_ATLAS: (u32, u32) = (2048, 1024);
const GLYPH_ATLAS: (u32, u32) = (1024, 1024);

const VS: &str = r#"#version 300 es
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_color;
layout(location = 2) in vec4 a_uv;
layout(location = 3) in vec4 a_params;
layout(location = 4) in vec4 a_extra;
uniform vec2 u_screen;
out vec2 v_local;
out vec2 v_uv;
flat out vec4 v_color;
flat out vec4 v_params;
flat out vec4 v_extra;
flat out vec2 v_size;
void main() {
    vec2 c = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    vec2 p = a_rect.xy + c * a_rect.zw;
    v_local = c * a_rect.zw;
    v_uv = mix(a_uv.xy, a_uv.zw, c);
    v_color = a_color;
    v_params = a_params;
    v_extra = a_extra;
    v_size = a_rect.zw;
    gl_Position = vec4(p.x / u_screen.x * 2.0 - 1.0, 1.0 - p.y / u_screen.y * 2.0, 0.0, 1.0);
}"#;

const FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_local;
in vec2 v_uv;
flat in vec4 v_color;
flat in vec4 v_params;
flat in vec4 v_extra;
flat in vec2 v_size;
uniform sampler2D u_icons;
uniform sampler2D u_glyphs;
out vec4 o;

// Signed distance to a rounded rectangle centred on 0 (negative inside), in px.
float sd_round_rect(vec2 p, vec2 half_size, float r) {
    r = min(r, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 half_size = v_size * 0.5;
    vec2 p = v_local - half_size;
    float kind = v_params.w;
    if (kind > 1.5) {                                   // glyph
        o = v_color * texture(u_glyphs, v_uv).r;
        return;
    }
    if (kind > 0.5) {                                   // image
        vec4 c = texture(u_icons, v_uv) * v_color.a;
        if (v_extra.x > 0.0) {
            c *= clamp(0.5 - sd_round_rect(p, half_size, v_extra.x), 0.0, 1.0);
        }
        o = c;
        return;
    }
    float d = sd_round_rect(p, half_size, v_params.x);  // shape
    float soft = v_params.z;
    float a = soft > 0.0 ? 1.0 - smoothstep(-soft, 0.0, d) : clamp(0.5 - d, 0.0, 1.0);
    if (v_params.y > 0.0) {
        a *= clamp(0.5 + d + v_params.y, 0.0, 1.0);
    }
    o = v_color * a;
}"#;

#[derive(Clone, Copy)]
struct GlyphSlot {
    uv: [f32; 4],
    w: f32,
    h: f32,
    left: f32,
    top: f32,
}

struct Atlas {
    tex: glow::Texture,
    packer: ShelfPacker,
    /// Set when it filled up; it is emptied at the start of the next frame so
    /// nothing already queued this frame points at overwritten pixels.
    full: bool,
}

impl Atlas {
    fn new(gl: &glow::Context, internal: u32, format: u32, (w, h): (u32, u32)) -> Atlas {
        unsafe {
            let tex = gl.create_texture().expect("atlas texture");
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            for (k, v) in [
                (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
            }
            gl.tex_image_2d(
                glow::TEXTURE_2D, 0, internal as i32, w as i32, h as i32, 0, format,
                glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None),
            );
            Atlas { tex, packer: ShelfPacker::new(w, h), full: false }
        }
    }

    /// Copy `pixels` in; returns normalised uv, or `None` (and marks full).
    fn put(&mut self, gl: &glow::Context, w: u32, h: u32, format: u32, pixels: &[u8]) -> Option<[f32; 4]> {
        if self.full {
            return None;
        }
        let Some((x, y)) = self.packer.alloc(w, h) else {
            self.full = true;
            return None;
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D, 0, x as i32, y as i32, w as i32, h as i32, format,
                glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(pixels)),
            );
        }
        let (sw, sh) = self.packer.size();
        let (sw, sh) = (sw as f32, sh as f32);
        Some([x as f32 / sw, y as f32 / sh, (x + w) as f32 / sw, (y + h) as f32 / sh])
    }
}

pub struct Renderer {
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    u_screen: Option<glow::UniformLocation>,
    icons: Atlas,
    /// Icon slots inside `icons`; sized by the first icon drawn.
    icon_slots: Option<SlotPool>,
    glyphs: Atlas,
    glyph_keys: HashMap<(char, u32, bool), Option<GlyphSlot>>,
    fonts: FontSet,
    bold: FontSet,
    last_log: Instant,
    redraw: bool,
}

impl Renderer {
    pub fn new(gl: &glow::Context, fonts: FontSet, bold: FontSet) -> Renderer {
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
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_icons").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_glyphs").as_ref(), 1);

            let vao = gl.create_vertex_array().expect("vao");
            let vbo = gl.create_buffer().expect("vbo");
            gl.bind_vertex_array(Some(vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            let stride = std::mem::size_of::<Instance>() as i32;
            for loc in 0..5u32 {
                gl.enable_vertex_attrib_array(loc);
                gl.vertex_attrib_pointer_f32(loc, 4, glow::FLOAT, false, stride, (loc * 16) as i32);
                gl.vertex_attrib_divisor(loc, 1);
            }

            Renderer {
                u_screen: gl.get_uniform_location(program, "u_screen"),
                program,
                vao,
                vbo,
                icons: Atlas::new(gl, glow::RGBA8, glow::RGBA, ICON_ATLAS),
                glyphs: Atlas::new(gl, glow::R8, glow::RED, GLYPH_ATLAS),
                icon_slots: None,
                glyph_keys: HashMap::new(),
                fonts,
                bold,
                last_log: Instant::now() - Duration::from_secs(60),
                redraw: false,
            }
        }
    }

    /// Call before building each frame: empties any atlas that filled up last frame.
    pub fn begin_frame(&mut self) {
        self.redraw = false;
        if let Some(slots) = self.icon_slots.as_mut() {
            slots.begin_frame();
        }
        if self.glyphs.full {
            log::info!("glyph atlas full, reset");
            self.glyphs.packer.reset();
            self.glyphs.full = false;
            self.glyph_keys.clear();
        }
    }

    /// True when something was skipped this frame and another frame is needed.
    pub fn wants_redraw(&self) -> bool {
        self.redraw
    }

    fn slot_uv(&self, (x, y): (u32, u32), size: u32) -> [f32; 4] {
        let (sw, sh) = ICON_ATLAS;
        let (sw, sh) = (sw as f32, sh as f32);
        [x as f32 / sw, y as f32 / sh, (x + size) as f32 / sw, (y + size) as f32 / sh]
    }

    /// Where an icon already in the atlas sits, without needing its pixels.
    pub fn icon_uv(&mut self, key: &str) -> Option<[f32; 4]> {
        let slots = self.icon_slots.as_mut()?;
        let size = slots.slot_size();
        let at = slots.get(key)?;
        Some(self.slot_uv(at, size))
    }

    /// Drop an icon whose app changed.
    pub fn forget_icon(&mut self, key: &str) {
        if let Some(slots) = self.icon_slots.as_mut() {
            slots.forget(key);
        }
    }

    /// Put a square icon in a slot (reusing the slot drawn longest ago when
    /// all are taken); returns its uv. `None` when every slot is already
    /// drawn this frame; a redraw is then requested.
    pub fn icon(&mut self, gl: &glow::Context, key: &str, rgba: &[u8], w: u32, h: u32) -> Option<[f32; 4]> {
        if w != h || rgba.len() != (w * h * 4) as usize {
            log::warn!("icon {key}: not a square RGBA image ({w}x{h})");
            return None;
        }
        if self.icon_slots.as_ref().is_none_or(|p| p.slot_size() != w) {
            let pool = SlotPool::new(ICON_ATLAS.0, ICON_ATLAS.1, w);
            log::info!("icon slots: {} of {w}px", pool.capacity());
            self.icon_slots = Some(pool);
        }
        let slots = self.icon_slots.as_mut()?;
        if let Some(at) = slots.get(key) {
            return Some(self.slot_uv(at, w));
        }
        let Some((x, y)) = slots.insert(key) else {
            self.redraw = true;
            return None;
        };
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.icons.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D, 0, x as i32, y as i32, w as i32, h as i32, glow::RGBA,
                glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)),
            );
        }
        Some(self.slot_uv((x, y), w))
    }

    /// Width of one line of text, without drawing it.
    pub fn measure(&self, s: &str, em_px: f32, max_w: f32, bold: bool) -> f32 {
        let fonts = if bold { &self.bold } else { &self.fonts };
        layout_line(s, fonts.px_for_em(em_px), max_w, fonts).width
    }

    /// Height from baseline to the top of capitals-and-ascenders for `em_px`.
    pub fn ascent(&self, em_px: f32) -> f32 {
        self.fonts.ascent(self.fonts.px_for_em(em_px))
    }

    /// Lay out one line at an Android-style em size and queue its glyphs.
    /// Returns the laid-out width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        gl: &glow::Context,
        batch: &mut Batch,
        s: &str,
        x: f32,
        baseline: f32,
        em_px: f32,
        max_w: f32,
        color: Rgba,
        bold: bool,
    ) -> f32 {
        let fonts = if bold { &self.bold } else { &self.fonts };
        let px = fonts.px_for_em(em_px);
        let line = layout_line(s, px, max_w, fonts);
        let size_key = (px * 2.0).round() as u32;
        for &(c, gx) in &line.glyphs {
            let key = (c, size_key, bold);
            let slot = match self.glyph_keys.get(&key) {
                Some(slot) => *slot,
                None => {
                    let fonts = if bold { &self.bold } else { &self.fonts };
                    let slot = match fonts.raster(c, px) {
                        None => Some(None),
                        Some(g) => self.glyphs.put(gl, g.w, g.h, glow::RED, &g.coverage).map(|uv| {
                            Some(GlyphSlot { uv, w: g.w as f32, h: g.h as f32, left: g.left, top: g.top })
                        }),
                    };
                    match slot {
                        Some(slot) => {
                            self.glyph_keys.insert(key, slot);
                            slot
                        }
                        None => {
                            self.redraw = true;
                            None
                        }
                    }
                }
            };
            if let Some(g) = slot {
                batch.glyph(Rect::new(x + gx + g.left, baseline + g.top, g.w, g.h), g.uv, color);
            }
        }
        line.width
    }


    pub fn draw(&mut self, gl: &glow::Context, batch: &Batch, screen: (i32, i32)) {
        let items = batch.instances();
        unsafe {
            gl.viewport(0, 0, screen.0, screen.1);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            if items.is_empty() {
                return;
            }
            gl.use_program(Some(self.program));
            gl.uniform_2_f32(self.u_screen.as_ref(), screen.0 as f32, screen.1 as f32);
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytemuck::cast_slice(items), glow::STREAM_DRAW);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.icons.tex));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.glyphs.tex));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, items.len() as i32);
        }
        if self.last_log.elapsed() >= Duration::from_secs(5) {
            log::info!("draw calls: 1, instances: {}", items.len());
            self.last_log = Instant::now();
        }
    }
}
