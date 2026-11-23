// The FPV view as goggles show it, ported from libfpv's docs/renderers/fpv_video_gl3.h line for
// line: the fisheye lens and, when the pilot has them on, the video feed's effects (the analog
// link's noise, digital breakup, the camera's resolution and aspect).
//
// `signal` draws one pixel of the feed's raster per fragment, per mode: VIDEO_ANALOG,
// VIDEO_DIGITAL, or neither (the lens alone). video_present.wgsl then scales that signal into
// the picture's rectangle on screen.
//
// Coordinates follow the GL reference inside the maths (uv and p with y up, line counted from
// the top); only the samples flip, since wgpu textures start at the top. The reference works on
// the gamma-encoded picture a GL framebuffer holds, so the scene (linear here) is encoded as it
// is sampled and the presented picture decoded again.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct Signal {
    raster: vec2<f32>,
    k: vec2<f32>,
    digital: vec2<f32>,
    noise: f32,
    res: f32,
    osd_in: f32,
    pic_aspect: f32,
    scene_aspect: f32,
    zoom: f32,
    luma_cycles: f32,
    chroma_cycles: f32,
    subcarrier: f32,
    sharpen: f32,
    parity: f32,
    roll: f32,
    seed: f32,
    active_lines: f32,
    total_lines: f32,
    ntsc: f32,
    blocks: f32,
    softness: f32,
    osd_lines: f32,
    _pad: f32,
    interf: vec4<f32>,
    // chroma boost, tint I, tint Q, hue sweep (turns)
    upset: vec4<f32>,
}


@group(0) @binding(0) var<uniform> u: Signal;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;
@group(0) @binding(3) var osd: texture_2d<f32>;
@group(0) @binding(4) var lines: texture_2d<f32>;
@group(0) @binding(5) var history: texture_2d<f32>;


const PI: f32 = 3.14159265;

// GLSL's mod: x - y * floor(x / y).
fn fmod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3(0.0)), vec3(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3(0.0031308));
}

// Dave Hoskins' hash without sine: the same on every GPU.
fn h13(p_in: vec3<f32>) -> f32 {
    var p = fract(p_in * 0.1031);
    p += dot(p, p.zyx + 31.32);
    return fract((p.x + p.y) * p.z);
}

fn gn(p: vec3<f32>) -> f32 {
    return (h13(p) + h13(p + vec3(17.1, 3.7, 9.3)) - 1.0) * 2.45;
}

// Picture (0..1, y up) to the scene texture through the lens (GL coordinates, y up).
fn scene_uv(p: vec2<f32>) -> vec2<f32> {
    let scale = vec2(u.pic_aspect, 1.0) / (0.5 * sqrt(u.pic_aspect * u.pic_aspect + 1.0));
    let c = (p - 0.5) * scale;
    let r2 = dot(c, c);
    var d = c * (1.0 + u.k.x * r2 + u.k.y * r2 * r2) / u.zoom / scale;
    let s = u.pic_aspect / u.scene_aspect;
    if s <= 1.0 {
        d.x *= s;
    } else {
        d.y /= s;
    }
    return d + 0.5;
}

// The scene, gamma-encoded as the reference's framebuffer holds it; black outside.
fn tap(p: vec2<f32>) -> vec3<f32> {
    let s = scene_uv(p);
    if s.x < 0.0 || s.x > 1.0 || s.y < 0.0 || s.y > 1.0 {
        return vec3(0.0);
    }
    let c = textureSampleLevel(scene, linear_sampler, vec2(s.x, 1.0 - s.y), 0.0).rgb;
    return linear_to_srgb(clamp(c, vec3(0.0), vec3(1.0)));
}

fn osd_texel(i: f32, ln: f32) -> vec4<f32> {
    if i < 0.0 || i >= 360.0 || ln < 0.0 || ln >= u.osd_lines {
        return vec4(0.0);
    }
    return textureLoad(osd, vec2<i32>(i32(i), i32(ln)), 0);
}

