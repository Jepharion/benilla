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

use benilla_app::{HostBody, HostChat, HostInput, HostModel};
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
                (drive, fit_model, place_model, stand_in)
                    .chain()
                    .after(WorldStage::Input)
                    .before(CameraPoseSet)
                    .before(WorldStage::Stream),
            );
    }
}

/// The drone's model: "Xiggs Fuselighter's Flyingmachine" (creature 8446), display 8011,
/// `Creature\FlyingMachineCreature\FlyingMachineCreature.mdx`, read from the player's install.
const MODEL_DISPLAY: u32 = 8011;

/// A quad's tip-to-tip span over its motor-to-motor diagonal: the props reach past the motors by
/// about a prop radius at each end (225 mm frame, 5.1" props: 355 mm, ×1.58; a 65 mm whoop with
/// 31 mm props, ×1.48).
const SPAN_PER_DIAGONAL: f32 = 1.55;

/// Marks the drone's model entity.
#[derive(Component)]
struct DroneModel;

/// The model as loaded, at scale 1, in its own frame: its widest horizontal extent (yards) and the
/// centre of its bounds, which goes on the drone's position.
#[derive(Clone, Copy, Debug)]
struct ModelFit {
    width: f32,
    center: Vec3,
}

/// Measures the model once its parts have loaded: every descendant's mesh bounds, in the model's
/// own frame. Parts arrive over a few frames, so it waits until their count holds still.
fn fit_model(
    mut client: ResMut<Client>,
    roots: Query<&GlobalTransform, With<DroneModel>>,
    children: Query<&Children>,
    parts: Query<(&GlobalTransform, &bevy::camera::primitives::Aabb)>,
) {
    let client = &mut *client;
    let (Some(root), None) = (client.model, client.fit) else {
        return;
    };
    let Ok(root_gt) = roots.get(root) else {
        return;
    };
    let to_local = root_gt.affine().inverse();
    let (mut lo, mut hi, mut n) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN), 0usize);
    for e in children.iter_descendants(root) {
        let Ok((gt, aabb)) = parts.get(e) else {
            continue;
        };
        let m = to_local * gt.affine();
        let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
        for corner in 0..8 {
            let s = Vec3::new(
                if corner & 1 == 0 { -1.0 } else { 1.0 },
                if corner & 2 == 0 { -1.0 } else { 1.0 },
                if corner & 4 == 0 { -1.0 } else { 1.0 },
            );
            let p = m.transform_point3(c + h * s);
            lo = lo.min(p);
            hi = hi.max(p);
        }
        n += 1;
    }
    if n == 0 || n != client.fit_parts.0 {
        client.fit_parts = (n, 0);
        return;
    }
    client.fit_parts.1 += 1;
    if client.fit_parts.1 < 10 {
        return;
    }
    let size = hi - lo;
    let fit = ModelFit {
        width: size.x.max(size.z),
        center: (lo + hi) * 0.5,
    };
    info!(
        "libfpv: the drone's model loaded: {n} parts, {:.2} x {:.2} x {:.2} yd at scale 1, centre {:.2?}",
        size.x, size.y, size.z, fit.center
    );
    client.fit = Some(fit);
}

