//! The M1 roll test: the world camera flies a scripted path while rolling 360° over 4 s, to show
//! benilla's world draws under a roll its own camera never makes (culling, sky, billboards, the
//! interior and submersion tests).
//!
//! Live (`--roll-test <site>`), the path plays forward then back, rolling, so a person can watch it
//! for popping. With `--capture <dir>` it steps the path in [`POSES`] poses instead and, at each,
//! photographs the same position flat and rolled, a few frames apart, then exits. It writes:
//!
//! - `<dir>/<site>/flat/NN.png`: roll 0, masked to the centre disc.
//! - `<dir>/<site>/rolled/NN.png`: the rolled frame turned back upright, masked the same way, so it
//!   matches `flat/NN.png` pixel for pixel wherever the draw does not depend on roll.
//! - `<dir>/<site>/raw/NN_rDDD.png`: the rolled frame as rendered.
//! - `<dir>/<site>/still/NN.png`: before the path, [`STILL_SHOTS`] flat shots of pose 0, every
//!   other frame, with nothing moving: what changes between them is not the roll's doing.
//!
//! `benilla-visual diff-dir <site>/flat <site>/rolled --out <site>/diff` compares them. Pose 0 and
//! the last pose roll 0° and 360°: their diff is the noise floor (resampling, animation between the
//! two shots). `benilla-visual burst <site>/still` reads the stillness.

use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use benilla_assets::coords::{bevy_to_wow, wow_to_bevy};
use benilla_assets::WmoModel;
use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::WorldLoadProgress;
use benilla_world::view::{CameraPoseSet, WorldCamera};
use benilla_world::wmo_portal::{CameraInteriorClaim, WmoPortalInstance, WmoPvsSet};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

/// Seconds the path takes, and the one full roll over it.
const PATH_SECS: f32 = 4.0;

/// Poses a capture steps through, evenly over [`PATH_SECS`]: every 45°, both ends included.
const POSES: usize = 9;

/// Frames a capture pose is held before its shot, so a system reading last frame's camera has
/// caught up.
const HOLD_FRAMES: u32 = 6;

/// Shots in the still burst, two frames apart.
const STILL_SHOTS: usize = 8;

/// Wall seconds the world must read ready, at the probe point and again at the eye, before the
/// path starts; the readiness latch can read true before the last doodads are up.
const SETTLE_SECS: f32 = 6.0;

/// A place to fly, in raw WoW coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub name: &'static str,
    /// Start of the path, WoW x (north) and y (west).
    xy: [f32; 2],
    /// Where the ground probe starts, WoW z: below any roof over the start.
    probe_z: f32,
    /// Eye height over what the probe hits, yards.
    clearance: f32,
    /// Compass heading of the path, degrees: 0 north, 90 east. `None` takes the clearest of
    /// [`HEADINGS`] at the eye, by the collision ray, so the path does not run into a wall.
    heading: Option<f32>,
    /// Camera pitch, degrees, up positive.
    pitch: f32,
    /// Yards per second along the heading.
    speed: f32,
}

impl Site {
    const ALL: [Site; 4] = [
        // Stormwind's Trade District, at street level, up the street.
        Site {
            name: "streets",
            xy: [-8835.0, 630.0],
            probe_z: 120.0,
            clearance: 2.5,
            heading: Some(45.0),
            pitch: 0.0,
            speed: 4.0,
        },
        // Inside the Cathedral of Light.
        Site {
            name: "cathedral",
            xy: [-8515.0, 860.0],
            probe_z: 115.0,
            clearance: 3.0,
            heading: None,
            pitch: 5.0,
            speed: 3.0,
        },
        // Westfall farmland: terrain and sky, the horizon through the middle.
        Site {
            name: "open",
            xy: [-10500.0, 1300.0],
            probe_z: 400.0,
            clearance: 30.0,
            heading: None,
            pitch: -8.0,
            speed: 15.0,
        },
        // High over the Trade District, looking down: where the M0 viewer shot showed only fog.
        Site {
            name: "over-city",
            xy: [-8830.0, 620.0],
            probe_z: 400.0,
            clearance: 100.0,
            heading: Some(45.0),
            pitch: -30.0,
            speed: 10.0,
        },
    ];

    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name == name)
    }
}

/// What `--roll-test` asked for.
#[derive(Clone, Debug)]
pub struct RollTest {
    pub site: Site,
    /// Capture into this folder and exit, instead of playing live.
    pub capture: Option<PathBuf>,
}

pub struct RollTestPlugin(pub RollTest);

