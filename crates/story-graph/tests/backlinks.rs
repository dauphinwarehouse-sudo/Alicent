mod support;

use alicent_story_graph::*;
use support::*;

#[test]
fn backlinks_cover_both_relation_roles_and_event_roles() {
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(1, EntityKind::Character, "Alicent", None))
        .unwrap();
    graph
        .insert_entity(entity(2, EntityKind::Character, "Rhaenyra", None))
        .unwrap();
    graph
        .insert_entity(entity(3, EntityKind::Place, "Red Keep", None))
        .unwrap();
    graph
        .insert_relation(
            Relation::new(
                relation_id(10),
                entity_id(1),
                entity_id(2),
                RelationKind::new("rival").unwrap(),
                RelationDirection::Directed,
                None,
                ContextRule::global(),
            )
            .unwrap(),
        )
        .unwrap();
    graph
        .insert_event(
            Event::new(
                event_id(20),
                "Council",
                TimelineRange::at(StoryInstant::new(5)),
                [entity_id(1)],
                [entity_id(3)],
                ContextRule::global(),
            )
            .unwrap(),
        )
        .unwrap();

    let scope = Scope::global();
    let view = graph.scoped(&scope);
    assert_eq!(
        view.backlinks(entity_id(1)).collect::<Vec<_>>(),
        [
            Backlink::RelationSource(relation_id(10)),
            Backlink::EventParticipant(event_id(20)),
        ]
    );
    assert_eq!(
        view.backlinks(entity_id(2)).collect::<Vec<_>>(),
        [Backlink::RelationTarget(relation_id(10))]
    );
    assert_eq!(
        view.backlinks(entity_id(3)).collect::<Vec<_>>(),
        [Backlink::EventPlace(event_id(20))]
    );
}