/// The model at the drone, with its attitude, sized to the quad flown (its motor diagonal ×
/// [`SPAN_PER_DIAGONAL`]); hidden in the drone's own view and until it is measured.
fn place_model(
    client: Res<Client>,
    mut models: Query<(&mut Transform, &mut Visibility, &mut HostModel), With<DroneModel>>,
) {
    let Some(root) = client.model else {
        return;
    };
    let Ok((mut t, mut vis, mut model)) = models.get_mut(root) else {
        return;
    };
    let Some(fit) = client.fit else {
        return;
    };
    let frame = &*client.frame;
    let diagonal = frame
        .motors
        .iter()
        .map(|m| 2.0 * Vec2::new(m.position[0], m.position[2]).length())
        .fold(0.0f32, f32::max);
    if diagonal > 0.0 && fit.width > 0.0 {
        // `LIBFPV_TEST_MODEL_SCALE` enlarges it for an unattended photo.
        let test = std::env::var("LIBFPV_TEST_MODEL_SCALE")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0);
        let scale = diagonal * SPAN_PER_DIAGONAL / fit.width * test;
        if (model.scale - scale).abs() > scale * 0.01 {
            info!(
                "libfpv: the drone's model at scale {scale:.4}: {:.2} m across (motor diagonal {:.0} mm)",
                fit.width * scale / crate::runtime::UNITS_PER_METER,
                diagonal / crate::runtime::UNITS_PER_METER * 1000.0
            );
            model.scale = scale;
        }
    }
    // A unit model faces -Z (benilla's `net::motion::wire_yaw`), as the drone flies.
    let rot = Quat::from_array(frame.rotation).normalize();
    t.rotation = rot;
    // The render scale: a guid-less model takes `HostModel.scale` once, at its build.
    t.scale = Vec3::splat(model.scale);
    t.translation = Vec3::from_array(frame.position) - rot * (fit.center * model.scale);
    // In the drone's own view it must not be seen: its props spun through the camera. The
    // `Visibility` alone doesn't hold, since benilla's exterior cull writes every world unit's
    // each frame (the user saw them "from time to time"), so it also goes far below the world.
    let fpv = client.view == View::Fpv;
    if fpv {
        t.translation -= Vec3::Y * OUT_OF_SIGHT;
    }
    vis.set_if_neq(if fpv {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    });
}

/// How far below the drone its model waits in the drone's own view, yards.
const OUT_OF_SIGHT: f32 = 1000.0;

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
    /// The drone's model entity, its measured fit, and the fit's part count and how many frames
    /// it has held.
    model: Option<Entity>,
    fit: Option<ModelFit>,
    fit_parts: (usize, u32),
    /// The stand-in is on ([`stand_in`]), and until when our own body keeps its look after it.
    standing: bool,
    look_until: Option<f32>,
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
            model: None,
            fit: None,
            fit_parts: (0, 0),
            standing: false,
            look_until: None,
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

