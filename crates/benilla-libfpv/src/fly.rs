//! Live flight in the world viewer: libfpv on its own real-time thread, reading the pilot's
//! radio, with the shared data folder (the radio setup and tune follow the pilot from game to
//! game) and the Configurator on. The drone starts on the ground at a site, the world camera is
//! its FPV camera, and the OSD and the F8 overlay draw over it ([`crate::draw`]).
//!
//! `benilla-libfpv --worldview --fly [site]`. Keys go to libfpv ([`crate::input`]): F8 the
//! overlay, and the keyboard flying fallback. This mode's own: R respawns (repair, then back to
//! the start); P logs a MARK line with the drone's position and heading, for reporting a spot;
//! Y moves the pilot to the ground under the drone (libfpv's "move here"). The pilot, the video
//! link's receiver, follows the drone by default, whenever it gets [`PILOT_FOLLOW_M`] away, so an
//! open world does not run the link out of range; F8 > Fly toggles it.
//! None of these acts while the overlay takes the keyboard, nor for a key still held from then.
//! The lens and the video feed draw after the world ([`crate::video`]), under the overlay.

use std::collections::HashSet;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_world::build_id::BuildId;
use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::WorldLoadProgress;
use benilla_world::view::{CameraPoseSet, FovZoom, ViewDistance, WorldCamera};
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::input::{ButtonState, InputSystems};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorMoved, CursorOptions, PrimaryWindow};

use crate::camera;
use crate::draw::{FpvDraw, FpvDrawPlugin};
use crate::ffi::{self, FpvFrame};
use crate::input::{fpv_key, printable, KeyEdges};
use crate::roll_test::{clearest_heading, world_ready, Site};
use crate::runtime::{dll_path, InitOptions, Libfpv, RayHit};
use crate::video::{OsdChipCache, VideoPlugin};

/// Frames are ignored until `sim_time` is this far past its first reading.
const STARTUP_SECS: f64 = 0.05;

/// Wall seconds the world must read ready before the drone is made.
const SETTLE_SECS: f32 = 3.0;

/// How often the drone's state goes to the log, wall seconds.
const LOG_EVERY: f32 = 2.0;

/// Pixels per wheel line, for a wheel that reports pixels (a touchpad).
const WHEEL_PIXELS_PER_LINE: f32 = 40.0;

/// With the pilot following, how far the drone may get from the pilot (metres) before the pilot
/// moves to the ground under it: the video link then rarely runs out of range, and its effects
/// come from walls and corners close by, fades and hits, as in a small level.
const PILOT_FOLLOW_M: f32 = 30.0;

/// The overlay's Fly-tab buttons this mode adds.
const ACTION_MOVE_HERE: u32 = 1;
const ACTION_FOLLOW: u32 = 2;

fn follow_label(on: bool) -> &'static str {
    if on {
        "Pilot follows the drone: on (click to stay)"
    } else {
        "Pilot follows the drone: off (click to follow)"
    }
}

pub struct FlyPlugin(pub Site);

