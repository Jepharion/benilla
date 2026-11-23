//! FPV in the full client (M8): the character is the pilot, standing where it was when the drone
//! launched. `benilla-libfpv` bare runs the client with this plugin.
//!
//! - **F7** launches the drone at the character (the first time: libfpv starts, the drone on the
//!   ground just ahead), or goes back to it. The body detaches ([`benilla_app::HostBody`]), so it
//!   stays put while the world streams around the drone; the game's UI hides and the world camera
//!   is the drone's.
//! - **F** goes back to the pilot: the body attaches, the UI shows, the game's camera returns, and
//!   the drone flies on, heard from the game's camera (`FPV_VIEW_PILOT`).
//! - In FPV, **R** respawns the drone where it launched, and **P** logs a MARK. F8 opens libfpv's
//!   overlay in either view.
//! - The game never sees what libfpv takes ([`benilla_app::HostInput`]): in FPV every key and the
//!   mouse; in the pilot view F7 and F8, and all of it while the overlay has the input; and a key
//!   pressed while taken until it is released (INTEGRATION.md "Input").
//!
//! Not yet (M8's next steps, NOTES): the character moving with the drone on the server, Y ("move
//! here"), and the pilot's stand-in.

use std::collections::HashSet;

use benilla_app::{HostBody, HostInput};
use benilla_assets::coords::bevy_to_wow;
use benilla_world::build_id::BuildId;
use benilla_world::collision::WorldCollision;
use benilla_world::ffx_glow::FfxSwitches;
use benilla_world::schedule::WorldStage;
use benilla_world::view::{CameraPoseSet, FovZoom, ViewDistance, WorldCamera};
use bevy::camera::visibility::RenderLayers;
use bevy::input::InputSystems;
use bevy::prelude::*;
use bevy::ui::IsDefaultUiCamera;
use bevy::window::{CursorOptions, PrimaryWindow};

use crate::camera;
use crate::draw::{FpvDraw, FpvDrawPlugin};
use crate::ffi::{self, FpvFrame};
use crate::fly::{
    log_state, publish, DroneSound, InputBridge, InputMessages, LOG_EVERY, STARTUP_SECS,
};
use crate::runtime::{dll_path, InitOptions, Libfpv, RayHit};
use crate::video::{OsdChipCache, VideoPlugin};

/// The keys libfpv always has, in either view: F7 (launch) and F8 (the overlay). Stock 1.12 binds
/// F8 to TOGGLEBAG1 and leaves F7 free (NOTES, Q8).
const ALWAYS_TAKEN: [KeyCode; 2] = [KeyCode::F7, KeyCode::F8];

/// A render layer nothing draws on: the UI camera looks there while the UI is hidden.
const HIDDEN_LAYER: usize = 31;

/// How far ahead of the character the drone launches, yards.
const LAUNCH_AHEAD: f32 = 1.5;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((FpvDrawPlugin, VideoPlugin))
            .insert_resource(Client::default())
            .insert_resource(HostInput {
                keys: ALWAYS_TAKEN.to_vec(),
                ..default()
            })
            .add_systems(PreUpdate, forward_input.after(InputSystems))
            // The camera contract (NOTES, M1 and M8): after every rig writer (`WorldStage::Input`),
            // before the pose is published.
            .add_systems(
                Update,
                drive
                    .after(WorldStage::Input)
                    .before(CameraPoseSet)
                    .before(WorldStage::Stream),
            );
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum View {
    /// The game as usual; the drone, if launched, flies on.
    #[default]
    Pilot,
    /// The drone's camera.
    Fpv,
}

#[derive(Resource)]
struct Client {
    fpv: Option<Libfpv>,
    failed: bool,
    frame: Box<FpvFrame>,
    view: View,
    /// Where the drone launched, for R.
    launch: Transform,
    first_sim: Option<f64>,
    next_log: f32,
    input: InputBridge,
    /// Keys pressed while libfpv had the input, kept from the game until released.
    held: HashSet<KeyCode>,
    osd_chip: OsdChipCache,
    /// The UI camera's layers while the UI is hidden.
    ui_layers: Option<RenderLayers>,
    /// The game's FOV zoom and near plane, put back in the pilot view.
    game_view: Option<(f32, f32)>,
    /// The game's FFX switches, put back in the pilot view.
    game_ffx: Option<FfxSwitches>,
    /// `LIBFPV_CLIENT_TEST=<dir>`: the unattended check ([`Script`]).
    script: Option<Script>,
}

