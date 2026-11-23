//! M4's live flight in the world viewer: libfpv on its own real-time thread, reading the pilot's
//! radio, with the shared data folder (the radio setup and tune follow the pilot from game to
//! game) and the Configurator on. The drone starts on the ground at a site, and the world camera
//! is its FPV camera.
//!
//! `benilla-libfpv --worldview --fly [site]`. Keys: R respawns (repair, then back to the start).
//! The OSD and the F8 overlay come in M5, the lens in M6.

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_world::build_id::BuildId;
use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::WorldLoadProgress;
use benilla_world::view::{CameraPoseSet, FovZoom, ViewDistance, WorldCamera};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::camera;
use crate::ffi::{self, FpvFrame};
use crate::roll_test::{clearest_heading, world_ready, Site};
use crate::runtime::{dll_path, InitOptions, Libfpv, RayHit};

/// Frames are ignored until `sim_time` is this far past its first reading.
const STARTUP_SECS: f64 = 0.05;

/// Wall seconds the world must read ready before the drone is made.
const SETTLE_SECS: f32 = 3.0;

/// How often the drone's state goes to the log, wall seconds.
const LOG_EVERY: f32 = 2.0;

pub struct FlyPlugin(pub Site);

impl Plugin for FlyPlugin {
    fn build(&self, app: &mut App) {
        info!("libfpv: flying at `{}`", self.0.name);
        app.insert_resource(Fly {
            site: self.0,
            phase: Phase::Settle { ready_since: None },
            fpv: None,
            frame: FpvFrame::new(),
            spawn: Transform::IDENTITY,
            first_sim: None,
            next_log: 0.0,
        })
        // The camera contract (NOTES, M1): after the viewer's `fly`, before `CameraPoseSet`.
        .add_systems(
            Update,
            drive
                .after(WorldStage::Input)
                .before(CameraPoseSet)
                .before(WorldStage::Stream),
        );
    }
}

#[derive(Resource)]
struct Fly {
    site: Site,
    phase: Phase,
    fpv: Option<Libfpv>,
    frame: Box<FpvFrame>,
    /// Where the drone starts, frame bottom on the ground, and its heading.
    spawn: Transform,
    first_sim: Option<f64>,
    next_log: f32,
}

enum Phase {
    Settle { ready_since: Option<f32> },
    Flying,
    Failed,
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time<bevy::time::Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fly: ResMut<Fly>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    build: Option<Res<BuildId>>,
    mut camera: Query<(&mut Transform, &Camera), With<WorldCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut zoom: ResMut<FovZoom>,
    mut view: ResMut<ViewDistance>,
) {
    let Ok((mut cam, camera_info)) = camera.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let fly = &mut *fly;
    let site = fly.site;
    let mut world = |o: Vec3, d: Dir3, max: f32| {
        collision.ray_body(o, d, max).map(|h| RayHit {
            distance: h.distance,
            normal: h.normal,
            material: 0,
        })
    };

    match fly.phase {
        Phase::Failed => {}
        Phase::Settle {
            ref mut ready_since,
        } => {
            // Hover over the start while it streams in.
            let above = wow_to_bevy([site.xy[0], site.xy[1], site.probe_z]);
            *cam = Transform::from_translation(above).looking_to(Vec3::NEG_Z, Vec3::Y);
            if !world_ready(progress.as_deref()) {
                *ready_since = None;
                return;
            }
            if now - *ready_since.get_or_insert(now) < SETTLE_SECS {
                return;
            }
            match start(&site, &collision, build.as_deref(), &mut world) {
                Ok((fpv, spawn)) => {
                    fly.fpv = Some(fpv);
                    fly.spawn = spawn;
                    fly.phase = Phase::Flying;
                }
                Err(e) => {
                    error!("libfpv: cannot fly — {e}");
                    fly.phase = Phase::Failed;
                }
            }
        }
        Phase::Flying => {
            let Some(fpv) = fly.fpv.as_mut() else { return };
            if keys.just_pressed(KeyCode::KeyR) {
                let (p, q) = (fly.spawn.translation, fly.spawn.rotation);
                if let Err(e) = fpv
                    .repair()
                    .and_then(|()| fpv.reset_drone(p, q, &mut world))
                {
                    error!("libfpv: respawn — {e}");
                } else {
                    info!("libfpv: respawned at the start");
                }
            }
            fpv.set_view(ffi::FPV_VIEW_FPV);
            let dt = time.delta_secs();
            if let Err(e) = fpv.tick(dt, &mut fly.frame, &mut world) {
                error!("libfpv: {e}");
                return;
            }
            // Every frame, also while closed: the collision view draws through it (M5 draws it).
            if let Ok(win) = window.single() {
                fpv.set_overlay_display(win.width(), win.height(), 1.0);
            }
            let _ = fpv.overlay(dt);

            let frame = &*fly.frame;
            let first = *fly.first_sim.get_or_insert(frame.sim_time);
            if frame.sim_time < first + STARTUP_SECS {
                return;
            }
            let near = camera::apply(
                &frame.camera,
                &mut cam,
                camera_info,
                &mut zoom,
                &mut view,
                &collision,
            );
            if now >= fly.next_log {
                fly.next_log = now + LOG_EVERY;
                log_state(frame, near);
            }
        }
    }
}

/// Inits libfpv for live flight and makes the drone on the ground at the site.
fn start(
    site: &Site,
    collision: &WorldCollision,
    build: Option<&BuildId>,
    world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
) -> Result<(Libfpv, Transform), String> {
    let top = wow_to_bevy([site.xy[0], site.xy[1], site.probe_z]);
    let ground = collision
        .ray_body(top, Dir3::NEG_Y, 1000.0)
        .map(|h| top - Vec3::Y * h.distance)
        .ok_or("no ground under the start")?;
    let heading = match site.heading {
        Some(h) => h,
        None => clearest_heading(collision, ground + Vec3::Y).0,
    };
    let facing = Quat::from_rotation_y(-heading.to_radians());

    let build = build.map_or_else(|| env!("CARGO_PKG_VERSION").to_owned(), |b| b.summary());
    let mut fpv = Libfpv::init(
        &dll_path(),
        &InitOptions {
            manual_timing: false,
            // No lens pass until M6: keep libfpv's FOV unwidened.
            flags: ffi::FPV_INIT_NO_LENS_DISTORTION,
            host_build: build,
        },
    )?;
    let behind = ground - facing * Vec3::NEG_Z * 2.0;
    fpv.create_drone(ground, facing, behind, world)?;
    info!(
        "libfpv: drone on the ground at WoW {:.2?}, heading {heading:.0}° — arm and fly; R \
         respawns",
        bevy_to_wow(ground)
    );
    Ok((
        fpv,
        Transform::from_translation(ground).with_rotation(facing),
    ))
}

fn log_state(frame: &FpvFrame, near: camera::NearChoice) {
    info!(
        "libfpv: pos WoW {:.1?} {:.0} km/h alt {:.2} armed {} angle {} fov {:.1}° near {:.3} yd \
         (closest {:?}) battery {:.2} V arm-text {:?} impacts {}",
        bevy_to_wow(Vec3::from_array(frame.position)),
        frame.speed_kmh,
        frame.altitude,
        frame.flags & ffi::FPV_DRONE_ARMED != 0,
        frame.flags & ffi::FPV_DRONE_MODE_ANGLE != 0,
        frame.camera.fov_y_degrees,
        near.near,
        near.closest.map(|c| (c * 1000.0).round() / 1000.0),
        frame.battery.voltage,
        frame.arming_disable_text(),
        frame.impact_count
    );
}
