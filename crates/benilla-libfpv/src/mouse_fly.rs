//! Mouse & keyboard flying, for a guest with no radio: F8 > Fly > "Mouse & keyboard flying". The
//! sticks are made here and handed to libfpv as host channels (`fpv_input_set_channels`), which
//! take over from any device while they are fresh (`libfpv.h`, "Host-injected channels"). Always
//! in angle mode, which levels the drone by itself.
//!
//! - **Mouse:** the right stick. Moving it deflects pitch (forward, away from you, is nose down)
//!   and roll, and the deflection stays where it is left, so steady forward flight is a mouse
//!   held forward; **C** recentres it.
//! - **W / S:** throttle up and down; it stays where it is left, as a real throttle stick does.
//! - **A / D:** yaw left and right, while held.
//! - **Space:** arm and disarm.
//!
//! Out of the drone's view, or while the overlay has the input, the sticks centre and the
//! throttle holds: the drone levels and hangs where it is. The choice is kept in benilla's local
//! state folder (`libfpv-client.txt`).

use std::path::PathBuf;

use bevy::prelude::*;

use crate::ffi;

/// Stick travel per pixel of mouse movement: full deflection at 300 px.
const STICK_PER_PIXEL: f32 = 1.0 / 300.0;
/// Throttle travel per second while W or S is held (-1..1 is 2.0).
const THROTTLE_PER_SECOND: f32 = 0.6;
/// Yaw while A or D is held.
const YAW: f32 = 0.6;

/// What the player does this frame, read by the caller.
#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// The mouse's movement, pixels (right, down).
    pub mouse: Vec2,
    pub throttle_up: bool,
    pub throttle_down: bool,
    pub yaw_left: bool,
    pub yaw_right: bool,
    /// Space went down this frame.
    pub arm_toggle: bool,
    /// C went down this frame.
    pub recentre: bool,
}

/// The mode's state between frames.
#[derive(Debug, Clone)]
pub struct MouseFly {
    pub on: bool,
    /// Roll and pitch, -1..1.
    stick: Vec2,
    throttle: f32,
    armed: bool,
}

impl Default for MouseFly {
    fn default() -> Self {
        Self {
            on: false,
            stick: Vec2::ZERO,
            throttle: -1.0,
            armed: false,
        }
    }
}

impl MouseFly {
    /// The saved choice, off when there is none.
    pub fn load() -> Self {
        let on = settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .is_some_and(|t| t.lines().any(|l| l.trim() == "mouse_flying=1"));
        Self { on, ..default() }
    }

    /// Turns the mode on or off and keeps the choice. Off disarms, so turning it back on never
    /// starts with the motors spinning.
    pub fn set(&mut self, on: bool) {
        *self = Self { on, ..default() };
        if let Some(path) = settings_path() {
            let text = format!("mouse_flying={}\n", u8::from(on));
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, text));
            if let Err(e) = written {
                warn!("libfpv: cannot save {}: {e}", path.display());
            }
        }
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn throttle(&self) -> f32 {
        self.throttle
    }

    /// This frame's channels (AETR, AUX1 arm, AUX2 angle). `live`: the player is flying (the
    /// drone's view, the overlay closed); otherwise the sticks centre and the throttle holds.
    pub fn step(&mut self, c: Controls, dt: f32, live: bool) -> [f32; ffi::FPV_CHANNEL_COUNT] {
        let mut yaw = 0.0;
        if live {
            // Away from you (the mouse up, -y) pushes the stick forward: nose down.
            self.stick += Vec2::new(c.mouse.x, -c.mouse.y) * STICK_PER_PIXEL;
            self.stick = self.stick.clamp(Vec2::splat(-1.0), Vec2::ONE);
            if c.recentre {
                self.stick = Vec2::ZERO;
            }
            let t = f32::from(u8::from(c.throttle_up)) - f32::from(u8::from(c.throttle_down));
            self.throttle = (self.throttle + t * THROTTLE_PER_SECOND * dt).clamp(-1.0, 1.0);
            yaw = (f32::from(u8::from(c.yaw_right)) - f32::from(u8::from(c.yaw_left))) * YAW;
            if c.arm_toggle {
                self.armed = !self.armed;
                if self.armed {
                    // Betaflight arms only at idle throttle.
                    self.throttle = -1.0;
                }
                info!(
                    "libfpv: mouse flying — {}",
                    if self.armed { "armed" } else { "disarmed" }
                );
            }
        } else {
            self.stick = Vec2::ZERO;
        }
        let mut ch = [-1.0f32; ffi::FPV_CHANNEL_COUNT];
        ch[0] = self.stick.x;
        ch[1] = self.stick.y;
        ch[2] = self.throttle;
        ch[3] = yaw;
        ch[4] = if self.armed { 1.0 } else { -1.0 };
        ch[5] = 1.0; // angle
        ch
    }
}

