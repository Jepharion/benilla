//! M3's headless flights in the world viewer: libfpv in `FPV_TIMING_MANUAL`, no radio and no
//! Configurator, the drone on the ground of Stormwind's Trade District, scripted channels, and
//! benilla's collision answering libfpv's rays. Each scenario logs the drone every half second
//! of simulated time, prints `libfpv: TEST <name> PASS|FAIL …` and exits non-zero on a fail.
//!
//! - `park`: disarmed for 6 s, then armed on AUX1 with the throttle low. Passes when the
//!   disarmed drone stays within [`PARK_TOLERANCE_M`] of where it settled and arming works.
//! - `wall`: the nearest wall 5–12 yd away, the drone pushed at it at 10 m/s. Passes when it
//!   never passes the wall's plane, reaches it, and libfpv reports an impact facing back.
//! - `climb`: armed in angle mode at 1 s, a climb from 1.5 s, then about hover, seen through
//!   the FPV camera ([`crate::camera`]). Passes when it arms, climbs over a metre, the camera
//!   rides on the drone, and on the ground the picture is upright with the near plane probes
//!   finding the street.
//!
//! The camera follows libfpv's chase camera, or its FPV camera for `climb`. `--capture <dir>`
//! writes the view at the wall test's impact with the collision view drawn over it (run with
//! `LIBFPV_COLLISION_VIEW=xray`), and the climb's FPV view at [`CLIMB_SHOTS`].
//!
//! `LIBFPV_DLL` names the DLL and `LIBFPV_DATA_DIR` a scratch data folder, so a test never
//! touches the pilot's own setup in `%APPDATA%\libfpv`.

use std::path::PathBuf;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_world::build_id::BuildId;
use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::WorldLoadProgress;
use benilla_world::view::{world_fovy, CameraPoseSet, FovZoom, ViewDistance, WorldCamera};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;

use crate::ffi::{self, FpvFrame};
use crate::roll_test::{world_ready, Site};
use crate::runtime::{dll_path, DrawData, InitOptions, Libfpv, RayHit, UNITS_PER_METER};

/// Simulated seconds per frame, whatever the wall clock does.
const DT: f32 = 1.0 / 60.0;

/// Frames are ignored until `sim_time` is this far past its first reading (INTEGRATION.md
/// "Spawning": the first frames can still show the origin).
const STARTUP_SECS: f64 = 0.05;

/// Wall seconds the world must read ready before the drone is made.
const SETTLE_SECS: f32 = 6.0;

/// The most a disarmed, parked drone may wander, metres.
const PARK_TOLERANCE_M: f32 = 0.05;

/// Where the tests fly: a Trade District street.
const SITE: &str = "streets";

/// The climb's FPV shots: simulated time and name. On the ground disarmed, early in the climb,
/// and holding near the top.
const CLIMB_SHOTS: [(f64, &str); 3] = [(0.8, "fpv-ground"), (2.2, "fpv-climb"), (4.5, "fpv-hold")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    Park,
    Wall,
    Climb,
}

