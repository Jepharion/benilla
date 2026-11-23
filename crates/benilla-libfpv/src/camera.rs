//! The world camera at one of libfpv's cameras: position and rotation as-is (libfpv's rotation
//! maps the host's forward, -Z, to the view, which is Bevy's camera rotation), the vertical FOV
//! through benilla's [`FovZoom`], and a near plane chosen per frame from collision probes.
//!
//! Benilla stamps the projection every frame from `FovZoom` and `ViewDistance.nearclip`, so this
//! never writes `Projection` itself.

use benilla_world::collision::WorldCollision;
use benilla_world::view::{world_fovy, FovZoom, ViewDistance};
use bevy::prelude::*;

use crate::ffi::FpvFrame;
use crate::runtime::UNITS_PER_METER;
use crate::video::{self, OsdChipCache, VideoView};

/// INTEGRATION.md "Coordinates and the camera": the near plane is 0.8 of the closest depth the
/// probes find along the view axis, clamped to 0.02–0.2 m.
const NEAR_FRACTION: f32 = 0.8;
const NEAR_MIN_M: f32 = 0.02;
const NEAR_MAX_M: f32 = 0.2;

/// What the near plane probes chose this frame, for the log.
#[derive(Clone, Copy, Debug, Default)]
pub struct NearChoice {
    /// The near plane set, yards.
    pub near: f32,
    /// The closest depth a probe found along the view axis, yards; `None` = nothing within reach.
    pub closest: Option<f32>,
}

/// What [`apply`] set: the near plane, the vertical FOV rendered at (degrees), and the view
/// through the goggles (lens and video feed), `None` when there is nothing to draw.
#[derive(Clone, Default)]
pub struct Applied {
    pub near: NearChoice,
    pub fov_deg: f32,
    pub view: Option<VideoView>,
}

/// Puts the world camera at `frame.camera`: transform, FOV (as [`video::view_for`] decides it)
/// and near plane. The caller puts `view` on the camera ([`set_view`]).
pub fn apply(
    frame: &FpvFrame,
    transform: &mut Transform,
    camera: &Camera,
    zoom: &mut FovZoom,
    view: &mut ViewDistance,
    collision: &WorldCollision,
    osd_chip: &mut OsdChipCache,
) -> Applied {
    let cam = &frame.camera;
    transform.translation = Vec3::from_array(cam.position);
    transform.rotation = Quat::from_array(cam.rotation).normalize();
    let aspect = camera
        .logical_viewport_size()
        .map_or(16.0 / 9.0, |s| s.x / s.y.max(1.0));
    let (video, fov_y) = video::view_for(frame, aspect, osd_chip);
    zoom.0 = fov_y / world_fovy(aspect);
    // The probes cover what is rendered: the widened frustum the lens then warps.
    let near = near_plane(transform, fov_y, aspect, collision);
    view.set_nearclip(near.near);
    Applied {
        near,
        fov_deg: fov_y.to_degrees(),
        view: video,
    }
}

/// Puts this frame's view through the goggles on the world camera, or takes it off.
pub fn set_view(commands: &mut Commands, camera: Entity, view: Option<VideoView>) {
    match view {
        Some(v) => {
            commands.entity(camera).insert(v);
        }
        None => {
            commands.entity(camera).remove::<VideoView>();
        }
    }
}

/// Casts rays from the eye through the frustum's centre, edge midpoints and corners, and
/// returns the near plane: 0.8 of the closest depth along the view axis, clamped to 0.02–0.2 m
/// (benilla clamps its own range, 0.01–0.33 yd, which holds this one).
fn near_plane(eye: &Transform, fov_y: f32, aspect: f32, collision: &WorldCollision) -> NearChoice {
    let (fwd, right, up) = (*eye.forward(), *eye.right(), *eye.up());
    let ty = (fov_y * 0.5).tan();
    let tx = ty * aspect;
    let (min, max) = (NEAR_MIN_M * UNITS_PER_METER, NEAR_MAX_M * UNITS_PER_METER);
    // Far enough that anything nearer would pull the plane in.
    let reach = max / NEAR_FRACTION * 2.0;
    let mut closest: Option<f32> = None;
    for sy in [-1.0f32, 0.0, 1.0] {
        for sx in [-1.0f32, 0.0, 1.0] {
            let along = fwd + right * (sx * tx) + up * (sy * ty);
            let Ok(dir) = Dir3::new(along) else { continue };
            // The ray's length per unit of depth along the view axis.
            let per_depth = along.length();
            let Some(hit) = collision.ray_body(eye.translation, dir, reach * per_depth) else {
                continue;
            };
            let depth = hit.distance / per_depth;
            closest = Some(closest.map_or(depth, |c: f32| c.min(depth)));
        }
    }
    let near = closest.map_or(max, |d| (d * NEAR_FRACTION).clamp(min, max));
    NearChoice { near, closest }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clamp_sits_inside_benillas_own_nearclip_range() {
        let range = benilla_world::view::NEARCLIP_RANGE;
        assert!(range.contains(&(NEAR_MIN_M * UNITS_PER_METER)));
        assert!(range.contains(&(NEAR_MAX_M * UNITS_PER_METER)));
    }
}
