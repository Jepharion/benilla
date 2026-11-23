// libfpv draw data (the OSD and the F8 overlay), as libfpv's reference renderer draws it
// (docs/renderers/fpv_render_gl3.h): pixel positions from the top-left of the display, vertex
// colour times the command's texture, straight alpha. The colours are sRGB bytes and the target
// is sRGB, so they are linearised here; the texture views are sRGB, so sampling linearises them.

struct Display {
    // The draw data's display size, in its own units (window points).
    size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> display: Display;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    // RGBA8, R in the lowest byte: unorm 0..1, sRGB-encoded.
    @location(2) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = vec2(in.pos.x / display.size.x * 2.0 - 1.0, 1.0 - in.pos.y / display.size.y * 2.0);
    out.position = vec4(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.color = vec4(srgb_to_linear(in.color.rgb), in.color.a);
    return out;
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color * textureSample(tex, tex_sampler, in.uv);
}
