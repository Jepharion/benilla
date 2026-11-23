//! The FPV view as goggles show it, ported from libfpv's `docs/renderers/fpv_video_gl3.h`
//! (docs/INTEGRATION.md "The video feed"): the fisheye lens and, when the pilot has them on
//! (F8 > Camera > "Video feed"; this crate passes `FPV_INIT_VIDEO_FEED`), the analog link's
//! noise, digital breakup, and the camera's resolution and aspect with black bars.
//!
//! - The world camera carries a [`VideoView`] for the frame, built by [`view_for`] from
//!   `frame.camera` and `frame.video_feed`; none when there is nothing to draw (no lens, no
//!   effects).
//! - A render-graph node on that camera, after benilla's own post-processing, draws in two
//!   passes. **The signal** (`video_signal.wgsl`) renders the feed's raster from the world's
//!   picture through the lens into one of two textures, only when a new field (analog, 59.94 or
//!   50 a second) or frame (digital) arrives, so the feed runs at the link's rate whatever the
//!   game's; the other texture is the last frame, which digital breakup reuses. **The goggles**
//!   (`video_present.wgsl`) then scale the signal into the picture's rectangle each frame, black
//!   around it, bobbing an analog field half a line.
//! - An analog OSD goes into the signal, as the flight controller's OSD chip writes it
//!   ([`osd_chip`]), when the feed says so (`FPV_VIDEO_FEED_OSD_IN_SIGNAL`): then
//!   [`crate::draw`] does not draw the OSD over the picture.
//! - The pipelines are queued at startup, so they compile behind the loading.

use std::sync::Arc;

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::core_3d::graph::{Core3d, Node3d};
use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::query::QueryItem;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::encase::UniformBuffer;
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;

use crate::ffi::{self, FpvFrame, FpvVideoFeed};
use crate::lens;

/// The signal's modes, one pipeline each as in the reference: the lens alone, analog, digital.
const MODE_LENS: usize = 0;
const MODE_ANALOG: usize = 1;
const MODE_DIGITAL: usize = 2;

/// The OSD chip's picture: 360 font pixels across (each two of 720 samples), 18 lines a row.
const OSD_W: u32 = 360;
const GLYPH_W: usize = 12;
const GLYPH_H: usize = 18;

/// This frame's view through the goggles, on the world camera.
#[derive(Component, Clone, ExtractComponent)]
pub struct VideoView {
    /// `fpv_video_feed_flags`; 0 = the lens alone.
    flags: u32,
    mode: usize,
    noise: bool,
    res: bool,
    osd_in: bool,
    k: Vec2,
    zoom: f32,
    feed: Feed,
    /// The field's lines, 8 bytes each (`fpv_video_feed.lines`), and how many.
    lines: Arc<Vec<u8>>,
    line_count: u32,
    /// The OSD chip's picture and its height in lines (an analog OSD in the signal).
    osd: Option<Arc<(Vec<u8>, u32)>>,
}

/// The parts of `fpv_video_feed` the shaders read, plain.
#[derive(Clone, Copy, Debug, Default)]
struct Feed {
    system: u32,
    raster: [u32; 2],
    luma_cycles: f32,
    chroma_cycles: f32,
    subcarrier: f32,
    sharpening: f32,
    field: u32,
    parity: u32,
    roll: f32,
    seed: f32,
    interference: [f32; 4],
    block_errors: f32,
    softness: f32,
    frozen: bool,
    upset: [f32; 4],
    picture_aspect: f32,
    display_aspect: f32,
    osd_rows: u32,
}

/// The OSD chip picture last built, kept until the grid changes.
#[derive(Default)]
pub struct OsdChipCache {
    grid: Option<[u8; ffi::FPV_OSD_ROWS * ffi::FPV_OSD_COLS]>,
    rows: usize,
    picture: Option<Arc<(Vec<u8>, u32)>>,
    font: Option<(Vec<u8>, u32, u32)>,
    font_revision: Option<u32>,
}

