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
    /// The crate's: puts the body's feet here (Bevy coords), as "move here"; taken when applied,
    /// dropped out of the world. Attached at rest, the new position goes out as a heartbeat.
    pub place: Option<Vec3>,
    /// In the world, driving our own body.
    pub active: bool,
    /// The body is detached: free-fly, or the crate's request.
    pub detached: bool,
    /// The body's feet, Bevy coords.
    pub feet: Vec3,
    /// The body's facing, radians, the wire's.
    pub facing: f32,
    /// The crate's: while the body is detached, where the server is told it stands (feet, Bevy
    /// coords) and its facing (radians, the wire's), streamed as heartbeats with no movement
    /// flags ([`stream_stand_in`]). The body here stays put, so the server's character can be
    /// elsewhere (the drone) while this client draws it at home. Re-attached at rest, the body's
    /// own position goes out as soon as it differs from the last one sent (`movement_net`).
    pub stand_in: Option<(Vec3, f32)>,
    /// The crate's: this client draws our own body with its own display and scale, whatever the
    /// server changes them to (a GM morph that disguises it for other players).
    pub keep_look: bool,
}

/// The least gap between stand-in heartbeats, seconds: 20 a second while it moves.
const STAND_IN_EVERY: f32 = 0.05;
/// A still stand-in is restated this often, seconds.
const STAND_IN_IDLE: f32 = 1.0;
/// A move this small (yards) or turn (radians) is not worth a packet before the idle restatement.
const STAND_IN_STEP: f32 = 0.02;

/// Streams [`HostBody::stand_in`] while the body is detached: a `MSG_MOVE_HEARTBEAT` with no
/// movement flags (no fall, no swim) at the stand-in's position and facing, at most every
/// [`STAND_IN_EVERY`], when it moved or turned or [`STAND_IN_IDLE`] passed. vmangos relocates the
/// mover to each one (`HandleMoverRelocation`); `last_pos` keeps what was sent, so the body's own
/// position goes out once it is attached again.
pub(super) fn stream_stand_in(
    host: Res<HostBody>,
    player: Option<ResMut<Player>>,
    net: Option<Res<crate::net::NetCommands>>,
    time: Res<Time<bevy::time::Real>>,
    mut last: Local<Option<(f32, [f32; 3], f32)>>,
) {
    let (Some(mut player), Some(net)) = (player, net) else {
        return;
    };
    let Some((pos, facing)) = host.stand_in.filter(|_| player.active && player.detached) else {
        *last = None;
        return;
    };
    let now = time.elapsed_secs();
    let wow = benilla_assets::coords::bevy_to_wow(pos);
    let facing = facing.rem_euclid(std::f32::consts::TAU);
    if let Some((at, sent, sent_facing)) = *last {
        let moved = Vec3::from(wow).distance(Vec3::from(sent)) > STAND_IN_STEP
            || (facing - sent_facing).abs() > STAND_IN_STEP;
        let due = now - at >= STAND_IN_IDLE || (moved && now - at >= STAND_IN_EVERY);
        if !due {
            return;
        }
    }
    let _ = net.0.send(crate::net::ClientCommand::Move {
        kind: crate::net::MoveKind::Heartbeat,
        flags: 0,
        pos: wow,
        orientation: facing,
        pitch: 0.0,
        fall_time: 0,
        jump: None,
        transport: None,
    });
    player.last_pos = wow;
    player.sent_facing = facing;
    *last = Some((now, wow, facing));
}

/// Applies the crate's request and publishes the body's state. In `PreUpdate`, so a request takes
/// effect before the frame's player systems run.
pub(super) fn sync_host_body(mut host: ResMut<HostBody>, player: Option<ResMut<Player>>) {
    // No player (a harness without one): no body.
    let Some(mut player) = player else {
        host.request = None;
        host.place = None;
        host.set_if_neq(HostBody::default());
        return;
    };
    if let Some(at) = host.place.take() {
        if player.active {
            player.pos = at;
            info!(
                "host body: placed at WoW {:.2?}",
                benilla_assets::coords::bevy_to_wow(at)
            );
        }
    }
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
        place: None,
        active: player.active,
        detached: player.detached,
        feet: player.pos,
        facing: player.face_yaw,
        ..host.clone()
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
    fn the_crates_fields_survive_the_sync() {
        let mut app = app();
        app.world_mut().resource_mut::<Player>().active = true;
        {
            let mut host = app.world_mut().resource_mut::<HostBody>();
            host.stand_in = Some((Vec3::ONE, 1.0));
            host.keep_look = true;
        }
        app.update();
        let host = app.world().resource::<HostBody>();
        assert_eq!(host.stand_in, Some((Vec3::ONE, 1.0)));
        assert!(host.keep_look && host.active);
    }

    #[test]
    fn a_detached_body_streams_its_stand_in_as_a_flagless_heartbeat() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut app = App::new();
        app.init_resource::<HostBody>()
            .init_resource::<Player>()
            .init_resource::<Time<bevy::time::Real>>()
            .insert_resource(crate::net::NetCommands(tx))
            .add_systems(Update, stream_stand_in);
        {
            let mut p = app.world_mut().resource_mut::<Player>();
            p.active = true;
            p.detached = true;
        }
        let at = Vec3::new(10.0, 20.0, 30.0);
        app.world_mut().resource_mut::<HostBody>().stand_in = Some((at, 0.5));
        app.update();
        let Ok(crate::net::ClientCommand::Move {
            kind,
            flags,
            pos,
            orientation,
            ..
        }) = rx.try_recv()
        else {
            panic!("a heartbeat");
        };
        assert!(matches!(kind, crate::net::MoveKind::Heartbeat));
        assert_eq!(flags, 0);
        assert_eq!(pos, benilla_assets::coords::bevy_to_wow(at));
        assert_eq!(orientation, 0.5);
        assert_eq!(app.world().resource::<Player>().last_pos, pos);
        // Unmoved and not yet idle: nothing more.
        app.update();
        assert!(rx.try_recv().is_err());
        // Attached: the stand-in stops.
        app.world_mut().resource_mut::<Player>().detached = false;
        app.world_mut().resource_mut::<HostBody>().stand_in = Some((Vec3::ZERO, 0.0));
        app.update();
        assert!(rx.try_recv().is_err());
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
