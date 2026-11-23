//! libfpv's draw data on screen: the OSD and the F8 overlay (and the collision view, which draws
//! through the overlay), as textured triangles, the way `docs/renderers/fpv_render_gl3.h` draws
//! them, in a render-graph node of this crate's own.
//!
//! - A 2D camera above the world camera ([`OVERLAY_ORDER`]), marked [`FpvOverlayCamera`],
//!   clears its own texture to transparent, has no tonemapping (it would shift the UI's
//!   colours), and composites onto the window with premultiplied alpha: straight-alpha
//!   triangles drawn over transparent black leave premultiplied colour.
//! - Its node ([`FpvOverlayNode`], after the 2D main pass) uploads libfpv's vertices as they
//!   come (`fpv_draw_vertex` is a vertex layout as it stands: position, uv, RGBA8 colour) and
//!   draws each command with its scissor rectangle, its texture and its base vertex, blending as
//!   the reference does. Its pipeline is queued at startup, so it compiles behind the loading,
//!   never on the frame that first shows the overlay.
//! - libfpv's textures are uploaded once per revision as sRGB textures, the OSD font sampled
//!   nearest (pixel art, as the reference keeps it) and the overlay font linearly.
//!
//! Whoever owns libfpv fills [`FpvDraw`] each frame: the layers to draw, bottom first, and any
//! texture whose revision changed.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::camera::{CameraOutputMode, ClearColorConfig};
use bevy::core_pipeline::core_2d::graph::{Core2d, Node2d};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::query::QueryItem;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_graph::{
    NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use bevy::render::view::ViewTarget;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

use crate::ffi;
use crate::runtime::DrawData;

/// The overlay camera's order: above the world camera (0).
const OVERLAY_ORDER: isize = 100;

/// What to draw this frame, filled by whoever owns libfpv.
#[derive(Resource, Default)]
pub struct FpvDraw {
    /// Draw data, bottom first: the OSD, then the overlay.
    pub layers: Vec<DrawData>,
    /// Textures to (re)upload: id, straight-alpha RGBA, width, height.
    pub uploads: Vec<(u64, Vec<u8>, u32, u32)>,
    /// The revision of each texture id already handed over, so a producer copies a texture only
    /// when it changes.
    pub known: [Option<u32>; 3],
}

impl FpvDraw {
    /// Queues libfpv's two font textures when their revision moved.
    pub fn refresh_textures(
        &mut self,
        get: impl Fn(u64, Option<u32>) -> Option<(Vec<u8>, u32, u32, u32)>,
    ) {
        for id in [ffi::FPV_TEXTURE_OVERLAY_FONT, ffi::FPV_TEXTURE_OSD_FONT] {
            if let Some((rgba, w, h, rev)) = get(id, self.known[id as usize]) {
                self.known[id as usize] = Some(rev);
                self.uploads.push((id, rgba, w, h));
            }
        }
    }
}

/// The camera the OSD and the overlay draw on.
#[derive(Component, Clone, Copy, Default, ExtractComponent)]
pub struct FpvOverlayCamera;

pub struct FpvDrawPlugin;

impl Plugin for FpvDrawPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "draw.wgsl");
        app.add_plugins(ExtractComponentPlugin::<FpvOverlayCamera>::default())
            .init_resource::<FpvDraw>()
            .add_systems(Startup, spawn_overlay_camera)
            // The uploads were extracted at the end of the last frame.
            .add_systems(First, |mut draw: ResMut<FpvDraw>| draw.uploads.clear());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<Extracted>()
            .init_resource::<Gpu>()
            .add_systems(RenderStartup, init_pipeline)
            .add_systems(ExtractSchedule, extract)
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_render_graph_node::<ViewNodeRunner<FpvOverlayNode>>(Core2d, FpvOverlayLabel)
            .add_render_graph_edges(
                Core2d,
                (
                    Node2d::EndMainPass,
                    FpvOverlayLabel,
                    Node2d::StartMainPassPostProcessing,
                ),
            );
    }
}

