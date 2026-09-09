mod support;

use alicent_story_graph::*;
use support::*;

#[test]
fn tree_and_text_queries_are_deterministic_and_incremental() {
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(3, EntityKind::Place, "Oldtown", None))
        .unwrap();
    graph
        .insert_entity(entity(1, EntityKind::Place, "King's Landing", None))
        .unwrap();
    graph
        .insert_entity(
            entity(2, EntityKind::Character, "Alicent Hightower", None)
                .with_aliases(["The Green Queen".to_owned()])
                .unwrap(),
        )
        .unwrap();

    let scope = Scope::global();
    let view = graph.scoped(&scope);
    let root_ids = view
        .roots(EntityKind::Place)
        .map(Entity::id)
        .collect::<Vec<_>>();
    assert_eq!(root_ids, [entity_id(1), entity_id(3)]);

    let query = EntityQuery::new()
        .kind(EntityKind::Character)
        .text("green queen")
        .unwrap();
    assert_eq!(
        view.query(query).map(Entity::id).collect::<Vec<_>>(),
        [entity_id(2)]
    );
}

#[test]
fn timeline_is_ordered_and_windowed_without_materializing_a_tree() {
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(1, EntityKind::Character, "Alicent", None))
        .unwrap();
    for (id, title, instant) in [(30, "Third", 30), (10, "First", 10), (20, "Second", 20)] {
        graph
            .insert_event(
                Event::new(
                    event_id(id),
                    title,
                    TimelineRange::at(StoryInstant::new(instant)),
                    [entity_id(1)],
                    [],
                    ContextRule::global(),
                )
                .unwrap(),
            )
            .unwrap();
    }

    let window = TimelineRange::new(StoryInstant::new(15), Some(StoryInstant::new(25))).unwrap();
    let titles = graph
        .scoped(&Scope::global())
        .timeline(Some(window))
        .map(Event::title)
        .collect::<Vec<_>>();
    assert_eq!(titles, ["Second"]);
}
