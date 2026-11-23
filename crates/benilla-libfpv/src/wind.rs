//! The wind the drone flies in (M9), from benilla's weather (docs/INTEGRATION.md "Wind"). WoW
//! 1.12 has no wind of its own: `SMSG_WEATHER` carries a kind and an intensity, and the client's
//! "weather wind" is the player's own motion, which only tilts the rain
//! (`benilla-world/src/weather/precip/wind.rs`). So the speed comes from the weather, at the
//! guide's figures (2–3 m/s a light breeze, 5–8 breezy, 10–12 a storm), and the direction is
//! made up: a heading per map that wanders slowly. libfpv adds the rest where the drone is: gusts,
//! more wind higher up, the lee of buildings, updrafts at walls. Inside a building no weather
//! reaches (`FPV_WIND_ENCLOSED`). The player turns it off or scales it in F8 > Fly > Wind.

use benilla_world::weather::{WeatherKind, WeatherState};
use bevy::prelude::*;

use crate::runtime::{Libfpv, UNITS_PER_METER};

/// Metres per second: fine weather's breeze, and each weather's calm and full-strength speeds.
const FINE: f32 = 2.5;
const RAIN: (f32, f32) = (3.0, 9.0);
const SNOW: (f32, f32) = (2.0, 7.0);
const SAND: (f32, f32) = (4.0, 12.0);

/// How far the heading swings either side of the map's own, degrees, and its period, seconds.
const SWING_DEG: f32 = 40.0;
const SWING_PERIOD: f64 = 600.0;

/// The mean wind speed for a weather, m/s; `density` is its effect strength, 0..1 (benilla's
/// `WeatherState::effect_density`, which ramps over about 10 s as the reference's does).
pub fn speed(kind: WeatherKind, density: f32) -> f32 {
    let d = density.clamp(0.0, 1.0);
    let lerp = |(calm, full): (f32, f32)| calm + (full - calm) * d;
    match kind {
        WeatherKind::Fine => FINE,
        WeatherKind::Rain => lerp(RAIN),
        WeatherKind::Snow => lerp(SNOW),
        WeatherKind::Sand => lerp(SAND),
    }
}

/// The way the air moves, a horizontal unit vector in benilla's axes: a heading fixed per map,
/// swinging [`SWING_DEG`] either side over [`SWING_PERIOD`].
pub fn direction(map: u32, t: f64) -> Vec3 {
    // A map's own heading, spread over the circle (the golden angle).
    let base = (map as f32 * 137.507_76).to_radians();
    let swing = SWING_DEG.to_radians() * (t / SWING_PERIOD * std::f64::consts::TAU).sin() as f32;
    let a = base + swing;
    Vec3::new(a.sin(), 0.0, -a.cos())
}

/// Hands libfpv this frame's wind at `drone`: the weather's, or none inside a building.
pub fn apply(
    fpv: &Libfpv,
    weather: Option<&WeatherState>,
    map: u32,
    t: f64,
    indoors: bool,
) -> Vec3 {
    let (kind, density) = weather.map_or((WeatherKind::Fine, 0.0), |w| {
        (w.effect_kind, w.effect_density)
    });
    let velocity = direction(map, t) * speed(kind, density) * UNITS_PER_METER;
    fpv.set_wind(velocity, indoors);
    velocity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speed_follows_the_weather_inside_the_guides_ranges() {
        assert_eq!(speed(WeatherKind::Fine, 0.7), FINE);
        assert_eq!(speed(WeatherKind::Rain, 0.0), 3.0);
        assert_eq!(speed(WeatherKind::Rain, 1.0), 9.0);
        assert_eq!(speed(WeatherKind::Sand, 2.0), 12.0, "clamped");
        assert!((speed(WeatherKind::Snow, 0.5) - 4.5).abs() < 1e-6);
    }

    #[test]
    fn the_direction_is_level_and_swings_slowly_about_the_maps_heading() {
        for map in [0, 1, 530] {
            let d = direction(map, 0.0);
            assert!((d.length() - 1.0).abs() < 1e-5 && d.y == 0.0);
            // A quarter period later it has swung the full amount.
            let later = direction(map, SWING_PERIOD / 4.0);
            let turned = d.angle_between(later).to_degrees();
            assert!((turned - SWING_DEG).abs() < 0.01, "{turned}");
        }
        assert!(
            direction(0, 0.0).distance(direction(1, 0.0)) > 0.5,
            "maps differ"
        );
    }
}
