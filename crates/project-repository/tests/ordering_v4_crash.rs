use alicent_domain::DocumentKind;
use alicent_project_repository::Repository;
use proptest::prelude::*;
use rusqlite::{params, Connection};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn create_crash_child() {
    let Ok(path) = std::env::var("ALICENT_CREATE_CRASH_DB") else {
        return;
    };
    let operation_id = std::env::var("ALICENT_CREATE_OPERATION_ID").unwrap();
    let document_id = Uuid::new_v4().to_string();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE;")
        .unwrap();
    conn.execute(
        "INSERT INTO documents(id,title,kind,order_key) VALUES(?1,'Незавершённая','scene',1024)",
        [&document_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'','user:local')",
        [&document_id],
    )
    .unwrap();
    conn.execute("INSERT INTO operations(id,document_id,actor,kind,after_hash,revision) VALUES(?1,?2,'user:local','create','pending',0)", params![operation_id, document_id]).unwrap();
    conn.execute(
        "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,'pending','{}')",
        [operation_id],
    )
    .unwrap();
    std::process::exit(23);
}

#[test]
fn crash_before_create_commit_leaves_no_partial_rows_and_retry_is_safe() {
    let dir = TempDir::new().unwrap();
    let repo = Repository::create(dir.path(), "Аварийный create").unwrap();
    let root = repo.root().to_owned();
    let operation_id = Uuid::new_v4();
    drop(repo);
    let exit = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "create_crash_child", "--nocapture"])
        .env("ALICENT_CREATE_CRASH_DB", root.join("project.sqlite3"))
        .env("ALICENT_CREATE_OPERATION_ID", operation_id.to_string())
        .status()
        .unwrap();
    assert_eq!(exit.code(), Some(23));
    let mut repo = Repository::open(&root).unwrap();
    assert!(repo.list(None, 20, 0).unwrap().is_empty());
    let created = repo
        .create_document_with_operation_id(operation_id, "Завершённая", DocumentKind::Scene, None)
        .unwrap();
    drop(repo);
    let mut repo = Repository::open(&root).unwrap();
    let replay = repo
        .create_document_with_operation_id(operation_id, "Завершённая", DocumentKind::Scene, None)
        .unwrap();
    assert_eq!(replay.summary.id, created.summary.id);
    assert_eq!(repo.list(None, 20, 0).unwrap().len(), 1);
    let audit = Connection::open(root.join("project.sqlite3")).unwrap();
    assert_eq!(audit.query_row("SELECT (SELECT count(*) FROM operations WHERE id=?1) + (SELECT count(*) FROM receipts WHERE command_id=?1)", [operation_id.to_string()], |row| row.get::<_, i64>(0)).unwrap(), 2);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn creation_order_is_stable_for_arbitrary_duplicate_titles(values in prop::collection::vec(any::<u16>(), 1..40)) {
        let dir = TempDir::new().unwrap();
        let mut repo = Repository::create(dir.path(), "Свойство порядка").unwrap();
        let mut expected = Vec::with_capacity(values.len());
        for value in values {
            let document = repo.create_document_with_operation_id(Uuid::new_v4(), &format!("Сцена {}", value % 5), DocumentKind::Scene, None).unwrap();
            expected.push(document.summary.id);
        }
        let actual = repo.list(None, 200, 0).unwrap().into_iter().map(|row| row.id).collect::<Vec<_>>();
        prop_assert_eq!(actual, expected);
    }
}
