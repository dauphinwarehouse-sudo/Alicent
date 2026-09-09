use alicent_domain::{DocumentKind, MoveDocument};
use alicent_project_repository::{Error, RelativePosition, Repository};
use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, Repository) {
    let dir = TempDir::new().unwrap();
    let repo = Repository::create(dir.path(), "Порядок").unwrap();
    (dir, repo)
}

#[test]
fn idempotent_create_replays_after_reopen_and_rejects_another_payload() {
    let (_dir, mut repo) = setup();
    let operation_id = Uuid::new_v4();
    let first = repo
        .create_document_with_operation_id(
            operation_id,
            "Первая",
            DocumentKind::Scene,
            None,
        )
        .unwrap();
    let root = repo.root().to_owned();
    drop(repo);

    let mut repo = Repository::open(&root).unwrap();
    let replay = repo
        .create_document_with_operation_id(
            operation_id,
            "Первая",
            DocumentKind::Scene,
            None,
        )
        .unwrap();
    assert_eq!(replay.summary.id, first.summary.id);
    assert_eq!(repo.list(None, 20, 0).unwrap().len(), 1);
    assert!(matches!(
        repo.create_document_with_operation_id(
            operation_id,
            "Другая",
            DocumentKind::Scene,
            None,
        ),
        Err(Error::CommandMismatch)
    ));
}

#[test]
fn relative_order_survives_reopen_and_retry() {
    let (_dir, mut repo) = setup();
    let a = repo
        .create_document("A", DocumentKind::Scene, None)
        .unwrap();
    let b = repo
        .create_document("B", DocumentKind::Scene, None)
        .unwrap();
    let c = repo
        .create_document("C", DocumentKind::Scene, None)
        .unwrap();
    let command = MoveDocument {
        command_id: Uuid::new_v4(),
        document_id: c.summary.id,
        parent_id: None,
        expected_revision: c.summary.revision,
    };
    let moved = repo
        .move_document_relative(command.clone(), RelativePosition::Before(a.summary.id))
        .unwrap();
    assert_eq!(moved.summary.revision, 1);
    assert_eq!(
        repo.move_document_relative(command.clone(), RelativePosition::Before(a.summary.id))
            .unwrap()
            .summary
            .revision,
        1
    );
    assert_eq!(
        repo.list(None, 20, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![c.summary.id, a.summary.id, b.summary.id]
    );
    assert!(matches!(
        repo.move_document_relative(command, RelativePosition::After(b.summary.id)),
        Err(Error::CommandMismatch)
    ));
    let root = repo.root().to_owned();
    drop(repo);
    let repo = Repository::open(&root).unwrap();
    assert_eq!(
        repo.list(None, 20, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![c.summary.id, a.summary.id, b.summary.id]
    );
}

#[test]
fn stale_relative_move_conflicts_without_changing_order() {
    let (_dir, mut repo) = setup();
    let a = repo
        .create_document("A", DocumentKind::Scene, None)
        .unwrap();
    let b = repo
        .create_document("B", DocumentKind::Scene, None)
        .unwrap();
    let c = repo
        .create_document("C", DocumentKind::Scene, None)
        .unwrap();
    let mut other = Repository::open(repo.root()).unwrap();
    repo.move_document_relative(
        MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: b.summary.id,
            parent_id: None,
            expected_revision: 0,
        },
        RelativePosition::After(c.summary.id),
    )
    .unwrap();
    assert!(matches!(
        other.move_document_relative(
            MoveDocument {
                command_id: Uuid::new_v4(),
                document_id: b.summary.id,
                parent_id: None,
                expected_revision: 0,
            },
            RelativePosition::First,
        ),
        Err(Error::Conflict)
    ));
    assert_eq!(
        other
            .list(None, 20, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![a.summary.id, c.summary.id, b.summary.id]
    );
}

#[test]
fn v3_migration_preserves_legacy_order_and_creates_backup() {
    let dir = TempDir::new().unwrap();
    let root = dir.path().join("legacy-v3.alicent");
    std::fs::create_dir(&root).unwrap();
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    db.execute_batch(include_str!("../src/schema.sql")).unwrap();
    db.execute_batch(include_str!("../src/schema-v2.sql")).unwrap();
    db.execute_batch(include_str!("../src/schema-v3.sql")).unwrap();
    db.execute(
        "INSERT INTO project(id,title,schema_version) VALUES(?1,'v3',3)",
        [Uuid::new_v4().to_string()],
    )
    .unwrap();
    let folder = Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
    let scene = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let note = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
    for (id, title, kind) in [
        (scene, "A", "scene"),
        (note, "B", "note"),
        (folder, "Z", "folder"),
    ] {
        db.execute(
            "INSERT INTO documents(id,title,kind) VALUES(?1,?2,?3)",
            rusqlite::params![id.to_string(), title, kind],
        )
        .unwrap();
        db.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'','user:local')",
            [id.to_string()],
        )
        .unwrap();
    }
    drop(db);

    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.project().unwrap().schema_version, 4);
    assert_eq!(
        repo.list(None, 20, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![folder, scene, note]
    );
    assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 1);
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
    assert_eq!(
        db.query_row(
            "SELECT count(DISTINCT order_key) FROM documents",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        3
    );
}