impl Plugin for RollTestPlugin {
    fn build(&self, app: &mut App) {
        info!(
            "libfpv: roll test at `{}` ({})",
            self.0.site.name,
            if self.0.capture.is_some() {
                "capture"
            } else {
                "live"
            }
        );
        app.insert_resource(State {
            test: self.0.clone(),
            eye_z: None,
            heading: self.0.site.heading.unwrap_or(0.0),
            phase: Phase::Settle { ready_since: None },
            pending: Arc::new(AtomicUsize::new(0)),
        })
        // After the viewer's own `fly`, which rewrites the camera every frame, and before
        // `publish_camera_pose` copies it into `GlobalTransform` for this frame's PVS, model
        // visibility and claim: unordered against it, some frames culled for `fly`'s view.
        .add_systems(
            Update,
            drive
                .after(WorldStage::Input)
                .before(CameraPoseSet)
                .before(WorldStage::Stream),
        )
        .add_systems(Update, log_claim.after(WmoPvsSet));
    }
}

#[derive(Resource)]
struct State {
    test: RollTest,
    /// The eye's WoW z, once the ground probe has run.
    eye_z: Option<f32>,
    /// The path's heading, the site's or the clearest found by the probe.
    heading: f32,
    phase: Phase,
    /// Shots requested and not yet written.
    pending: Arc<AtomicUsize>,
}

enum Phase {
    /// Hovering at the start until the world reads ready for [`SETTLE_SECS`].
    Settle { ready_since: Option<f32> },
    /// Capture only: shot `shot` of the still burst at pose 0, `held` frames since the last.
    Still { shot: usize, held: u32 },
    /// Holding pose `pose`, flat or rolled, for `held` frames so far.
    Capture {
        pose: usize,
        rolled: bool,
        held: u32,
    },
    /// Every shot requested; waiting for the writes.
    Drain { since: Option<f32> },
    /// Live: `t` seconds into the forward-and-back loop.
    Play { t: f32 },
}

/// The camera at `t` seconds along the path, with or without its roll.
fn pose(site: &Site, eye_z: f32, heading: f32, t: f32, rolled: bool) -> Transform {
    let yaw = -heading.to_radians();
    let start = wow_to_bevy([site.xy[0], site.xy[1], eye_z]);
    let along = Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
    let roll = if rolled { TAU * t / PATH_SECS } else { 0.0 };
    Transform::from_translation(start + along * site.speed * t).with_rotation(Quat::from_euler(
        EulerRot::YXZ,
        yaw,
        site.pitch.to_radians(),
        roll,
    ))
}

/// Logs each change of the camera's WMO interior claim: a claimed camera draws the world beyond
/// its building only through the building's portals, so the claim explains a view that loses the
/// outside.
fn log_claim(
    claim: Res<CameraInteriorClaim>,
    camera: Query<&GlobalTransform, With<WorldCamera>>,
    instances: Query<&WmoPortalInstance>,
    wmos: Res<Assets<WmoModel>>,
    assets: Res<AssetServer>,
) {
    if !claim.is_changed() {
        return;
    }
    let at = camera
        .single()
        .map(|t| bevy_to_wow(t.translation()))
        .unwrap_or_default();
    match claim.0 {
        Some(c) => {
            let inst = instances.get(c.room.instance).ok();
            let path = inst.and_then(|i| assets.get_path(&i.handle));
            let nav = inst
                .and_then(|i| wmos.get(&i.handle))
                .and_then(|m| m.group_nav.get(c.room.group as usize));
            info!(
                "libfpv: camera claimed by WMO group {} of {} (flags {:#x}, model-space bbox \
                 {:.1?}..{:.1?}), camera at WoW {:.1?}",
                c.room.group,
                path.map_or("?".into(), |p| p.to_string()),
                nav.map_or(0, |n| n.flags),
                nav.map(|n| n.bbox_min),
                nav.map(|n| n.bbox_max),
                at
            );
        }
        None => info!("libfpv: camera unclaimed (outdoors), camera at WoW {at:.1?}"),
    }
}

/// Headings the clear-path search tries, evenly round the compass.
const HEADINGS: usize = 32;

/// How far the clear-path search looks, yards.
const CLEAR_REACH: f32 = 200.0;

/// The compass heading, of [`HEADINGS`], whose level ray from `eye` runs furthest before it hits
/// anything, and that distance.
fn clearest_heading(collision: &WorldCollision, eye: Vec3) -> (f32, f32) {
    (0..HEADINGS)
        .map(|k| {
            let heading = k as f32 * 360.0 / HEADINGS as f32;
            let dir = Quat::from_rotation_y(-heading.to_radians()) * Vec3::NEG_Z;
            let clear = Dir3::new(dir)
                .ok()
                .and_then(|d| collision.ray_body(eye, d, CLEAR_REACH))
                .map_or(CLEAR_REACH, |hit| hit.distance);
            (heading, clear)
        })
        .fold((0.0, -1.0), |best, h| if h.1 > best.1 { h } else { best })
}