// The OSD chip's picture: see the reference's long comment. x 0..1 across the line, line from
// the top.
fn osd_chip(x_in: f32, line: f32) -> vec4<f32> {
    var x = x_in;
    if u.res < 0.5 {
        x = (x - 0.5) * u.pic_aspect / (4.0 / 3.0) + 0.5;
    }
    if u.osd_in < 0.5 {
        return vec4(0.0);
    }
    var s = vec2(1.0, 1.0);
    var left = 9.0;
    var top = select(u.active_lines - u.osd_lines, -4.0, u.active_lines < 288.0);
    if u.osd_lines > u.active_lines {
        s = vec2(0.95, 0.95 * u.active_lines / u.osd_lines);
        left = 360.0 * (1.0 - s.x);
        top = u.active_lines - u.osd_lines * s.y;
    }
    let gx = (x * 720.0 - left) / (2.0 * s.x) - 0.5;
    let gy = (2.0 * line + u.parity + 0.5 - top) / (2.0 * s.y) - 0.5;
    let ix = floor(gx);
    let iy = floor(gy);
    let f = clamp((vec2(gx - ix, gy - iy) - 0.5) * (2.0 * s) + 0.5, vec2(0.0), vec2(1.0));
    return mix(
        mix(osd_texel(ix, iy), osd_texel(ix + 1.0, iy), f.x),
        mix(osd_texel(ix, iy + 1.0), osd_texel(ix + 1.0, iy + 1.0), f.x),
        f.y
    );
}

fn osd_tap(p: vec2<f32>) -> vec4<f32> {
    return osd_chip(p.x, floor((1.0 - p.y) * u.active_lines));
}

// GLSL's column-major mat3(...) and WGSL's agree.
const TO_YIQ: mat3x3<f32> = mat3x3<f32>(0.299, 0.596, 0.211, 0.587, -0.274, -0.523, 0.114, -0.322, 0.312);
const TO_RGB: mat3x3<f32> = mat3x3<f32>(1.0, 1.0, 1.0, 0.956, -0.272, -1.106, 0.621, -0.647, 1.703);
const LUMA: vec3<f32> = vec3(0.299, 0.587, 0.114);

