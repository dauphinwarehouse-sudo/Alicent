use alicent_domain::*;
use alicent_project_repository::{Error, Repository};
use proptest::prelude::*;
use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

fn setup() -> (TempDir, Repository, Document) {
    let dir = TempDir::new().unwrap();
    let mut repo = Repository::create(dir.path(), "Хроники Севера").unwrap();
    let doc = repo
        .create_document("Глава 1", DocumentKind::Scene, None)
        .unwrap();
    (dir, repo, doc)
}
fn command(doc: &Document, content: &str) -> SaveDocument {
    SaveDocument {
        command_id: Uuid::new_v4(),
        document_id: doc.summary.id,
        expected_revision: doc.summary.revision,
        content: content.into(),
    }
}
#[test]
fn reopen_preserves_unicode_and_versions() {
    let (_dir, mut repo, doc) = setup();
    let saved = repo
        .save(command(&doc, "Алиса встретила дракона.\n\n雪 ❄️"))
        .unwrap();
    let root = repo.root().to_owned();
    drop(repo);
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.project().unwrap().title, "Хроники Севера");
    assert_eq!(repo.read(doc.summary.id).unwrap().content, saved.content);
    assert_eq!(repo.versions(doc.summary.id, 20, 0).unwrap().len(), 2);
    assert_eq!(repo.search("дракона", 20).unwrap().len(), 1);
}
#[test]
fn conflict_never_overwrites_or_appends_history() {
    let (_dir, mut repo, doc) = setup();
    let mut other = Repository::open(repo.root()).unwrap();
    let first = repo.save(command(&doc, "Первый автор")).unwrap();
    assert!(matches!(
        other.save(command(&doc, "Потерянная правка")),
        Err(Error::Conflict)
    ));
    assert_eq!(other.read(doc.summary.id).unwrap().content, first.content);
    assert_eq!(other.versions(doc.summary.id, 20, 0).unwrap().len(), 2);
}
#[test]
fn idempotency_is_bound_to_payload() {
    let (_dir, mut repo, doc) = setup();
    let cmd = command(&doc, "Ветер");
    let result = repo.save(cmd.clone()).unwrap();
    assert_eq!(
        repo.save(cmd.clone()).unwrap().summary.revision,
        result.summary.revision
    );
    let mut wrong = cmd;
    wrong.content = "Другой текст".into();
    assert!(matches!(repo.save(wrong), Err(Error::CommandMismatch)));
    assert_eq!(repo.versions(doc.summary.id, 20, 0).unwrap().len(), 2);
}
#[test]
fn restoration_is_a_new_reversible_version() {
    let (_dir, mut repo, doc) = setup();
    let a = repo.save(command(&doc, "Север")).unwrap();
    let b = repo.save(command(&a, "Юг")).unwrap();
    let restored = repo
        .restore(doc.summary.id, 1, b.summary.revision, Uuid::new_v4())
        .unwrap();
    assert_eq!(restored.content, "Север");
    assert_eq!(restored.summary.revision, 3);
    assert_eq!(repo.version_content(doc.summary.id, 2).unwrap(), "Юг");
    assert_eq!(repo.search("Север", 10).unwrap().len(), 1);
    assert!(repo.search("Юг", 10).unwrap().is_empty());
}
#[test]
fn rename_is_revision_guarded_and_idempotent() {
    let (_dir, mut repo, doc) = setup();
    let command_id = Uuid::new_v4();
    let renamed = repo
        .rename_document(doc.summary.id, "Новая глава", 0, command_id)
        .unwrap();
    assert_eq!(renamed.summary.title, "Новая глава");
    assert_eq!(renamed.summary.revision, 1);
    assert_eq!(renamed.content, doc.content);
    assert_eq!(
        repo.rename_document(doc.summary.id, "Новая глава", 0, command_id)
            .unwrap()
            .summary
            .revision,
        1
    );
    assert!(matches!(
        repo.rename_document(doc.summary.id, "Другое имя", 0, command_id),
        Err(Error::CommandMismatch)
    ));
}
#[test]
fn duplicate_copies_text_once() {
    let (_dir, mut repo, doc) = setup();
    let saved = repo.save(command(&doc, "Текст для копии")).unwrap();
    let command_id = Uuid::new_v4();
    let copy = repo
        .duplicate_document(saved.summary.id, "Глава 1 — копия", None, command_id)
        .unwrap();
    assert_ne!(copy.summary.id, saved.summary.id);
    assert_eq!(copy.content, saved.content);
    assert_eq!(copy.summary.revision, 0);
    assert_eq!(
        repo.duplicate_document(saved.summary.id, "Глава 1 — копия", None, command_id)
            .unwrap()
            .summary
            .id,
        copy.summary.id
    );
    assert_eq!(repo.list(None, 20, 0).unwrap().len(), 2);
}
#[test]
fn archive_hides_scene_from_tree_search_and_direct_read_then_restores_it() {
    let (_dir, mut repo, doc) = setup();
    let saved = repo.save(command(&doc, "Скрытый дракон")).unwrap();
    let archive = ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: saved.summary.id,
        expected_revision: saved.summary.revision,
    };
    let receipt = repo.archive_document(archive.clone()).unwrap();
    assert_eq!(receipt.affected_count, 1);
    assert!(repo.list(None, 20, 0).unwrap().is_empty());
    assert!(repo.search("дракон", 20).unwrap().is_empty());
    assert!(matches!(repo.read(saved.summary.id), Err(Error::NotFound)));
    assert_eq!(
        repo.archived(20, 0).unwrap()[0].summary.id,
        saved.summary.id
    );
    assert_eq!(
        repo.archive_document(archive).unwrap().command_id,
        receipt.command_id
    );
    let audit = Connection::open(repo.root().join("project.sqlite3")).unwrap();
    assert_eq!(
        audit
            .query_row(
                "SELECT affected_count FROM archive_journal WHERE command_id=?1",
                [receipt.command_id.to_string()],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        audit
            .query_row(
                "SELECT count(*) FROM receipts WHERE command_id=?1",
                [receipt.command_id.to_string()],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );

    let restored = repo
        .restore_archived(ArchiveDocument {
            command_id: Uuid::new_v4(),
            document_id: saved.summary.id,
            expected_revision: saved.summary.revision,
        })
        .unwrap();
    assert_eq!(restored.affected_count, 1);
    assert_eq!(
        repo.read(saved.summary.id).unwrap().content,
        "Скрытый дракон"
    );
    assert_eq!(repo.search("дракон", 20).unwrap().len(), 1);
    assert!(repo.archived(20, 0).unwrap().is_empty());
}
#[test]
fn non_empty_folder_is_archived_and_restored_as_one_atomic_subtree() {
    let (_dir, mut repo, _doc) = setup();
    let folder = repo
        .create_document("Часть", DocumentKind::Folder, None)
        .unwrap();
    let scene = repo
        .create_document("Внутри", DocumentKind::Scene, Some(folder.summary.id))
        .unwrap();
    let command_id = Uuid::new_v4();
    let receipt = repo
        .archive_document(ArchiveDocument {
            command_id,
            document_id: folder.summary.id,
            expected_revision: folder.summary.revision,
        })
        .unwrap();
    assert_eq!(receipt.affected_count, 2);
    assert!(repo.read(scene.summary.id).is_err());
    assert_eq!(repo.archived(20, 0).unwrap()[0].affected_count, 2);
    assert!(matches!(
        repo.restore_archived(ArchiveDocument {
            command_id,
            document_id: folder.summary.id,
            expected_revision: folder.summary.revision,
        }),
        Err(Error::CommandMismatch)
    ));
    repo.restore_archived(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: folder.summary.id,
        expected_revision: folder.summary.revision,
    })
    .unwrap();
    assert_eq!(
        repo.list(Some(folder.summary.id), 20, 0).unwrap()[0].id,
        scene.summary.id
    );
}
#[test]
fn independently_archived_child_is_not_restored_with_later_parent_archive() {
    let (_dir, mut repo, _doc) = setup();
    let folder = repo
        .create_document("Часть", DocumentKind::Folder, None)
        .unwrap();
    let scene = repo
        .create_document("Внутри", DocumentKind::Scene, Some(folder.summary.id))
        .unwrap();
    repo.archive_document(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: scene.summary.id,
        expected_revision: scene.summary.revision,
    })
    .unwrap();
    repo.archive_document(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: folder.summary.id,
        expected_revision: folder.summary.revision,
    })
    .unwrap();
    assert_eq!(repo.archived(20, 0).unwrap().len(), 2);
    assert!(matches!(
        repo.restore_archived(ArchiveDocument {
            command_id: Uuid::new_v4(),
            document_id: scene.summary.id,
            expected_revision: scene.summary.revision,
        }),
        Err(Error::InvalidParent)
    ));
    repo.restore_archived(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: folder.summary.id,
        expected_revision: folder.summary.revision,
    })
    .unwrap();
    assert!(repo.read(scene.summary.id).is_err());
}
#[test]
fn parent_must_be_a_folder_in_this_project() {
    let (_dir, mut repo, doc) = setup();
    assert!(matches!(
        repo.create_document("Неверная", DocumentKind::Scene, Some(doc.summary.id)),
        Err(Error::InvalidParent)
    ));
    assert!(matches!(
        repo.create_document("Неверная", DocumentKind::Scene, Some(Uuid::new_v4())),
        Err(Error::InvalidParent)
    ));
    let folder = repo
        .create_document("Часть I", DocumentKind::Folder, None)
        .unwrap();
    let scene = repo
        .create_document("../Не путь", DocumentKind::Scene, Some(folder.summary.id))
        .unwrap();
    assert_eq!(
        repo.list(Some(folder.summary.id), 10, 0).unwrap()[0].id,
        scene.summary.id
    );
    assert!(matches!(
        repo.save(command(&folder, "Нельзя")),
        Err(Error::InvalidParent)
    ));
}
#[test]
fn move_is_revision_guarded_idempotent_and_atomically_journaled() {
    let (_dir, mut repo, doc) = setup();
    let folder = repo
        .create_document("Часть I", DocumentKind::Folder, None)
        .unwrap();
    let command_id = Uuid::new_v4();
    let command = MoveDocument {
        command_id,
        document_id: doc.summary.id,
        parent_id: Some(folder.summary.id),
        expected_revision: doc.summary.revision,
    };
    let moved = repo.move_document(command.clone()).unwrap();
    assert_eq!(moved.summary.parent_id, Some(folder.summary.id));
    assert_eq!(moved.summary.revision, 1);
    assert_eq!(repo.move_document(command).unwrap().summary.revision, 1);
    assert!(repo
        .list(None, 20, 0)
        .unwrap()
        .iter()
        .all(|row| row.id != doc.summary.id));
    assert_eq!(
        repo.list(Some(folder.summary.id), 20, 0).unwrap()[0].id,
        doc.summary.id
    );

    let audit = Connection::open(repo.root().join("project.sqlite3")).unwrap();
    let recorded: (String, i64) = audit
        .query_row(
            "SELECT kind,revision FROM operations WHERE id=?1",
            [command_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(recorded, ("move".into(), 1));
    assert_eq!(
        audit
            .query_row(
                "SELECT count(*) FROM receipts WHERE command_id=?1",
                [command_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn move_rejects_stale_commands_and_reused_ids_with_another_payload() {
    let (_dir, mut repo, doc) = setup();
    let folder = repo
        .create_document("Часть I", DocumentKind::Folder, None)
        .unwrap();
    let command_id = Uuid::new_v4();
    let moved = repo
        .move_document(MoveDocument {
            command_id,
            document_id: doc.summary.id,
            parent_id: Some(folder.summary.id),
            expected_revision: 0,
        })
        .unwrap();
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id,
            document_id: doc.summary.id,
            parent_id: None,
            expected_revision: moved.summary.revision,
        }),
        Err(Error::CommandMismatch)
    ));
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: doc.summary.id,
            parent_id: None,
            expected_revision: 0,
        }),
        Err(Error::Conflict)
    ));
    assert_eq!(
        repo.read(doc.summary.id).unwrap().summary.parent_id,
        Some(folder.summary.id)
    );
}