/// The world reads fully loaded around the camera.
fn world_ready(progress: Option<&WorldLoadProgress>) -> bool {
    progress.is_some_and(|p| p.complete && p.presentable())
}

fn drive(
    time: Res<Time<bevy::time::Real>>,
    mut state: ResMut<State>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let now = time.elapsed_secs();
    let state = &mut *state;
    let site = state.test.site;
    let eye_z = state.eye_z.unwrap_or(site.probe_z);
    let heading = state.heading;

    match &mut state.phase {
        Phase::Settle { ready_since } => {
            *cam = pose(&site, eye_z, heading, 0.0, false);
            if !world_ready(progress.as_deref()) {
                *ready_since = None;
                return;
            }
            let since = *ready_since.get_or_insert(now);
            if now - since < SETTLE_SECS {
                return;
            }
            if state.eye_z.is_none() {
                // Ground under the start, from below any roof: the eye goes `clearance` above it.
                let origin = wow_to_bevy([site.xy[0], site.xy[1], site.probe_z]);
                let z = match collision.ray_body(origin, Dir3::NEG_Y, 1000.0) {
                    Some(hit) => site.probe_z - hit.distance + site.clearance,
                    None => {
                        warn!("libfpv: roll test probe found no ground — flying at the probe z");
                        site.probe_z
                    }
                };
                info!("libfpv: roll test eye at WoW z {z:.2}");
                state.eye_z = Some(z);
                if site.heading.is_none() {
                    let eye = wow_to_bevy([site.xy[0], site.xy[1], z]);
                    let (h, clear) = clearest_heading(&collision, eye);
                    info!("libfpv: roll test heading {h:.1}°, clear for {clear:.1} yd");
                    state.heading = h;
                }
                *ready_since = None;
                return;
            }
            state.phase = if state.test.capture.is_some() {
                Phase::Still { shot: 0, held: 0 }
            } else {
                info!("libfpv: roll test playing");
                Phase::Play { t: 0.0 }
            };
        }
        Phase::Still { shot, held } => {
            *cam = pose(&site, eye_z, heading, 0.0, false);
            *held += 1;
            if *held < 2 {
                return;
            }
            *held = 0;
            let dir = state.test.capture.clone().unwrap_or_default();
            shoot(
                &mut commands,
                dir.join(site.name),
                *shot,
                Shot::Still,
                state.pending.clone(),
            );
            *shot += 1;
            if *shot == STILL_SHOTS {
                state.phase = Phase::Capture {
                    pose: 0,
                    rolled: false,
                    held: 0,
                };
            }
        }
        Phase::Capture {
            pose: i,
            rolled,
            held,
        } => {
            let t = PATH_SECS * *i as f32 / (POSES - 1) as f32;
            *cam = pose(&site, eye_z, heading, t, *rolled);
            *held += 1;
            if *held < HOLD_FRAMES {
                return;
            }
            let dir = state.test.capture.clone().unwrap_or_default();
            let kind = if *rolled {
                Shot::Rolled(TAU * t / PATH_SECS)
            } else {
                Shot::Flat
            };
            shoot(
                &mut commands,
                dir.join(site.name),
                *i,
                kind,
                state.pending.clone(),
            );
            *held = 0;
            if !*rolled {
                *rolled = true;
            } else if *i + 1 < POSES {
                *i += 1;
                *rolled = false;
            } else {
                state.phase = Phase::Drain { since: None };
            }
        }
        Phase::Drain { since } => {
            if state.pending.load(Ordering::Acquire) > 0 {
                return;
            }
            // A beat after the last write, so its log line lands before the exit.
            if now - *since.get_or_insert(now) > 1.0 {
                info!("libfpv: roll test capture written");
                exit.write(AppExit::Success);
            }
        }
        Phase::Play { t } => {
            *t = (*t + time.delta_secs()) % (2.0 * PATH_SECS);
            // Forward along the path, then back, rolling the whole time.
            let along = if *t < PATH_SECS {
                *t
            } else {
                2.0 * PATH_SECS - *t
            };
            let mut p = pose(&site, eye_z, heading, along, false);
            p.rotate_local_z(TAU * *t / PATH_SECS);
            *cam = p;
        }
    }
}

/// What a shot is, and so where it goes.
#[derive(Clone, Copy)]
enum Shot {
    /// The still burst: as rendered, into `still/`.
    Still,
    /// Roll 0, masked, into `flat/`.
    Flat,
    /// Rolled by this many radians: as rendered into `raw/`, turned upright and masked into
    /// `rolled/`.
    Rolled(f32),
}

