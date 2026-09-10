use alicent_domain::{ArchiveDocument, Document, DocumentKind, DomainError, SetDocumentAiContext};
use alicent_project_repository::{Error, Repository};
use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, Repository) {
    let directory = TempDir::new().unwrap();
    let repository = Repository::create(directory.path(), "Контекст").unwrap();
    (directory, repository)
}

fn set_policy(
    repository: &mut Repository,
    document: &Document,
    excluded: bool,
    pinned: bool,
) -> Document {
    repository
        .set_document_ai_context(SetDocumentAiContext {
            command_id: Uuid::new_v4(),
            document_id: document.summary.id,
            expected_revision: document.summary.revision,
            excluded,
            pinned,
        })
        .unwrap()
}

#[test]
fn policy_persists_and_is_returned_by_reads_lists_and_search() {
    let (_directory, mut repository) = setup();
    let pinned = repository
        .create_document("Опорная сцена", DocumentKind::Scene, None)
        .unwrap();
    let excluded = repository
        .create_document("Секретная заметка", DocumentKind::Note, None)
        .unwrap();
    let pinned = set_policy(&mut repository, &pinned, false, true);
    let excluded = set_policy(&mut repository, &excluded, true, false);

    assert!(pinned.summary.ai_context_pinned);
    assert!(excluded.summary.ai_context_excluded);
    assert_eq!(
        repository.pinned_ai_context().unwrap()[0].id,
        pinned.summary.id
    );
    assert!(
        repository
            .list(None, 20, 0)
            .unwrap()
            .iter()
            .find(|row| row.id == excluded.summary.id)
            .unwrap()
            .ai_context_excluded
    );
    assert!(repository.search("Секретная", 20).unwrap()[0].ai_context_excluded);

    let root = repository.root().to_owned();
    drop(repository);
    let repository = Repository::open(&root).unwrap();
    assert!(
        repository
            .read(pinned.summary.id)
            .unwrap()
            .summary
            .ai_context_pinned
    );
    assert!(
        repository
            .read(excluded.summary.id)
            .unwrap()
            .summary
            .ai_context_excluded
    );
}

#[test]
fn policy_command_is_idempotent_and_rejects_stale_or_changed_payloads() {
    let (_directory, mut repository) = setup();
    let document = repository
        .create_document("Сцена", DocumentKind::Scene, None)
        .unwrap();
    let command = SetDocumentAiContext {
        command_id: Uuid::new_v4(),
        document_id: document.summary.id,
        expected_revision: document.summary.revision,
        excluded: false,
        pinned: true,
    };
    let first = repository.set_document_ai_context(command.clone()).unwrap();
    let replay = repository.set_document_ai_context(command.clone()).unwrap();
    assert_eq!(first.summary.revision, 1);
    assert_eq!(replay.summary.revision, 1);
    assert!(matches!(
        repository.set_document_ai_context(SetDocumentAiContext {
            excluded: true,
            pinned: false,
            ..command
        }),
        Err(Error::CommandMismatch)
    ));
    assert!(matches!(
        repository.set_document_ai_context(SetDocumentAiContext {
            command_id: Uuid::new_v4(),
            document_id: document.summary.id,
            expected_revision: document.summary.revision,
            excluded: true,
            pinned: false,
        }),
        Err(Error::Conflict)
    ));
}

#[test]
fn invalid_policy_and_ninth_pin_are_rejected() {
    let (_directory, mut repository) = setup();
    let invalid = repository
        .create_document("Нельзя", DocumentKind::Scene, None)
        .unwrap();
    assert!(matches!(
        repository.set_document_ai_context(SetDocumentAiContext {
            command_id: Uuid::new_v4(),
            document_id: invalid.summary.id,
            expected_revision: invalid.summary.revision,
            excluded: true,
            pinned: true,
        }),
        Err(Error::Domain(DomainError::InvalidAiContext))
    ));

    let mut ninth = None;
    for index in 0..9 {
        let document = repository
            .create_document(&format!("Контекст {index}"), DocumentKind::Note, None)
            .unwrap();
        if index < 8 {
            set_policy(&mut repository, &document, false, true);
        } else {
            ninth = Some(document);
        }
    }
    let ninth = ninth.unwrap();
    assert!(matches!(
        repository.set_document_ai_context(SetDocumentAiContext {
            command_id: Uuid::new_v4(),
            document_id: ninth.summary.id,
            expected_revision: ninth.summary.revision,
            excluded: false,
            pinned: true,
        }),
        Err(Error::AiContextLimit)
    ));
}

