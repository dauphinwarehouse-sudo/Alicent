use alicent_domain::*;
use alicent_project_repository::{Error, Repository};
use rusqlite::Connection;
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
use tempfile::TempDir;
use uuid::Uuid;
fn setup() -> (TempDir, Repository, Document) {
    let dir = TempDir::new().unwrap();
    let mut repo = Repository::create(dir.path(), "Рукопись").unwrap();
    let doc = repo
        .create_document("Сцена", DocumentKind::Scene, None)
        .unwrap();
    (dir, repo, doc)
}
fn save(repo: &mut Repository, doc: &Document, text: &str) -> Document {
    repo.save(SaveDocument {
        command_id: Uuid::new_v4(),
        document_id: doc.summary.id,
        expected_revision: doc.summary.revision,
        content: text.into(),
    })
    .unwrap()
}
fn legacy(parent: &Path) -> (PathBuf, Uuid) {
    let root = parent.join("legacy.alicent");
    std::fs::create_dir(&root).unwrap();
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    db.execute_batch(include_str!("../src/schema.sql")).unwrap();
    let id = Uuid::new_v4();
    db.execute(
        "INSERT INTO project(id,title,schema_version) VALUES(?1,'Старая рукопись',1)",
        [Uuid::new_v4().to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO documents(id,title,kind,content) VALUES(?1,'Глава','scene','Прежний текст')",
        [id.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'Прежний текст','user:local')",[id.to_string()]).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL").unwrap();
    (root, id)
}
fn version_two(parent: &Path) -> (PathBuf, Uuid) {
    let root = parent.join("v2.alicent");
    std::fs::create_dir(&root).unwrap();
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    db.execute_batch(include_str!("../src/schema.sql")).unwrap();
    db.execute_batch(include_str!("../src/schema-v2.sql"))
        .unwrap();
    let id = Uuid::new_v4();
    db.execute(
        "INSERT INTO project(id,title,schema_version) VALUES(?1,'Формат 2',2)",
        [Uuid::new_v4().to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO documents(id,title,kind,content) VALUES(?1,'Глава','scene','Текст v2')",
        [id.to_string()],
    )
    .unwrap();
    db.execute("INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'Текст v2','user:local')",[id.to_string()]).unwrap();
    (root, id)
}
#[test]
fn live_backup_is_standalone_and_restore_never_replaces_original() {
    let (_dir, mut repo, doc) = setup();
    let target = TempDir::new().unwrap();
    let first = save(&mut repo, &doc, "Кириллица, 雪 и дракон");
    let cp = repo.create_checkpoint(Uuid::new_v4(), "До правки").unwrap();
    let backup = repo
        .backup_to(target.path(), &AtomicBool::new(false))
        .unwrap();
    assert!(backup.bytes > 0);
    assert_eq!(backup.schema_version, 4);
    assert!(!PathBuf::from(format!("{}-wal", backup.path)).exists());
    let _later = save(&mut repo, &first, "Новая версия");
    let imported = Repository::restore_backup(
        Path::new(&backup.path),
        target.path(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_ne!(imported.root(), repo.root());
    assert_eq!(imported.read(doc.summary.id).unwrap().content, first.content);
    assert_eq!(imported.checkpoints(20, 0).unwrap()[0].id, cp.id);
    assert_eq!(imported.search("дракон", 20).unwrap().len(), 1);
    assert_eq!(repo.read(doc.summary.id).unwrap().content, "Новая версия");
    assert!(Path::new(&backup.path).exists());
}
#[test]
fn cancelled_backup_and_restore_publish_nothing() {
    let (_dir, repo, _doc) = setup();
    let target = TempDir::new().unwrap();
    assert!(matches!(repo.backup_to(target.path(), &AtomicBool::new(true)), Err(Error::Cancelled)));
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
    let backup = repo.backup_to(target.path(), &AtomicBool::new(false)).unwrap();
    assert!(matches!(Repository::restore_backup(Path::new(&backup.path), target.path(), &AtomicBool::new(true)), Err(Error::Cancelled)));
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 1);
}
#[test]
fn repeated_backups_never_overwrite_each_other() {
    let (_dir, repo, _doc) = setup();
    let target = TempDir::new().unwrap();
    let first = repo.backup_to(target.path(), &AtomicBool::new(false)).unwrap();
    let original = std::fs::read(&first.path).unwrap();
    let second = repo.backup_to(target.path(), &AtomicBool::new(false)).unwrap();
    assert_ne!(first.path, second.path);
    assert_eq!(std::fs::read(first.path).unwrap(), original);
}
#[test]
fn migration_creates_readable_v1_backup_before_upgrading() {
    let dir = TempDir::new().unwrap();
    let (root, id) = legacy(dir.path());
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.project().unwrap().schema_version, 4);
    assert_eq!(repo.read(id).unwrap().content, "Прежний текст");
    let backups = std::fs::read_dir(root.join("backups")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(backups.len(), 3);
    let old_path = backups.iter().find(|entry| entry.file_name().to_string_lossy().starts_with("pre-migration-v1-")).unwrap().path();
    let old = Connection::open(old_path).unwrap();
    assert_eq!(old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(old.query_row("SELECT content FROM documents", [], |r| r.get::<_, String>(0)).unwrap(), "Прежний текст");
    drop(repo);
    Repository::open(&root).unwrap();
    assert_eq!(std::fs::read_dir(root.join("backups")).unwrap().count(), 3);
}
#[test]
fn migration_from_v2_creates_strict_backups_through_v4() {
    let dir = TempDir::new().unwrap();
    let (root, id) = version_two(dir.path());
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.project().unwrap().schema_version, 4);
    assert_eq!(repo.read(id).unwrap().content, "Текст v2");
    let backups = std::fs::read_dir(root.join("backups")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(backups.len(), 2);
    let v2_backup = backups.iter().find(|entry| entry.file_name().to_string_lossy().starts_with("pre-migration-v2-")).unwrap();
    let old = Connection::open(v2_backup.path()).unwrap();
    assert_eq!(old.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    assert!(old.prepare("SELECT archived_at FROM documents").is_err());
}
#[test]
fn invalid_archive_metadata_is_rejected_by_strict_open_validation() {
    let (_dir, repo, doc) = setup();
    let root = repo.root().to_owned();
    drop(repo);
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    db.execute_batch("PRAGMA foreign_keys=OFF").unwrap();
    db.execute("UPDATE documents SET archived_at='2026-01-01T00:00:00Z',archive_root_id=?1 WHERE id=?2", [Uuid::new_v4().to_string(), doc.summary.id.to_string()]).unwrap();
    drop(db);
    assert!(matches!(Repository::open(&root), Err(Error::Integrity)));
}
#[test]
fn failure_to_create_pre_migration_backup_leaves_schema_v1() {
    let dir = TempDir::new().unwrap();
    let (root, _) = legacy(dir.path());
    std::fs::write(root.join("backups"), "not a directory").unwrap();
    assert!(Repository::open(&root).is_err());
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    assert_eq!(db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT content FROM documents", [], |r| r.get::<_, String>(0)).unwrap(), "Прежний текст");
}
#[test]
fn imported_unknown_schema_and_triggers_are_rejected_without_touching_source() {
    let (_dir, repo, _doc) = setup();
    let target = TempDir::new().unwrap();
    let backup = repo.backup_to(target.path(), &AtomicBool::new(false)).unwrap();
    let db = Connection::open(&backup.path).unwrap();
    db.execute_batch("CREATE TRIGGER unexpected AFTER UPDATE ON documents BEGIN DELETE FROM versions; END;").unwrap();
    drop(db);
    let before = std::fs::read(&backup.path).unwrap();
    assert!(matches!(Repository::restore_backup(Path::new(&backup.path), target.path(), &AtomicBool::new(false)), Err(Error::Integrity)));
    assert_eq!(std::fs::read(&backup.path).unwrap(), before);
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 1);
}
#[test]
fn checkpoint_restores_all_texts_atomically_and_preserves_new_documents() {
    let (_dir, mut repo, a) = setup();
    let a = save(&mut repo, &a, "Север");
    let b = repo.create_document("Вторая", DocumentKind::Note, None).unwrap();
    let b = save(&mut repo, &b, "Маяк");
    let cp = repo.create_checkpoint(Uuid::new_v4(), "Первый вариант").unwrap();
    assert_eq!(cp.document_count, 2);
    let a = save(&mut repo, &a, "Юг");
    let b = save(&mut repo, &b, "Город");
    let later = repo.create_document("После снимка", DocumentKind::Scene, None).unwrap();
    let preview = repo.checkpoint_preview(cp.id, 1, 0).unwrap();
    assert_eq!(preview.changed_count, 2);
    assert_eq!(preview.newer_document_count, 1);
    assert!(preview.has_more);
    let command = Uuid::new_v4();
    let result = repo.restore_checkpoint(cp.id, preview.project_revision, command).unwrap();
    assert_eq!(result.changed_count, 2);
    assert_eq!(repo.read(a.summary.id).unwrap().content, "Север");
    assert_eq!(repo.read(b.summary.id).unwrap().content, "Маяк");
    assert_eq!(repo.version_content(a.summary.id, a.summary.revision).unwrap(), "Юг");
    assert!(repo.read(later.summary.id).is_ok());
    assert_eq!(repo.search("Маяк", 20).unwrap().len(), 1);
    assert!(repo.search("Город", 20).unwrap().is_empty());
    let replay = repo.restore_checkpoint(cp.id, preview.project_revision, command).unwrap();
    assert_eq!(replay.changed_count, 2);
    assert_eq!(repo.read(a.summary.id).unwrap().summary.revision, a.summary.revision + 1);
}
#[test]
fn concurrent_project_change_invalidates_checkpoint_preview() {
    let (_dir, mut repo, doc) = setup();
    let cp = repo.create_checkpoint(Uuid::new_v4(), "До").unwrap();
    let doc = save(&mut repo, &doc, "Текст");
    let preview = repo.checkpoint_preview(cp.id, 20, 0).unwrap();
    let mut other = Repository::open(repo.root()).unwrap();
    save(&mut other, &doc, "Правка другого окна");
    assert!(matches!(repo.restore_checkpoint(cp.id, preview.project_revision, Uuid::new_v4()), Err(Error::ProjectConflict)));
    assert_eq!(repo.read(doc.summary.id).unwrap().content, "Правка другого окна");
}
#[test]
fn checkpoint_creation_is_idempotent_and_restore_noop_has_no_new_versions() {
    let (_dir, mut repo, doc) = setup();
    let id = Uuid::new_v4();
    repo.create_checkpoint(id, "Снимок").unwrap();
    repo.create_checkpoint(id, "Снимок").unwrap();
    assert_eq!(repo.checkpoints(20, 0).unwrap().len(), 1);
    assert!(matches!(repo.create_checkpoint(id, "Другой"), Err(Error::CommandMismatch)));
    let preview = repo.checkpoint_preview(id, 20, 0).unwrap();
    let cmd = Uuid::new_v4();
    assert_eq!(repo.restore_checkpoint(id, preview.project_revision, cmd).unwrap().changed_count, 0);
    assert_eq!(repo.versions(doc.summary.id, 20, 0).unwrap().len(), 1);
    assert!(matches!(repo.restore_checkpoint(id, 999, cmd), Err(Error::CommandMismatch)));
}
#[test]
fn mid_restore_failure_rolls_back_every_document_version_and_index() {
    let (_dir, mut repo, a) = setup();
    let b = repo.create_document("Вторая", DocumentKind::Scene, None).unwrap();
    let cp = repo.create_checkpoint(Uuid::new_v4(), "Пустые").unwrap();
    let a = save(&mut repo, &a, "Первая");
    let b = save(&mut repo, &b, "Вторая");
    let preview = repo.checkpoint_preview(cp.id, 20, 0).unwrap();
    let db = Connection::open(repo.root().join("project.sqlite3")).unwrap();
    let last = std::cmp::max(a.summary.id.to_string(), b.summary.id.to_string());
    db.execute_batch(&format!("CREATE TRIGGER test_abort BEFORE UPDATE ON documents WHEN old.id='{last}' BEGIN SELECT RAISE(ABORT,'injected'); END;")).unwrap();
    assert!(repo.restore_checkpoint(cp.id, preview.project_revision, Uuid::new_v4()).is_err());
    assert_eq!(repo.read(a.summary.id).unwrap().content, "Первая");
    assert_eq!(repo.read(b.summary.id).unwrap().content, "Вторая");
    assert_eq!(repo.versions(a.summary.id, 20, 0).unwrap().len(), 2);
    assert_eq!(repo.search("Первая", 20).unwrap().len(), 1);
    db.execute_batch("DROP TRIGGER test_abort;").unwrap();
}
#[test]
fn migration_crash_child() {
    if let Ok(path) = std::env::var("ALICENT_MIGRATION_CRASH") {
        let db = Connection::open(path).unwrap();
        db.execute_batch("BEGIN IMMEDIATE; ALTER TABLE project ADD COLUMN revision INTEGER NOT NULL DEFAULT 0; CREATE TABLE checkpoints(id TEXT PRIMARY KEY);").unwrap();
        std::process::exit(19);
    }
}
#[test]
fn crash_during_schema_transaction_recovers_then_migrates_cleanly() {
    let dir = TempDir::new().unwrap();
    let (root, id) = legacy(dir.path());
    let exit = std::process::Command::new(std::env::current_exe().unwrap()).args(["--exact", "migration_crash_child", "--nocapture"]).env("ALICENT_MIGRATION_CRASH", root.join("project.sqlite3")).status().unwrap();
    assert_eq!(exit.code(), Some(19));
    let db = Connection::open(root.join("project.sqlite3")).unwrap();
    assert_eq!(db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    drop(db);
    let repo = Repository::open(&root).unwrap();
    assert_eq!(repo.read(id).unwrap().content, "Прежний текст");
    assert_eq!(repo.project().unwrap().schema_version, 4);
}
#[cfg(unix)]
#[test]
fn symlink_backup_directory_is_rejected() {
    let (dir, repo, _) = setup();
    let target = TempDir::new().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(target.path(), &link).unwrap();
    assert!(matches!(repo.backup_to(&link, &AtomicBool::new(false)), Err(Error::UnsafePath)));
    assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 0);
}
#[test]
fn automatic_undo_checkpoint_restores_the_pre_rollback_texts() {
    let (_dir, mut repo, doc) = setup();
    let before = save(&mut repo, &doc, "Первая редакция");
    let cp = repo.create_checkpoint(Uuid::new_v4(), "Первая редакция").unwrap();
    let _after = save(&mut repo, &before, "Вторая редакция");
    let preview = repo.checkpoint_preview(cp.id, 200, 0).unwrap();
    let command = Uuid::new_v4();
    let result = repo.restore_checkpoint(cp.id, preview.project_revision, command).unwrap();
    let undo_id = result.undo_checkpoint_id.unwrap();
    assert_eq!(repo.read(doc.summary.id).unwrap().content, "Первая редакция");
    let retry = repo.restore_checkpoint(cp.id, preview.project_revision, command).unwrap();
    assert_eq!(retry.undo_checkpoint_id, Some(undo_id));
    assert_eq!(repo.checkpoints(200, 0).unwrap().len(), 2);
    let preview = repo.checkpoint_preview(undo_id, 200, 0).unwrap();
    repo.restore_checkpoint(undo_id, preview.project_revision, Uuid::new_v4()).unwrap();
    assert_eq!(repo.read(doc.summary.id).unwrap().content, "Вторая редакция");
}
#[test]
fn cancelled_checkpoint_restore_keeps_text_and_publishes_no_undo_point() {
    let (_dir, mut repo, doc) = setup();
    let cp = repo.create_checkpoint(Uuid::new_v4(), "До правки").unwrap();
    let edited = save(&mut repo, &doc, "Новый текст");
    let preview = repo.checkpoint_preview(cp.id, 200, 0).unwrap();
    let result = repo.restore_checkpoint_cancellable(cp.id, preview.project_revision, Uuid::new_v4(), &AtomicBool::new(true));
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(repo.read(doc.summary.id).unwrap().content, edited.content);
    assert_eq!(repo.checkpoints(200, 0).unwrap().len(), 1);
    assert_eq!(repo.versions(doc.summary.id, 200, 0).unwrap().len(), 2);
}
#[test]
fn checkpoint_preview_and_list_are_paginated_without_loading_texts() {
    let (_dir, mut repo, _) = setup();
    for i in 0..4 {
        repo.create_document(&format!("Сцена {i}"), DocumentKind::Scene, None).unwrap();
    }
    let cp = repo.create_checkpoint(Uuid::new_v4(), "Пять сцен").unwrap();
    let first = repo.checkpoint_preview(cp.id, 2, 0).unwrap();
    let last = repo.checkpoint_preview(cp.id, 2, 4).unwrap();
    assert_eq!(first.documents.len(), 2);
    assert!(first.has_more);
    assert_eq!(last.documents.len(), 1);
    assert!(!last.has_more);
    assert_eq!(cp.document_count, 5);
    assert!(repo.checkpoints(1, 1).unwrap().is_empty());
    assert!(matches!(repo.checkpoint_preview(cp.id, 201, 0), Err(Error::InvalidPagination)));
}