impl Scenario {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "park" => Some(Self::Park),
            "wall" => Some(Self::Wall),
            "climb" => Some(Self::Climb),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Park => "park",
            Self::Wall => "wall",
            Self::Climb => "climb",
        }
    }

    /// Simulated seconds it runs for.
    fn length(self) -> f64 {
        match self {
            Self::Park => 10.0,
            Self::Wall => 4.0,
            Self::Climb => 5.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlightTest {
    pub scenario: Scenario,
    pub capture: Option<PathBuf>,
}

pub struct FlightTestPlugin(pub FlightTest);

impl Plugin for FlightTestPlugin {
    fn build(&self, app: &mut App) {
        info!("libfpv: flight test `{}`", self.0.scenario.name());
        app.insert_resource(Run {
            test: self.0.clone(),
            phase: Phase::Settle { ready_since: None },
            fpv: None,
            frame: FpvFrame::new(),
            log: Log::default(),
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
struct Run {
    test: FlightTest,
    phase: Phase,
    fpv: Option<Libfpv>,
    frame: Box<FpvFrame>,
    log: Log,
}

enum Phase {
    Settle { ready_since: Option<f32> },
    Fly(Box<Flight>),
    Done { at: f32, pass: bool },
}

/// One scenario's flight in progress.
struct Flight {
    /// Where the drone was made: frame bottom on the ground.
    spawn: Vec3,
    /// The path's direction, level (the wall test's push; the drone's heading).
    dir: Vec3,
    /// Yards from the start to the wall along `dir` (wall test).
    wall: Option<f32>,
    /// Simulated seconds since the drone was made.
    t: f64,
    /// The first `sim_time` read, for the startup skip.
    first_sim: Option<f64>,
    /// Channels this frame: AETR, AUX1..AUX6.
    ch: [f32; ffi::FPV_CHANNEL_COUNT],
    pushed: bool,
    shot: bool,
    /// Climb: shots taken so far, of [`CLIMB_SHOTS`].
    climb_shots: usize,
    /// Climb: what the FPV camera's near plane probes chose this frame.
    near: crate::camera::NearChoice,
}

/// What a scenario measured.
#[derive(Default)]
struct Log {
    /// Where the parked drone settled, and the farthest it went from there (yards).
    settle: Option<Vec3>,
    park_max: f32,
    armed_idle_max: f32,
    arm_text_before: Option<String>,
    /// Betaflight's reason not to arm with the throttle raised; must not be empty.
    arm_text_throttle: String,
    arm_text_after: Option<String>,
    armed_at: Option<f64>,
    /// The farthest along the wall direction the drone got, yards from the start.
    max_along: f32,
    last_along: f32,
    /// The least room the drone had left to the wall at its own height (yards): the wall's
    /// face along the push, cast from the drone's line at the start, minus how far along it
    /// was. Negative is past the face.
    min_margin: f32,
    /// Climb: the highest the drone got over its spawn (yards), the farthest its FPV camera was
    /// from it (yards), and the camera on the ground: forward, up, near plane, vertical FOV.
    climb_top: f32,
    cam_offset_max: f32,
    ground_cam: Option<(Vec3, Vec3, crate::camera::NearChoice, f32)>,
    /// The first impact's normal and speed.
    impact: Option<(Vec3, f32)>,
    impacts: u32,
    next_log: f64,
}

fn drive(
    time: Res<Time<bevy::time::Real>>,
    mut run: ResMut<Run>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    build: Option<Res<BuildId>>,
    mut camera: Query<(&mut Transform, &Camera), With<WorldCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut zoom: ResMut<FovZoom>,
    mut view: ResMut<ViewDistance>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok((mut cam, camera_info)) = camera.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let run = &mut *run;
    let site = Site::named(SITE).expect("the streets site");
    let mut world = |o: Vec3, d: Dir3, max: f32| {
        collision.ray_body(o, d, max).map(|h| RayHit {
            distance: h.distance,
            normal: h.normal,
            material: 0,
        })
    };

    match &mut run.phase {
        Phase::Settle { ready_since } => {
            // Hover over the start while the street streams in.
            let above = wow_to_bevy([site.xy[0], site.xy[1], site.probe_z]);
            *cam = Transform::from_translation(above).looking_to(Vec3::NEG_Z, Vec3::Y);
            if !world_ready(progress.as_deref()) {
                *ready_since = None;
                return;
            }
            if now - *ready_since.get_or_insert(now) < SETTLE_SECS {
                return;
            }
            match start(run, &site, &collision, build.as_deref(), &mut world) {
                Ok(flight) => run.phase = Phase::Fly(Box::new(flight)),
                Err(e) => {
                    error!("libfpv: TEST {} FAIL — {e}", run.test.scenario.name());
                    run.phase = Phase::Done {
                        at: now,
                        pass: false,
                    };
                }
            }
        }
        Phase::Fly(flight) => {
            let Some(fpv) = run.fpv.as_mut() else { return };
            let scenario = run.test.scenario;
            script(scenario, flight);
            fpv.set_channels(&flight.ch);
            if scenario == Scenario::Wall && !flight.pushed && flight.t >= 0.5 {
                // 10 m/s at the wall and 4 m/s up: a lob over any curb, about 0.7 yd up at the
                // wall, inside the band `wall_ahead` found clear.
                let v = flight.dir * 10.0 * UNITS_PER_METER + Vec3::Y * 4.0 * UNITS_PER_METER;
                if let Err(e) = fpv.push(v) {
                    error!("libfpv: {e}");
                }
                flight.pushed = true;
            }
            let stepped = fpv
                .step(DT)
                .and_then(|()| fpv.tick(DT, &mut run.frame, &mut world));
            if let Err(e) = stepped {
                error!("libfpv: TEST {} FAIL — {e}", scenario.name());
                run.phase = Phase::Done {
                    at: now,
                    pass: false,
                };
                return;
            }
            flight.t += DT as f64;

            // The overlay must render every frame: the collision view draws through it.
            if let Ok(win) = window.single() {
                fpv.set_overlay_display(win.width(), win.height(), 1.0);
            }
            let overlay = fpv.overlay(DT);

            let first = *flight.first_sim.get_or_insert(run.frame.sim_time);
            if run.frame.sim_time < first + STARTUP_SECS {
                return;
            }
            let frame = &*run.frame;
            if scenario == Scenario::Climb {
                flight.near = crate::camera::apply(
                    &frame.camera,
                    &mut cam,
                    camera_info,
                    &mut zoom,
                    &mut view,
                    &collision,
                );
            } else {
                follow(frame, &mut cam, camera_info, &mut zoom, &mut view);
            }
            // Upright faces only: near the ground, the street rising ahead is not the wall.
            let wall_at = |from: Vec3| {
                let d = Dir3::new(flight.dir).ok()?;
                let hit = collision.ray_body(from, d, 25.0)?;
                (hit.normal.y.abs() < 0.3).then_some(hit.distance)
            };
            measure(scenario, flight, frame, &mut run.log, &wall_at);

            if scenario == Scenario::Wall && !flight.shot && run.log.impact.is_some() {
                if let Some(dir) = run.test.capture.clone() {
                    let font = fpv.texture(ffi::FPV_TEXTURE_OVERLAY_FONT);
                    let osd = fpv.texture(ffi::FPV_TEXTURE_OSD_FONT);
                    shoot(
                        &mut commands,
                        dir,
                        "wall-impact",
                        overlay.clone(),
                        font,
                        osd,
                    );
                }
                flight.shot = true;
            }
            if scenario == Scenario::Climb {
                if let Some(&(at, name)) = CLIMB_SHOTS.get(flight.climb_shots) {
                    if flight.t >= at {
                        info!(
                            "libfpv: shot {name} at t {:.2}, near {:.3} yd (closest {:?})",
                            flight.t, flight.near.near, flight.near.closest
                        );
                        if let Some(dir) = run.test.capture.clone() {
                            shoot(&mut commands, dir, name, None, None, None);
                        }
                        flight.climb_shots += 1;
                    }
                }
            }

            if flight.t >= scenario.length() {
                let pass = verdict(scenario, flight, &run.log);
                run.phase = Phase::Done { at: now, pass };
            }
        }
        Phase::Done { at, pass } => {
            // A beat for the log and any screenshot write.
            if now - *at > 2.0 {
                exit.write(if *pass {
                    AppExit::Success
                } else {
                    AppExit::error()
                });
            }
        }
    }
}

/// Inits libfpv, finds the ground (and the wall), and makes the drone.
fn start(
    run: &mut Run,
    site: &Site,
    collision: &WorldCollision,
    build: Option<&BuildId>,
    world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
) -> Result<Flight, String> {
    let ground_under = |p: Vec3| {
        // WoW z is Bevy y.
        let top = Vec3::new(p.x, site.probe_z, p.z);
        collision
            .ray_body(top, Dir3::NEG_Y, 1000.0)
            .map(|h| top - Vec3::Y * h.distance)
    };
    let mut ground = ground_under(wow_to_bevy([site.xy[0], site.xy[1], site.probe_z]))
        .ok_or("no ground under the start")?;

    // The wall test's wall: the nearest of 32 headings within 25 yd with a clear run to an
    // upright face ([`wall_ahead`]). The drone then starts on the ground WALL_RUN short of it.
    const WALL_RUN: f32 = 6.0;
    let mut dir = Vec3::NEG_Z;
    let mut wall = None;
    if run.test.scenario == Scenario::Wall {
        let best = (0..32)
            .filter_map(|k| {
                let a = (k as f32 * 360.0 / 32.0).to_radians();
                let d = Quat::from_rotation_y(-a) * Vec3::NEG_Z;
                let dist = wall_ahead(collision, ground, d)?;
                (dist > WALL_RUN).then_some((d, dist))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .ok_or("no wall with a clear run within 25 yd of the start")?;
        dir = best.0;
        ground = ground_under(ground + dir * (best.1 - WALL_RUN))
            .ok_or("no ground short of the wall")?;
        // Measured again from where the drone starts.
        let from =
            wall_ahead(collision, ground, dir).ok_or("the wall is not ahead of the start")?;
        wall = Some(from);
        // Bevy +X is east and -Z north.
        info!(
            "libfpv: wall {:.2} yd ahead, heading {:.0}°",
            from,
            dir.x.atan2(-dir.z).to_degrees().rem_euclid(360.0)
        );
    }

    let build = build.map_or_else(|| env!("CARGO_PKG_VERSION").to_owned(), |b| b.summary());
    let mut fpv = Libfpv::init(
        &dll_path(),
        &InitOptions {
            manual_timing: true,
            flags: ffi::FPV_INIT_NO_DEVICE_INPUT
                | ffi::FPV_INIT_NO_CONFIGURATOR
                | ffi::FPV_INIT_NO_LENS_DISTORTION,
            host_build: build,
        },
    )?;
    let facing = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    fpv.create_drone(ground, facing, ground - dir * 2.0, world)?;
    fpv.set_view(if run.test.scenario == Scenario::Climb {
        ffi::FPV_VIEW_FPV
    } else {
        ffi::FPV_VIEW_CHASE
    });
    info!(
        "libfpv: drone made at WoW {:.2?} (ground {:.2} yd)",
        bevy_to_wow(ground),
        ground.y
    );
    run.fpv = Some(fpv);
    Ok(Flight {
        spawn: ground,
        dir,
        wall,
        t: 0.0,
        first_sim: None,
        // Throttle idle (-1), every switch off.
        ch: [0.0, 0.0, -1.0, 0.0, -1.0, -1.0, -1.0, -1.0, -1.0, -1.0],
        pushed: false,
        shot: false,
        climb_shots: 0,
        near: Default::default(),
    })
}

/// Yards from `ground` along `dir` (level) to a wall the drone can reach: level rays at 0.3,
/// 0.8 and 1.5 yd up all hit an upright face (normal within about 17° of level), within
/// 0.75 yd of each other, so no curb, step or slope is in the way. The nearest of the three.
fn wall_ahead(collision: &WorldCollision, ground: Vec3, dir: Vec3) -> Option<f32> {
    let d = Dir3::new(dir).ok()?;
    let hits: Vec<f32> = [0.3, 0.8, 1.5]
        .iter()
        .map(|h| {
            let hit = collision.ray_body(ground + Vec3::Y * *h, d, 25.0)?;
            (hit.normal.y.abs() < 0.3).then_some(hit.distance)
        })
        .collect::<Option<_>>()?;
    let (lo, hi) = hits
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    (hi - lo <= 0.75).then_some(lo)
}

/// The scenario's channels at `flight.t`.
fn script(scenario: Scenario, flight: &mut Flight) {
    // AETR + AUX1 (arm) .. AUX6. Park raises the throttle, disarmed, from 3 s to 4 s, so
    // Betaflight names a reason not to arm, then lowers it and arms at 6 s.
    let throttle_up = scenario == Scenario::Park && (3.0..4.0).contains(&flight.t);
    flight.ch[2] = if throttle_up { 0.0 } else { -1.0 };
    flight.ch[4] = match scenario {
        Scenario::Park if flight.t >= 6.0 => 1.0,
        Scenario::Climb if flight.t >= 1.0 => 1.0,
        _ => -1.0,
    };
    if scenario == Scenario::Climb {
        // Angle mode (AUX2) holds it level; arm at 1 s, climb from 1.5 s, then about hover
        // (INTEGRATION.md: about -0.56 for the default 5" in angle mode).
        flight.ch[5] = 1.0;
        flight.ch[2] = match flight.t {
            t if t < 1.5 => -1.0,
            t if t < 2.5 => -0.3,
            _ => -0.5,
        };
    }
}

/// The world camera at libfpv's chase camera, its FOV and near plane through benilla's own
/// stamps.
fn follow(
    frame: &FpvFrame,
    cam: &mut Transform,
    camera: &Camera,
    zoom: &mut FovZoom,
    view: &mut ViewDistance,
) {
    let c = &frame.chase_camera;
    cam.translation = Vec3::from_array(c.position);
    // libfpv's rotation maps the host's forward (-Z) to the view: Bevy's camera rotation as-is.
    cam.rotation = Quat::from_array(c.rotation).normalize();
    let aspect = camera
        .logical_viewport_size()
        .map_or(16.0 / 9.0, |s| s.x / s.y.max(1.0));
    zoom.0 = c.fov_y_degrees.to_radians() / world_fovy(aspect);
    view.set_nearclip(c.near_plane);
}

/// Folds this frame into the scenario's measurements, and logs every half simulated second.
/// `wall_at(from)` is the collision ray's distance along the push from `from`.
fn measure(
    scenario: Scenario,
    flight: &Flight,
    frame: &FpvFrame,
    log: &mut Log,
    wall_at: &dyn Fn(Vec3) -> Option<f32>,
) {
    let pos = Vec3::from_array(frame.position);
    let armed = frame.flags & ffi::FPV_DRONE_ARMED != 0;
    match scenario {
        Scenario::Park => {
            if flight.t >= 1.0 {
                let settle = *log.settle.get_or_insert(pos);
                let off = pos.distance(settle);
                if flight.t < 6.0 {
                    log.park_max = log.park_max.max(off);
                } else if armed {
                    log.armed_idle_max = log.armed_idle_max.max(off);
                }
            }
            // Late in the raised-throttle window, and just before arming.
            if (3.5..4.0).contains(&flight.t) && log.arm_text_throttle.is_empty() {
                log.arm_text_throttle = frame.arming_disable_text();
            }
            if flight.t < 6.0 {
                log.arm_text_before = Some(frame.arming_disable_text());
            }
            if armed && log.armed_at.is_none() {
                log.armed_at = Some(flight.t);
                log.arm_text_after = Some(frame.arming_disable_text());
            }
        }
        Scenario::Climb => {
            log.climb_top = log.climb_top.max(pos.y - flight.spawn.y);
            if armed && log.armed_at.is_none() {
                log.armed_at = Some(flight.t);
            }
            let c = &frame.camera;
            let eye = Vec3::from_array(c.position);
            log.cam_offset_max = log.cam_offset_max.max(eye.distance(pos));
            // On the ground, disarmed, before the climb.
            if flight.t < 1.0 {
                log.ground_cam = Some((
                    Vec3::from_array(c.forward),
                    Vec3::from_array(c.up),
                    flight.near,
                    c.fov_y_degrees,
                ));
            }
        }
        Scenario::Wall => {
            let along = (pos - flight.spawn).dot(flight.dir);
            if log.max_along == 0.0 && log.last_along == 0.0 {
                log.min_margin = f32::MAX;
            }
            log.max_along = log.max_along.max(along);
            log.last_along = along;
            // The upright face at the drone's own height and line, seen from the start's side;
            // a drone past it would have the face short of it. No upright face on this line
            // (the ray met the street first) says nothing either way.
            let back = pos - flight.dir * along;
            if let Some(face) = wall_at(back) {
                log.min_margin = log.min_margin.min(face - along);
            }
            for imp in &frame.impacts[..(frame.impact_count as usize).min(ffi::FPV_MAX_IMPACTS)] {
                log.impacts += 1;
                let n = Vec3::from_array(imp.normal);
                // The wall's own impact faces back along the push; the street's faces up.
                if log.impact.is_none() && n.dot(flight.dir) < -0.7 {
                    log.impact = Some((n, imp.speed));
                    info!(
                        "libfpv: impact at {:.2} yd along, normal {:.2?}, {:.2} m/s",
                        along,
                        n,
                        imp.speed / UNITS_PER_METER
                    );
                }
            }
        }
    }
    if flight.t >= log.next_log {
        log.next_log = flight.t + 0.5;
        info!(
            "libfpv: t {:.2} pos WoW {:.2?} along {:.2} vel {:.2?} armed {} ground {} alt {:.2} \
             impacts {} arm-text {:?}",
            flight.t,
            bevy_to_wow(pos),
            (pos - flight.spawn).dot(flight.dir),
            Vec3::from_array(frame.velocity),
            armed,
            frame.flags & ffi::FPV_DRONE_ON_GROUND != 0,
            frame.altitude,
            frame.impact_count,
            frame.arming_disable_text()
        );
    }
}

/// Prints the scenario's result line and whether it passed.
fn verdict(scenario: Scenario, flight: &Flight, log: &Log) -> bool {
    let m = |yd: f32| yd / UNITS_PER_METER;
    match scenario {
        Scenario::Park => {
            let still = m(log.park_max) <= PARK_TOLERANCE_M;
            let armed = log.armed_at.is_some();
            // Refused with a reason while the throttle was up, armable once it was down.
            let reasons =
                !log.arm_text_throttle.is_empty() && log.arm_text_before.as_deref() == Some("");
            let pass = still && armed && reasons;
            info!(
                "libfpv: TEST park {} — disarmed drift {:.3} m (limit {PARK_TOLERANCE_M}), arm \
                 text with throttle up {:?}, armed at {:?} s, arm text before {:?} after {:?}, \
                 armed-idle drift {:.3} m",
                if pass { "PASS" } else { "FAIL" },
                m(log.park_max),
                log.arm_text_throttle,
                log.armed_at.map(|t| (t * 100.0).round() / 100.0),
                log.arm_text_before.as_deref().unwrap_or(""),
                log.arm_text_after.as_deref().unwrap_or(""),
                m(log.armed_idle_max)
            );
            pass
        }
        Scenario::Climb => {
            let armed = log.armed_at.is_some();
            let climbed = m(log.climb_top) > 1.0;
            // The camera rides on the drone (a 5" frame: well inside half a metre).
            let onboard = m(log.cam_offset_max) < 0.5;
            // On the ground the picture is upright, and the probes saw the street close below.
            let ground_ok = log
                .ground_cam
                .is_some_and(|(_, up, near, _)| up.y > 0.7 && near.closest.is_some());
            let pass = armed && climbed && onboard && ground_ok;
            info!(
                "libfpv: TEST climb {} — armed at {:?} s, top {:.2} m over the spawn, camera at \
                 most {:.3} m from the drone, on the ground: {}",
                if pass { "PASS" } else { "FAIL" },
                log.armed_at.map(|t| (t * 100.0).round() / 100.0),
                m(log.climb_top),
                m(log.cam_offset_max),
                log.ground_cam
                    .map_or("no frame".into(), |(fwd, up, near, fov)| format!(
                        "forward {fwd:.2} (pitch {:.1}°), up {up:.2}, fov {fov:.1}°, near {:.3} m \
                     (closest {:?} m)",
                        fwd.y.asin().to_degrees(),
                        m(near.near),
                        near.closest.map(|c| (m(c) * 1000.0).round() / 1000.0)
                    ))
            );
            pass
        }
        Scenario::Wall => {
            let wall = flight.wall.unwrap_or(0.0);
            // Never past the face at its own height, and stopped against it rather than short.
            let stopped = log.min_margin > 0.0;
            let reached = log.min_margin < 0.5;
            let hit = log.impact.is_some();
            let pass = stopped && reached && hit;
            info!(
                "libfpv: TEST wall {} — wall at {:.2} yd, farthest {:.2} yd, closest to the face \
                 {:.3} yd ({:.3} m), end {:.2} yd, wall impact {}, impacts {}",
                if pass { "PASS" } else { "FAIL" },
                wall,
                log.max_along,
                log.min_margin,
                m(log.min_margin),
                log.last_along,
                log.impact.map_or("none".into(), |(n, s)| format!(
                    "normal {n:.2} at {:.2} m/s",
                    m(s)
                )),
                log.impacts
            );
            pass
        }
    }
}

/// Writes `dir/<name>.png` (the view) and, given an overlay, `dir/<name>-xray.png` with its
/// draw data (the collision view) rasterised over it.
fn shoot(
    commands: &mut Commands,
    dir: PathBuf,
    name: &'static str,
    overlay: Option<DrawData>,
    font: Option<(Vec<u8>, u32, u32, u32)>,
    osd: Option<(Vec<u8>, u32, u32, u32)>,
) {
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |shot: On<ScreenshotCaptured>| {
            let write = || -> Result<(), String> {
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let mut rgb = shot
                    .image
                    .clone()
                    .try_into_dynamic()
                    .map_err(|e| e.to_string())?
                    .to_rgb8();
                rgb.save(dir.join(format!("{name}.png")))
                    .map_err(|e| e.to_string())?;
                let (w, h) = rgb.dimensions();
                let Some(data) = &overlay else {
                    info!("libfpv: wrote {name}.png");
                    return Ok(());
                };
                {
                    let tex = |id: u64| {
                        let t = match id {
                            ffi::FPV_TEXTURE_OVERLAY_FONT => font.as_ref(),
                            ffi::FPV_TEXTURE_OSD_FONT => osd.as_ref(),
                            _ => None,
                        }?;
                        Some(crate::draw_cpu::Texture {
                            rgba: &t.0,
                            width: t.1,
                            height: t.2,
                        })
                    };
                    crate::draw_cpu::rasterize(data, &mut rgb, w, h, &tex);
                }
                rgb.save(dir.join(format!("{name}-xray.png")))
                    .map_err(|e| e.to_string())?;
                info!(
                    "libfpv: wrote {name}.png and -xray.png ({} overlay triangles)",
                    data.lists
                        .iter()
                        .map(|l| l.indices.len() / 3)
                        .sum::<usize>()
                );
                Ok(())
            };
            if let Err(e) = write() {
                error!("libfpv: capture failed: {e}");
            }
        });
}
