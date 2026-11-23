//! A host's own sound on the mixer: a crate built on top (`benilla-libfpv`'s drone motors) fills
//! stereo frames on the audio thread, mixed on the main track with the game's sounds, under the
//! master volume and the limiter.
//!
//! In the client it plays on the client's own mixer ([`SoundOutput`]). An app without one (the
//! world viewer) gets a mixer of its own, opened on first use, with the same silence rules:
//! `$WOW_NOSOUND`, `$WOW_CAPTURE` and a missing device play nothing.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use kira::info::Info;
use kira::sound::{Sound, SoundData};
use kira::Frame;

use super::mixer::Mixer;
use super::SoundOutput;

/// Fills `out` with the next stereo frames, `[left, right]`, on the audio thread. It must not
/// block, and should not allocate.
pub type HostFill = Box<dyn FnMut(&mut [[f32; 2]]) + Send>;

/// A playing host stream. Dropping it stops the stream.
pub struct HostStream {
    stop: Arc<AtomicBool>,
}

impl Drop for HostStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Plays a host stream: `make` gets the output's sample rate and returns the fill. `None`, and
/// `make` is not called, when the app runs silent.
pub fn play_host_stream(
    world: &mut World,
    make: impl FnOnce(u32) -> HostFill,
) -> Option<HostStream> {
    if world.get_non_send_resource::<SoundOutput>().is_none()
        && world.get_non_send_resource::<OwnMixer>().is_none()
    {
        let mixer = open_own();
        world.insert_non_send_resource(OwnMixer(mixer));
    }
    let mixer = match world.get_non_send_resource_mut::<SoundOutput>() {
        Some(out) => out.into_inner().mixer.as_mut(),
        None => world
            .get_non_send_resource_mut::<OwnMixer>()
            .and_then(|m| m.into_inner().0.as_mut()),
    }?;
    let rate = mixer.sample_rate()?;
    let stop = Arc::new(AtomicBool::new(false));
    let data = HostSoundData {
        fill: make(rate),
        stop: stop.clone(),
    };
    match mixer.play_host(data) {
        Ok(()) => Some(HostStream { stop }),
        Err(e) => {
            warn!("host stream: {e:#}");
            None
        }
    }
}

/// The mixer of an app without the client's sound ([`play_host_stream`]).
struct OwnMixer(Option<Mixer>);

fn open_own() -> Option<Mixer> {
    if let Some(v) = ["WOW_NOSOUND", "WOW_CAPTURE"]
        .into_iter()
        .find(|v| std::env::var_os(v).is_some())
    {
        info!("host stream: ${v} set — silent");
        return None;
    }
    match Mixer::new(None) {
        Ok(m) => Some(m),
        Err(e) => {
            warn!("host stream: no audio device — silent: {e:#}");
            None
        }
    }
}

pub(super) struct HostSoundData {
    fill: HostFill,
    stop: Arc<AtomicBool>,
}

impl SoundData for HostSoundData {
    type Error = std::convert::Infallible;
    type Handle = ();

    fn into_sound(self) -> Result<(Box<dyn Sound>, Self::Handle), Self::Error> {
        Ok((
            Box::new(HostSound {
                fill: self.fill,
                stop: self.stop,
                buf: vec![[0.0; 2]; BLOCK],
            }),
            (),
        ))
    }
}

/// The frames asked of the fill per call; kira's batches are split to fit, so the audio thread
/// never allocates.
const BLOCK: usize = 1024;

struct HostSound {
    fill: HostFill,
    stop: Arc<AtomicBool>,
    buf: Vec<[f32; 2]>,
}

impl Sound for HostSound {
    fn process(&mut self, out: &mut [Frame], _dt: f64, _info: &Info) {
        for chunk in out.chunks_mut(BLOCK) {
            let buf = &mut self.buf[..chunk.len()];
            (self.fill)(buf);
            for (o, [l, r]) in chunk.iter_mut().zip(buf.iter()) {
                *o = Frame {
                    left: *l,
                    right: *r,
                };
            }
        }
    }

    fn finished(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_sound_passes_its_frames_through_in_blocks() {
        let mut n = 0u32;
        let data = HostSoundData {
            fill: Box::new(move |buf| {
                for f in buf.iter_mut() {
                    *f = [n as f32, -(n as f32)];
                    n += 1;
                }
            }),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let (mut sound, ()) = data.into_sound().unwrap();
        let mut out = vec![Frame::ZERO; BLOCK * 2 + 7];
        let info = kira::info::MockInfoBuilder::new().build();
        sound.process(&mut out, 1.0 / 48_000.0, &info);
        assert_eq!(
            out[0],
            Frame {
                left: 0.0,
                right: 0.0
            }
        );
        assert_eq!(
            out[BLOCK + 3],
            Frame {
                left: (BLOCK + 3) as f32,
                right: -((BLOCK + 3) as f32)
            }
        );
        assert_eq!(out.last().unwrap().left, (BLOCK * 2 + 6) as f32);
        assert!(!sound.finished());
    }

    #[test]
    fn dropping_the_handle_finishes_the_sound() {
        let stop = Arc::new(AtomicBool::new(false));
        let data = HostSoundData {
            fill: Box::new(|_| {}),
            stop: stop.clone(),
        };
        let (sound, ()) = data.into_sound().unwrap();
        drop(HostStream { stop });
        assert!(sound.finished());
    }
}
