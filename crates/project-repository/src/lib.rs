//! Transactional storage. Every write includes its version and operation in one WAL transaction.
mod agent_queue;
mod checkpoints;
mod ordering;
mod recovery;
use alicent_domain::*;
pub use ordering::{OrderedDocumentSummary, RelativePosition};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("Ошибка базы проекта")]
    Database(#[from] rusqlite::Error),
    #[error("Не удалось открыть каталог проекта")]
    Io(#[from] std::io::Error),
    #[error("Некорректные данные проекта")]
    Json(#[from] serde_json::Error),
    #[error("Документ не найден")]
    NotFound,
    #[error("Документ изменён в другом окне. Перечитайте его перед сохранением")]
    Conflict,
    #[error("Неподдерживаемая версия формата проекта")]
    UnsupportedSchema,
    #[error("Недопустимый каталог или символьная ссылка")]
    UnsafePath,
    #[error("Недопустимый родитель или тип документа")]
    InvalidParent,
    #[error("Идентификатор команды уже использован с другими параметрами")]
    CommandMismatch,
    #[error("Нарушена целостность проекта")]
    Integrity,
    #[error("Размер страницы должен быть от 1 до 200")]
    InvalidPagination,
    #[error("Можно закрепить не более 8 документов для ИИ")]
    AiContextLimit,
    #[error("Операция отменена; исходные данные не изменены")]
    Cancelled,
    #[error("Превышено время операции; исходные данные не изменены")]
    TimedOut,
    #[error("Проект изменился после предпросмотра. Откройте контрольную точку ещё раз")]
    ProjectConflict,
}
pub type Result<T> = std::result::Result<T, Error>;
pub struct Repository {
    conn: Connection,
    root: PathBuf,
}

fn hash(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}
fn uuid_column(row: &Row<'_>, index: usize) -> rusqlite::Result<Uuid> {
    let value: String = row.get(index)?;
    Uuid::parse_str(&value).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e))
    })
}
fn summary(row: &Row<'_>) -> rusqlite::Result<DocumentSummary> {
    let parent: Option<String> = row.get(1)?;
    let parent_id = parent
        .map(|s| Uuid::parse_str(&s))
        .transpose()
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;
    let kind: String = row.get(3)?;
    let kind = match kind.as_str() {
        "folder" => DocumentKind::Folder,
        "scene" => DocumentKind::Scene,
        "note" => DocumentKind::Note,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(DocumentSummary {
        id: uuid_column(row, 0)?,
        parent_id,
        title: row.get(2)?,
        kind,
        revision: row.get(4)?,
        updated_at: row.get(5)?,
        ai_context_excluded: row.get::<_, i64>(6)? != 0,
        ai_context_pinned: row.get::<_, i64>(7)? != 0,
    })
}
fn document(row: &Row<'_>) -> rusqlite::Result<Document> {
    Ok(Document {
        summary: summary(row)?,
        content: row.get(8)?,
    })
}
// Check all path components, including junctions/reparse points on Windows.
fn ensure_plain_path(path: &Path) -> Result<()> {
    for part in path.ancestors() {
        if part.as_os_str().is_empty() {
            continue;
        }
        let meta = fs::symlink_metadata(part)?;
        if meta.file_type().is_symlink() {
            return Err(Error::UnsafePath);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(Error::UnsafePath);
            }
        }
    }
    Ok(())
}
fn configure(conn: &Connection) -> Result<()> {
    conn.busy_timeout(Duration::from_secs(3))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA trusted_schema=OFF;")?;
    Ok(())
}
impl Repository {
    /// Creates a new UUID-named directory; never overwrites an existing project.
    pub fn create(parent: &Path, title: &str) -> Result<Self> {
        validate_title(title)?;
        ensure_plain_path(parent)?;
        let root = parent
            .canonicalize()?
            .join(format!("{}.alicent", Uuid::new_v4()));
        fs::create_dir(&root)?;
        let conn = Connection::open(root.join("project.sqlite3"))?;
        configure(&conn)?;
        conn.execute_batch("BEGIN IMMEDIATE;")?;
        conn.execute_batch(include_str!("schema.sql"))?;
        conn.execute_batch(include_str!("schema-v2.sql"))?;
        conn.execute_batch(include_str!("schema-v3.sql"))?;
        conn.execute_batch(include_str!("schema-v4.sql"))?;
        conn.execute_batch(include_str!("schema-v5.sql"))?;
        conn.execute(
            "INSERT INTO project(id,title,schema_version) VALUES(?1,?2,?3)",
            params![Uuid::new_v4().to_string(), title.trim(), SCHEMA_VERSION],
        )?;
        conn.execute_batch("COMMIT;")?;
        Ok(Self { conn, root })
    }
    pub fn open(root: &Path) -> Result<Self> {
        ensure_plain_path(root)?;
        let root = root.canonicalize()?;
        let db = root.join("project.sqlite3");
        ensure_plain_path(&db)?;
        for name in [
            "project.sqlite3-wal",
            "project.sqlite3-shm",
            "project.sqlite3-journal",
        ] {
            let sidecar = root.join(name);
            if fs::symlink_metadata(&sidecar).is_ok() {
                ensure_plain_path(&sidecar)?;
            }
        }
        let mut conn = Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;")?;
        let mut version = recovery::validate_database(&conn)?;
        configure(&conn)?;
        if version == 1 {
            recovery::migrate_v1(&mut conn, &root)?;
            version = 2;
        }
        if version == 2 {
            recovery::migrate_v2(&mut conn, &root)?;
            version = 3;
        }
        if version == 3 {
            recovery::migrate_v3(&mut conn, &root)?;
            version = 4;
        }
        if version == 4 {
            recovery::migrate_v4(&mut conn, &root)?;
        }
        let repo = Self { conn, root };
        recovery::validate_database(&repo.conn)?;
        if repo.project()?.schema_version != SCHEMA_VERSION {
            return Err(Error::UnsupportedSchema);
        }
        Ok(repo)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn project(&self) -> Result<Project> {
        Ok(self.conn.query_row(
            "SELECT id,title,schema_version,created_at FROM project",
            [],
            |r| {
                Ok(Project {
                    id: uuid_column(r, 0)?,
                    title: r.get(1)?,
                    schema_version: r.get(2)?,
                    created_at: r.get(3)?,
                })
            },
        )?)
    }
    pub fn list(
        &self,
        parent: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<DocumentSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidPagination);
        }
        let mut stmt = self.conn.prepare("SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned FROM documents WHERE parent_id IS ?1 AND archived_at IS NULL ORDER BY order_key,id LIMIT ?2 OFFSET ?3")?;
        let rows = stmt.query_map(
            params![parent.map(|v| v.to_string()), limit, offset],
            summary,
        )?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn create_document(
        &mut self,
        title: &str,
        kind: DocumentKind,
        parent: Option<Uuid>,
    ) -> Result<Document> {
        self.create_document_with_operation_id(Uuid::new_v4(), title, kind, parent)
    }
    pub fn rename_document(
        &mut self,
        id: Uuid,
        title: &str,
        expected_revision: i64,
        command_id: Uuid,
    ) -> Result<Document> {
        validate_title(title)?;
        let title = title.trim();
        let payload_hash = hash(&format!("rename:{id}:{expected_revision}:{title}"));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command_id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let current: Document = tx
            .query_row(
                "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL",
                [id.to_string()],
                |r| Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                }),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if current.summary.revision != expected_revision {
            return Err(Error::Conflict);
        }
        let revision = expected_revision + 1;
        tx.execute(
            "UPDATE documents SET title=?1,revision=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3",
            params![title, revision, id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,?2,?3,'user:local')",
            params![id.to_string(), revision, current.content],
        )?;
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,before_hash,after_hash,revision) VALUES(?1,?2,'user:local','rename',?3,?4,?5)",
            params![
                command_id.to_string(),
                id.to_string(),
                hash(&current.summary.title),
                hash(title),
                revision
            ],
        )?;
        let result: Document = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1",
            [id.to_string()],
            |r| {
                Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                })
            },
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn duplicate_document(
        &mut self,
        id: Uuid,
        title: &str,
        parent: Option<Uuid>,
        command_id: Uuid,
    ) -> Result<Document> {
        validate_title(title)?;
        let title = title.trim();
        let payload_hash = hash(&format!("duplicate:{id}:{title}:{parent:?}"));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command_id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        if let Some(parent_id) = parent {
            let kind: Option<String> = tx
                .query_row(
                    "SELECT kind FROM documents WHERE id=?1 AND archived_at IS NULL",
                    [parent_id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if kind.as_deref() != Some("folder") {
                return Err(Error::InvalidParent);
            }
        }
        let source: Document = tx
            .query_row(
                "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL",
                [id.to_string()],
                |r| Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                }),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if source.summary.kind == DocumentKind::Folder {
            return Err(Error::InvalidParent);
        }
        let order_key = ordering::last_order_key(&tx, parent, None)?;
        let new_id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO documents(id,parent_id,title,kind,content,order_key,ai_context_excluded)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                new_id.to_string(),
                parent.map(|value| value.to_string()),
                title,
                source.summary.kind.as_str(),
                source.content,
                order_key,
                source.summary.ai_context_excluded
            ],
        )?;
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,?2,'user:local')",
            params![new_id.to_string(), source.content],
        )?;
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,after_hash,revision) VALUES(?1,?2,'user:local','duplicate',?3,0)",
            params![command_id.to_string(), new_id.to_string(), hash(&source.content)],
        )?;
        let result: Document = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1",
            [new_id.to_string()],
            |r| {
                Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                })
            },
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }
    /// Moves a scene, note or folder and journals the metadata revision atomically.
    pub fn move_document(&mut self, command: MoveDocument) -> Result<Document> {
        let payload_hash = hash(&format!("move:{}", serde_json::to_string(&command)?));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command.command_id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let current: Document = tx
            .query_row(
                "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL",
                [command.document_id.to_string()],
                |r| {
                    Ok(Document {
                        summary: summary(r)?,
                        content: r.get(8)?,
                    })
                },
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if current.summary.revision != command.expected_revision {
            return Err(Error::Conflict);
        }
        if let Some(parent_id) = command.parent_id {
            let parent_kind: Option<String> = tx
                .query_row(
                    "SELECT kind FROM documents WHERE id=?1 AND archived_at IS NULL",
                    [parent_id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if parent_kind.as_deref() != Some("folder") {
                return Err(Error::InvalidParent);
            }
            if current.summary.kind == DocumentKind::Folder {
                let creates_cycle: i64 = tx.query_row(
                    "WITH RECURSIVE descendants(id) AS (SELECT id FROM documents WHERE id=?1 AND archived_at IS NULL UNION ALL SELECT d.id FROM documents d JOIN descendants p ON d.parent_id=p.id WHERE d.archived_at IS NULL) SELECT EXISTS(SELECT 1 FROM descendants WHERE id=?2)",
                    params![command.document_id.to_string(), parent_id.to_string()],
                    |r| r.get(0),
                )?;
                if creates_cycle != 0 {
                    return Err(Error::InvalidParent);
                }
            }
        }
        if current.summary.parent_id == command.parent_id {
            tx.execute(
                "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
                params![
                    command.command_id.to_string(),
                    payload_hash,
                    serde_json::to_string(&current)?
                ],
            )?;
            tx.commit()?;
            return Ok(current);
        }
        let order_key =
            ordering::last_order_key(&tx, command.parent_id, Some(command.document_id))?;
        let revision = current.summary.revision + 1;
        let changed = tx.execute(
            "UPDATE documents SET parent_id=?1,order_key=?2,revision=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?4 AND revision=?5 AND archived_at IS NULL",
            params![
                command.parent_id.map(|value| value.to_string()),
                order_key,
                revision,
                command.document_id.to_string(),
                command.expected_revision
            ],
        )?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,?2,?3,'user:local')",
            params![command.document_id.to_string(), revision, &current.content],
        )?;
        let before_parent = current
            .summary
            .parent_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "root".into());
        let after_parent = command
            .parent_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "root".into());
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,before_hash,after_hash,revision) VALUES(?1,?2,'user:local','move',?3,?4,?5)",
            params![
                command.command_id.to_string(),
                command.document_id.to_string(),
                hash(&before_parent),
                hash(&after_parent),
                revision
            ],
        )?;
        let result: Document = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL",
            [command.document_id.to_string()],
            |r| {
                Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                })
            },
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command.command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn pinned_ai_context(&self) -> Result<Vec<DocumentSummary>> {
        let mut statement = self.conn.prepare(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned
             FROM documents
             WHERE archived_at IS NULL AND kind!='folder'
               AND ai_context_pinned=1 AND ai_context_excluded=0
             ORDER BY updated_at DESC,id LIMIT 8",
        )?;
        let rows = statement.query_map([], summary)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn set_document_ai_context(&mut self, command: SetDocumentAiContext) -> Result<Document> {
        if command.excluded && command.pinned {
            return Err(DomainError::InvalidAiContext.into());
        }
        let payload_hash = hash(&format!("ai_context:{}", serde_json::to_string(&command)?));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command.command_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let current = tx
            .query_row(
                "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content
                 FROM documents WHERE id=?1 AND archived_at IS NULL",
                [command.document_id.to_string()],
                document,
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if current.summary.kind == DocumentKind::Folder {
            return Err(Error::InvalidParent);
        }
        if current.summary.revision != command.expected_revision {
            return Err(Error::Conflict);
        }
        if current.summary.ai_context_excluded == command.excluded
            && current.summary.ai_context_pinned == command.pinned
        {
            tx.execute(
                "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
                params![
                    command.command_id.to_string(),
                    payload_hash,
                    serde_json::to_string(&current)?
                ],
            )?;
            tx.commit()?;
            return Ok(current);
        }
        if command.pinned {
            let pinned: i64 = tx.query_row(
                "SELECT count(*) FROM documents
                 WHERE id!=?1 AND archived_at IS NULL
                   AND ai_context_pinned=1 AND ai_context_excluded=0",
                [command.document_id.to_string()],
                |row| row.get(0),
            )?;
            if pinned >= 8 {
                return Err(Error::AiContextLimit);
            }
        }
        let revision = current.summary.revision + 1;
        let changed = tx.execute(
            "UPDATE documents
             SET ai_context_excluded=?1,ai_context_pinned=?2,revision=?3,
                 updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE id=?4 AND revision=?5 AND archived_at IS NULL",
            params![
                command.excluded,
                command.pinned,
                revision,
                command.document_id.to_string(),
                command.expected_revision
            ],
        )?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor)
             VALUES(?1,?2,?3,'user:local')",
            params![command.document_id.to_string(), revision, &current.content],
        )?;
        let before = format!(
            "excluded={};pinned={}",
            current.summary.ai_context_excluded, current.summary.ai_context_pinned
        );
        let after = format!("excluded={};pinned={}", command.excluded, command.pinned);
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,before_hash,after_hash,revision)
             VALUES(?1,?2,'user:local','ai_context',?3,?4,?5)",
            params![
                command.command_id.to_string(),
                command.document_id.to_string(),
                hash(&before),
                hash(&after),
                revision
            ],
        )?;
        let result = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content
             FROM documents WHERE id=?1",
            [command.document_id.to_string()],
            document,
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command.command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn read(&self, id: Uuid) -> Result<Document> {
        self.conn.query_row("SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL", [id.to_string()], |r|Ok(Document { summary: summary(r)?, content: r.get(8)? })).optional()?.ok_or(Error::NotFound)
    }
    pub fn save(&mut self, command: SaveDocument) -> Result<Document> {
        self.save_as(command, "save")
    }
    fn save_as(&mut self, command: SaveDocument, operation: &str) -> Result<Document> {
        validate_content(&command.content)?;
        let payload_hash = hash(&format!("{operation}:{}", serde_json::to_string(&command)?));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let receipt: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command.command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((previous_hash, result)) = receipt {
            if previous_hash != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let current: Document = tx.query_row("SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1 AND archived_at IS NULL", [command.document_id.to_string()], |r|Ok(Document { summary: summary(r)?, content: r.get(8)? })).optional()?.ok_or(Error::NotFound)?;
        if current.summary.kind == DocumentKind::Folder {
            return Err(Error::InvalidParent);
        }
        if current.summary.revision != command.expected_revision {
            return Err(Error::Conflict);
        }
        let revision = current.summary.revision + 1;
        tx.execute("UPDATE documents SET content=?1,revision=?2,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?3", params![command.content,revision,command.document_id.to_string()])?;
        tx.execute("INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,?2,?3,'user:local')", params![command.document_id.to_string(),revision,command.content])?;
        tx.execute("INSERT INTO operations(id,document_id,actor,kind,before_hash,after_hash,revision) VALUES(?1,?2,'user:local',?3,?4,?5,?6)", params![command.command_id.to_string(),command.document_id.to_string(),operation,hash(&current.content),hash(&command.content),revision])?;
        let result = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1",
            [command.document_id.to_string()],
            |r| {
                Ok(Document {
                    summary: summary(r)?,
                    content: r.get(8)?,
                })
            },
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command.command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }
    pub fn versions(&self, id: Uuid, limit: u32, offset: u32) -> Result<Vec<VersionSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidPagination);
        }
        let mut stmt = self.conn.prepare("SELECT revision,created_at,actor FROM versions WHERE document_id=?1 ORDER BY revision DESC LIMIT ?2 OFFSET ?3")?;
        let rows = stmt.query_map(params![id.to_string(), limit, offset], |r| {
            Ok(VersionSummary {
                revision: r.get(0)?,
                created_at: r.get(1)?,
                actor: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn version_content(&self, id: Uuid, revision: i64) -> Result<String> {
        self.conn
            .query_row(
                "SELECT content FROM versions WHERE document_id=?1 AND revision=?2",
                params![id.to_string(), revision],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }
    pub fn restore(
        &mut self,
        id: Uuid,
        revision: i64,
        expected_revision: i64,
        command_id: Uuid,
    ) -> Result<Document> {
        let content = self.version_content(id, revision)?;
        self.save_as(
            SaveDocument {
                command_id,
                document_id: id,
                expected_revision,
                content,
            },
            &format!("restore:{revision}"),
        )
    }
    /// Literal term search, not an arbitrary FTS expression. No content loaded in results.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<DocumentSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidPagination);
        }
        let expression = query
            .split_whitespace()
            .take(32)
            .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        if expression.is_empty() {
            return Ok(vec![]);
        }
        let mut stmt = self.conn.prepare("SELECT d.id,d.parent_id,d.title,d.kind,d.revision,d.updated_at,d.ai_context_excluded,d.ai_context_pinned FROM document_fts JOIN documents d ON d.rowid=document_fts.rowid WHERE document_fts MATCH ?1 AND d.archived_at IS NULL ORDER BY rank LIMIT ?2")?;
        let rows = stmt.query_map(params![expression, limit], summary)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn archived(&self, limit: u32, offset: u32) -> Result<Vec<ArchivedDocument>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidPagination);
        }
        let mut stmt = self.conn.prepare(
            "SELECT d.id,d.parent_id,d.title,d.kind,d.revision,d.updated_at,d.ai_context_excluded,d.ai_context_pinned,d.archived_at,
                    (SELECT count(*) FROM documents x WHERE x.archive_root_id=d.id)
             FROM documents d
             WHERE d.archived_at IS NOT NULL AND d.archive_root_id=d.id
             ORDER BY d.archived_at DESC,d.id DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit, offset], |row| {
            Ok(ArchivedDocument {
                summary: summary(row)?,
                archived_at: row.get(8)?,
                affected_count: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn archive_document(&mut self, command: ArchiveDocument) -> Result<ArchiveReceipt> {
        self.change_archive_state(command, true)
    }

    pub fn restore_archived(&mut self, command: ArchiveDocument) -> Result<ArchiveReceipt> {
        self.change_archive_state(command, false)
    }

    fn change_archive_state(
        &mut self,
        command: ArchiveDocument,
        archive: bool,
    ) -> Result<ArchiveReceipt> {
        let action = if archive { "archive" } else { "restore" };
        let payload_hash = hash(&format!(
            "{action}:{}:{}",
            command.document_id, command.expected_revision
        ));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command.command_id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let (revision, parent_id): (i64, Option<String>) = tx
            .query_row(
                if archive {
                    "SELECT revision,parent_id FROM documents WHERE id=?1 AND archived_at IS NULL"
                } else {
                    "SELECT revision,parent_id FROM documents WHERE id=?1 AND archived_at IS NOT NULL AND archive_root_id=id"
                },
                [command.document_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if revision != command.expected_revision {
            return Err(Error::Conflict);
        }
        if !archive {
            if let Some(parent_id) = parent_id {
                let parent_active: bool = tx
                    .query_row(
                        "SELECT archived_at IS NULL FROM documents WHERE id=?1",
                        [parent_id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or(Error::InvalidParent)?;
                if !parent_active {
                    return Err(Error::InvalidParent);
                }
            }
        }
        let affected = if archive {
            tx.execute(
                "WITH RECURSIVE subtree(id) AS (
                   SELECT id FROM documents WHERE id=?1 AND archived_at IS NULL
                   UNION ALL
                   SELECT d.id FROM documents d JOIN subtree s ON d.parent_id=s.id
                   WHERE d.archived_at IS NULL
                 )
                 UPDATE documents
                 SET archived_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                     archive_root_id=?1,ai_context_pinned=0
                 WHERE id IN (SELECT id FROM subtree)",
                [command.document_id.to_string()],
            )?
        } else {
            tx.execute(
                "UPDATE documents SET archived_at=NULL,archive_root_id=NULL WHERE archive_root_id=?1",
                [command.document_id.to_string()],
            )?
        };
        if affected == 0 {
            return Err(Error::NotFound);
        }
        tx.execute(
            "INSERT INTO archive_journal(command_id,document_id,action,affected_count)
             VALUES(?1,?2,?3,?4)",
            params![
                command.command_id.to_string(),
                command.document_id.to_string(),
                action,
                affected as i64
            ],
        )?;
        let result = ArchiveReceipt {
            command_id: command.command_id,
            document_id: command.document_id,
            affected_count: affected,
        };
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                command.command_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }
}
