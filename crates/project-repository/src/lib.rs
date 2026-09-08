//! Transactional storage. Every write includes its version and operation in one WAL transaction.
mod checkpoints;
mod recovery;
use alicent_domain::*;
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
        let version = recovery::validate_database(&conn)?;
        configure(&conn)?;
        if version == 1 {
            recovery::migrate_v1(&mut conn, &root)?;
        }
        let repo = Self { conn, root };
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
        let mut stmt = self.conn.prepare("SELECT id,parent_id,title,kind,revision,updated_at FROM documents WHERE parent_id IS ?1 ORDER BY kind='folder' DESC, title, id LIMIT ?2 OFFSET ?3")?;
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
        validate_title(title)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(id) = parent {
            let k: Option<String> = tx
                .query_row(
                    "SELECT kind FROM documents WHERE id=?1",
                    [id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if k.as_deref() != Some("folder") {
                return Err(Error::InvalidParent);
            }
        }
        let id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO documents(id,parent_id,title,kind) VALUES(?1,?2,?3,?4)",
            params![
                id.to_string(),
                parent.map(|v| v.to_string()),
                title.trim(),
                kind.as_str()
            ],
        )?;
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'','user:local')",
            [id.to_string()],
        )?;
        tx.execute("INSERT INTO operations(id,document_id,actor,kind,after_hash,revision) VALUES(?1,?2,'user:local','create',?3,0)",params![Uuid::new_v4().to_string(),id.to_string(),hash("")])?;
        tx.commit()?;
        self.read(id)
    }
    pub fn read(&self, id: Uuid) -> Result<Document> {
        self.conn.query_row("SELECT id,parent_id,title,kind,revision,updated_at,content FROM documents WHERE id=?1", [id.to_string()], |r|Ok(Document { summary: summary(r)?, content: r.get(6)? })).optional()?.ok_or(Error::NotFound)
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
        let current: Document = tx.query_row("SELECT id,parent_id,title,kind,revision,updated_at,content FROM documents WHERE id=?1", [command.document_id.to_string()], |r|Ok(Document { summary: summary(r)?, content: r.get(6)? })).optional()?.ok_or(Error::NotFound)?;
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
            "SELECT id,parent_id,title,kind,revision,updated_at,content FROM documents WHERE id=?1",
            [command.document_id.to_string()],
            |r| {
                Ok(Document {
                    summary: summary(r)?,
                    content: r.get(6)?,
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
        let mut stmt = self.conn.prepare("SELECT d.id,d.parent_id,d.title,d.kind,d.revision,d.updated_at FROM document_fts JOIN documents d ON d.rowid=document_fts.rowid WHERE document_fts MATCH ?1 ORDER BY rank LIMIT ?2")?;
        let rows = stmt.query_map(params![expression, limit], summary)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}
