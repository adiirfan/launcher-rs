#version 450
// Vulkan copy of render.rs's VS. Vulkan's screen y runs downward.
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_color;
layout(location = 2) in vec4 a_uv;
layout(location = 3) in vec4 a_params;
layout(location = 4) in vec4 a_extra;
layout(push_constant) uniform Push { vec2 screen; } pc;
layout(location = 0) out vec2 v_local;
layout(location = 1) out vec2 v_uv;
layout(location = 2) flat out vec4 v_color;
layout(location = 3) flat out vec4 v_params;
layout(location = 4) flat out vec4 v_extra;
layout(location = 5) flat out vec2 v_size;
void main() {
    vec2 c = vec2(float(gl_VertexIndex & 1), float(gl_VertexIndex >> 1));
    vec2 p = a_rect.xy + c * a_rect.zw;
    v_local = c * a_rect.zw;
    v_uv = mix(a_uv.xy, a_uv.zw, c);
    v_color = a_color;
    v_params = a_params;
    v_extra = a_extra;
    v_size = a_rect.zw;
    gl_Position = vec4(p.x / pc.screen.x * 2.0 - 1.0, p.y / pc.screen.y * 2.0 - 1.0, 0.0, 1.0);
}
