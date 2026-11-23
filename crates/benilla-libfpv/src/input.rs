//! Keys and the mouse for libfpv: the overlay (F8 and everything in it) and the keyboard flying
//! fallback, per INTEGRATION.md "Input".
//!
//! - Keys go by physical position: Bevy's [`KeyCode`] is the key's place on a US QWERTY board,
//!   whatever the layout labels it, which is what `fpv_input_key` wants (`'W'` is the key right
//!   of Tab).
//! - On edges only, and only while the window has focus. A key's first reading is its baseline:
//!   a key already down when forwarding starts sends nothing until it is released and pressed
//!   again, so a key read as held at startup cannot open the overlay by itself.
//! - Losing focus releases every key sent down, so none sticks.

use std::collections::HashSet;

use bevy::input::keyboard::KeyCode;

use crate::ffi;

/// libfpv's code for a key, or `None` for a key libfpv has no use for.
pub fn fpv_key(key: KeyCode) -> Option<i32> {
    use KeyCode::*;
    let letter = |c: u8| Some(i32::from(c));
    Some(match key {
        KeyA => return letter(b'A'),
        KeyB => return letter(b'B'),
        KeyC => return letter(b'C'),
        KeyD => return letter(b'D'),
        KeyE => return letter(b'E'),
        KeyF => return letter(b'F'),
        KeyG => return letter(b'G'),
        KeyH => return letter(b'H'),
        KeyI => return letter(b'I'),
        KeyJ => return letter(b'J'),
        KeyK => return letter(b'K'),
        KeyL => return letter(b'L'),
        KeyM => return letter(b'M'),
        KeyN => return letter(b'N'),
        KeyO => return letter(b'O'),
        KeyP => return letter(b'P'),
        KeyQ => return letter(b'Q'),
        KeyR => return letter(b'R'),
        KeyS => return letter(b'S'),
        KeyT => return letter(b'T'),
        KeyU => return letter(b'U'),
        KeyV => return letter(b'V'),
        KeyW => return letter(b'W'),
        KeyX => return letter(b'X'),
        KeyY => return letter(b'Y'),
        KeyZ => return letter(b'Z'),
        Digit0 => return letter(b'0'),
        Digit1 => return letter(b'1'),
        Digit2 => return letter(b'2'),
        Digit3 => return letter(b'3'),
        Digit4 => return letter(b'4'),
        Digit5 => return letter(b'5'),
        Digit6 => return letter(b'6'),
        Digit7 => return letter(b'7'),
        Digit8 => return letter(b'8'),
        Digit9 => return letter(b'9'),
        Tab => ffi::FPV_KEY_TAB,
        Enter | NumpadEnter => ffi::FPV_KEY_ENTER,
        Escape => ffi::FPV_KEY_ESCAPE,
        Space => ffi::FPV_KEY_SPACE,
        ArrowLeft => ffi::FPV_KEY_LEFT,
        ArrowRight => ffi::FPV_KEY_RIGHT,
        ArrowUp => ffi::FPV_KEY_UP,
        ArrowDown => ffi::FPV_KEY_DOWN,
        PageUp => ffi::FPV_KEY_PAGE_UP,
        PageDown => ffi::FPV_KEY_PAGE_DOWN,
        Home => ffi::FPV_KEY_HOME,
        End => ffi::FPV_KEY_END,
        Insert => ffi::FPV_KEY_INSERT,
        Delete => ffi::FPV_KEY_DELETE,
        Backspace => ffi::FPV_KEY_BACKSPACE,
        ShiftLeft => ffi::FPV_KEY_LSHIFT,
        ShiftRight => ffi::FPV_KEY_RSHIFT,
        ControlLeft => ffi::FPV_KEY_LCTRL,
        ControlRight => ffi::FPV_KEY_RCTRL,
        AltLeft => ffi::FPV_KEY_LALT,
        AltRight => ffi::FPV_KEY_RALT,
        F1 => ffi::FPV_KEY_F1,
        F2 => ffi::FPV_KEY_F1 + 1,
        F3 => ffi::FPV_KEY_F1 + 2,
        F4 => ffi::FPV_KEY_F1 + 3,
        F5 => ffi::FPV_KEY_F1 + 4,
        F6 => ffi::FPV_KEY_F1 + 5,
        F7 => ffi::FPV_KEY_F1 + 6,
        F8 => ffi::FPV_KEY_F1 + 7,
        F9 => ffi::FPV_KEY_F1 + 8,
        F10 => ffi::FPV_KEY_F1 + 9,
        F11 => ffi::FPV_KEY_F1 + 10,
        F12 => ffi::FPV_KEY_F1 + 11,
        _ => return None,
    })
}

