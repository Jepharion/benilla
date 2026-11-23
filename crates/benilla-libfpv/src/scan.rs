//! A collision survey of an area, a dev tool for finding holes in what the drone collides with:
//! `benilla-libfpv --worldview --scan x0,y0,x1,y1[,z]` (a rectangle in raw WoW coordinates). Once it
//! has streamed in, it looks for covered spots (tunnels, arches, rooms) on a [`STEP`] yd grid,
//! and checks their walls, with the drone's own collision ray (`WorldCollision::ray_body`):
//!
//! - Up from below the level surveyed (the optional fifth number, default [`UNDER_Z`]). The ray
//!   is one-sided, so it passes up through the backs of floors and stops at the first ceiling's
//!   underside.
//! - Down from just under that ceiling to the floor. A ceiling 2–9 yd over its floor is cover.
//! - From 1.5 yd over that floor, [`SIDES`] level rays out to [`WALL_REACH`] yd, counting upright
//!   faces.
//!
//! It logs a map, one character per cell (`.` open, `#` covered with walls, `?` covered with
//! at most one wall within reach: a passage whose walls the drone does not see), the `?` cells
//! with their numbers, and exits.

use benilla_assets::coords::wow_to_bevy;
use benilla_world::collision::WorldCollision;
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::WorldLoadProgress;
use benilla_world::view::{CameraPoseSet, WorldCamera};
use bevy::prelude::*;

use crate::roll_test::world_ready;

/// A ray's distance and the hit's normal, or a miss.
type RayAnswer = Option<(f32, Vec3)>;
/// A cast: origin, direction, reach.
type RayCast<'a> = dyn Fn(Vec3, Vec3, f32) -> RayAnswer + 'a;

/// Yards between grid points.
const STEP: f32 = 3.0;
/// Where the up rays start by default, WoW z: under Stormwind's canal level and over the land below.
const UNDER_Z: f32 = 75.0;
/// Level rays per covered point, and how far they look (yards).
const SIDES: usize = 8;
const WALL_REACH: f32 = 12.0;

#[derive(Clone, Copy, Debug)]
pub struct Scan {
    pub from: [f32; 2],
    pub to: [f32; 2],
    /// Where the up rays start, WoW z.
    pub under: f32,
}

impl Scan {
    /// `x0,y0,x1,y1[,z]`: the rectangle, and where the up rays start.
    pub fn parse(s: &str) -> Option<Self> {
        let v: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        let (x0, y0, x1, y1, under) = match *v.as_slice() {
            [a, b, c, d] => (a, b, c, d, UNDER_Z),
            [a, b, c, d, z] => (a, b, c, d, z),
            _ => return None,
        };
        Some(Self {
            from: [x0.min(x1), y0.min(y1)],
            to: [x0.max(x1), y0.max(y1)],
            under,
        })
    }
}

pub struct ScanPlugin(pub Scan);

#[derive(Resource)]
struct ScanRun {
    scan: Scan,
    ready_since: Option<f32>,
    done_at: Option<f32>,
}

impl Plugin for ScanPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ScanRun {
            scan: self.0,
            ready_since: None,
            done_at: None,
        })
        .add_systems(
            Update,
            survey
                .after(WorldStage::Input)
                .before(CameraPoseSet)
                .before(WorldStage::Stream),
        );
    }
}

/// One covered grid point.
struct Cover {
    wow: [f32; 2],
    floor: f32,
    ceiling: f32,
    walls: usize,
    nearest: Option<f32>,
    /// The wall each level ray found, N, NE, E, SE, S, SW, W, NW (yards).
    dirs: [Option<f32>; SIDES],
}

/// Areas up to this many points also list every covered point with its walls.
const DETAIL_POINTS: usize = 1500;

