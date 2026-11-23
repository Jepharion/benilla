//! A crate built on top's own chat line (`benilla-libfpv`'s GM commands for its stand-in): a
//! [`HostChat`] message goes out as a /say, as the dev "land" sends its `.go xyz`
//! ([`super::land`]). A dot command is the server's; anything else is said aloud.
//!
//! A server command such as `.modify morph` acts on the sender's selection when there is one
//! (vmangos), so a dot command goes out between two `CMSG_SET_SELECTION`s: our own guid, then the
//! selection the player had. The server takes one session's packets in order, so the command can
//! only reach our own character, never the player we have targeted (the user's report: a friend's
//! launch disguised the player he had selected).

use bevy::prelude::*;

use crate::net::{ChatKind, ClientCommand, Guid, NetCommands, SelfPlayer};
use crate::target::Selection;

/// A line to say on the wire, in the character's default language; a dot command acts on our
/// own character.
#[derive(Message, Clone, Debug, PartialEq, Eq)]
pub struct HostChat(pub String);

/// Sends the frame's [`HostChat`] lines, in order. In `PostUpdate`, so a line written anywhere in
/// `Update` goes out the same frame.
pub(super) fn send_host_chat(
    mut lines: MessageReader<HostChat>,
    net: Option<Res<NetCommands>>,
    me: Query<&Guid, With<SelfPlayer>>,
    selection: Option<Res<Selection>>,
) {
    for HostChat(text) in lines.read() {
        let Some(net) = net.as_ref() else {
            warn!("host chat: no connection — dropped {text:?}");
            continue;
        };
        for command in commands(text, me.single().ok().map(|g| g.0), selection.as_deref()) {
            let _ = net.0.send(command);
        }
        info!("host chat: {text}");
    }
}

/// The packets for one line: a dot command bracketed by selecting ourselves and restoring the
/// player's selection; a dot command with no self guid yet is dropped, as it could reach someone
/// else.
fn commands(text: &str, me: Option<u64>, selection: Option<&Selection>) -> Vec<ClientCommand> {
    let chat = ClientCommand::Chat {
        kind: ChatKind::Say,
        target: None,
        text: text.to_owned(),
        language: None,
    };
    if !text.starts_with('.') {
        return vec![chat];
    }
    let Some(me) = me else {
        warn!("host chat: no character yet — dropped {text:?}");
        return Vec::new();
    };
    let restore = selection.and_then(|s| s.guid).unwrap_or(0);
    vec![
        ClientCommand::SetSelection { guid: me },
        chat,
        ClientCommand::SetSelection { guid: restore },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selections(cmds: &[ClientCommand]) -> Vec<u64> {
        cmds.iter()
            .filter_map(|c| match c {
                ClientCommand::SetSelection { guid } => Some(*guid),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_dot_command_is_aimed_at_us_and_the_target_comes_back() {
        let sel = Selection {
            guid: Some(42),
            ..Default::default()
        };
        let cmds = commands(".modify scale 0.1", Some(7), Some(&sel));
        assert_eq!(cmds.len(), 3);
        assert_eq!(
            selections(&cmds),
            [7, 42],
            "us, then the friend we had targeted"
        );
        assert!(
            matches!(&cmds[1], ClientCommand::Chat { text, .. } if text == ".modify scale 0.1")
        );
        // No target: cleared again afterwards.
        assert_eq!(selections(&commands(".demorph", Some(7), None)), [7, 0]);
    }

    #[test]
    fn plain_chat_is_just_said_and_a_command_without_us_is_dropped() {
        assert_eq!(commands("hello", None, None).len(), 1);
        assert!(commands(".modify morph 8011", None, None).is_empty());
    }
}