#ifdef VIDEO_ANALOG
fn analog(uv: vec2<f32>) -> vec3<f32> {
    let raster = u.raster;
    let px = floor(uv * raster);
    // the field line this pixel shows (from the top) and, when the picture rolls, the
    // transmitted line it carries
    let line_f = select((1.0 - uv.y) * u.active_lines, raster.y - 1.0 - px.y, u.res > 0.5);
    let rolled = fmod(line_f + u.roll * u.total_lines, u.total_lines);
    let line = floor(rolled);
    var t0 = vec4(0.0, 0.0, 1.0, 1.0);
    var t1 = vec4(0.5, 0.5, 0.5, 0.5);
    if u.noise > 0.5 {
        t0 = textureLoad(lines, vec2<i32>(0, i32(line)), 0);
        t1 = textureLoad(lines, vec2<i32>(1, i32(line)), 0);
    }
    var shift = 0.0;
    if u.noise > 0.5 {
        shift = (floor(t1.r * 255.0 + 0.5) * 256.0 + floor(t1.g * 255.0 + 0.5)) / 65535.0 * 0.5 - 0.25;
    }
    let blank = rolled >= u.active_lines; // the vertical blanking: a black bar
    // where the line sits in the camera's picture: interlaced, a field carries every other line
    let y_top = select(rolled / u.active_lines, (2.0 * line + u.parity + 0.5) / (2.0 * u.active_lines), u.res > 0.5);
    var x = select(uv.x, (px.x + 0.5) / raster.x, u.res > 0.5) + shift;
    // a line torn past its end shows the black porch, then the next line's start
    var porch = false;
    if x > 1.0 {
        x -= 1.035;
        porch = x < 0.0;
    } else if x < 0.0 {
        x += 1.035;
        porch = x > 1.0;
    }
    let p = vec2(x, 1.0 - y_top);
    var yiq: vec3<f32>;
    let ph = 2.0 * PI * (x * u.subcarrier + 0.5 * line + 0.5 * u.parity);
    let carrier = vec2(cos(ph), sin(ph));
    if u.res > 0.5 {
        // the camera and the link: luma limited to lumaCycles (about -6 dB there), colour to
        // chromaCycles, colour a little late; a quarter line up and down against vertical aliasing
        let sl = 0.25 / u.luma_cycles;
        let sc = 0.35 / u.chroma_cycles;
        let dv = 0.25 / (2.0 * u.active_lines);
        var yy = 0.0;
        var yring = 0.0;
        var wsum = 0.0;
        var wide = vec3(0.0);
        var wcs = 0.0;
        for (var i: i32 = -3; i <= 3; i++) {
            let fi = f32(i);
            let w = exp(-0.5 * (fi * 0.75) * (fi * 0.75));
            let odd = (i - (i / 2) * 2) != 0;
            let q = p + vec2(fi * 0.75 * sl, select(-dv, dv, odd));
            let y = dot(tap(q), LUMA);
            yy += w * y;
            wsum += w;
            if abs(i) >= 2 {
                yring += y * 0.25;
            }
            wide += w * tap(p + vec2(fi * 0.75 * sc + 0.3 * sc, 0.0));
            wcs += w;
        }
        yy /= wsum;
        wide /= wcs;
        // the camera's edge enhancement and the goggles' peaking: crisp edges with halos
        yy += (0.6 + u.sharpen * 1.6) * (yy - yring);
        yiq = vec3(yy, (TO_YIQ * wide).yz);
        // composite: fine detail near the subcarrier decodes as colour (rainbows)
        let hp = 0.5 / u.subcarrier;
        let yc = dot(tap(p), LUMA);
        let yhf = yc - 0.5 * (dot(tap(p + vec2(hp, 0.0)), LUMA) + dot(tap(p - vec2(hp, 0.0)), LUMA));
        yiq = vec3(yiq.x, yiq.yz + 0.45 * yhf * carrier);
    } else {
        yiq = TO_YIQ * tap(p);
    }
    // The OSD chip switches the video to its black or white level after the camera.
    var osd_a = 0.0;
    if u.osd_in > 0.5 && !blank {
        let s1 = 1.0 / 720.0;
        let oc = osd_chip(x, line);
        let ol = osd_chip(x - s1, line);
        let orr = osd_chip(x + s1, line);
        let ol2 = osd_chip(x - 2.0 * s1, line);
        let or2 = osd_chip(x + 2.0 * s1, line);
        let a = 0.7 * oc.a + 0.15 * (ol.a + orr.a);
        let oy = 1.1 * (0.7 * oc.r + 0.15 * (ol.r + orr.r)); // the chip's white sits over the peak
        let y0 = yiq.x;
        yiq = vec3(yiq.x * (1.0 - a) + oy, yiq.yz * (1.0 - a));
        osd_a = a;
        // NTSC cross-colour over the OSD's one-pixel strokes
        var cx = vec2(0.0);
        var cw = 1.0;
        if oc.a + ol.a + orr.a + ol2.a + or2.a > 0.0 {
            cw = 0.0;
            for (var kk: i32 = -4; kk <= 4; kk++) {
                let fk = f32(kk);
                let xs = x + fk * s1;
                let o = osd_chip(xs, line);
                let w = exp(-0.08 * fk * fk);
                let phk = 2.0 * PI * (xs * u.subcarrier + 0.5 * u.parity);
                cx += w * (o.r + (1.0 - o.a) * y0) * vec2(cos(phk), sin(phk));
                cw += w;
            }
        }
        yiq = vec3(yiq.x, yiq.yz + 0.35 * cx / cw);
    }
    // the black porch where a torn line starts
    if porch {
        yiq = vec3(0.0);
    }
    if u.noise > 0.5 {
        let grain = t0.r * 0.5;
        let density = t0.g * t0.g * t0.g;
        var chroma = t0.b;
        let pic = t0.a;
        let hue = (t1.a - 0.5) * 2.0 * PI + 2.0 * PI * u.upset.w * x;
        let sd = vec3(0.0, line, u.seed);
        // grain: the link's bandwidth stretches it along the line
        let gc = x * u.luma_cycles * 2.0;
        let gi = floor(gc);
        let gf = gc - gi;
        let g = mix(gn(sd + vec3(gi, 0.0, 0.0)), gn(sd + vec3(gi + 1.0, 0.0, 0.0)), gf);
        let cc = x * 360.0;
        let ci = floor(cc);
        let cf = cc - ci;
        let gcol = mix(
            vec2(gn(sd + vec3(ci, 51.0, 0.0)), gn(sd + vec3(ci, 77.0, 0.0))),
            vec2(gn(sd + vec3(ci + 1.0, 51.0, 0.0)), gn(sd + vec3(ci + 1.0, 77.0, 0.0))),
            cf
        );
        // no picture: the receiver shows its own noise, coarse grey streaks
        let sc2 = x * 260.0;
        let si = floor(sc2);
        let sf = sc2 - si;
        let snow = 0.42 + 0.30 * mix(gn(sd + vec3(si, 9.0, 1.0)), gn(sd + vec3(si + 1.0, 9.0, 1.0)), sf) + 0.12 * g;
        if blank {
            yiq = vec3(0.01, 0.0, 0.0);
            chroma = 0.0;
        }
        yiq = vec3(mix(snow, yiq.x, pic), yiq.yz * pic);
        // colour: the burst's phase (hue), the colour killer, and the decoder thrown off
        let cs = cos(hue);
        let sn = sin(hue);
        var iq = mat2x2<f32>(cs, sn, -sn, cs) * yiq.yz * chroma * (1.0 + u.upset.x);
        iq += u.upset.yz * clamp(yiq.x, 0.0, 1.0) * chroma;
        var luma = yiq.x + grain * g;
        // FM noise is strongest where the colour lives: saturated colour speckle
        iq += grain * 1.0 * gcol * chroma;
        // over the OSD's white strokes the noise turns the colour much harder
        if osd_a > 0.0 {
            let wc = x * 90.0;
            let wi = floor(wc);
            let wf = wc - wi;
            let wd = vec3(0.0, floor(line * 0.25), u.seed);
            let wash = mix(
                vec2(gn(wd + vec3(wi, 23.0, 0.0)), gn(wd + vec3(wi, 41.0, 0.0))),
                vec2(gn(wd + vec3(wi + 1.0, 23.0, 0.0)), gn(wd + vec3(wi + 1.0, 41.0, 0.0))),
                wf
            );
            iq += osd_a * clamp(luma, 0.0, 1.0) * (grain * 3.0 * wash + grain * 2.0 * gcol) * chroma;
        }
        // FM clicks: specks a sample or two long
        if density > 0.0 {
            let lum = clamp(luma, 0.0, 1.0);
            let dens = density * (0.15 + 1.7 * lum * lum) * smoothstep(0.004, 0.04, density);
            let kc = x * 360.0;
            let k0 = floor(kc);
            for (var j: i32 = 0; j < 2; j++) {
                let cell = k0 - f32(j);
                let q = sd + vec3(cell, 0.0, 33.0);
                if h13(q) < dens {
                    let dist = kc - cell - h13(q + 1.3) * 0.5;
                    let len = 0.35 + 0.6 * h13(q + 2.9);
                    let v = select(1.0, 0.0, dist < 0.0 || dist > len);
                    var pol = select(-1.0, 1.0, h13(q + 6.1) < 0.5);
                    if h13(q + 4.7) < 0.85 {
                        pol = select(-1.0, 1.0, lum < 0.4);
                    }
                    let hue_k = 2.0 * PI * h13(q + 7.7);
                    let sat = h13(q + 8.3);
                    luma += pol * v * (1.0 - 0.6 * sat) * 0.9;
                    iq += vec2(cos(hue_k), sin(hue_k)) * v * sat * 0.6 * max(chroma, 0.2);
                }
            }
        }
        // motor and ESC noise on the power wiring, and the line's own offset
        luma += u.interf.x * sin(2.0 * PI * (x * u.interf.y + line * u.interf.z + u.interf.w));
        luma += t1.b - 0.5;
        yiq = vec3(luma, iq);
    }
    let rgb = TO_RGB * yiq;
    return clamp(rgb, vec3(0.0), vec3(1.0));
}
#endif

