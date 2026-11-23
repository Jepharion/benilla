//! A crate built on top holding the body (`benilla-libfpv`'s drone): it detaches the body as the
//! dev chord's free-fly does (`Player::detached`), so the avatar and its server position stay put
//! while the camera goes elsewhere and the world streams around the camera
//! ([`super::world_focus`]), and attaches it again. It also reads where the body stands.

use bevy::prelude::*;

use super::state::Player;

/// The body, for a crate on top: `request` is the crate's, the rest the client's, rewritten
/// every frame in the world.
#[derive(Resource, Default, Debug, Clone, PartialEq)]
pub struct HostBody {
    /// `Some(true)` detaches the body, `Some(false)` attaches it; taken when applied, or dropped
    /// while there is no body to hold (not in the world).
    pub request: Option<bool>,
    /// In the world, driving our own body.
    pub active: bool,
    /// The body is detached: free-fly, or the crate's request.
    pub detached: bool,
    /// The body's feet, Bevy coords.
    pub feet: Vec3,
}

/// Applies the crate's request and publishes the body's state. In `PreUpdate`, so a request takes
/// effect before the frame's player systems run.
pub(super) fn sync_host_body(mut host: ResMut<HostBody>, player: Option<ResMut<Player>>) {
    // No player (a harness without one): no body.
    let Some(mut player) = player else {
        host.request = None;
        host.set_if_neq(HostBody::default());
        return;
    };
    if let Some(detach) = host.request.take() {
        if player.active && player.detached != detach {
            player.detached = detach;
            info!(
                "host body: {}",
                if detach { "detached" } else { "attached" }
            );
        }
    }
    let state = HostBody {
        request: None,
        active: player.active,
        detached: player.detached,
        feet: player.pos,
    };
    // Only on a change: a crate on top may watch `HostBody` with change detection.
    host.set_if_neq(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<HostBody>()
            .init_resource::<Player>()
            .add_systems(Update, sync_host_body);
        app
    }

    #[test]
    fn a_request_detaches_and_attaches_an_active_body_and_is_taken() {
        let mut app = app();
        {
            let mut p = app.world_mut().resource_mut::<Player>();
            p.active = true;
            p.pos = Vec3::new(1.0, 2.0, 3.0);
        }
        app.world_mut().resource_mut::<HostBody>().request = Some(true);
        app.update();
        assert!(app.world().resource::<Player>().detached);
        let host = app.world().resource::<HostBody>().clone();
        assert_eq!(host.request, None);
        assert!(host.active && host.detached);
        assert_eq!(host.feet, Vec3::new(1.0, 2.0, 3.0));

        app.world_mut().resource_mut::<HostBody>().request = Some(false);
        app.update();
        assert!(!app.world().resource::<Player>().detached);
        assert!(!app.world().resource::<HostBody>().detached);
    }

    #[test]
    fn a_request_outside_the_world_is_dropped() {
        let mut app = app();
        app.world_mut().resource_mut::<HostBody>().request = Some(true);
        app.update();
        assert!(!app.world().resource::<Player>().detached);
        assert_eq!(app.world().resource::<HostBody>().request, None);
    }
}
