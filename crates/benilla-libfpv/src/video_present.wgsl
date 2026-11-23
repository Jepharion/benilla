// The goggles, after libfpv's docs/renderers/fpv_video_gl3.h: the feed's signal (gamma-encoded,
// from video_signal.wgsl) scaled into the picture's rectangle on screen, black around it, and
// decoded to the linear picture benilla's view holds.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Present {
    // The picture's rectangle on the target, pixels from the top-left: x, y, w, h.
    rect: vec4<f32>,
    // Field lines (0 = progressive) and the field's parity.
    rows: vec2<f32>,
    tex_size: vec2<f32>,
}

@group(0) @binding(0) var<uniform> pu: Present;
@group(0) @binding(1) var signal_tex: texture_2d<f32>;
@group(0) @binding(2) var present_sampler: sampler;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(hi, lo, c <= vec3(0.04045));
}

// The goggles: the signal scaled into the picture, black around it, decoded to linear.
@fragment
fn present(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // The reference's gl_FragCoord-relative p, y up within the rectangle.
    let frag = in.position.xy;
    var p = vec2((frag.x - pu.rect.x) / pu.rect.z, 1.0 - (frag.y - pu.rect.y) / pu.rect.w);
    if p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    if pu.rows.x > 0.0 {
        // bob: a field's lines sit half a line lower when it carries the odd lines
        let from_top = (1.0 - p.y) * pu.rows.x - 0.5 * pu.rows.y;
        p.y = 1.0 - from_top / pu.rows.x;
    }
    // the goggles' scaler: bilinear, each step kept to about a screen pixel
    let t = p * pu.tex_size - 0.5;
    let i = floor(t);
    let f = clamp((t - i - 0.5) * max(pu.rect.zw / pu.tex_size, vec2(1.0)) + 0.5, vec2(0.0), vec2(1.0));
    let st = (i + f + 0.5) / pu.tex_size;
    let c = textureSampleLevel(signal_tex, present_sampler, vec2(st.x, 1.0 - st.y), 0.0).rgb;
    return vec4(srgb_to_linear(c), 1.0);
}