fn survey(
    time: Res<Time<bevy::time::Real>>,
    mut run: ResMut<ScanRun>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if let Some(at) = run.done_at {
        if now - at > 1.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    let s = run.scan;
    // Hold the camera over the middle, so all of it streams in.
    let mid = [(s.from[0] + s.to[0]) * 0.5, (s.from[1] + s.to[1]) * 0.5];
    if let Ok(mut cam) = camera.single_mut() {
        *cam = Transform::from_translation(wow_to_bevy([mid[0], mid[1], 150.0]))
            .looking_to(Vec3::NEG_Y, Vec3::NEG_Z);
    }
    if !world_ready(progress.as_deref()) {
        run.ready_since = None;
        return;
    }
    if now - *run.ready_since.get_or_insert(now) < 6.0 {
        return;
    }

    // What the drone collides with.
    let body = |o: Vec3, d: Vec3, reach: f32| {
        let hit = collision.ray_body(o, Dir3::new(d).ok()?, reach)?;
        Some((hit.distance, hit.normal))
    };
    survey_pass(&body, &s);
    run.done_at = Some(now);
}

/// One survey of `s` with `cast`: the map, then the details.
fn survey_pass(cast: &RayCast<'_>, s: &Scan) {
    let (nx, ny) = (
        ((s.to[0] - s.from[0]) / STEP) as usize + 1,
        ((s.to[1] - s.from[1]) / STEP) as usize + 1,
    );
    info!(
        "libfpv: scan WoW {:?}..{:?}, {nx}×{ny} points {STEP} yd apart; map rows run north (x) \
         down the page, columns west (y) to the right: `.` open, `#` covered with walls, `?` \
         covered with at most one wall within {WALL_REACH} yd",
        s.from, s.to
    );
    let mut holes: Vec<Cover> = Vec::new();
    let mut detail: Vec<Cover> = Vec::new();
    let mut covered = 0usize;
    for i in (0..nx).rev() {
        let mut row = String::with_capacity(ny);
        for j in 0..ny {
            let wow = [s.from[0] + i as f32 * STEP, s.from[1] + j as f32 * STEP];
            let cell = cover_at(cast, wow, s.under);
            row.push(match &cell {
                None => '.',
                Some(c) if c.walls <= 1 => '?',
                Some(_) => '#',
            });
            if let Some(c) = cell {
                covered += 1;
                if nx * ny <= DETAIL_POINTS {
                    detail.push(c);
                } else if c.walls <= 1 {
                    holes.push(c);
                }
            }
        }
        info!("libfpv: scan {:7.0} {row}", s.from[0] + i as f32 * STEP);
    }
    info!(
        "libfpv: scan found {covered} covered points, {} with at most one wall",
        holes.len()
    );
    for h in &holes {
        info!(
            "libfpv: scan ? WoW [{:.0}, {:.0}] floor {:.2} ceiling {:.2} ({:.1} yd), walls {}/{SIDES}, \
             nearest {:?}",
            h.wow[0],
            h.wow[1],
            h.floor,
            h.ceiling,
            h.ceiling - h.floor,
            h.walls,
            h.nearest.map(|d| (d * 10.0).round() / 10.0)
        );
    }
    let d = |v: Option<f32>| v.map_or("  -  ".into(), |x| format!("{x:5.1}"));
    for c in &detail {
        info!(
            "libfpv: scan cell WoW [{:.0}, {:.0}] floor {:6.2} ceiling {:6.2} | N {} NE {} E {} SE \
             {} S {} SW {} W {} NW {}",
            c.wow[0],
            c.wow[1],
            c.floor,
            c.ceiling,
            d(c.dirs[0]),
            d(c.dirs[1]),
            d(c.dirs[2]),
            d(c.dirs[3]),
            d(c.dirs[4]),
            d(c.dirs[5]),
            d(c.dirs[6]),
            d(c.dirs[7])
        );
    }
}

/// The cover over `wow`, if a ceiling 2–9 yd over a floor is there, with its walls counted.
fn cover_at(cast: &RayCast<'_>, wow: [f32; 2], under_z: f32) -> Option<Cover> {
    let under = wow_to_bevy([wow[0], wow[1], under_z]);
    let (up, n) = cast(under, Vec3::Y, 60.0)?;
    if n.y > -0.5 {
        return None; // not a ceiling's underside
    }
    let ceiling = under_z + up;
    let below = Vec3::new(under.x, ceiling - 0.3, under.z);
    let (down, n) = cast(below, Vec3::NEG_Y, 12.0)?;
    if n.y < 0.5 {
        return None;
    }
    let floor = ceiling - 0.3 - down;
    let height = ceiling - floor;
    if !(2.0..=9.0).contains(&height) {
        return None;
    }
    let eye = Vec3::new(under.x, floor + 1.5, under.z);
    let mut walls = 0;
    let mut nearest: Option<f32> = None;
    let mut dirs = [None; SIDES];
    for (k, slot) in dirs.iter_mut().enumerate() {
        // Compass order from north: Bevy -Z is north and +X east.
        let a = k as f32 * std::f32::consts::TAU / SIDES as f32;
        let d = Vec3::new(a.sin(), 0.0, -a.cos());
        if let Some((dist, n)) = cast(eye, d, WALL_REACH) {
            if n.y.abs() < 0.3 {
                walls += 1;
                nearest = Some(nearest.map_or(dist, |m: f32| m.min(dist)));
                *slot = Some(dist);
            }
        }
    }
    Some(Cover {
        wow,
        floor,
        ceiling,
        walls,
        nearest,
        dirs,
    })
}

/// A look along a path, the dev tool for a reported spot: `benilla-libfpv --worldview --probe
/// x,y,z,heading[,shot.png]`. From that point (raw WoW coordinates) it casts level rays along the
/// heading and 15° either side, at the point's height and 1 yd and 2 yd above and below it, with
/// the drone's collision ray (`ray_body`), logs what each hits, and with a file name photographs
/// the view along the heading: a face the photo shows and the rays miss is one the drone flies
/// through.
#[derive(Clone, Debug)]
pub struct Probe {
    pub at: [f32; 3],
    pub heading: f32,
    pub shot: Option<std::path::PathBuf>,
}

impl Probe {
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.split(',');
        let mut num = || parts.next()?.trim().parse::<f32>().ok();
        let (x, y, z, heading) = (num()?, num()?, num()?, num()?);
        let shot = s
            .split(',')
            .nth(4)
            .map(|p| std::path::PathBuf::from(p.trim()));
        Some(Self {
            at: [x, y, z],
            heading,
            shot,
        })
    }
}