impl Default for Client {
    fn default() -> Self {
        Self {
            fpv: None,
            failed: false,
            frame: FpvFrame::new(),
            view: View::Pilot,
            launch: Transform::IDENTITY,
            first_sim: None,
            next_log: 0.0,
            input: InputBridge::default(),
            held: HashSet::new(),
            osd_chip: OsdChipCache::default(),
            ui_layers: None,
            game_view: None,
            game_ffx: None,
            script: std::env::var_os("LIBFPV_CLIENT_TEST").map(|d| Script {
                dir: d.into(),
                in_world_at: None,
                step: 0,
            }),
        }
    }
}

/// `LIBFPV_CLIENT_TEST=<dir>`, with a scripted login: SCRIPT_LAUNCH seconds in the world it
/// launches as F7 does, photographs the FPV view (`client-fpv.png`, the UI hidden), goes back as
/// F does, photographs the pilot's view (`client-pilot.png`, the UI back) and exits, logging the
/// state at each step.
struct Script {
    dir: std::path::PathBuf,
    in_world_at: Option<f32>,
    step: u8,
}

/// Seconds in the world before the scripted launch: the world around the character streams in.
const SCRIPT_LAUNCH: f32 = 10.0;

/// Keys, text and the mouse to libfpv, right after Bevy reads them.
fn forward_input(
    mut client: ResMut<Client>,
    mut msgs: InputMessages,
    keys: Res<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut window: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let client = &mut *client;
    let Some(fpv) = client.fpv.as_ref() else {
        msgs.clear();
        return;
    };
    let Ok((win, mut cursor)) = window.single_mut() else {
        return;
    };
    let wants = client
        .input
        .forward(fpv, &mut msgs, &keys, &mut buttons, win, &mut cursor);
    if client.view == View::Fpv && !wants {
        // The drone's view: the game's mouse look and clicks see nothing.
        buttons.reset_all();
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    time: Res<Time<bevy::time::Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut client: ResMut<Client>,
    mut body: ResMut<HostBody>,
    mut taken: ResMut<HostInput>,
    mut draw: ResMut<FpvDraw>,
    collision: WorldCollision,
    build: Option<Res<BuildId>>,
    mut camera: Query<(Entity, &mut Transform, &Camera), With<WorldCamera>>,
    mut ui_camera: Query<(Entity, Option<&RenderLayers>), With<IsDefaultUiCamera>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut zoom: ResMut<FovZoom>,
    mut view: ResMut<ViewDistance>,
    mut ffx: Option<ResMut<FfxSwitches>>,
    mut commands: Commands,
) {
    let Ok((cam_entity, mut cam, camera_info)) = camera.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let client = &mut *client;
    let wants = client.fpv.as_ref().is_some_and(Libfpv::overlay_wants_input);
    // The game's keys, unless libfpv has the input or had it when the key went down.
    let own = |k: KeyCode| keys.just_pressed(k) && !wants && !client.held.contains(&k);
    let (mut launch, mut back, respawn, mark) = (
        own(KeyCode::F7),
        client.view == View::Fpv && own(KeyCode::KeyF),
        client.view == View::Fpv && own(KeyCode::KeyR),
        client.view == View::Fpv && own(KeyCode::KeyP),
    );
    if let Some(script) = client.script.as_mut() {
        let at =
            *script.in_world_at.get_or_insert_with(
                || {
                    if body.active {
                        now
                    } else {
                        f32::INFINITY
                    }
                },
            );
        if at.is_infinite() {
            script.in_world_at = None;
        }
        let t = now - at;
        let mut step = |n: u8, when: f32| {
            let due = script.step == n && t >= when;
            if due {
                script.step += 1;
            }
            due
        };
        if step(0, SCRIPT_LAUNCH) {
            info!("libfpv: TEST launch (as F7)");
            launch = true;
        } else if step(1, SCRIPT_LAUNCH + 3.0) {
            info!(
                "libfpv: TEST shot fpv — view {:?}, detached {}, keys all {}",
                client.view, body.detached, taken.all_keys
            );
            crate::flight_test::shoot(
                &mut commands,
                script.dir.clone(),
                "client-fpv",
                None,
                None,
                None,
            );
        } else if step(2, SCRIPT_LAUNCH + 4.0) {
            info!("libfpv: TEST back (as F)");
            back = true;
        } else if step(3, SCRIPT_LAUNCH + 7.0) {
            info!(
                "libfpv: TEST shot pilot — view {:?}, detached {}, keys all {}",
                client.view, body.detached, taken.all_keys
            );
            crate::flight_test::shoot(
                &mut commands,
                script.dir.clone(),
                "client-pilot",
                None,
                None,
                None,
            );
        } else if step(4, SCRIPT_LAUNCH + 9.0) {
            info!("libfpv: TEST done");
            commands.write_message(AppExit::Success);
        }
    }
    let mut world = |o: Vec3, d: Dir3, max: f32| {
        collision.ray_body(o, d, max).map(|h| RayHit {
            distance: h.distance,
            normal: h.normal,
            material: 0,
        })
    };

    if !body.active {
        // Out of the world (the glue screens, a loading screen): the pilot's view, untouched.
        if client.view == View::Fpv {
            leave_fpv(
                client,
                &mut body,
                &mut ui_camera,
                &mut commands,
                cam_entity,
                &mut zoom,
                &mut view,
                ffx.as_deref_mut(),
            );
        }
    } else if launch {
        if client.fpv.is_none() && !client.failed {
            let facing = yaw_of(&cam);
            match start(body.feet, facing, &collision, build.as_deref(), &mut world) {
                Ok((fpv, at)) => {
                    let source = fpv.audio_source();
                    commands.queue(move |world: &mut World| {
                        let stream = benilla_app::play_host_stream(world, source);
                        world.insert_resource(DroneSound(stream));
                    });
                    client.fpv = Some(fpv);
                    client.launch = at;
                }
                Err(e) => {
                    error!("libfpv: cannot fly — {e}");
                    client.failed = true;
                }
            }
        }
        if client.fpv.is_some() && client.view == View::Pilot {
            client.view = View::Fpv;
            client.game_view = Some((zoom.0, view.nearclip));
            // In the client WoW's glow combines after the lens (the UI camera's backdrop), over
            // the goggles' bars; a camera has none of it anyway (NOTES, M8).
            if let Some(ffx) = ffx.as_deref_mut() {
                client.game_ffx = Some(*ffx);
                ffx.master = false;
            }
            body.request = Some(true);
            hide_ui(client, &mut ui_camera, &mut commands);
            info!("libfpv: FPV — F back to the pilot, F7 here again, F8 the overlay");
        }
    } else if back {
        leave_fpv(
            client,
            &mut body,
            &mut ui_camera,
            &mut commands,
            cam_entity,
            &mut zoom,
            &mut view,
            ffx.as_deref_mut(),
        );
        info!("libfpv: back to the pilot; the drone flies on (F7 to return)");
    }

    // What the game must not see, for the next frame's records.
    if client.view == View::Fpv || wants {
        client.held.extend(keys.get_pressed().copied());
    }
    client.held.retain(|k| keys.pressed(*k));
    let all = client.fpv.is_some() && (client.view == View::Fpv || wants);
    let mut keys_taken: Vec<KeyCode> = ALWAYS_TAKEN.to_vec();
    keys_taken.extend(client.held.iter().copied());
    // A stable order, so an unchanged set leaves the resource unchanged.
    keys_taken.sort_by_cached_key(|k| format!("{k:?}"));
    keys_taken.dedup();
    let want = HostInput {
        keys: keys_taken,
        all_keys: all,
        mouse: all,
    };
    taken.set_if_neq(want);

    let Some(fpv) = client.fpv.as_mut() else {
        return;
    };
    if mark {
        let pos = Vec3::from_array(client.frame.position);
        let fwd = Vec3::from_array(client.frame.camera.forward);
        info!(
            "libfpv: MARK drone at WoW {:.1?}, camera heading {:.0}°",
            bevy_to_wow(pos),
            fwd.x.atan2(-fwd.z).to_degrees().rem_euclid(360.0)
        );
    }
    if respawn {
        // At the pilot: on the ground just ahead of the character, at the launch heading.
        let q = client.launch.rotation;
        let p = launch_point(body.feet, q, &collision);
        match fpv
            .repair()
            .and_then(|()| fpv.reset_drone(p, q, &mut world))
        {
            Ok(()) => info!("libfpv: respawned at the pilot"),
            Err(e) => error!("libfpv: respawn — {e}"),
        }
    }
    // The pilot is the character, wherever it stands.
    if body.active {
        if let Err(e) = fpv.set_pilot_position(body.feet) {
            error!("libfpv: {e}");
        }
    }
    match client.view {
        View::Fpv => {
            fpv.set_view(ffi::FPV_VIEW_FPV);
            fpv.set_audio_listener(None);
        }
        View::Pilot => {
            fpv.set_view(ffi::FPV_VIEW_PILOT);
            fpv.set_audio_listener(Some((cam.translation, cam.rotation)));
        }
    }
    let dt = time.delta_secs();
    if let Err(e) = fpv.tick(dt, &mut client.frame, &mut world) {
        error!("libfpv: {e}");
        return;
    }
    let frame = &*client.frame;
    let first = *client.first_sim.get_or_insert(frame.sim_time);
    let started = frame.sim_time >= first + STARTUP_SECS;
    let fpv_view = client.view == View::Fpv && started;
    let mut osd_over = fpv_view;
    if fpv_view {
        client
            .osd_chip
            .refresh_font(|id, known| fpv.texture_if_changed(id, known));
        let applied = camera::apply(
            frame,
            &mut cam,
            camera_info,
            &mut zoom,
            &mut view,
            &collision,
            &mut client.osd_chip,
        );
        osd_over = !applied.view.as_ref().is_some_and(|v| v.osd_in_signal());
        camera::set_view(&mut commands, cam_entity, applied.view.clone());
        if now >= client.next_log {
            client.next_log = now + LOG_EVERY;
            log_state(frame, applied);
        }
    }
    if let Ok(win) = window.single() {
        publish(fpv, frame, win, dt, osd_over, &mut draw);
    }
}

/// Back to the pilot's view: the body attached, the UI shown, the game's camera settings back.
fn leave_fpv(
    client: &mut Client,
    body: &mut HostBody,
    ui_camera: &mut Query<(Entity, Option<&RenderLayers>), With<IsDefaultUiCamera>>,
    commands: &mut Commands,
    cam_entity: Entity,
    zoom: &mut FovZoom,
    view: &mut ViewDistance,
    ffx: Option<&mut FfxSwitches>,
) {
    client.view = View::Pilot;
    if let (Some(ffx), Some(game)) = (ffx, client.game_ffx.take()) {
        *ffx = game;
    }
    body.request = Some(false);
    camera::set_view(commands, cam_entity, None);
    if let Some((z, near)) = client.game_view.take() {
        zoom.0 = z;
        view.set_nearclip(near);
    }
    if let Some(layers) = client.ui_layers.take() {
        for (e, _) in ui_camera.iter() {
            commands.entity(e).insert(layers.clone());
        }
    }
}

/// Hides the game's UI: its camera looks at a layer nothing draws on, so its quads go while the
/// world it composites stays (NOTES, M8).
fn hide_ui(
    client: &mut Client,
    ui_camera: &mut Query<(Entity, Option<&RenderLayers>), With<IsDefaultUiCamera>>,
    commands: &mut Commands,
) {
    for (e, layers) in ui_camera.iter() {
        client.ui_layers = Some(layers.cloned().unwrap_or_default());
        commands.entity(e).insert(RenderLayers::layer(HIDDEN_LAYER));
    }
}

/// The camera's heading as a rotation about Y.
fn yaw_of(cam: &Transform) -> Quat {
    let f = cam.forward();
    Quat::from_rotation_y(f32::atan2(-f.x, -f.z))
}

/// The ground [`LAUNCH_AHEAD`] ahead of the pilot's feet along `facing`, or the feet.
fn launch_point(feet: Vec3, facing: Quat, collision: &WorldCollision) -> Vec3 {
    let ahead = feet + facing * Vec3::NEG_Z * LAUNCH_AHEAD + Vec3::Y * 2.0;
    collision
        .ray_body(ahead, Dir3::NEG_Y, 10.0)
        .map_or(feet, |h| ahead - Vec3::Y * h.distance)
}

/// Inits libfpv and makes the drone on the ground just ahead of the character, facing `facing`.
fn start(
    feet: Vec3,
    facing: Quat,
    collision: &WorldCollision,
    build: Option<&BuildId>,
    world: &mut dyn FnMut(Vec3, Dir3, f32) -> Option<RayHit>,
) -> Result<(Libfpv, Transform), String> {
    let ground = launch_point(feet, facing, collision);
    let build = build.map_or_else(|| env!("CARGO_PKG_VERSION").to_owned(), |b| b.summary());
    let mut fpv = Libfpv::init(
        &dll_path(),
        &InitOptions {
            manual_timing: false,
            flags: ffi::FPV_INIT_VIDEO_FEED,
            host_build: build,
        },
    )?;
    fpv.create_drone(ground, facing, feet, world)?;
    info!(
        "libfpv: drone launched at WoW {:.2?}, the pilot at WoW {:.2?}",
        bevy_to_wow(ground),
        bevy_to_wow(feet)
    );
    Ok((
        fpv,
        Transform::from_translation(ground).with_rotation(facing),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_heading_follows_the_camera() {
        // Looking down -Z (WoW north) is no turn; looking down -X a quarter turn left.
        let north = Transform::default().looking_to(Vec3::NEG_Z, Vec3::Y);
        assert!(yaw_of(&north).angle_between(Quat::IDENTITY) < 1e-4);
        let west = Transform::default().looking_to(Vec3::NEG_X, Vec3::Y);
        let fwd = yaw_of(&west) * Vec3::NEG_Z;
        assert!(fwd.distance(Vec3::NEG_X) < 1e-4, "{fwd:?}");
        // Pitch does not tilt the drone.
        let down = Transform::default().looking_to(Vec3::new(0.0, -1.0, -1.0), Vec3::Y);
        assert!((yaw_of(&down) * Vec3::Y).distance(Vec3::Y) < 1e-4);
    }
}