/// Seconds of the scripted flight in FPV before going back (as F); it disarms a second later.
const SCRIPT_FLIGHT: f32 = 7.0;

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
    let mut script_channels = None;
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
        // The scripted flight after the launch: arm in angle mode, climb, then fly gently
        // forward, hover, and disarm once back at the pilot.
        let s = t - SCRIPT_LAUNCH;
        if (0.0..SCRIPT_FLIGHT + 2.0).contains(&s) {
            let mut ch = [-1.0f32; ffi::FPV_CHANNEL_COUNT];
            ch[0] = 0.0;
            ch[1] = 0.0;
            ch[3] = 0.0;
            if (1.0..SCRIPT_FLIGHT + 1.0).contains(&s) {
                ch[4] = 1.0; // AUX1 arm
                ch[5] = 1.0; // AUX2 angle
                ch[2] = match s {
                    s if s < 1.5 => -1.0,
                    s if s < 2.5 => -0.3,
                    _ => -0.5,
                };
                if (2.5..SCRIPT_FLIGHT - 1.0).contains(&s) {
                    ch[1] = 0.25; // pitch forward
                }
            }
            script_channels = Some(ch);
        }
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
        } else if step(1, SCRIPT_LAUNCH + SCRIPT_FLIGHT - 1.5) {
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
        } else if step(2, SCRIPT_LAUNCH + SCRIPT_FLIGHT) {
            info!("libfpv: TEST back (as F)");
            back = true;
        } else if step(3, SCRIPT_LAUNCH + SCRIPT_FLIGHT + 3.0) {
            info!(
                "libfpv: TEST shot pilot — view {:?}, detached {}, keys all {}, drone WoW {:.2?}, feet WoW {:.2?}",
                client.view,
                body.detached,
                taken.all_keys,
                bevy_to_wow(Vec3::from_array(client.frame.position)),
                bevy_to_wow(body.feet)
            );
            crate::flight_test::shoot(
                &mut commands,
                script.dir.clone(),
                "client-pilot",
                None,
                None,
                None,
            );
        } else if step(4, SCRIPT_LAUNCH + SCRIPT_FLIGHT + 5.0) {
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
                    // The drone as the pilot sees it ([`place_model`]), measured at scale 1.
                    client.model = Some(
                        commands
                            .spawn((
                                DroneModel,
                                HostModel {
                                    display_id: MODEL_DISPLAY,
                                    scale: 1.0,
                                },
                                Transform::from_translation(at.translation),
                                Visibility::Hidden,
                            ))
                            .id(),
                    );
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
    if let Some(ch) = script_channels {
        fpv.set_channels(&ch);
    }
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
    // Y, or the overlay's button: the pilot (the character) moves to the ground under the drone,
    // and home with it.
    let mut move_here = client.view == View::Fpv
        && keys.just_pressed(KeyCode::KeyY)
        && !wants
        && !client.held.contains(&KeyCode::KeyY);
    while let Some(id) = fpv.poll_overlay_action() {
        move_here |= id == ACTION_MOVE_HERE;
    }
    if move_here && body.active {
        let drone = Vec3::from_array(client.frame.position);
        match collision.ray_body(drone, Dir3::NEG_Y, 500.0) {
            Some(h) => {
                let ground = drone - Vec3::Y * h.distance;
                body.place = Some(ground);
                fpv.gps_set_home(ground);
                info!(
                    "libfpv: the pilot moves here, WoW {:.1?}",
                    bevy_to_wow(ground)
                );
            }
            None => warn!("libfpv: no ground under the drone to move to"),
        }
    }
    // The pilot is the character at home, wherever it stands.
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

/// The stand-in's default distances, metres (INTEGRATION.md "The player's choices": it takes over
/// beyond the first and ends within the second).
const STAND_IN_M: f32 = 20.0;
const STAND_IN_RETURN_M: f32 = 10.0;

/// The overlay's Fly-tab button for "move here".
const ACTION_MOVE_HERE: u32 = 1;

/// Seconds our own body keeps its look after the demorph is sent: the server's answer, which
/// would rebuild it, comes back within this.
const LOOK_HOLD: f32 = 3.0;

/// The stand-in (INTEGRATION.md "Standing in for the drone"): in FPV, once the drone is past the
/// player's stand-in distance, the server's character goes with it, so the world loads, populates
/// and reacts around the drone. To other players it is the drone: a GM morph into the flying
/// machine at the drone's size. Here the body stays home, as itself ([`HostBody::stand_in`],
/// [`HostBody::keep_look`]), which is the pilot left standing where the flight started. Back
/// within the return distance, or in the pilot's view, the character is home and itself again.
fn stand_in(
    mut client: ResMut<Client>,
    mut body: ResMut<HostBody>,
    mut chat: MessageWriter<HostChat>,
    time: Res<Time<bevy::time::Real>>,
) {
    let now = time.elapsed_secs();
    let client = &mut *client;
    if let Some(until) = client.look_until {
        if !client.standing && now >= until {
            body.keep_look = false;
            client.look_until = None;
        }
    }
    let Some(fpv) = client.fpv.as_ref() else {
        return;
    };
    if !body.active {
        client.standing = false;
        body.stand_in = None;
        return;
    }
    let cfg = fpv.pilot_config();
    let frame = &*client.frame;
    let drone = Vec3::from_array(frame.position);
    let far = drone.distance(body.feet);
    let want = client.view == View::Fpv
        && cfg.features & ffi::FPV_PILOT_STAND_IN != 0
        && if client.standing {
            far > cfg.stand_in_return_distance
        } else {
            far > cfg.stand_in_distance
        };
    // The server's scale for the disguise: the model's, as on this screen.
    let scale = client.fit.map_or(0.1, |fit| {
        let diagonal = frame
            .motors
            .iter()
            .map(|m| 2.0 * Vec2::new(m.position[0], m.position[2]).length())
            .fold(0.0f32, f32::max);
        diagonal * SPAN_PER_DIAGONAL / fit.width
    });
    if want != client.standing {
        client.standing = want;
        if want {
            body.keep_look = true;
            client.look_until = None;
            chat.write(HostChat(format!(".modify morph {MODEL_DISPLAY}")));
            chat.write(HostChat(format!(".modify scale {scale:.3}")));
            info!(
                "libfpv: stand-in on — the character goes with the drone ({:.0} m from the pilot), \
                 disguised as the flying machine at scale {scale:.3}",
                far / crate::runtime::UNITS_PER_METER
            );
        } else {
            chat.write(HostChat(".demorph".into()));
            chat.write(HostChat(".modify scale 1".into()));
            client.look_until = Some(now + LOOK_HOLD);
            info!("libfpv: stand-in off — the character is home and itself");
        }
    }
    body.stand_in = if !body.detached {
        None
    } else if client.standing {
        // Its feet so the disguise's centre is on the drone, facing the drone's heading.
        let rot = Quat::from_array(frame.rotation).normalize();
        let lift = client.fit.map_or(0.0, |fit| fit.center.y * scale);
        let f = rot * Vec3::NEG_Z;
        Some((drone - Vec3::Y * lift, f32::atan2(-f.x, -f.z)))
    } else {
        Some((body.feet, body.facing))
    };
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
    // `LIBFPV_TEST_LAUNCH_AHEAD` moves it further out, for an unattended photo of the model.
    let reach = std::env::var("LIBFPV_TEST_LAUNCH_AHEAD")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(LAUNCH_AHEAD);
    let ahead = feet + facing * Vec3::NEG_Z * reach + Vec3::Y * 2.0;
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
    // The stand-in, libfpv's distances (the player changes them in F8 > Fly > Pilot); no piloting
    // pose yet. Home is where the flight starts.
    fpv.pilot_set_support(
        ffi::FPV_PILOT_STAND_IN,
        STAND_IN_M * crate::runtime::UNITS_PER_METER,
        STAND_IN_RETURN_M * crate::runtime::UNITS_PER_METER,
    );
    fpv.gps_set_home(feet);
    fpv.set_overlay_action(ACTION_MOVE_HERE, "Move the pilot here (Y)");
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

/// `LIBFPV_WATCH=<seconds>`: a second client's eye for the M8 check. Ten times a second it logs
/// every other player as this client draws it (position, display, scale), then exits after that
/// many seconds in the world.
#[derive(Resource)]
pub struct Watch {
    secs: f32,
    since: Option<f32>,
    next: f32,
    /// `LIBFPV_WATCH_AT=x,y,z` (WoW): where the watcher goes first, by GM `.go xyz`.
    go: Option<String>,
}

impl Watch {
    /// From the environment; `None` without `LIBFPV_WATCH`.
    pub fn from_env() -> Option<Self> {
        let secs = std::env::var("LIBFPV_WATCH").ok()?.parse().ok()?;
        Some(Self {
            secs,
            since: None,
            next: 0.0,
            go: std::env::var("LIBFPV_WATCH_AT")
                .ok()
                .map(|v| format!(".go xyz {}", v.replace(',', " "))),
        })
    }
}

pub fn watch_players(
    mut watch: ResMut<Watch>,
    time: Res<Time<bevy::time::Real>>,
    body: Res<HostBody>,
    objects: benilla_app::Objects,
    transforms: Query<&GlobalTransform>,
    mut exit: MessageWriter<AppExit>,
    mut chat: MessageWriter<HostChat>,
) {
    if !body.active {
        return;
    }
    if let Some(go) = watch.go.take() {
        chat.write(HostChat(go));
    }
    let now = time.elapsed_secs();
    let since = *watch.since.get_or_insert(now);
    if now - since > watch.secs {
        info!("libfpv: WATCH done");
        exit.write(AppExit::Success);
        return;
    }
    if now < watch.next {
        return;
    }
    watch.next = now + 0.1;
    // TYPEMASK_PLAYER.
    const PLAYER: u32 = 0x10;
    for (guid, entity, fields) in objects.iter() {
        if fields.object_type_mask().unwrap_or(0) & PLAYER == 0 {
            continue;
        }
        let Ok(gt) = transforms.get(entity) else {
            continue;
        };
        let p = gt.translation();
        if p.distance(body.feet) < 0.01 {
            continue; // our own body
        }
        info!(
            "libfpv: WATCH t {:.1} guid {guid:#x} at WoW {:.2?} display {:?} scale {:?}",
            now - since,
            bevy_to_wow(p),
            fields.unit_displayid(),
            fields.object_scale_x()
        );
    }
}
