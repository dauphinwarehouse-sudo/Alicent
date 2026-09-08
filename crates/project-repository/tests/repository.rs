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