#[test]
fn move_rejects_non_folder_missing_and_cyclic_parents() {
    let (_dir, mut repo, scene) = setup();
    let parent = repo
        .create_document("Часть I", DocumentKind::Folder, None)
        .unwrap();
    let child = repo
        .create_document("Подпапка", DocumentKind::Folder, Some(parent.summary.id))
        .unwrap();
    for invalid_parent in [Some(scene.summary.id), Some(Uuid::new_v4())] {
        assert!(matches!(
            repo.move_document(MoveDocument {
                command_id: Uuid::new_v4(),
                document_id: child.summary.id,
                parent_id: invalid_parent,
                expected_revision: child.summary.revision,
            }),
            Err(Error::InvalidParent)
        ));
    }
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: parent.summary.id,
            parent_id: Some(child.summary.id),
            expected_revision: parent.summary.revision,
        }),
        Err(Error::InvalidParent)
    ));
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: child.summary.id,
            parent_id: Some(child.summary.id),
            expected_revision: child.summary.revision,
        }),
        Err(Error::InvalidParent)
    ));
    assert_eq!(
        repo.read(parent.summary.id).unwrap().summary.parent_id,
        None
    );
    assert_eq!(
        repo.read(child.summary.id).unwrap().summary.parent_id,
        Some(parent.summary.id)
    );
}