/// `libfpv-client.txt` in benilla's local state folder.
fn settings_path() -> Option<PathBuf> {
    benilla_app::config_dir().map(|d| d.join("libfpv-client.txt"))
}

/// The overlay button's label for the mode.
pub fn label(on: bool) -> &'static str {
    if on {
        "Mouse & keyboard flying: ON (mouse = pitch/roll, W/S throttle, A/D yaw, Space arm, C centre)"
    } else {
        "Mouse & keyboard flying: off (click to fly without a radio)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fly() -> MouseFly {
        MouseFly {
            on: true,
            ..default()
        }
    }

    #[test]
    fn the_mouse_holds_its_deflection_and_c_recentres_it() {
        let mut m = fly();
        let ch = m.step(
            Controls {
                mouse: Vec2::new(150.0, -150.0),
                ..default()
            },
            0.016,
            true,
        );
        assert!((ch[0] - 0.5).abs() < 1e-5, "roll right half");
        assert!((ch[1] - 0.5).abs() < 1e-5, "mouse away: nose down");
        assert_eq!(ch[5], 1.0, "angle mode");
        // No movement: it stays.
        let ch = m.step(Controls::default(), 0.016, true);
        assert!((ch[0] - 0.5).abs() < 1e-5);
        // Far past the end: clamped.
        let ch = m.step(
            Controls {
                mouse: Vec2::new(5000.0, 0.0),
                ..default()
            },
            0.016,
            true,
        );
        assert_eq!(ch[0], 1.0);
        let ch = m.step(
            Controls {
                recentre: true,
                ..default()
            },
            0.016,
            true,
        );
        assert_eq!((ch[0], ch[1]), (0.0, 0.0));
    }

    #[test]
    fn the_throttle_ramps_and_holds_and_arming_needs_idle() {
        let mut m = fly();
        let up = Controls {
            throttle_up: true,
            ..default()
        };
        let ch = m.step(up, 1.0, true);
        assert!((ch[2] - (-1.0 + THROTTLE_PER_SECOND)).abs() < 1e-5);
        let held = m.step(Controls::default(), 1.0, true)[2];
        assert!((held - ch[2]).abs() < 1e-6, "it holds");
        // Arming puts the throttle at idle.
        let ch = m.step(
            Controls {
                arm_toggle: true,
                ..default()
            },
            0.016,
            true,
        );
        assert_eq!((ch[4], ch[2]), (1.0, -1.0));
        assert!(m.armed());
    }

    #[test]
    fn out_of_the_drones_view_the_sticks_centre_and_the_throttle_holds() {
        let mut m = fly();
        m.step(
            Controls {
                mouse: Vec2::new(300.0, 0.0),
                throttle_up: true,
                ..default()
            },
            0.5,
            true,
        );
        let t = m.throttle();
        let ch = m.step(
            Controls {
                mouse: Vec2::new(300.0, 0.0),
                yaw_left: true,
                arm_toggle: true,
                ..default()
            },
            0.5,
            false,
        );
        assert_eq!((ch[0], ch[1], ch[3]), (0.0, 0.0, 0.0));
        assert_eq!(ch[2], t);
        assert!(!m.armed(), "no keys while not live");
    }
}