fn spawn_overlay_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            order: OVERLAY_ORDER,
            // Its own texture starts transparent each frame...
            clear_color: ClearColorConfig::Custom(Color::NONE),
            // ...and lands on the world's picture: straight-alpha triangles drawn over
            // transparent black leave premultiplied colour.
            output_mode: CameraOutputMode::Write {
                blend_state: Some(BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                clear_color: ClearColorConfig::None,
            },
            ..default()
        },
        Tonemapping::None,
        Msaa::Off,
        FpvOverlayCamera,
        Name::new("libfpv overlay camera"),
    ));
}

// ── Render world ──────────────────────────────────────────────────────────────────────────────

/// This frame's draw data and texture uploads, copied from the main world.
#[derive(Resource, Default)]
struct Extracted {
    layers: Vec<DrawData>,
    uploads: Vec<(u64, Vec<u8>, u32, u32)>,
}

fn extract(draw: Extract<Res<FpvDraw>>, mut out: ResMut<Extracted>) {
    out.layers.clone_from(&draw.layers);
    out.uploads.extend(draw.uploads.iter().cloned());
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct FpvOverlayLabel;

#[derive(Resource)]
struct Pipeline {
    display_layout: BindGroupLayoutDescriptor,
    texture_layout: BindGroupLayoutDescriptor,
    linear: Sampler,
    nearest: Sampler,
    /// For an 8-bit view target (the overlay camera's) and an HDR one.
    sdr: CachedRenderPipelineId,
    hdr: CachedRenderPipelineId,
}

/// `fpv_draw_vertex`: position, uv, RGBA8 colour (R lowest), 20 bytes.
const VERTEX_STRIDE: u64 = std::mem::size_of::<ffi::FpvDrawVertex>() as u64;

fn init_pipeline(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    cache: Res<PipelineCache>,
) {
    let display_layout = BindGroupLayoutDescriptor::new(
        "libfpv_overlay_display",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer_sized(false, None)),
    );
    let texture_layout = BindGroupLayoutDescriptor::new(
        "libfpv_overlay_texture",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let sampler_with = |filter: FilterMode| {
        device.create_sampler(&SamplerDescriptor {
            mag_filter: filter,
            min_filter: filter,
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..default()
        })
    };
    let shader = load_embedded_asset!(assets.as_ref(), "draw.wgsl");
    let vertex_layout = VertexBufferLayout {
        array_stride: VERTEX_STRIDE,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![
            VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: std::mem::offset_of!(ffi::FpvDrawVertex, pos) as u64,
                shader_location: 0,
            },
            VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: std::mem::offset_of!(ffi::FpvDrawVertex, uv) as u64,
                shader_location: 1,
            },
            VertexAttribute {
                format: VertexFormat::Unorm8x4,
                offset: std::mem::offset_of!(ffi::FpvDrawVertex, color) as u64,
                shader_location: 2,
            },
        ],
    };
    // libfpv's blend: straight alpha over, `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` on colour and `ONE,
    // ONE_MINUS_SRC_ALPHA` on alpha (fpv_render_gl3.h).
    let blend = BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::SrcAlpha,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent::OVER,
    };
    let queue = |format: TextureFormat| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("libfpv_overlay_pipeline".into()),
            layout: vec![display_layout.clone(), texture_layout.clone()],
            vertex: VertexState {
                shader: shader.clone(),
                entry_point: Some("vertex".into()),
                buffers: vec![vertex_layout.clone()],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: shader.clone(),
                entry_point: Some("fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format,
                    blend: Some(blend),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            primitive: PrimitiveState {
                cull_mode: None,
                ..default()
            },
            ..default()
        })
    };
    let sdr = queue(TextureFormat::bevy_default());
    let hdr = queue(ViewTarget::TEXTURE_FORMAT_HDR);
    commands.insert_resource(Pipeline {
        display_layout,
        texture_layout,
        linear: sampler_with(FilterMode::Linear),
        nearest: sampler_with(FilterMode::Nearest),
        sdr,
        hdr,
    });
}

