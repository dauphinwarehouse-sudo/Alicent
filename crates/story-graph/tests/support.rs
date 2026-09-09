#![allow(dead_code)]

use alicent_story_graph::*;
use uuid::Uuid;

pub fn entity_id(value: u128) -> EntityId {
    EntityId::from_uuid(Uuid::from_u128(value))
}

pub fn relation_id(value: u128) -> RelationId {
    RelationId::from_uuid(Uuid::from_u128(value))
}

pub fn event_id(value: u128) -> EventId {
    EventId::from_uuid(Uuid::from_u128(value))
}

pub fn context(value: &str) -> ContextId {
    ContextId::new(value).unwrap()
}

pub fn entity(id: u128, kind: EntityKind, name: &str, parent: Option<EntityId>) -> Entity {
    Entity::new(entity_id(id), kind, name, parent, ContextRule::global()).unwrap()
}