#[test]
fn archive_clears_pin_and_duplicate_only_inherits_exclusion() {
    let (_directory, mut repository) = setup();
    let pinned = repository
        .create_document("Закреплённая", DocumentKind::Scene, None)
        .unwrap();
    let pinned = set_policy(&mut repository, &pinned, false, true);
    repository
        .archive_document(ArchiveDocument {
            command_id: Uuid::new_v4(),
            document_id: pinned.summary.id,
            expected_revision: pinned.summary.revision,
        })
        .unwrap();
    assert!(
        !repository.archived(20, 0).unwrap()[0]
            .summary
            .ai_context_pinned
    );
    repository
        .restore_archived(ArchiveDocument {
            command_id: Uuid::new_v4(),
            document_id: pinned.summary.id,
            expected_revision: pinned.summary.revision,
        })
        .unwrap();
    assert!(
        !repository
            .read(pinned.summary.id)
            .unwrap()
            .summary
            .ai_context_pinned
    );

    let excluded = repository
        .create_document("Исключённая", DocumentKind::Note, None)
        .unwrap();
    let excluded = set_policy(&mut repository, &excluded, true, false);
    let duplicate = repository
        .duplicate_document(
            excluded.summary.id,
            "Исключённая — копия",
            None,
            Uuid::new_v4(),
        )
        .unwrap();
    assert!(duplicate.summary.ai_context_excluded);
    assert!(!duplicate.summary.ai_context_pinned);
}

#[test]
fn v4_migration_adds_policy_defaults_and_validated_backup() {
    let directory = TempDir::new().unwrap();
    let root = directory.path().join("legacy-v4.alicent");
    std::fs::create_dir(&root).unwrap();
    let database = Connection::open(root.join("project.sqlite3")).unwrap();
    database
        .execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    database
        .execute_batch(include_str!("../src/schema-v2.sql"))
        .unwrap();
    database
        .execute_batch(include_str!("../src/schema-v3.sql"))
        .unwrap();
    database
        .execute_batch(include_str!("../src/schema-v4.sql"))
        .unwrap();
    database
        .execute(
            "INSERT INTO project(id,title,schema_version) VALUES(?1,'v4',4)",
            [Uuid::new_v4().to_string()],
        )
        .unwrap();
    let document_id = Uuid::new_v4();
    database
        .execute(
            "INSERT INTO documents(id,title,kind,order_key) VALUES(?1,'Сцена','scene',1024)",
            [document_id.to_string()],
        )
        .unwrap();
    database
        .execute(
            "INSERT INTO versions(document_id,revision,content,actor)
             VALUES(?1,0,'','user:local')",
            [document_id.to_string()],
        )
        .unwrap();
    drop(database);

    let repository = Repository::open(&root).unwrap();
    assert_eq!(repository.project().unwrap().schema_version, 5);
    let migrated = repository.read(document_id).unwrap();
    assert!(!migrated.summary.ai_context_excluded);
    assert!(!migrated.summary.ai_context_pinned);
    let backups = std::fs::read_dir(root.join("backups"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(backups.len(), 1);
    assert!(backups[0]
        .file_name()
        .to_string_lossy()
        .starts_with("pre-migration-v4-"));
    let backup = Connection::open(backups[0].path()).unwrap();
    assert_eq!(
        backup
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        4
    );
}