/// One command, ready to draw.
struct Draw {
    /// Its layer's display bind group, an index into `Gpu::displays`.
    layer: usize,
    /// The clip rectangle in display units, and the layer's display size.
    clip: [f32; 4],
    display: [f32; 2],
    texture: u64,
    indices: std::ops::Range<u32>,
    base_vertex: i32,
}

/// The GPU side: textures by id, this frame's buffers and draws.
#[derive(Resource, Default)]
struct Gpu {
    textures: [Option<BindGroup>; 3],
    white: Option<BindGroup>,
    vertices: Option<Buffer>,
    indices: Option<Buffer>,
    displays: Vec<BindGroup>,
    draws: Vec<Draw>,
}

/// A slice of plain `repr(C)` values as bytes.
fn bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: `FpvDrawVertex` (two `[f32; 2]` and a `u32`, no padding) and `u32` have no
    // uninitialised bytes; the length is the slice's own size in bytes.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), std::mem::size_of_val(v)) }
}

fn texture_group(
    device: &RenderDevice,
    queue: &RenderQueue,
    cache: &PipelineCache,
    pipeline: &Pipeline,
    rgba: &[u8],
    (w, h): (u32, u32),
    nearest: bool,
) -> BindGroup {
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("libfpv_overlay_texture"),
            size: Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        rgba,
    );
    let view = texture.create_view(&TextureViewDescriptor::default());
    let sampler = if nearest {
        &pipeline.nearest
    } else {
        &pipeline.linear
    };
    device.create_bind_group(
        "libfpv_overlay_texture",
        &cache.get_bind_group_layout(&pipeline.texture_layout),
        &BindGroupEntries::sequential((&view, sampler)),
    )
}

fn prepare(
    mut extracted: ResMut<Extracted>,
    mut gpu: ResMut<Gpu>,
    pipeline: Option<Res<Pipeline>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
) {
    let Some(pipeline) = pipeline else { return };
    if gpu.white.is_none() {
        gpu.white = Some(texture_group(
            &device,
            &queue,
            &cache,
            &pipeline,
            &[255; 4],
            (1, 1),
            true,
        ));
    }
    for (id, rgba, w, h) in extracted.uploads.drain(..) {
        if w == 0 || h == 0 || rgba.len() < (w * h * 4) as usize {
            continue;
        }
        let nearest = id == ffi::FPV_TEXTURE_OSD_FONT;
        if let Some(slot) = gpu.textures.get_mut(id as usize) {
            *slot = Some(texture_group(
                &device,
                &queue,
                &cache,
                &pipeline,
                &rgba,
                (w, h),
                nearest,
            ));
        }
    }

    // Every list's vertices and indices in one buffer each; each command keeps its own range.
    let (mut vertices, mut indices): (Vec<u8>, Vec<u8>) = (Vec::new(), Vec::new());
    let (mut displays, mut draws) = (Vec::new(), Vec::new());
    for data in &extracted.layers {
        let display = data.display_size;
        if display[0] <= 0.0 || display[1] <= 0.0 {
            continue;
        }
        let uniform: [f32; 4] = [display[0], display[1], 0.0, 0.0];
        let buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("libfpv_overlay_display"),
            contents: bytes(&uniform),
            usage: BufferUsages::UNIFORM,
        });
        displays.push(device.create_bind_group(
            "libfpv_overlay_display",
            &cache.get_bind_group_layout(&pipeline.display_layout),
            &BindGroupEntries::single(buffer.as_entire_binding()),
        ));
        let display_index = displays.len() - 1;
        for list in &data.lists {
            let vertex_base = (vertices.len() as u64 / VERTEX_STRIDE) as i32;
            let index_base = (indices.len() / 4) as u32;
            vertices.extend_from_slice(bytes(&list.vertices));
            indices.extend_from_slice(bytes(&list.indices));
            for cmd in &list.commands {
                let start = cmd.index_offset;
                let end = (start + cmd.index_count).min(list.indices.len() as u32);
                if end <= start {
                    continue;
                }
                draws.push(Draw {
                    layer: display_index,
                    clip: cmd.clip_rect,
                    display,
                    texture: cmd.texture_id,
                    indices: index_base + start..index_base + end,
                    base_vertex: vertex_base + cmd.vertex_offset as i32,
                });
            }
        }
    }
    gpu.vertices = (!vertices.is_empty()).then(|| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("libfpv_overlay_vertices"),
            contents: &vertices,
            usage: BufferUsages::VERTEX,
        })
    });
    gpu.indices = (!indices.is_empty()).then(|| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("libfpv_overlay_indices"),
            contents: &indices,
            usage: BufferUsages::INDEX,
        })
    });
    gpu.displays = displays;
    gpu.draws = draws;
}