pub struct ProbePlugin(pub Probe);

#[derive(Resource)]
struct ProbeRun {
    probe: Probe,
    ready_since: Option<f32>,
    fired: Option<f32>,
}

impl Plugin for ProbePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ProbeRun {
            probe: self.0.clone(),
            ready_since: None,
            fired: None,
        })
        .add_systems(
            Update,
            probe
                .after(WorldStage::Input)
                .before(CameraPoseSet)
                .before(WorldStage::Stream),
        );
    }
}

fn probe(
    time: Res<Time<bevy::time::Real>>,
    mut run: ResMut<ProbeRun>,
    progress: Option<Res<WorldLoadProgress>>,
    collision: WorldCollision,
    mut camera: Query<&mut Transform, With<WorldCamera>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let p = run.probe.clone();
    let eye = wow_to_bevy(p.at);
    let fwd = Quat::from_rotation_y(-p.heading.to_radians()) * Vec3::NEG_Z;
    if let Ok(mut cam) = camera.single_mut() {
        *cam = Transform::from_translation(eye).looking_to(fwd, Vec3::Y);
    }
    if let Some(at) = run.fired {
        if now - at > 3.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    if !world_ready(progress.as_deref()) {
        run.ready_since = None;
        return;
    }
    if now - *run.ready_since.get_or_insert(now) < 6.0 {
        return;
    }
    info!(
        "libfpv: probe at WoW {:?} heading {}°: distance (yd) to the first face; `-` = nothing \
         within 40 yd",
        p.at, p.heading
    );
    let show = |h: Option<f32>| h.map_or("  -  ".to_owned(), |d| format!("{d:5.1}"));
    for dz in [-2.0f32, -1.0, 0.0, 1.0, 2.0] {
        let mut line = format!("z {:+.0}:", dz);
        for side in [-15.0f32, 0.0, 15.0] {
            let d = Quat::from_rotation_y(-side.to_radians()) * fwd;
            let Ok(dir) = Dir3::new(d) else { continue };
            let o = eye + Vec3::Y * dz;
            let b = collision.ray_body(o, dir, 40.0).map(|h| h.distance);
            line += &format!("  {side:+.0}° {}", show(b));
        }
        info!("libfpv: probe {line}");
    }
    if let Some(path) = p.shot.clone() {
        commands
            .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
            .observe(bevy::render::view::screenshot::save_to_disk(path));
    }
    run.fired = Some(now);
}
