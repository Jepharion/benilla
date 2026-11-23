//! A crate built on top's own chat line (`benilla-libfpv`'s GM commands for its stand-in): a
//! [`HostChat`] message goes out as a /say, as the dev "land" sends its `.go xyz`
//! ([`super::land`]). A dot command is the server's; anything else is said aloud.

use bevy::prelude::*;

use crate::net::{ChatKind, ClientCommand, NetCommands};

/// A line to say on the wire, in the character's default language.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct HostChat(pub String);

/// Sends the frame's [`HostChat`] lines, in order. In `PostUpdate`, so a line written anywhere in
/// `Update` goes out the same frame.
pub(super) fn send_host_chat(mut lines: MessageReader<HostChat>, net: Option<Res<NetCommands>>) {
    for HostChat(text) in lines.read() {
        let Some(net) = net.as_ref() else {
            warn!("host chat: no connection — dropped {text:?}");
            continue;
        };
        info!("host chat: {text}");
        let _ = net.0.send(ClientCommand::Chat {
            kind: ChatKind::Say,
            target: None,
            text: text.clone(),
            language: None,
        });
    }
}