#[derive(Default)]
struct FpvOverlayNode;

impl ViewNode for FpvOverlayNode {
    type ViewQuery = (&'static ViewTarget, &'static FpvOverlayCamera);

    fn run(
        &self,
        _graph: &mut RenderGraphContext,
        ctx: &mut RenderContext,
        (target, _): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let gpu = world.resource::<Gpu>();
        let (Some(vertices), Some(indices)) = (&gpu.vertices, &gpu.indices) else {
            return Ok(());
        };
        if gpu.draws.is_empty() {
            return Ok(());
        }
        let Some(pipeline) = world.get_resource::<Pipeline>() else {
            return Ok(());
        };
        let cache = world.resource::<PipelineCache>();
        let id = if target.is_hdr() {
            pipeline.hdr
        } else {
            pipeline.sdr
        };
        let Some(render_pipeline) = cache.get_render_pipeline(id) else {
            return Ok(());
        };
        let size = target.main_texture().size();
        let (tw, th) = (size.width, size.height);
        let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("libfpv_overlay_pass"),
            color_attachments: &[Some(target.get_color_attachment())],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(render_pipeline);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        for d in &gpu.draws {
            // The clip rectangle from display units to the target's pixels, clamped to it.
            let (sx, sy) = (tw as f32 / d.display[0], th as f32 / d.display[1]);
            let x1 = (d.clip[0] * sx).max(0.0) as u32;
            let y1 = (d.clip[1] * sy).max(0.0) as u32;
            let x2 = ((d.clip[2] * sx).max(0.0) as u32).min(tw);
            let y2 = ((d.clip[3] * sy).max(0.0) as u32).min(th);
            if x2 <= x1 || y2 <= y1 {
                continue;
            }
            let Some(texture) = gpu
                .textures
                .get(d.texture as usize)
                .and_then(Option::as_ref)
                .or(gpu.white.as_ref())
            else {
                continue;
            };
            let Some(display) = gpu.displays.get(d.layer) else {
                continue;
            };
            pass.set_scissor_rect(x1, y1, x2 - x1, y2 - y1);
            pass.set_bind_group(0, display, &[]);
            pass.set_bind_group(1, texture, &[]);
            pass.draw_indexed(d.indices.clone(), d.base_vertex, 0..1);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libfpvs_vertex_is_a_gpu_vertex_layout_as_it_stands() {
        // Two float pairs and a packed RGBA8 word, no padding: what the pipeline's layout reads.
        assert_eq!(VERTEX_STRIDE, 20);
        assert_eq!(std::mem::offset_of!(ffi::FpvDrawVertex, uv), 8);
        assert_eq!(std::mem::offset_of!(ffi::FpvDrawVertex, color), 16);
    }

    #[test]
    fn bytes_are_the_values_little_endian() {
        let v = [ffi::FpvDrawVertex {
            pos: [1.0, 0.0],
            uv: [0.0, 0.0],
            color: 0x4433_2211,
        }];
        let b = bytes(&v);
        assert_eq!(b.len(), 20);
        // R in the lowest byte, as Unorm8x4 reads it into .r.
        assert_eq!(&b[16..20], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(&b[0..4], &1.0f32.to_le_bytes());
    }
}