impl OsdChipCache {
    /// Keeps a copy of libfpv's OSD font, refreshed when its revision moves.
    pub fn refresh_font(
        &mut self,
        get: impl Fn(u64, Option<u32>) -> Option<(Vec<u8>, u32, u32, u32)>,
    ) {
        if let Some((rgba, w, h, rev)) = get(ffi::FPV_TEXTURE_OSD_FONT, self.font_revision) {
            self.font = Some((rgba, w, h));
            self.font_revision = Some(rev);
            self.grid = None; // the font changed: rebuild
        }
    }
}

/// The view for `frame` on a screen of `screen_aspect`, and the vertical FOV to render the
/// world at (radians). `None` for the view when there is nothing to draw: no lens, no effects.
pub fn view_for(
    frame: &FpvFrame,
    screen_aspect: f32,
    osd_cache: &mut OsdChipCache,
) -> (Option<VideoView>, f32) {
    let cam = &frame.camera;
    let vf = &frame.video_feed;
    let flags = vf.flags;
    let noise = flags & ffi::FPV_VIDEO_FEED_NOISE != 0;
    let res = flags & ffi::FPV_VIDEO_FEED_RESOLUTION != 0;
    let analog = vf.system != ffi::FPV_VIDEO_DIGITAL;
    let mode = match (flags, analog) {
        (0, _) => MODE_LENS,
        (_, true) => MODE_ANALOG,
        (_, false) => MODE_DIGITAL,
    };
    let osd_in = mode == MODE_ANALOG && flags & ffi::FPV_VIDEO_FEED_OSD_IN_SIGNAL != 0;
    let zoom = if cam.distortion_zoom > 0.0 {
        cam.distortion_zoom
    } else {
        1.0
    };
    // With the camera's resolution and aspect drawn, the lens works on the camera's own aspect,
    // so libfpv's FOV is exact; otherwise it is recomputed for the screen's (INTEGRATION.md).
    let (lens_on, fov) = if res && vf.picture_aspect > 0.1 {
        let on = cam.distortion_k1 != 0.0 || cam.distortion_k2 != 0.0 || zoom != 1.0;
        (on, cam.fov_y_degrees.to_radians())
    } else {
        let (l, fov) = lens::fit(cam, screen_aspect);
        (l.is_some(), fov)
    };
    if flags == 0 && !lens_on {
        return (None, fov);
    }
    let active = if vf.system == ffi::FPV_VIDEO_PAL {
        288
    } else {
        240
    };
    let osd = osd_in
        .then(|| osd_chip(frame, osd_rows(vf, active), osd_cache))
        .flatten();
    let view = VideoView {
        flags,
        mode,
        noise,
        res,
        osd_in: osd_in && osd.is_some(),
        k: Vec2::new(cam.distortion_k1, cam.distortion_k2),
        zoom,
        feed: Feed {
            system: vf.system,
            raster: [vf.raster_width, vf.raster_height],
            luma_cycles: vf.luma_cycles,
            chroma_cycles: vf.chroma_cycles,
            subcarrier: vf.subcarrier_cycles,
            sharpening: vf.sharpening,
            field: vf.field,
            parity: vf.field_parity & 1,
            roll: vf.roll,
            seed: vf.seed,
            interference: vf.interference,
            block_errors: vf.block_errors,
            softness: vf.softness,
            frozen: vf.frozen != 0,
            upset: [
                vf.chroma_boost,
                vf.chroma_tint[0],
                vf.chroma_tint[1],
                vf.hue_sweep,
            ],
            picture_aspect: vf.picture_aspect,
            display_aspect: vf.display_aspect,
            osd_rows: osd_rows(vf, active) as u32,
        },
        lines: Arc::new(vf.lines.to_vec()),
        line_count: vf.line_count.clamp(1, ffi::FPV_VIDEO_MAX_LINES as u32),
        osd,
    };
    (Some(view), fov)
}