#ifdef VIDEO_DIGITAL
fn box_tap(p: vec2<f32>, size: vec2<f32>) -> vec3<f32> {
    let o = 0.25 * size;
    return 0.25 * (tap(p + vec2(-o.x, -o.y)) + tap(p + vec2(o.x, -o.y)) + tap(p + vec2(-o.x, o.y)) + tap(p + vec2(o.x, o.y)));
}

// The last frame's signal at a GL point (y up).
fn history_at(p: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(history, linear_sampler, vec2(p.x, 1.0 - p.y), 0.0).rgb;
}

fn digital_pix(uv: vec2<f32>) -> vec3<f32> {
    let p = uv;
    var c = select(tap(p), box_tap(p, 1.0 / u.raster), u.res > 0.5);
    let bsize = 16.0 / u.digital;
    let b = floor(p / bsize);
    if u.softness > 0.0 {
        // the bitrate drops: detail goes first, in blocks
        let bc = (b + 0.5) * bsize;
        let dc = box_tap(bc, bsize);
        let mid = box_tap(p, bsize * 0.5);
        c = mix(c, mix(mid, dc, 0.35 * u.softness), u.softness);
    }
    if u.noise > 0.5 && u.blocks > 0.0 {
        let q = vec3(b, u.seed);
        // a lost packet takes a run of blocks along the row, then the decoder resyncs
        let run_start = floor(b.x / 6.0);
        let run = h13(vec3(run_start, b.y, u.seed + 3.1));
        let run_len = 1.0 + floor(h13(vec3(run_start, b.y, u.seed + 9.7)) * 6.0);
        let in_run = run < u.blocks * 0.7 && b.x - run_start * 6.0 < run_len;
        if in_run || h13(q) < u.blocks * 0.3 {
            let kind = h13(q + 5.5);
            if kind < 0.6 {
                // stale: last frame's block, moved by a broken motion vector
                let mv = (vec2(h13(q + 1.1), h13(q + 2.2)) - 0.5) * bsize * 3.0;
                c = history_at(p + mv);
            } else if kind < 0.9 {
                // only the DC: a flat block, colour off
                let dc = box_tap((b + 0.5) * bsize, bsize);
                var y = TO_YIQ * dc;
                y = vec3(y.x, y.yz + (vec2(h13(vec3(run_start, b.y, u.seed + 3.3)), h13(vec3(run_start, b.y, u.seed + 4.4))) - 0.5) * 0.18);
                let f = fract(p / bsize) * 2.0 - 1.0;
                y.x += 0.08 * (h13(q + 6.6) - 0.5) * (f.x + f.y);
                c = TO_RGB * y;
            } else {
                // garbage coefficients: stripes
                let f = fract(p / bsize);
                let s = cos(PI * floor(h13(q + 7.7) * 8.0) * f.x) * cos(PI * floor(h13(q + 8.8) * 8.0) * f.y);
                c = clamp(box_tap((b + 0.5) * bsize, bsize) + 0.25 * s * vec3(0.8, 1.0, 0.7), vec3(0.0), vec3(1.0));
            }
        }
    }
    return clamp(c, vec3(0.0), vec3(1.0));
}
#endif

// The signal: one pixel of the feed's raster per fragment, gamma-encoded.
@fragment
fn signal(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // The reference's uv, y up.
    let uv = vec2(in.uv.x, 1.0 - in.uv.y);
#ifdef VIDEO_ANALOG
    let c = analog(uv);
#else
#ifdef VIDEO_DIGITAL
    let c = digital_pix(uv);
#else
    var c = tap(uv);
    if u.osd_in > 0.5 {
        let od = osd_tap(uv);
        c = c * (1.0 - od.a) + od.rgb;
    }
#endif
#endif
    return vec4(c, 1.0);
}

