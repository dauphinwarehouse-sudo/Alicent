mod support;

use alicent_story_graph::*;
use support::*;

#[test]
fn graph_rejects_dangling_and_wrong_kind_event_links() {
    let mut graph = StoryGraph::new();
    let character = entity(1, EntityKind::Character, "Alicent", None);
    graph.insert_entity(character).unwrap();

    let dangling = Event::new(
        event_id(10),
        "Arrival",
        TimelineRange::at(StoryInstant::new(1)),
        [entity_id(99)],
        [],
        ContextRule::global(),
    )
    .unwrap();
    assert_eq!(
        graph.insert_event(dangling),
        Err(GraphError::MissingEntity(entity_id(99)))
    );

    let wrong_kind = Event::new(
        event_id(11),
        "Wrong location",
        TimelineRange::at(StoryInstant::new(2)),
        [],
        [entity_id(1)],
        ContextRule::global(),
    )
    .unwrap();
    assert_eq!(
        graph.insert_event(wrong_kind),
        Err(GraphError::WrongEntityKind {
            id: entity_id(1),
            expected: EntityKind::Place,
            actual: EntityKind::Character,
        })
    );
}

#[test]
fn hierarchy_rejects_cross_kind_parents_and_cycles() {
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(1, EntityKind::Place, "Westeros", None))
        .unwrap();
    graph
        .insert_entity(entity(
            2,
            EntityKind::Place,
            "King's Landing",
            Some(entity_id(1)),
        ))
        .unwrap();
    graph
        .insert_entity(entity(3, EntityKind::Character, "Alicent", None))
        .unwrap();

    assert_eq!(
        graph.reparent_entity(entity_id(3), Some(entity_id(1))),
        Err(GraphError::ParentKindMismatch {
            child: entity_id(3),
            parent: entity_id(1),
        })
    );
    assert_eq!(
        graph.reparent_entity(entity_id(1), Some(entity_id(2))),
        Err(GraphError::ParentCycle(entity_id(1)))
    );
}

#[test]
fn constructors_reject_ambiguous_values() {
    assert_eq!(
        TimelineRange::new(StoryInstant::new(5), Some(StoryInstant::new(4))),
        Err(GraphError::InvalidTimeRange)
    );
    assert_eq!(
        ContextRule::new([context("dream")], [context("dream")]),
        Err(GraphError::OverlappingContextRule)
    );
    assert_eq!(
        Relation::new(
            relation_id(1),
            entity_id(2),
            entity_id(2),
            RelationKind::new("ally").unwrap(),
            RelationDirection::Directed,
            None,
            ContextRule::global(),
        ),
        Err(GraphError::SelfRelation(entity_id(2)))
    );
}

#[test]
fn ids_are_unique_across_graph_object_namespaces() {
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(7, EntityKind::Character, "Alicent", None))
        .unwrap();
    graph
        .insert_entity(entity(8, EntityKind::Character, "Rhaenyra", None))
        .unwrap();

    let relation = Relation::new(
        relation_id(7),
        entity_id(7),
        entity_id(8),
        RelationKind::new("rival").unwrap(),
        RelationDirection::Undirected,
        None,
        ContextRule::global(),
    )
    .unwrap();
    assert_eq!(
        graph.insert_relation(relation),
        Err(GraphError::IdCollision(entity_id(7).as_uuid()))
    );
}