impl VideoView {
    /// The analog OSD is in the signal, so it is not drawn again over the picture.
    pub fn osd_in_signal(&self) -> bool {
        self.osd_in
    }
}

/// How many OSD rows the chip's picture holds: 13 when Betaflight's video system is NTSC, else
/// all 16 (the reference's `fpv_video_gl3__osd_rows`).
fn osd_rows(vf: &FpvVideoFeed, active_lines: usize) -> usize {
    if vf.osd_rows != 0 {
        if (vf.osd_rows as usize) < ffi::FPV_OSD_ROWS {
            13
        } else {
            ffi::FPV_OSD_ROWS
        }
    } else if active_lines >= 288 {
        ffi::FPV_OSD_ROWS
    } else {
        13
    }
}

/// The OSD chip's picture (MAX7456 / AT7456E): Betaflight's grid of 12 × 18 font pixels from
/// `frame.osd` and libfpv's font, 360 × rows·18, premultiplied, rebuilt only when the grid or
/// the row count changes (the reference's `fpv_video_gl3__osd_chip`).
fn osd_chip(
    frame: &FpvFrame,
    rows: usize,
    cache: &mut OsdChipCache,
) -> Option<Arc<(Vec<u8>, u32)>> {
    if cache.grid.as_ref() == Some(&frame.osd) && cache.rows == rows {
        return cache.picture.clone();
    }
    let (font, fw, _fh) = cache.font.as_ref()?;
    let fw = *fw as usize;
    let lines = rows * GLYPH_H;
    let mut out = vec![0u8; OSD_W as usize * lines * 4];
    for r in 0..rows {
        for c in 0..ffi::FPV_OSD_COLS {
            let g = frame.osd[r * ffi::FPV_OSD_COLS + c] as usize;
            if g == 0 || g == usize::from(b' ') {
                continue;
            }
            for y in 0..GLYPH_H {
                let src = (((g / 16) * GLYPH_H + y) * fw + (g % 16) * GLYPH_W) * 4;
                let dst = ((r * GLYPH_H + y) * OSD_W as usize + c * GLYPH_W) * 4;
                if let (Some(s), Some(d)) = (
                    font.get(src..src + GLYPH_W * 4),
                    out.get_mut(dst..dst + GLYPH_W * 4),
                ) {
                    d.copy_from_slice(s);
                }
            }
        }
    }
    let picture = Arc::new((out, lines as u32));
    cache.grid = Some(frame.osd);
    cache.rows = rows;
    cache.picture = Some(picture.clone());
    Some(picture)
}

pub struct VideoPlugin;

