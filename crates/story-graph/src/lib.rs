//! In-memory story graph prototype for characters, places, relations and events.
//!
//! The crate is deliberately storage- and UI-agnostic. Its constructors are the
//! only way to build graph values, so invalid persisted DTOs must be rejected by
//! a future adapter before they enter the domain model.
#![forbid(unsafe_code)]

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            pub fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

id_type!(EntityId);
id_type!(RelationId);
id_type!(EventId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Character,
    Place,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ContextId(String);

impl ContextId {
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        validate_text("context_id", &value, 64)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ContextRule {
    include: BTreeSet<ContextId>,
    exclude: BTreeSet<ContextId>,
}

impl ContextRule {
    pub fn global() -> Self {
        Self::default()
    }

    pub fn new(
        include: impl IntoIterator<Item = ContextId>,
        exclude: impl IntoIterator<Item = ContextId>,
    ) -> Result<Self, GraphError> {
        let include = include.into_iter().collect::<BTreeSet<_>>();
        let exclude = exclude.into_iter().collect::<BTreeSet<_>>();
        if include.iter().any(|context| exclude.contains(context)) {
            return Err(GraphError::OverlappingContextRule);
        }
        Ok(Self { include, exclude })
    }

    pub fn include(&self) -> &BTreeSet<ContextId> {
        &self.include
    }

    pub fn exclude(&self) -> &BTreeSet<ContextId> {
        &self.exclude
    }

    pub fn is_visible_in(&self, scope: &Scope) -> bool {
        if self
            .exclude
            .iter()
            .any(|context| scope.active.contains(context))
        {
            return false;
        }
        self.include.is_empty()
            || self
                .include
                .iter()
                .any(|context| scope.active.contains(context))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Scope {
    active: BTreeSet<ContextId>,
}

impl Scope {
    pub fn global() -> Self {
        Self::default()
    }

    pub fn new(contexts: impl IntoIterator<Item = ContextId>) -> Self {
        Self {
            active: contexts.into_iter().collect(),
        }
    }

    pub fn active(&self) -> &BTreeSet<ContextId> {
        &self.active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StoryInstant(i64);

impl StoryInstant {
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn value(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimelineRange {
    start: StoryInstant,
    end: Option<StoryInstant>,
}

impl TimelineRange {
    pub fn new(start: StoryInstant, end: Option<StoryInstant>) -> Result<Self, GraphError> {
        if end.is_some_and(|end| end < start) {
            return Err(GraphError::InvalidTimeRange);
        }
        Ok(Self { start, end })
    }

    pub const fn at(instant: StoryInstant) -> Self {
        Self {
            start: instant,
            end: None,
        }
    }

    pub const fn start(self) -> StoryInstant {
        self.start
    }

    pub const fn end(self) -> Option<StoryInstant> {
        self.end
    }

    pub fn overlaps(self, other: Self) -> bool {
        let self_end = self.end.unwrap_or(self.start);
        let other_end = other.end.unwrap_or(other.start);
        self.start <= other_end && other.start <= self_end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entity {
    id: EntityId,
    kind: EntityKind,
    name: String,
    aliases: BTreeSet<String>,
    parent: Option<EntityId>,
    contexts: ContextRule,
}

impl Entity {
    pub fn new(
        id: EntityId,
        kind: EntityKind,
        name: impl Into<String>,
        parent: Option<EntityId>,
        contexts: ContextRule,
    ) -> Result<Self, GraphError> {
        let name = name.into();
        validate_text("entity_name", &name, 200)?;
        Ok(Self {
            id,
            kind,
            name,
            aliases: BTreeSet::new(),
            parent,
            contexts,
        })
    }

    pub fn with_aliases(
        mut self,
        aliases: impl IntoIterator<Item = String>,
    ) -> Result<Self, GraphError> {
        let aliases = aliases.into_iter().collect::<BTreeSet<_>>();
        for alias in &aliases {
            validate_text("entity_alias", alias, 200)?;
        }
        self.aliases = aliases;
        Ok(self)
    }

    pub const fn id(&self) -> EntityId {
        self.id
    }

    pub const fn kind(&self) -> EntityKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn aliases(&self) -> &BTreeSet<String> {
        &self.aliases
    }

    pub const fn parent(&self) -> Option<EntityId> {
        self.parent
    }

    pub fn contexts(&self) -> &ContextRule {
        &self.contexts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RelationKind(String);

impl RelationKind {
    pub fn new(value: impl Into<String>) -> Result<Self, GraphError> {
        let value = value.into();
        validate_text("relation_kind", &value, 80)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationDirection {
    Directed,
    Undirected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relation {
    id: RelationId,
    source: EntityId,
    target: EntityId,
    kind: RelationKind,
    direction: RelationDirection,
    active_during: Option<TimelineRange>,
    contexts: ContextRule,
}

impl Relation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: RelationId,
        source: EntityId,
        target: EntityId,
        kind: RelationKind,
        direction: RelationDirection,
        active_during: Option<TimelineRange>,
        contexts: ContextRule,
    ) -> Result<Self, GraphError> {
        if source == target {
            return Err(GraphError::SelfRelation(source));
        }
        let (source, target) = if direction == RelationDirection::Undirected && target < source {
            (target, source)
        } else {
            (source, target)
        };
        Ok(Self {
            id,
            source,
            target,
            kind,
            direction,
            active_during,
            contexts,
        })
    }

    pub const fn id(&self) -> RelationId {
        self.id
    }

    pub const fn source(&self) -> EntityId {
        self.source
    }

    pub const fn target(&self) -> EntityId {
        self.target
    }

    pub fn kind(&self) -> &RelationKind {
        &self.kind
    }

    pub const fn direction(&self) -> RelationDirection {
        self.direction
    }

    pub const fn active_during(&self) -> Option<TimelineRange> {
        self.active_during
    }

    pub fn contexts(&self) -> &ContextRule {
        &self.contexts
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Event {
    id: EventId,
    title: String,
    when: TimelineRange,
    participants: BTreeSet<EntityId>,
    places: BTreeSet<EntityId>,
    contexts: ContextRule,
}

impl Event {
    pub fn new(
        id: EventId,
        title: impl Into<String>,
        when: TimelineRange,
        participants: impl IntoIterator<Item = EntityId>,
        places: impl IntoIterator<Item = EntityId>,
        contexts: ContextRule,
    ) -> Result<Self, GraphError> {
        let title = title.into();
        validate_text("event_title", &title, 200)?;
        let participants = participants.into_iter().collect::<BTreeSet<_>>();
        let places = places.into_iter().collect::<BTreeSet<_>>();
        if participants.is_empty() && places.is_empty() {
            return Err(GraphError::UnlinkedEvent);
        }
        Ok(Self {
            id,
            title,
            when,
            participants,
            places,
            contexts,
        })
    }

    pub const fn id(&self) -> EventId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn when(&self) -> TimelineRange {
        self.when
    }

    pub fn participants(&self) -> &BTreeSet<EntityId> {
        &self.participants
    }

    pub fn places(&self) -> &BTreeSet<EntityId> {
        &self.places
    }

    pub fn contexts(&self) -> &ContextRule {
        &self.contexts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Backlink {
    RelationSource(RelationId),
    RelationTarget(RelationId),
    EventParticipant(EventId),
    EventPlace(EventId),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EntityQuery {
    kind: Option<EntityKind>,
    text: Option<String>,
}

impl EntityQuery {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn kind(mut self, kind: EntityKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn text(mut self, text: impl Into<String>) -> Result<Self, GraphError> {
        let text = text.into();
        validate_text("query_text", &text, 200)?;
        self.text = Some(text.to_lowercase());
        Ok(self)
    }

    fn matches(&self, entity: &Entity) -> bool {
        if self.kind.is_some_and(|kind| entity.kind != kind) {
            return false;
        }
        self.text.as_ref().is_none_or(|text| {
            entity.name.to_lowercase().contains(text)
                || entity
                    .aliases
                    .iter()
                    .any(|alias| alias.to_lowercase().contains(text))
        })
    }
}

#[derive(Debug, Default)]
pub struct StoryGraph {
    entities: BTreeMap<EntityId, Entity>,
    relations: BTreeMap<RelationId, Relation>,
    events: BTreeMap<EventId, Event>,
    children: BTreeMap<Option<EntityId>, BTreeSet<EntityId>>,
    backlinks: BTreeMap<EntityId, BTreeSet<Backlink>>,
    timeline: BTreeSet<(StoryInstant, EventId)>,
}

impl StoryGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_entity(&mut self, entity: Entity) -> Result<(), GraphError> {
        self.ensure_raw_id_available(entity.id.as_uuid())?;
        if let Some(parent_id) = entity.parent {
            let parent = self
                .entities
                .get(&parent_id)
                .ok_or(GraphError::MissingEntity(parent_id))?;
            if parent.kind != entity.kind {
                return Err(GraphError::ParentKindMismatch {
                    child: entity.id,
                    parent: parent_id,
                });
            }
        }
        self.children
            .entry(entity.parent)
            .or_default()
            .insert(entity.id);
        self.backlinks.entry(entity.id).or_default();
        self.entities.insert(entity.id, entity);
        Ok(())
    }

    pub fn reparent_entity(
        &mut self,
        entity_id: EntityId,
        new_parent: Option<EntityId>,
    ) -> Result<(), GraphError> {
        let entity = self
            .entities
            .get(&entity_id)
            .ok_or(GraphError::MissingEntity(entity_id))?;
        if let Some(parent_id) = new_parent {
            let parent = self
                .entities
                .get(&parent_id)
                .ok_or(GraphError::MissingEntity(parent_id))?;
            if parent.kind != entity.kind {
                return Err(GraphError::ParentKindMismatch {
                    child: entity_id,
                    parent: parent_id,
                });
            }
            let mut cursor = Some(parent_id);
            while let Some(candidate) = cursor {
                if candidate == entity_id {
                    return Err(GraphError::ParentCycle(entity_id));
                }
                cursor = self.entities.get(&candidate).and_then(Entity::parent);
            }
        }

        let old_parent = entity.parent;
        if old_parent == new_parent {
            return Ok(());
        }
        if let Some(siblings) = self.children.get_mut(&old_parent) {
            siblings.remove(&entity_id);
        }
        self.children
            .entry(new_parent)
            .or_default()
            .insert(entity_id);
        self.entities
            .get_mut(&entity_id)
            .expect("entity was validated above")
            .parent = new_parent;
        Ok(())
    }

    pub fn insert_relation(&mut self, relation: Relation) -> Result<(), GraphError> {
        self.ensure_raw_id_available(relation.id.as_uuid())?;
        self.require_entity(relation.source)?;
        self.require_entity(relation.target)?;
        self.backlinks
            .entry(relation.source)
            .or_default()
            .insert(Backlink::RelationSource(relation.id));
        self.backlinks
            .entry(relation.target)
            .or_default()
            .insert(Backlink::RelationTarget(relation.id));
        self.relations.insert(relation.id, relation);
        Ok(())
    }

    pub fn insert_event(&mut self, event: Event) -> Result<(), GraphError> {
        self.ensure_raw_id_available(event.id.as_uuid())?;
        for participant in &event.participants {
            self.require_kind(*participant, EntityKind::Character)?;
        }
        for place in &event.places {
            self.require_kind(*place, EntityKind::Place)?;
        }
        for participant in &event.participants {
            self.backlinks
                .entry(*participant)
                .or_default()
                .insert(Backlink::EventParticipant(event.id));
        }
        for place in &event.places {
            self.backlinks
                .entry(*place)
                .or_default()
                .insert(Backlink::EventPlace(event.id));
        }
        self.timeline.insert((event.when.start, event.id));
        self.events.insert(event.id, event);
        Ok(())
    }

    pub fn entity(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(&id)
    }

    pub fn relation(&self, id: RelationId) -> Option<&Relation> {
        self.relations.get(&id)
    }

    pub fn event(&self, id: EventId) -> Option<&Event> {
        self.events.get(&id)
    }

    pub fn scoped<'graph>(&'graph self, scope: &'graph Scope) -> GraphView<'graph> {
        GraphView { graph: self, scope }
    }

    fn require_entity(&self, id: EntityId) -> Result<&Entity, GraphError> {
        self.entities.get(&id).ok_or(GraphError::MissingEntity(id))
    }

    fn require_kind(&self, id: EntityId, expected: EntityKind) -> Result<(), GraphError> {
        let entity = self.require_entity(id)?;
        if entity.kind != expected {
            return Err(GraphError::WrongEntityKind {
                id,
                expected,
                actual: entity.kind,
            });
        }
        Ok(())
    }

    fn ensure_raw_id_available(&self, id: Uuid) -> Result<(), GraphError> {
        let used = self.entities.keys().any(|candidate| candidate.as_uuid() == id)
            || self
                .relations
                .keys()
                .any(|candidate| candidate.as_uuid() == id)
            || self.events.keys().any(|candidate| candidate.as_uuid() == id);
        if used {
            Err(GraphError::IdCollision(id))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GraphView<'graph> {
    graph: &'graph StoryGraph,
    scope: &'graph Scope,
}

impl<'graph> GraphView<'graph> {
    pub fn entity(self, id: EntityId) -> Option<&'graph Entity> {
        self.graph
            .entities
            .get(&id)
            .filter(|entity| entity.contexts.is_visible_in(self.scope))
    }

    pub fn roots(
        self,
        kind: EntityKind,
    ) -> impl Iterator<Item = &'graph Entity> + 'graph {
        self.graph
            .children
            .get(&None)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(move |id| self.entity(*id))
            .filter(move |entity| entity.kind == kind)
    }

    pub fn children(
        self,
        parent: EntityId,
    ) -> impl Iterator<Item = &'graph Entity> + 'graph {
        let parent_visible = self.entity(parent).is_some();
        self.graph
            .children
            .get(&Some(parent))
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(move |id| {
                if parent_visible {
                    self.entity(*id)
                } else {
                    None
                }
            })
    }

    pub fn query(
        self,
        query: EntityQuery,
    ) -> impl Iterator<Item = &'graph Entity> + 'graph {
        self.graph.entities.values().filter(move |entity| {
            entity.contexts.is_visible_in(self.scope) && query.matches(entity)
        })
    }

    pub fn relations(self) -> impl Iterator<Item = &'graph Relation> + 'graph {
        self.graph
            .relations
            .values()
            .filter(move |relation| self.relation_visible(relation))
    }

    pub fn timeline(
        self,
        window: Option<TimelineRange>,
    ) -> impl Iterator<Item = &'graph Event> + 'graph {
        self.graph.timeline.iter().filter_map(move |(_, id)| {
            let event = self.graph.events.get(id)?;
            let in_window = window.is_none_or(|window| event.when.overlaps(window));
            (in_window && self.event_visible(event)).then_some(event)
        })
    }

    pub fn backlinks(
        self,
        entity_id: EntityId,
    ) -> impl Iterator<Item = Backlink> + 'graph {
        let entity_visible = self.entity(entity_id).is_some();
        self.graph
            .backlinks
            .get(&entity_id)
            .into_iter()
            .flat_map(|links| links.iter().copied())
            .filter(move |link| entity_visible && self.backlink_visible(*link))
    }

    fn relation_visible(self, relation: &Relation) -> bool {
        relation.contexts.is_visible_in(self.scope)
            && self.entity(relation.source).is_some()
            && self.entity(relation.target).is_some()
    }

    fn event_visible(self, event: &Event) -> bool {
        event.contexts.is_visible_in(self.scope)
            && event
                .participants
                .iter()
                .chain(event.places.iter())
                .all(|id| self.entity(*id).is_some())
    }

    fn backlink_visible(self, backlink: Backlink) -> bool {
        match backlink {
            Backlink::RelationSource(id) | Backlink::RelationTarget(id) => self
                .graph
                .relations
                .get(&id)
                .is_some_and(|relation| self.relation_visible(relation)),
            Backlink::EventParticipant(id) | Backlink::EventPlace(id) => self
                .graph
                .events
                .get(&id)
                .is_some_and(|event| self.event_visible(event)),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GraphError {
    #[error("invalid {field}")]
    InvalidText { field: &'static str },
    #[error("context include and exclude sets overlap")]
    OverlappingContextRule,
    #[error("timeline end precedes its start")]
    InvalidTimeRange,
    #[error("an event must link at least one character or place")]
    UnlinkedEvent,
    #[error("entity {0} does not exist")]
    MissingEntity(EntityId),
    #[error("entity {id} has kind {actual:?}; expected {expected:?}")]
    WrongEntityKind {
        id: EntityId,
        expected: EntityKind,
        actual: EntityKind,
    },
    #[error("entity {child} and parent {parent} have different kinds")]
    ParentKindMismatch { child: EntityId, parent: EntityId },
    #[error("reparenting entity {0} would create a cycle")]
    ParentCycle(EntityId),
    #[error("entity {0} cannot relate to itself")]
    SelfRelation(EntityId),
    #[error("UUID {0} is already used by another story-graph object")]
    IdCollision(Uuid),
}

fn validate_text(field: &'static str, value: &str, max_chars: usize) -> Result<(), GraphError> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.chars().count() > max_chars
        || value.chars().any(char::is_control)
    {
        Err(GraphError::InvalidText { field })
    } else {
        Ok(())
    }
}
