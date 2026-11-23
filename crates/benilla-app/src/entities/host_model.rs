//! A crate built on top's own creature visual (`benilla-libfpv`'s drone, drawn as a creature
//! display): an entity carrying [`HostModel`] is built and animated as the ordinary creature path
//! builds a mount child ([`super::mount::spawn_mount_child`]): a guid-less [`NetEntity`] of the
//! display, at the entity's own `Transform`. It is no server object: no guid, so no nameplate,
//! target, tooltip or object list reaches it.

use benilla_protocol::EntityKind;
use bevy::prelude::*;

use crate::net::NetEntity;

/// Draw this entity as `CreatureDisplayInfo` row `display_id`, built at `scale` (the whole render
/// scale, as `OBJECT_FIELD_SCALE_X`). The crate owns the entity, its `Transform` and its
/// `Visibility`; after the build the render scale is the `Transform`'s, as no server field eases
/// it (`entities::live_display` eases a guid's `SCALE_X` only).
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct HostModel {
    pub display_id: u32,
    pub scale: f32,
}

/// Gives a new or changed [`HostModel`] its [`NetEntity`]; a scale change eases as a live one
/// does (`entities::live_display`).
pub(super) fn attach_host_models(
    mut commands: Commands,
    models: Query<(Entity, &HostModel, Option<&NetEntity>), Changed<HostModel>>,
) {
    for (entity, model, net) in &models {
        let want = NetEntity {
            kind: EntityKind::Unit,
            display_id: Some(model.display_id),
            scale: model.scale,
        };
        match net {
            Some(n) if n.display_id == want.display_id && n.scale == want.scale => {}
            Some(_) => {
                commands.entity(entity).insert(want);
            }
            None => {
                commands.entity(entity).insert((
                    want,
                    // Its own instance in the transparent order, as a mount child's.
                    benilla_world::model_render::order::SortInstance::of(entity),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_model_becomes_a_guidless_unit_and_follows_its_scale() {
        let mut app = App::new();
        app.add_systems(Update, attach_host_models);
        let e = app
            .world_mut()
            .spawn((
                HostModel {
                    display_id: 8011,
                    scale: 0.1,
                },
                Transform::default(),
            ))
            .id();
        app.update();
        let net = app.world().get::<NetEntity>(e).expect("a NetEntity");
        assert_eq!(net.display_id, Some(8011));
        assert_eq!(net.scale, 0.1);
        assert!(app.world().get::<crate::net::Guid>(e).is_none());
        app.world_mut().get_mut::<HostModel>(e).unwrap().scale = 0.2;
        app.update();
        assert_eq!(app.world().get::<NetEntity>(e).unwrap().scale, 0.2);
    }
}