impl Plugin for VideoPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "video_signal.wgsl");
        embedded_asset!(app, "video_present.wgsl");
        app.add_plugins(ExtractComponentPlugin::<VideoView>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<Gpu>()
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_render_graph_node::<ViewNodeRunner<VideoNode>>(Core3d, VideoLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    VideoLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct VideoLabel;

/// The signal shader's uniforms, laid out as `video_signal.wgsl`'s `Signal`.
#[derive(ShaderType, Clone, Copy, Default)]
struct SignalUniform {
    raster: Vec2,
    k: Vec2,
    digital: Vec2,
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
    interf: Vec4,
    upset: Vec4,
}

/// The goggles shader's uniforms, laid out as `video_present.wgsl`'s `Present`.
#[derive(ShaderType, Clone, Copy, Default)]
struct PresentUniform {
    rect: Vec4,
    rows: Vec2,
    tex_size: Vec2,
}

#[derive(Resource)]
struct Pipelines {
    signal_layout: BindGroupLayoutDescriptor,
    present_layout: BindGroupLayoutDescriptor,
    linear: Sampler,
    /// Per mode, into the signal's 8-bit texture.
    signal: [CachedRenderPipelineId; 3],
    /// Into the view: HDR (benilla's world camera) and 8-bit.
    present_hdr: CachedRenderPipelineId,
    present_sdr: CachedRenderPipelineId,
}

/// The signal texture's format: the gamma-encoded picture, as the reference's RGBA8 targets.
const SIGNAL_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let tex = || texture_2d(TextureSampleType::Float { filterable: true });
    let signal_layout = BindGroupLayoutDescriptor::new(
        "libfpv_video_signal",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer_sized(false, None),
                tex(),
                sampler(SamplerBindingType::Filtering),
                tex(),
                tex(),
                tex(),
            ),
        ),
    );
    let present_layout = BindGroupLayoutDescriptor::new(
        "libfpv_video_present",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                uniform_buffer_sized(false, None),
                tex(),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let linear = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        ..default()
    });
    let signal_shader = load_embedded_asset!(assets.as_ref(), "video_signal.wgsl");
    let present_shader = load_embedded_asset!(assets.as_ref(), "video_present.wgsl");
    let pipeline = |layout: &BindGroupLayoutDescriptor,
                    shader: &Handle<Shader>,
                    entry: &'static str,
                    defs: Vec<ShaderDefVal>,
                    format: TextureFormat| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("libfpv_video_pipeline".into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: shader.clone(),
                shader_defs: defs,
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            ..default()
        })
    };
    let signal = [
        pipeline(
            &signal_layout,
            &signal_shader,
            "signal",
            vec![],
            SIGNAL_FORMAT,
        ),
        pipeline(
            &signal_layout,
            &signal_shader,
            "signal",
            vec!["VIDEO_ANALOG".into()],
            SIGNAL_FORMAT,
        ),
        pipeline(
            &signal_layout,
            &signal_shader,
            "signal",
            vec!["VIDEO_DIGITAL".into()],
            SIGNAL_FORMAT,
        ),
    ];
    let present_hdr = pipeline(
        &present_layout,
        &present_shader,
        "present",
        vec![],
        ViewTarget::TEXTURE_FORMAT_HDR,
    );
    let present_sdr = pipeline(
        &present_layout,
        &present_shader,
        "present",
        vec![],
        TextureFormat::bevy_default(),
    );
    commands.insert_resource(Pipelines {
        signal_layout,
        present_layout,
        linear,
        signal,
        present_hdr,
        present_sdr,
    });
}

/// A texture with its default view.
struct Tex {
    texture: Texture,
    view: TextureView,
}

fn new_texture(device: &RenderDevice, w: u32, h: u32, format: TextureFormat, target: bool) -> Tex {
    let mut usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    if target {
        usage |= TextureUsages::RENDER_ATTACHMENT;
    }
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("libfpv_video"),
        size: Extent3d {
            width: w.max(1),
            height: h.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&TextureViewDescriptor::default());
    Tex { texture, view }
}

fn upload(queue: &RenderQueue, tex: &Tex, data: &[u8], w: u32, h: u32) {
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &tex.texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        data,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: None,
        },
        Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

/// The goggles' state across frames, and this frame's plan for the node.
#[derive(Resource, Default)]
struct Gpu {
    signal: Vec<Tex>,
    raster: (u32, u32),
    cur: usize,
    have: bool,
    field: u32,
    flags: u32,
    lines: Option<Tex>,
    osd: Option<(Tex, *const (Vec<u8>, u32))>,
    blank: Option<Tex>,
    plan: Option<Plan>,
}

// SAFETY: the OSD picture's pointer is only compared, never dereferenced, to tell a new picture.
unsafe impl Send for Gpu {}
unsafe impl Sync for Gpu {}

/// What the node draws this frame.
struct Plan {
    /// Draw the signal into `signal[next]` with this mode, `signal[1 - next]` as history.
    draw_signal: Option<(usize, usize)>,
    signal_uniform: Buffer,
    present_uniform: Buffer,
}

