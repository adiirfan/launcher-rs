#version 450
// Vulkan copy of render.rs's FS.
layout(location = 0) in vec2 v_local;
layout(location = 1) in vec2 v_uv;
layout(location = 2) flat in vec4 v_color;
layout(location = 3) flat in vec4 v_params;
layout(location = 4) flat in vec4 v_extra;
layout(location = 5) flat in vec2 v_size;
layout(set = 0, binding = 0) uniform sampler2D u_icons;
layout(set = 0, binding = 1) uniform sampler2D u_glyphs;
layout(location = 0) out vec4 o;

float sd_round_rect(vec2 p, vec2 half_size, float r) {
    r = min(r, min(half_size.x, half_size.y));
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    vec2 half_size = v_size * 0.5;
    vec2 p = v_local - half_size;
    float kind = v_params.w;
    if (kind > 1.5) {
        o = v_color * texture(u_glyphs, v_uv).r;
        return;
    }
    if (kind > 0.5) {
        vec4 c = texture(u_icons, v_uv) * v_color.a;
        if (v_extra.x > 0.0) {
            c *= clamp(0.5 - sd_round_rect(p, half_size, v_extra.x), 0.0, 1.0);
        }
        o = c;
        return;
    }
    float d = sd_round_rect(p, half_size, v_params.x);
    float soft = v_params.z;
    float a = soft > 0.0 ? 1.0 - smoothstep(-soft, 0.0, d) : clamp(0.5 - d, 0.0, 1.0);
    if (v_params.y > 0.0) {
        a *= clamp(0.5 + d + v_params.y, 0.0, 1.0);
    }
    o = v_color * a;
}