/// Photographs this frame into `dir` as `kind` says.
fn shoot(
    commands: &mut Commands,
    dir: PathBuf,
    index: usize,
    kind: Shot,
    pending: Arc<AtomicUsize>,
) {
    pending.fetch_add(1, Ordering::AcqRel);
    commands
        .spawn(Screenshot::primary_window())
        .observe(move |shot: On<ScreenshotCaptured>| {
            let result = write_shot(&shot.image, &dir, index, kind);
            if let Err(e) = result {
                error!("libfpv: roll test shot {index} not written: {e}");
            }
            pending.fetch_sub(1, Ordering::AcqRel);
        });
}

fn write_shot(
    image: &Image,
    dir: &std::path::Path,
    index: usize,
    kind: Shot,
) -> Result<(), String> {
    let mut rgb = image
        .clone()
        .try_into_dynamic()
        .map_err(|e| e.to_string())?
        .to_rgb8();
    let (w, h) = rgb.dimensions();
    let name = format!("{index:02}.png");
    // `dir/sub/file`, its folder made.
    let path = |sub: &str, file: &str| -> Result<PathBuf, String> {
        let d = dir.join(sub);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
        Ok(d.join(file))
    };
    let (sub, theta) = match kind {
        Shot::Still => ("still", None),
        Shot::Flat => ("flat", Some(0.0)),
        Shot::Rolled(theta) => {
            let raw = format!("{index:02}_r{:03.0}.png", theta.to_degrees());
            rgb.save(path("raw", &raw)?).map_err(|e| e.to_string())?;
            ("rolled", Some(theta))
        }
    };
    if let Some(theta) = theta {
        unroll_masked(&mut rgb, w, h, theta);
    }
    rgb.save(path(sub, &name)?).map_err(|e| e.to_string())
}

/// Turns an RGB8 frame rendered with the camera rolled by `theta` back upright, bilinearly, and
/// blacks out everything past the centre disc, the only part both a flat and a rolled frame see.
///
/// The roll turns the camera about its view axis, so a point at `(dx, dy)` from the centre of the
/// upright frame (y down) sits at `(dx cos θ − dy sin θ, dx sin θ + dy cos θ)` in the rolled one.
fn unroll_masked(buf: &mut [u8], w: u32, h: u32, theta: f32) {
    let src = buf.to_vec();
    let (w, h) = (w as usize, h as usize);
    let (cx, cy) = ((w as f32 - 1.0) * 0.5, (h as f32 - 1.0) * 0.5);
    let r = (w.min(h) as f32) * 0.5 - 2.0;
    let (s, c) = theta.sin_cos();
    let at = |x: usize, y: usize, ch: usize| src[(y * w + x) * 3 + ch] as f32;
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let o = (y * w + x) * 3;
            if dx * dx + dy * dy > r * r {
                buf[o..o + 3].fill(0);
                continue;
            }
            let sx = cx + dx * c - dy * s;
            let sy = cy + dx * s + dy * c;
            let (x0, y0) = (sx.floor().max(0.0) as usize, sy.floor().max(0.0) as usize);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            for ch in 0..3 {
                let top = at(x0, y0, ch) * (1.0 - fx) + at(x1, y0, ch) * fx;
                let bot = at(x0, y1, ch) * (1.0 - fx) + at(x1, y1, ch) * fx;
                buf[o + ch] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter roll moves what was right of centre to below it, as the camera's right axis
    /// turns up and its down axis turns to face the world's right.
    #[test]
    fn a_quarter_roll_turns_back_upright() {
        let (w, h) = (9u32, 9u32);
        let mut rolled = vec![0u8; 9 * 9 * 3];
        // A rolled-frame pixel straight below the centre...
        rolled[(6 * 9 + 4) * 3] = 255;
        unroll_masked(&mut rolled, w, h, TAU / 4.0);
        // ...is upright right of the centre.
        assert_eq!(rolled[(4 * 9 + 6) * 3], 255);
    }

    #[test]
    fn roll_zero_only_masks() {
        let (w, h) = (16u32, 12u32);
        let mut buf: Vec<u8> = (0..16 * 12 * 3).map(|i| (i % 251) as u8).collect();
        let before = buf.clone();
        unroll_masked(&mut buf, w, h, 0.0);
        // The centre survives untouched, the corner is masked.
        let centre = (6 * 16 + 8) * 3;
        assert_eq!(buf[centre..centre + 3], before[centre..centre + 3]);
        assert_eq!(buf[0..3], [0, 0, 0]);
    }
}