fn uniform_buffer<T: ShaderType + bevy::render::render_resource::encase::internal::WriteInto>(
    device: &RenderDevice,
    value: &T,
) -> Buffer {
    let mut bytes = UniformBuffer::new(Vec::new());
    bytes.write(value).expect("uniform fits");
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("libfpv_video_uniform"),
        contents: bytes.as_ref(),
        usage: BufferUsages::UNIFORM,
    })
}

fn prepare(
    views: Query<(&VideoView, &ViewTarget)>,
    mut gpu: ResMut<Gpu>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    gpu.plan = None;
    let Some((v, target)) = views.iter().next() else {
        return;
    };
    let size = target.main_texture().size();
    let (fb_w, fb_h) = (size.width as f32, size.height as f32);
    if fb_w < 1.0 || fb_h < 1.0 {
        return;
    }
    let f = &v.feed;
    let active = if f.system == ffi::FPV_VIDEO_PAL {
        288.0
    } else {
        240.0
    };
    let total = if f.system == ffi::FPV_VIDEO_PAL {
        313.0
    } else {
        263.0
    };

    // The picture on the screen: the camera's aspect with black bars, or all of it.
    let (mut rx, mut ry, mut rw, mut rh) = (0.0, 0.0, fb_w, fb_h);
    let mut pic_aspect = fb_w / fb_h;
    if v.res && f.picture_aspect > 0.1 {
        // Shown at the goggles' (or DVR's) aspect: the camera's picture stretched, not cropped.
        let shown = if f.display_aspect > 0.1 {
            f.display_aspect
        } else {
            f.picture_aspect
        };
        pic_aspect = f.picture_aspect;
        if rw / rh > shown {
            rw = rh * shown;
            rx = (fb_w - rw) * 0.5;
        } else {
            rh = rw / shown;
            ry = (fb_h - rh) * 0.5;
        }
    }
    // The signal's raster: an analog field, a digital frame, or the screen's pixels.
    let (mut w, mut h) = (rw as u32, rh as u32);
    if v.res && v.mode == MODE_ANALOG {
        (w, h) = (f.raster[0], active as u32);
    } else if v.res && v.mode == MODE_DIGITAL {
        (w, h) = (f.raster[0], f.raster[1]);
    }
    let (w, h) = (w.max(1), h.max(1));

    if gpu.raster != (w, h) || gpu.signal.len() != 2 {
        gpu.signal = (0..2)
            .map(|_| new_texture(&device, w, h, SIGNAL_FORMAT, true))
            .collect();
        gpu.raster = (w, h);
        gpu.have = false;
    }
    if gpu.blank.is_none() {
        let blank = new_texture(&device, 1, 1, SIGNAL_FORMAT, false);
        upload(&queue, &blank, &[0, 0, 0, 0], 1, 1);
        gpu.blank = Some(blank);
    }
    if gpu.lines.is_none() {
        gpu.lines = Some(new_texture(
            &device,
            2,
            ffi::FPV_VIDEO_MAX_LINES as u32,
            SIGNAL_FORMAT,
            false,
        ));
    }

    // A new field (frame), a change of settings, or no effects at all: draw the signal. The
    // goggles hold a frozen digital frame.
    let fresh = !gpu.have || v.flags == 0 || f.field != gpu.field || v.flags != gpu.flags;
    let hold = v.mode == MODE_DIGITAL && v.noise && f.frozen && gpu.have;
    let mut draw_signal = None;
    if fresh && !hold {
        if let Some(osd) = v.osd.as_ref() {
            let ptr = Arc::as_ptr(osd);
            let (pixels, lines) = (&osd.0, osd.1);
            let stale = gpu.osd.as_ref().is_none_or(|(_, p)| *p != ptr);
            if stale {
                let tex = match gpu.osd.take() {
                    Some((t, _)) if t.texture.height() == lines => t,
                    _ => new_texture(&device, OSD_W, lines, SIGNAL_FORMAT, false),
                };
                upload(&queue, &tex, pixels, OSD_W, lines);
                gpu.osd = Some((tex, ptr));
            }
        }
        if v.noise && v.mode == MODE_ANALOG {
            if let Some(lines) = gpu.lines.as_ref() {
                let n = v.line_count.min(ffi::FPV_VIDEO_MAX_LINES as u32);
                upload(&queue, lines, &v.lines[..(n as usize * 8)], 2, n);
            }
        }
        let next = if gpu.have { 1 - gpu.cur } else { gpu.cur };
        draw_signal = Some((v.mode, next));
        gpu.cur = next;
        gpu.have = true;
        gpu.field = f.field;
        gpu.flags = v.flags;
    }

    let or_default = |x: f32, d: f32| if x > 1.0 { x } else { d };
    let osd_rows = f.osd_rows.max(1) as f32;
    let signal = SignalUniform {
        raster: Vec2::new(w as f32, h as f32),
        k: v.k,
        digital: Vec2::new(
            if f.raster[0] > 0 {
                f.raster[0] as f32
            } else {
                w as f32
            },
            if f.raster[1] > 0 {
                f.raster[1] as f32
            } else {
                h as f32
            },
        ),
        noise: f32::from(u8::from(v.noise)),
        res: f32::from(u8::from(v.res)),
        osd_in: f32::from(u8::from(v.osd_in)),
        pic_aspect,
        scene_aspect: fb_w / fb_h,
        zoom: v.zoom,
        luma_cycles: or_default(f.luma_cycles, 250.0),
        chroma_cycles: or_default(f.chroma_cycles, 53.0),
        subcarrier: or_default(f.subcarrier, 188.5),
        sharpen: f.sharpening,
        parity: f.parity as f32,
        roll: f.roll,
        seed: f.seed,
        active_lines: active,
        total_lines: total,
        ntsc: f32::from(u8::from(f.system == ffi::FPV_VIDEO_NTSC)),
        blocks: f.block_errors,
        softness: if v.noise { f.softness } else { 0.0 },
        osd_lines: GLYPH_H as f32 * osd_rows,
        _pad: 0.0,
        interf: Vec4::from_array(f.interference),
        upset: if v.noise {
            Vec4::from_array(f.upset)
        } else {
            Vec4::ZERO
        },
    };
    let present = PresentUniform {
        rect: Vec4::new(rx, ry, rw, rh),
        rows: Vec2::new(
            if v.res && v.mode == MODE_ANALOG {
                active
            } else {
                0.0
            },
            f.parity as f32,
        ),
        tex_size: Vec2::new(w as f32, h as f32),
    };
    gpu.plan = Some(Plan {
        draw_signal,
        signal_uniform: uniform_buffer(&device, &signal),
        present_uniform: uniform_buffer(&device, &present),
    });
}