impl Plugin for FlyPlugin {
    fn build(&self, app: &mut App) {
        info!("libfpv: flying at `{}`", self.0.name);
        app.add_plugins((FpvDrawPlugin, VideoPlugin))
            .insert_resource(Fly {
                site: self.0,
                phase: Phase::Settle { ready_since: None },
                fpv: None,
                frame: FpvFrame::new(),
                spawn: Transform::IDENTITY,
                first_sim: None,
                next_log: 0.0,
                edges: KeyEdges::default(),
                swallowed: HashSet::new(),
                cursor_taken: false,
                osd_chip: OsdChipCache::default(),
                pilot: Vec3::ZERO,
                follow: true,
            })
            // Right after Bevy reads the input: libfpv gets it first, and while the overlay
            // takes it, the viewer's own mouse look never sees a button.
            .add_systems(PreUpdate, forward_input.after(InputSystems))
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

/// The drone's sound on the mixer, held for the run: dropping it ends the sound.
#[derive(Resource)]
struct DroneSound(#[allow(dead_code)] Option<benilla_app::HostStream>);

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
    /// The keys libfpv has been told are down.
    edges: KeyEdges,
    /// Keys pressed while the overlay took the keyboard, kept from this mode's own keys until
    /// they are released (INTEGRATION.md: return control only after the key is up).
    swallowed: HashSet<KeyCode>,
    /// This mode hid the cursor for the overlay, which draws its own.
    cursor_taken: bool,
    /// The analog OSD chip's picture, rebuilt when the grid changes.
    osd_chip: OsdChipCache,
    /// Where the pilot stands (feet): the video link's receiver, the LOS camera, the ear.
    pilot: Vec3,
    /// The pilot follows the drone ([`PILOT_FOLLOW_M`]); off, the pilot stays until moved (Y).
    follow: bool,
}

enum Phase {
    Settle { ready_since: Option<f32> },
    Flying,
    Failed,
}

/// Keys, text and the mouse to libfpv, on edges, while the window has focus.
#[allow(clippy::too_many_arguments)]
fn forward_input(
    mut fly: ResMut<Fly>,
    mut keyboard: MessageReader<KeyboardInput>,
    mut buttons_in: MessageReader<MouseButtonInput>,
    mut cursor_in: MessageReader<CursorMoved>,
    mut wheel_in: MessageReader<MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut window: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let fly = &mut *fly;
    let Some(fpv) = fly.fpv.as_ref() else {
        // Not flying yet: nothing to hand on.
        keyboard.clear();
        buttons_in.clear();
        cursor_in.clear();
        wheel_in.clear();
        return;
    };
    let Ok((win, mut cursor)) = window.single_mut() else {
        return;
    };
    if !win.focused {
        fly.edges.release_all(|k, d| fpv.key(k, d));
        keyboard.clear();
        buttons_in.clear();
        wheel_in.clear();
        cursor_in.clear();
        return;
    }
    let wants = fpv.overlay_wants_input();
    for ev in keyboard.read() {
        let Some(key) = fpv_key(ev.key_code) else {
            continue;
        };
        match ev.state {
            ButtonState::Pressed if !ev.repeat => {
                fly.edges.press(key, |k, d| fpv.key(k, d));
            }
            ButtonState::Released => fly.edges.release(key, |k, d| fpv.key(k, d)),
            ButtonState::Pressed => {}
        }
        if wants && ev.state == ButtonState::Pressed {
            if let Some(text) = ev.text.as_deref().and_then(printable) {
                fpv.text(text);
            }
        }
    }
    for ev in cursor_in.read() {
        fpv.mouse_move(ev.position.x, ev.position.y);
    }
    for ev in buttons_in.read() {
        let button = match ev.button {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
            _ => continue,
        };
        fpv.mouse_button(button, ev.state == ButtonState::Pressed);
    }
    for ev in wheel_in.read() {
        let k = match ev.unit {
            MouseScrollUnit::Line => 1.0,
            MouseScrollUnit::Pixel => 1.0 / WHEEL_PIXELS_PER_LINE,
        };
        fpv.mouse_wheel(ev.x * k, ev.y * k);
    }

    if wants {
        // The overlay has the keyboard: every key down now stays the overlay's until released.
        fly.swallowed.extend(keys.get_pressed().copied());
        // And the mouse: the viewer's mouse look grabs the cursor on a held button.
        buttons.reset_all();
        if !fly.cursor_taken {
            fly.cursor_taken = true;
            cursor.grab_mode = CursorGrabMode::None;
            // The overlay draws its own cursor (INTEGRATION.md "Input").
            cursor.visible = false;
        }
    } else if fly.cursor_taken {
        fly.cursor_taken = false;
        cursor.visible = true;
    }
    fly.swallowed.retain(|k| keys.pressed(*k));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time<bevy::time::Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut fly: ResMut<Fly>,
    mut draw: ResMut<FpvDraw>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    build: Option<Res<BuildId>>,
    mut camera: Query<(Entity, &mut Transform, &Camera), With<WorldCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut zoom: ResMut<FovZoom>,
    mut view: ResMut<ViewDistance>,
    mut commands: Commands,
) {
    let Ok((cam_entity, mut cam, camera_info)) = camera.single_mut() else {
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
                    fly.pilot = spawn.translation - spawn.rotation * Vec3::NEG_Z * 2.0;
                    if let Some(fpv) = fly.fpv.as_ref() {
                        fpv.set_overlay_action(ACTION_MOVE_HERE, "Move the pilot here (Y)");
                        fpv.set_overlay_action(ACTION_FOLLOW, follow_label(fly.follow));
                        // The motors and the beeper, pulled on the audio thread (M7).
                        let source = fpv.audio_source();
                        commands.queue(move |world: &mut World| {
                            let stream = benilla_app::play_host_stream(world, source);
                            world.insert_resource(DroneSound(stream));
                        });
                    }
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
            // This mode's own keys, unless the overlay has the keyboard or had it when pressed.
            let own = |k: KeyCode| {
                keys.just_pressed(k) && !fly.swallowed.contains(&k) && !fpv.overlay_wants_input()
            };
            let (mark, respawn, move_here) =
                (own(KeyCode::KeyP), own(KeyCode::KeyR), own(KeyCode::KeyY));
            if mark {
                // Marks a spot for a report: where the drone is, and which way it faces.
                let pos = Vec3::from_array(fly.frame.position);
                let fwd = Vec3::from_array(fly.frame.camera.forward);
                info!(
                    "libfpv: MARK drone at WoW {:.1?}, camera heading {:.0}°",
                    bevy_to_wow(pos),
                    fwd.x.atan2(-fwd.z).to_degrees().rem_euclid(360.0)
                );
            }
            if respawn {
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
            // The pilot: Y or the overlay's button moves them under the drone; following, they
            // move there whenever the drone gets PILOT_FOLLOW_M away.
            let mut move_here = move_here;
            while let Some(id) = fpv.poll_overlay_action() {
                match id {
                    ACTION_MOVE_HERE => move_here = true,
                    ACTION_FOLLOW => {
                        fly.follow = !fly.follow;
                        fpv.set_overlay_action(ACTION_FOLLOW, follow_label(fly.follow));
                        info!("libfpv: the pilot follows the drone: {}", fly.follow);
                    }
                    _ => {}
                }
            }
            let drone = Vec3::from_array(fly.frame.position);
            let far = drone.distance(fly.pilot) > PILOT_FOLLOW_M * crate::runtime::UNITS_PER_METER;
            if move_here || (fly.follow && far && fly.first_sim.is_some()) {
                if let Some(ground) = collision
                    .ray_body(drone, Dir3::NEG_Y, 500.0)
                    .map(|h| drone - Vec3::Y * h.distance)
                {
                    fly.pilot = ground;
                    if let Err(e) = fpv.set_pilot_position(ground) {
                        error!("libfpv: {e}");
                    } else if move_here {
                        info!("libfpv: the pilot moved to WoW {:.1?}", bevy_to_wow(ground));
                    }
                }
            }
            fpv.set_view(ffi::FPV_VIEW_FPV);
            let dt = time.delta_secs();
            if let Err(e) = fpv.tick(dt, &mut fly.frame, &mut world) {
                error!("libfpv: {e}");
                return;
            }

            let frame = &*fly.frame;
            let first = *fly.first_sim.get_or_insert(frame.sim_time);
            let started = frame.sim_time >= first + STARTUP_SECS;
            if !started {
                if let Ok(win) = window.single() {
                    publish(fpv, frame, win, dt, false, &mut draw);
                }
                return;
            }
            fly.osd_chip
                .refresh_font(|id, known| fpv.texture_if_changed(id, known));
            let applied = camera::apply(
                frame,
                &mut cam,
                camera_info,
                &mut zoom,
                &mut view,
                &collision,
                &mut fly.osd_chip,
            );
            // An analog OSD in the signal is already in the picture.
            let osd_over = !applied.view.as_ref().is_some_and(|v| v.osd_in_signal());
            if let Ok(win) = window.single() {
                publish(fpv, frame, win, dt, osd_over, &mut draw);
            }
            camera::set_view(&mut commands, cam_entity, applied.view.clone());
            if now >= fly.next_log {
                fly.next_log = now + LOG_EVERY;
                log_state(frame, applied);
            }
        }
    }
}

/// This frame's OSD and overlay for [`crate::draw`]: the OSD (once the frames are live) under
/// the overlay, which renders every frame, closed too, since the collision view draws through it.
pub fn publish(
    fpv: &Libfpv,
    frame: &FpvFrame,
    win: &Window,
    dt: f32,
    show_osd: bool,
    draw: &mut FpvDraw,
) {
    let (w, h) = (win.width(), win.height());
    fpv.set_overlay_display(w, h, win.scale_factor());
    draw.refresh_textures(|id, known| fpv.texture_if_changed(id, known));
    draw.layers.clear();
    if show_osd {
        // The pilot's style (F8 > OSD), analog for anything this build does not know.
        let style = match frame.video_feed.osd_style {
            s @ (ffi::FPV_OSD_STYLE_ANALOG | ffi::FPV_OSD_STYLE_HD) => s,
            _ => ffi::FPV_OSD_STYLE_ANALOG,
        };
        draw.layers.extend(fpv.osd(frame, w, h, style));
    }
    draw.layers.extend(fpv.overlay(dt));
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
            // The lens and the video feed are drawn ([`crate::video`]): libfpv widens its FOV
            // for the lens and offers the feed's effects in F8 > Camera.
            flags: ffi::FPV_INIT_VIDEO_FEED,
            host_build: build,
        },
    )?;
    let behind = ground - facing * Vec3::NEG_Z * 2.0;
    fpv.create_drone(ground, facing, behind, world)?;
    info!(
        "libfpv: drone on the ground at WoW {:.2?}, heading {heading:.0}° — arm and fly; F8 \
         opens the overlay, R respawns, P marks the spot in this log",
        bevy_to_wow(ground)
    );
    Ok((
        fpv,
        Transform::from_translation(ground).with_rotation(facing),
    ))
}