#[test]
fn moving_to_root_preserves_content_and_creates_a_metadata_version() {
    let (_dir, mut repo, doc) = setup();
    let folder = repo
        .create_document("Часть I", DocumentKind::Folder, None)
        .unwrap();
    let inside = repo
        .move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: doc.summary.id,
            parent_id: Some(folder.summary.id),
            expected_revision: 0,
        })
        .unwrap();
    let root = repo
        .move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: inside.summary.id,
            parent_id: None,
            expected_revision: inside.summary.revision,
        })
        .unwrap();
    assert_eq!(root.summary.parent_id, None);
    assert_eq!(root.content, doc.content);
    assert_eq!(root.summary.revision, 2);
    assert_eq!(repo.versions(doc.summary.id, 20, 0).unwrap().len(), 3);
}

#[test]
fn move_rejects_archived_documents_and_archived_destinations() {
    let (_dir, mut repo, scene) = setup();
    let folder = repo
        .create_document("Архивная часть", DocumentKind::Folder, None)
        .unwrap();
    repo.archive_document(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: folder.summary.id,
        expected_revision: folder.summary.revision,
    })
    .unwrap();
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: scene.summary.id,
            parent_id: Some(folder.summary.id),
            expected_revision: scene.summary.revision,
        }),
        Err(Error::InvalidParent)
    ));

    repo.archive_document(ArchiveDocument {
        command_id: Uuid::new_v4(),
        document_id: scene.summary.id,
        expected_revision: scene.summary.revision,
    })
    .unwrap();
    assert!(matches!(
        repo.move_document(MoveDocument {
            command_id: Uuid::new_v4(),
            document_id: scene.summary.id,
            parent_id: None,
            expected_revision: scene.summary.revision,
        }),
        Err(Error::NotFound)
    ));
}