#[derive(Default)]
struct VideoNode;

impl ViewNode for VideoNode {
    type ViewQuery = (&'static ViewTarget, &'static VideoView);

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        ctx: &mut RenderContext,
        (target, _view): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let gpu = world.resource::<Gpu>();
        let (Some(plan), Some(blank), Some(lines)) = (&gpu.plan, &gpu.blank, &gpu.lines) else {
            return Ok(());
        };
        let Some(pipes) = world.get_resource::<Pipelines>() else {
            return Ok(());
        };
        let cache = world.resource::<PipelineCache>();
        let present_id = if target.is_hdr() {
            pipes.present_hdr
        } else {
            pipes.present_sdr
        };
        let Some(present) = cache.get_render_pipeline(present_id) else {
            return Ok(());
        };
        let post = target.post_process_write();

        if let Some((mode, next)) = plan.draw_signal {
            if let Some(signal_pipeline) = cache.get_render_pipeline(pipes.signal[mode]) {
                let osd = gpu.osd.as_ref().map_or(&blank.view, |(t, _)| &t.view);
                let history = &gpu.signal[1 - next].view;
                let bind = ctx.render_device().create_bind_group(
                    "libfpv_video_signal",
                    &cache.get_bind_group_layout(&pipes.signal_layout),
                    &BindGroupEntries::sequential((
                        plan.signal_uniform.as_entire_binding(),
                        post.source,
                        &pipes.linear,
                        osd,
                        &lines.view,
                        history,
                    )),
                );
                let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("libfpv_video_signal"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &gpu.signal[next].view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations::default(),
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_render_pipeline(signal_pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        let bind = ctx.render_device().create_bind_group(
            "libfpv_video_present",
            &cache.get_bind_group_layout(&pipes.present_layout),
            &BindGroupEntries::sequential((
                plan.present_uniform.as_entire_binding(),
                &gpu.signal[gpu.cur].view,
                &pipes.linear,
            )),
        );
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("libfpv_video_present"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(present);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> Box<FpvFrame> {
        let mut f = FpvFrame::new();
        f.camera.fov_y_degrees = 100.0;
        f.camera.aspect_hint = 4.0 / 3.0;
        f.camera.distortion_zoom = 1.0;
        f
    }

    #[test]
    fn nothing_to_draw_without_lens_or_effects() {
        let (view, fov) = view_for(&frame(), 16.0 / 9.0, &mut OsdChipCache::default());
        assert!(view.is_none());
        assert!((fov.to_degrees() - 100.0).abs() < 1e-4);
    }

    #[test]
    fn analog_noise_picks_the_analog_mode_and_keeps_libfpvs_fov_with_resolution() {
        let mut f = frame();
        f.camera.distortion_k1 = 0.3;
        f.video_feed.flags = ffi::FPV_VIDEO_FEED_NOISE | ffi::FPV_VIDEO_FEED_RESOLUTION;
        f.video_feed.system = ffi::FPV_VIDEO_NTSC;
        f.video_feed.picture_aspect = 4.0 / 3.0;
        let (view, fov) = view_for(&f, 16.0 / 9.0, &mut OsdChipCache::default());
        let view = view.expect("a view");
        assert_eq!(view.mode, MODE_ANALOG);
        assert!(view.noise && view.res);
        assert!(
            (fov.to_degrees() - 100.0).abs() < 1e-4,
            "exact at the camera's aspect"
        );
    }

    #[test]
    fn the_osd_chip_copies_a_glyph_and_keeps_its_picture_until_the_grid_changes() {
        let mut cache = OsdChipCache::default();
        // A 16 × 16-glyph font, glyph 'A' (65) all white.
        let (fw, fh) = (16 * GLYPH_W as u32, 16 * GLYPH_H as u32);
        let mut font = vec![0u8; (fw * fh * 4) as usize];
        let g = 65usize;
        for y in 0..GLYPH_H {
            for x in 0..GLYPH_W {
                let i = (((g / 16) * GLYPH_H + y) * fw as usize + (g % 16) * GLYPH_W + x) * 4;
                font[i..i + 4].copy_from_slice(&[255; 4]);
            }
        }
        cache.refresh_font(|_, _| Some((font.clone(), fw, fh, 1)));
        let mut f = frame();
        f.osd[0] = 65;
        let pic = osd_chip(&f, 13, &mut cache).expect("a picture");
        assert_eq!(pic.1, 13 * GLYPH_H as u32);
        assert_eq!(&pic.0[0..4], &[255; 4], "row 0, column 0 is the glyph");
        assert_eq!(
            &pic.0[(GLYPH_W * 4)..(GLYPH_W * 4 + 4)],
            &[0; 4],
            "column 1 is empty"
        );
        let again = osd_chip(&f, 13, &mut cache).expect("a picture");
        assert!(
            Arc::ptr_eq(&pic, &again),
            "unchanged grid: the same picture"
        );
        f.osd[1] = 65;
        let changed = osd_chip(&f, 13, &mut cache).expect("a picture");
        assert!(!Arc::ptr_eq(&pic, &changed));
    }
}
