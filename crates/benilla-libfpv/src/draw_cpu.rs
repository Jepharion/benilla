//! libfpv draw data (overlay, collision view, OSD) rasterised on the CPU onto an RGB8 frame:
//! a test instrument for captures until the GPU renderer (M5). Each command's triangles are
//! filled with the vertex colour times its texture, straight alpha over the frame, inside the
//! command's clip rect.

use crate::runtime::DrawData;

/// A straight-alpha RGBA8 texture.
pub struct Texture<'a> {
    pub rgba: &'a [u8],
    pub width: u32,
    pub height: u32,
}

impl Texture<'_> {
    /// Nearest texel at `uv`, as 0..1 floats.
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        if self.width == 0 || self.height == 0 {
            return [1.0; 4];
        }
        let x = ((u * self.width as f32) as i64).clamp(0, self.width as i64 - 1) as usize;
        let y = ((v * self.height as f32) as i64).clamp(0, self.height as i64 - 1) as usize;
        let i = (y * self.width as usize + x) * 4;
        let t = &self.rgba[i..i + 4];
        [t[0], t[1], t[2], t[3]].map(|c| c as f32 / 255.0)
    }
}

/// Draws `data` over `rgb` (`w` × `h`, RGB8), scaling display coordinates to the frame.
/// `texture(id)` gives the texture a command names; `None` draws it untextured.
pub fn rasterize<'t>(
    data: &DrawData,
    rgb: &mut [u8],
    w: u32,
    h: u32,
    texture: &dyn Fn(u64) -> Option<Texture<'t>>,
) {
    let sx = if data.display_size[0] > 0.0 {
        w as f32 / data.display_size[0]
    } else {
        1.0
    };
    let sy = if data.display_size[1] > 0.0 {
        h as f32 / data.display_size[1]
    } else {
        1.0
    };
    for list in &data.lists {
        for cmd in &list.commands {
            let tex = texture(cmd.texture_id);
            let clip = [
                (cmd.clip_rect[0] * sx).max(0.0),
                (cmd.clip_rect[1] * sy).max(0.0),
                (cmd.clip_rect[2] * sx).min(w as f32),
                (cmd.clip_rect[3] * sy).min(h as f32),
            ];
            let start = cmd.index_offset as usize;
            let end = (start + cmd.index_count as usize).min(list.indices.len());
            for tri in list.indices[start..end].as_chunks::<3>().0 {
                let v = tri.map(|i| list.vertices.get((i + cmd.vertex_offset) as usize).copied());
                let [Some(a), Some(b), Some(c)] = v else {
                    continue;
                };
                fill(rgb, w, clip, [a, b, c].map(|v| (v, sx, sy)), tex.as_ref());
            }
        }
    }
}

/// Fills one triangle, interpolating colour and uv barycentrically.
fn fill(
    rgb: &mut [u8],
    w: u32,
    clip: [f32; 4],
    v: [(crate::ffi::FpvDrawVertex, f32, f32); 3],
    tex: Option<&Texture>,
) {
    let p = v.map(|(v, sx, sy)| [v.pos[0] * sx, v.pos[1] * sy]);
    let area =
        (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
    if area.abs() < 1e-6 {
        return;
    }
    let x0 = p
        .iter()
        .map(|q| q[0])
        .fold(f32::MAX, f32::min)
        .max(clip[0])
        .floor() as i32;
    let x1 = p
        .iter()
        .map(|q| q[0])
        .fold(f32::MIN, f32::max)
        .min(clip[2])
        .ceil() as i32;
    let y0 = p
        .iter()
        .map(|q| q[1])
        .fold(f32::MAX, f32::min)
        .max(clip[1])
        .floor() as i32;
    let y1 = p
        .iter()
        .map(|q| q[1])
        .fold(f32::MIN, f32::max)
        .min(clip[3])
        .ceil() as i32;
    let col = v.map(|(v, ..)| [0, 8, 16, 24].map(|s| ((v.color >> s) & 0xff) as f32 / 255.0));
    let uv = v.map(|(v, ..)| v.uv);
    for y in y0.max(0)..y1 {
        for x in x0.max(0)..x1 {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let e = |i: usize, j: usize| {
                (p[j][0] - p[i][0]) * (py - p[i][1]) - (px - p[i][0]) * (p[j][1] - p[i][1])
            };
            let (w0, w1, w2) = (e(1, 2) / area, e(2, 0) / area, e(0, 1) / area);
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let mut c = [0.0f32; 4];
            for k in 0..4 {
                c[k] = col[0][k] * w0 + col[1][k] * w1 + col[2][k] * w2;
            }
            if let Some(t) = tex {
                let u = uv[0][0] * w0 + uv[1][0] * w1 + uv[2][0] * w2;
                let vv = uv[0][1] * w0 + uv[1][1] * w1 + uv[2][1] * w2;
                let s = t.sample(u, vv);
                for k in 0..4 {
                    c[k] *= s[k];
                }
            }
            let o = (y as usize * w as usize + x as usize) * 3;
            if o + 3 > rgb.len() {
                continue;
            }
            for k in 0..3 {
                let dst = rgb[o + k] as f32 / 255.0;
                rgb[o + k] = ((c[k] * c[3] + dst * (1.0 - c[3])) * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::{FpvDrawCmd, FpvDrawVertex};
    use crate::runtime::DrawList;

    #[test]
    fn an_opaque_red_triangle_covers_its_inside_only() {
        let v = |x: f32, y: f32| FpvDrawVertex {
            pos: [x, y],
            uv: [0.0, 0.0],
            color: 0xff00_00ff, // R in the lowest byte, A = 255
        };
        let data = DrawData {
            display_size: [8.0, 8.0],
            lists: vec![DrawList {
                vertices: vec![v(0.0, 0.0), v(8.0, 0.0), v(0.0, 8.0)],
                indices: vec![0, 1, 2],
                commands: vec![FpvDrawCmd {
                    clip_rect: [0.0, 0.0, 8.0, 8.0],
                    texture_id: 0,
                    index_offset: 0,
                    index_count: 3,
                    vertex_offset: 0,
                }],
            }],
        };
        let mut rgb = vec![0u8; 8 * 8 * 3];
        rasterize(&data, &mut rgb, 8, 8, &|_| None);
        assert_eq!(&rgb[0..3], &[255, 0, 0], "top-left inside");
        let far = (7 * 8 + 7) * 3;
        assert_eq!(&rgb[far..far + 3], &[0, 0, 0], "bottom-right outside");
    }
}