fn log_state(frame: &FpvFrame, applied: camera::Applied) {
    let near = applied.near;
    info!(
        "libfpv: pos WoW {:.1?} {:.0} km/h alt {:.2} armed {} angle {} fov {:.1}° (libfpv \
         {:.1}°, lens k1 {:.3} k2 {:.3} zoom {:.3}, feed {}) near {:.3} yd (closest {:?}) battery \
         {:.2} V arm-text {:?} impacts {}",
        bevy_to_wow(Vec3::from_array(frame.position)),
        frame.speed_kmh,
        frame.altitude,
        frame.flags & ffi::FPV_DRONE_ARMED != 0,
        frame.flags & ffi::FPV_DRONE_MODE_ANGLE != 0,
        applied.fov_deg,
        frame.camera.fov_y_degrees,
        frame.camera.distortion_k1,
        frame.camera.distortion_k2,
        frame.camera.distortion_zoom,
        feed_summary(frame),
        near.near,
        near.closest.map(|c| (c * 1000.0).round() / 1000.0),
        frame.battery.voltage,
        frame.arming_disable_text(),
        frame.impact_count
    );
}

/// The video feed in a few words, for the log: off, or the system, the effects on and the link.
fn feed_summary(frame: &FpvFrame) -> String {
    let vf = &frame.video_feed;
    if vf.flags == 0 {
        return "off".into();
    }
    let system = match vf.system {
        ffi::FPV_VIDEO_NTSC => "NTSC",
        ffi::FPV_VIDEO_PAL => "PAL",
        _ => "digital",
    };
    format!(
        "{system}{}{}{} {}x{} rssi {:.0} dBm cnr {:.1} dB",
        if vf.flags & ffi::FPV_VIDEO_FEED_NOISE != 0 {
            " noise"
        } else {
            ""
        },
        if vf.flags & ffi::FPV_VIDEO_FEED_RESOLUTION != 0 {
            " res"
        } else {
            ""
        },
        if vf.flags & ffi::FPV_VIDEO_FEED_OSD_IN_SIGNAL != 0 {
            " osd-in"
        } else {
            ""
        },
        vf.raster_width,
        vf.raster_height,
        vf.rssi_dbm,
        vf.cnr_db
    )
}