/// Which keys libfpv has been told are down.
#[derive(Default)]
pub struct KeyEdges {
    down: HashSet<i32>,
}

impl KeyEdges {
    /// A press seen as an edge this frame: forward it, unless it is a repeat or already down.
    /// Returns whether it was sent.
    pub fn press(&mut self, key: i32, send: impl FnOnce(i32, bool)) -> bool {
        if self.down.insert(key) {
            send(key, true);
            true
        } else {
            false
        }
    }

    /// A release: forwarded only for a key libfpv saw go down, so a key held from before
    /// forwarding began (its baseline) never sends a stray edge.
    pub fn release(&mut self, key: i32, send: impl FnOnce(i32, bool)) {
        if self.down.remove(&key) {
            send(key, false);
        }
    }

    /// Focus lost: every key libfpv saw go down goes up.
    pub fn release_all(&mut self, mut send: impl FnMut(i32, bool)) {
        for key in self.down.drain() {
            send(key, false);
        }
    }
}

/// Typed text worth handing the overlay: printable characters only (no control characters, which
/// arrive as keys).
pub fn printable(text: &str) -> Option<&str> {
    (!text.is_empty() && text.chars().all(|c| !c.is_control())).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_digits_and_function_keys_map_by_position() {
        assert_eq!(fpv_key(KeyCode::KeyW), Some(i32::from(b'W')));
        assert_eq!(fpv_key(KeyCode::Digit0), Some(i32::from(b'0')));
        assert_eq!(fpv_key(KeyCode::F8), Some(ffi::FPV_KEY_F1 + 7));
        assert_eq!(fpv_key(KeyCode::F12), Some(ffi::FPV_KEY_F1 + 11));
        assert_eq!(fpv_key(KeyCode::Escape), Some(ffi::FPV_KEY_ESCAPE));
        assert_eq!(fpv_key(KeyCode::NumpadEnter), Some(ffi::FPV_KEY_ENTER));
        assert_eq!(fpv_key(KeyCode::CapsLock), None);
    }

    #[test]
    fn the_header_numbers_f_keys_after_ralt() {
        // libfpv.h:1288-1290: F1 follows RALT (272) in the enum.
        assert_eq!(ffi::FPV_KEY_RALT + 1, ffi::FPV_KEY_F1);
        assert_eq!(ffi::FPV_KEY_F1 + 11, 284);
    }

    #[test]
    fn a_key_held_before_forwarding_sends_nothing_on_release() {
        let mut edges = KeyEdges::default();
        let mut sent = vec![];
        // Released without a press seen: the baseline.
        edges.release(i32::from(b'F'), |k, d| sent.push((k, d)));
        assert!(sent.is_empty());
        // A real press and release go through, once each.
        edges.press(i32::from(b'F'), |k, d| sent.push((k, d)));
        edges.press(i32::from(b'F'), |k, d| sent.push((k, d)));
        edges.release(i32::from(b'F'), |k, d| sent.push((k, d)));
        assert_eq!(sent, vec![(70, true), (70, false)]);
    }

    #[test]
    fn losing_focus_releases_what_was_down() {
        let mut edges = KeyEdges::default();
        let mut sent = vec![];
        edges.press(ffi::FPV_KEY_F1 + 7, |_, _| {});
        edges.release_all(|k, d| sent.push((k, d)));
        assert_eq!(sent, vec![(ffi::FPV_KEY_F1 + 7, false)]);
        // Nothing left to release.
        edges.release_all(|k, d| sent.push((k, d)));
        assert_eq!(sent.len(), 1);
    }

    #[test]
    fn control_characters_are_not_text() {
        assert_eq!(printable("a"), Some("a"));
        assert_eq!(printable("é"), Some("é"));
        assert_eq!(printable("\u{8}"), None);
        assert_eq!(printable("\r"), None);
        assert_eq!(printable(""), None);
    }
}
