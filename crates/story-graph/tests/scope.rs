mod support;

use alicent_story_graph::*;
use support::*;

#[test]
fn exclusion_wins_when_a_scope_has_multiple_active_contexts() {
    let dream = context("dream");
    let canon = context("canon");
    let rule = ContextRule::new([dream.clone()], [canon.clone()]).unwrap();

    assert!(rule.is_visible_in(&Scope::new([dream.clone()])));
    assert!(!rule.is_visible_in(&Scope::new([canon.clone()])));
    assert!(!rule.is_visible_in(&Scope::new([dream, canon])));
    assert!(!rule.is_visible_in(&Scope::global()));
}

#[test]
fn excluded_entity_cannot_leak_through_relation_event_or_backlink() {
    let spoiler = context("spoiler");
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(1, EntityKind::Character, "Alicent", None))
        .unwrap();
    let hidden = Entity::new(
        entity_id(2),
        EntityKind::Character,
        "Hidden heir",
        None,
        ContextRule::new([], [spoiler.clone()]).unwrap(),
    )
    .unwrap();
    graph.insert_entity(hidden).unwrap();
    graph
        .insert_relation(
            Relation::new(
                relation_id(10),
                entity_id(1),
                entity_id(2),
                RelationKind::new("knows").unwrap(),
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
                "Revelation",
                TimelineRange::at(StoryInstant::new(20)),
                [entity_id(1), entity_id(2)],
                [],
                ContextRule::global(),
            )
            .unwrap(),
        )
        .unwrap();

    let global = graph.scoped(&Scope::global());
    assert_eq!(global.relations().count(), 1);
    assert_eq!(global.timeline(None).count(), 1);
    assert_eq!(global.backlinks(entity_id(1)).count(), 2);

    let spoiler_scope = Scope::new([spoiler]);
    let scoped = graph.scoped(&spoiler_scope);
    assert!(scoped.entity(entity_id(2)).is_none());
    assert_eq!(scoped.relations().count(), 0);
    assert_eq!(scoped.timeline(None).count(), 0);
    assert_eq!(scoped.backlinks(entity_id(1)).count(), 0);
}

#[test]
fn contextual_children_are_lazily_filtered_without_hiding_visible_siblings() {
    let alternate = context("alternate");
    let mut graph = StoryGraph::new();
    graph
        .insert_entity(entity(1, EntityKind::Place, "World", None))
        .unwrap();
    graph
        .insert_entity(entity(2, EntityKind::Place, "Oldtown", Some(entity_id(1))))
        .unwrap();
    graph
        .insert_entity(
            Entity::new(
                entity_id(3),
                EntityKind::Place,
                "Alternate Oldtown",
                Some(entity_id(1)),
                ContextRule::new([alternate.clone()], []).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

    let global_names = graph
        .scoped(&Scope::global())
        .children(entity_id(1))
        .map(Entity::name)
        .collect::<Vec<_>>();
    assert_eq!(global_names, ["Oldtown"]);

    let alternate_scope = Scope::new([alternate]);
    let alternate_names = graph
        .scoped(&alternate_scope)
        .children(entity_id(1))
        .map(Entity::name)
        .collect::<Vec<_>>();
    assert_eq!(alternate_names, ["Oldtown", "Alternate Oldtown"]);
}