#[test]
fn literal_search_cannot_execute_sql_or_fts_operators() {
    let (_dir, mut repo, doc) = setup();
    repo.save(command(&doc, "дракон север")).unwrap();
    assert!(repo.search("\" OR *; DROP TABLE documents; --", 10).is_ok());
    assert_eq!(repo.list(None, 20, 0).unwrap().len(), 1);
    assert!(repo.search("", 10).unwrap().is_empty());
    assert!(matches!(
        repo.list(None, 1000, 0),
        Err(Error::InvalidPagination)
    ));
}
#[test]
fn project_boundaries_and_missing_files() {
    let (dir, repo, doc) = setup();
    let other = Repository::create(dir.path(), "Второй").unwrap();
    assert!(matches!(other.read(doc.summary.id), Err(Error::NotFound)));
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    assert!(Repository::open(&empty).is_err());
    assert!(!empty.join("project.sqlite3").exists());
    assert_ne!(repo.project().unwrap().id, other.project().unwrap().id);
}
#[test]
fn future_schema_is_refused_not_downgraded() {
    let (_dir, repo, _doc) = setup();
    let root = repo.root().to_owned();
    drop(repo);
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    db.execute_batch("PRAGMA user_version=99").unwrap();
    drop(db);
    assert!(matches!(
        Repository::open(&root),
        Err(Error::UnsupportedSchema)
    ));
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        99
    );
}
#[cfg(unix)]
#[test]
fn symlinks_and_wal_escape_are_refused() {
    let (dir, repo, _doc) = setup();
    let root = repo.root().to_owned();
    drop(repo);
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&root, &link).unwrap();
    assert!(matches!(Repository::open(&link), Err(Error::UnsafePath)));
    let outside = dir.path().join("outside");
    std::fs::write(&outside, "do not touch").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("project.sqlite3-wal")).unwrap();
    assert!(matches!(Repository::open(&root), Err(Error::UnsafePath)));
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "do not touch");
}
#[test]
fn invalid_or_oversized_content_does_not_write() {
    let (_dir, mut repo, doc) = setup();
    assert!(repo
        .save(command(&doc, &"a".repeat(MAX_DOCUMENT_BYTES + 1)))
        .is_err());
    assert_eq!(repo.read(doc.summary.id).unwrap().summary.revision, 0);
    assert!(repo
        .create_document("\n", DocumentKind::Note, None)
        .is_err());
}
// A real process exit without destructors while a transaction is partially written.
#[test]
fn crash_child() {
    if let Ok(path) = std::env::var("ALICENT_CRASH_DB") {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE; UPDATE documents SET content='UNCOMMITTED',revision=999;").unwrap();
        std::process::exit(17);
    }
}
#[test]
fn process_crash_rolls_back_partial_transaction() {
    let (_dir, mut repo, doc) = setup();
    repo.save(command(&doc, "Сохранённый текст")).unwrap();
    let root = repo.root().to_owned();
    drop(repo);
    let exit = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_child", "--nocapture"])
        .env("ALICENT_CRASH_DB", root.join("project.sqlite3"))
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(17));
    let repo = Repository::open(&root).unwrap();
    let recovered = repo.read(doc.summary.id).unwrap();
    assert_eq!(recovered.content, "Сохранённый текст");
    assert_eq!(recovered.summary.revision, 1);
    assert_eq!(repo.versions(doc.summary.id, 20, 0).unwrap().len(), 2);
    assert_eq!(repo.search("Сохранённый", 20).unwrap().len(), 1);
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn arbitrary_unicode_survives_transaction_and_replay(text in ".{0,500}") {
        let (_dir,mut repo,doc) = setup();
        let cmd = command(&doc,&text);
        let first = repo.save(cmd.clone()).unwrap();
        let replay = repo.save(cmd).unwrap();
        prop_assert_eq!(first.content,text);
        prop_assert_eq!(replay.summary.revision,1);
        prop_assert_eq!(repo.version_content(doc.summary.id,0).unwrap(),"");
    }
}
