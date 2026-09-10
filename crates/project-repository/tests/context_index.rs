#![allow(dead_code, clippy::double_ended_iterator_last)]

#[path = "../src/context_index.rs"]
mod context_index;

use context_index::{
    ContextIndex, ContextIndexError, IndexDocument, IndexUpdateKind, MAX_CHUNK_BYTES,
};
use tempfile::TempDir;
use uuid::Uuid;

fn long_paragraph(label: &str) -> String {
    format!("{label}{}", " слово".repeat(300))
}

#[test]
fn localized_edit_reuses_unchanged_chunk_ids() {
    let mut index = ContextIndex::in_memory().unwrap();
    let document_id = Uuid::new_v4();
    let first = long_paragraph("Север");
    let middle = long_paragraph("Город");
    let last = long_paragraph("Маяк");
    let content = format!("{first}\n\n{middle}\n\n{last}");

    let initial = index
        .upsert_document(IndexDocument {
            document_id,
            revision: 1,
            title: "Глава",
            content: &content,
            excluded: false,
        })
        .unwrap();
    assert_eq!(initial.kind, IndexUpdateKind::Indexed);
    assert_eq!(initial.chunk_count, 3);
    let before = index.chunks_for_document(document_id).unwrap();

    let unchanged = index
        .upsert_document(IndexDocument {
            document_id,
            revision: 1,
            title: "Глава",
            content: &content,
            excluded: false,
        })
        .unwrap();
    assert_eq!(unchanged.kind, IndexUpdateKind::Unchanged);
    assert_eq!(unchanged.reused_chunks, 3);

    let edited_middle = long_paragraph("Город после дождя");
    let edited = format!("{first}\n\n{edited_middle}\n\n{last}");
    let update = index
        .upsert_document(IndexDocument {
            document_id,
            revision: 2,
            title: "Глава",
            content: &edited,
            excluded: false,
        })
        .unwrap();
    assert_eq!(update.reused_chunks, 2);
    assert_eq!(update.inserted_chunks, 1);
    assert_eq!(update.removed_chunks, 1);

    let after = index.chunks_for_document(document_id).unwrap();
    assert_eq!(before[0].chunk_id, after[0].chunk_id);
    assert_ne!(before[1].chunk_id, after[1].chunk_id);
    assert_eq!(before[2].chunk_id, after[2].chunk_id);
}

#[test]
fn excluded_document_is_removed_from_storage_and_search() {
    let mut index = ContextIndex::in_memory().unwrap();
    let document_id = Uuid::new_v4();
    let content = "Тайный маяк стоит у северной бухты";
    index
        .upsert_document(IndexDocument {
            document_id,
            revision: 1,
            title: "Секретная заметка",
            content,
            excluded: false,
        })
        .unwrap();
    assert_eq!(index.search("маяк", 10, None).unwrap().len(), 1);

    let removed = index
        .upsert_document(IndexDocument {
            document_id,
            revision: 2,
            title: "Секретная заметка",
            content,
            excluded: true,
        })
        .unwrap();
    assert_eq!(removed.kind, IndexUpdateKind::Removed);
    assert_eq!(removed.removed_chunks, 1);
    assert_eq!(index.indexed_revision(document_id).unwrap(), None);
    assert!(index.search("маяк", 10, None).unwrap().is_empty());
}

#[test]
fn lexical_search_is_bounded_and_excludes_the_active_document() {
    let mut index = ContextIndex::in_memory().unwrap();
    let active_id = Uuid::new_v4();
    let reference_id = Uuid::new_v4();
    for (document_id, title, content) in [
        (active_id, "Текущая сцена", "Маяк погас в полночь"),
        (
            reference_id,
            "Опорная сцена",
            "Старый маяк снова осветил берег",
        ),
    ] {
        index
            .upsert_document(IndexDocument {
                document_id,
                revision: 1,
                title,
                content,
                excluded: false,
            })
            .unwrap();
    }

    let hits = index.search("маяк", 10, Some(active_id)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].document_id, reference_id);
    assert!(matches!(
        index.search("маяк", 0, None),
        Err(ContextIndexError::InvalidSearchLimit)
    ));
    assert!(matches!(
        index.search(&"я".repeat(1025), 10, None),
        Err(ContextIndexError::QueryTooLarge)
    ));
}

#[test]
fn index_reopens_without_rebuilding_unchanged_documents() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("context-index.sqlite3");
    let document_id = Uuid::new_v4();
    {
        let mut index = ContextIndex::open(&path).unwrap();
        index
            .upsert_document(IndexDocument {
                document_id,
                revision: 7,
                title: "Глава",
                content: "Над башней летит дракон",
                excluded: false,
            })
            .unwrap();
    }

    let mut reopened = ContextIndex::open(&path).unwrap();
    assert_eq!(reopened.indexed_revision(document_id).unwrap(), Some(7));
    assert_eq!(reopened.search("дракон", 10, None).unwrap().len(), 1);
    let unchanged = reopened
        .upsert_document(IndexDocument {
            document_id,
            revision: 7,
            title: "Глава",
            content: "Над башней летит дракон",
            excluded: false,
        })
        .unwrap();
    assert_eq!(unchanged.kind, IndexUpdateKind::Unchanged);
}

#[test]
fn large_unicode_paragraph_is_split_on_utf8_boundaries() {
    let mut index = ContextIndex::in_memory().unwrap();
    let document_id = Uuid::new_v4();
    let content = "я".repeat(5_000);
    index
        .upsert_document(IndexDocument {
            document_id,
            revision: 1,
            title: "Длинная сцена",
            content: &content,
            excluded: false,
        })
        .unwrap();

    let chunks = index.chunks_for_document(document_id).unwrap();
    assert!(chunks.len() > 1);
    assert!(chunks
        .iter()
        .all(|chunk| chunk.content.len() <= MAX_CHUNK_BYTES));
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<String>(),
        content
    );
    assert!(chunks.iter().all(|chunk| {
        content.is_char_boundary(chunk.byte_start) && content.is_char_boundary(chunk.byte_end)
    }));
}

#[test]
fn revision_guards_reject_stale_or_changed_payloads() {
    let mut index = ContextIndex::in_memory().unwrap();
    let document_id = Uuid::new_v4();
    index
        .upsert_document(IndexDocument {
            document_id,
            revision: 3,
            title: "Глава",
            content: "Первая версия",
            excluded: false,
        })
        .unwrap();

    assert!(matches!(
        index.upsert_document(IndexDocument {
            document_id,
            revision: 2,
            title: "Глава",
            content: "Старая версия",
            excluded: false,
        }),
        Err(ContextIndexError::StaleRevision)
    ));
    assert!(matches!(
        index.upsert_document(IndexDocument {
            document_id,
            revision: 3,
            title: "Глава",
            content: "Другой текст той же ревизии",
            excluded: false,
        }),
        Err(ContextIndexError::RevisionMismatch)
    ));
}
