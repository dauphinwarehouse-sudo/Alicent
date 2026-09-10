use super::*;
use rusqlite::backup::{Backup, StepResult};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

fn schema(conn: &Connection) -> Result<BTreeMap<String, String>> {
    let mut stmt = conn.prepare("SELECT name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut result = BTreeMap::new();
    for row in rows {
        let (name, sql) = row?;
        result.insert(name, sql.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    Ok(result)
}
/// Reject unknown executable schema before copying/migrating a user-selected database.
pub(super) fn validate_database(conn: &Connection) -> Result<i64> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if !(1..=SCHEMA_VERSION).contains(&version) {
        return Err(Error::UnsupportedSchema);
    }
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(include_str!("schema.sql"))?;
    if version >= 2 {
        expected.execute_batch(include_str!("schema-v2.sql"))?;
    }
    if version >= 3 {
        expected.execute_batch(include_str!("schema-v3.sql"))?;
    }
    if version >= 4 {
        expected.execute_batch(include_str!("schema-v4.sql"))?;
    }
    if schema(conn)? != schema(&expected)? {
        return Err(Error::Integrity);
    }
    let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(Error::Integrity);
    }
    if conn
        .prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        return Err(Error::Integrity);
    }
    let (count, stored): (i64, Option<i64>) = conn.query_row(
        "SELECT count(*),min(schema_version) FROM project",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if count != 1 || stored != Some(version) {
        return Err(Error::Integrity);
    }
    let inconsistent: i64 = conn.query_row("SELECT count(*) FROM documents d WHERE NOT EXISTS(SELECT 1 FROM versions v WHERE v.document_id=d.id AND v.revision=d.revision AND v.content=d.content)",[],|r|r.get(0))?;
    if inconsistent != 0 {
        return Err(Error::Integrity);
    }
    if version >= 3 {
        let invalid_archive: i64 = conn.query_row(
            "SELECT count(*) FROM documents d
             WHERE (d.archived_at IS NULL)!=(d.archive_root_id IS NULL)
                OR (d.archive_root_id IS NOT NULL AND NOT EXISTS(
                    SELECT 1 FROM documents r
                    WHERE r.id=d.archive_root_id AND r.archived_at IS NOT NULL
                      AND r.archive_root_id=r.id))
                OR (d.archived_at IS NULL AND d.parent_id IS NOT NULL AND EXISTS(
                    SELECT 1 FROM documents p WHERE p.id=d.parent_id AND p.archived_at IS NOT NULL))",
            [],
            |r| r.get(0),
        )?;
        if invalid_archive != 0 {
            return Err(Error::Integrity);
        }
    }
    if version >= 4 {
        let invalid_order: i64 = conn.query_row(
            "SELECT (SELECT count(*) FROM documents WHERE order_key<=0)
                    + (SELECT count(*) FROM (
                        SELECT parent_id,order_key FROM documents
                        GROUP BY parent_id,order_key HAVING count(*)>1))",
            [],
            |r| r.get(0),
        )?;
        if invalid_order != 0 {
            return Err(Error::Integrity);
        }
    }
    Ok(version)
}

fn copy_database(
    source: &Connection,
    parent: &Path,
    name: &str,
    cancelled: &AtomicBool,
) -> Result<BackupInfo> {
    ensure_plain_path(parent)?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let parent = parent.canonicalize()?;
    let temporary = tempfile::Builder::new()
        .prefix(".alicent-backup-")
        .tempfile_in(&parent)?;
    let mut destination = Connection::open(temporary.path())?;
    destination.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA synchronous=FULL;")?;
    let started = Instant::now();
    {
        let backup = Backup::new(source, &mut destination)?;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            if started.elapsed() > Duration::from_secs(300) {
                return Err(Error::TimedOut);
            }
            match backup.step(128)? {
                StepResult::Done => break,
                StepResult::More => {}
                StepResult::Busy | StepResult::Locked => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => return Err(Error::Integrity),
            }
        }
    }
    // The exported file is self-contained; no committed state remains in a WAL sidecar.
    destination.execute_batch("PRAGMA journal_mode=DELETE;")?;
    let version = validate_database(&destination)?;
    destination.close().map_err(|(_, e)| e)?;
    temporary.as_file().sync_all()?;
    if cancelled.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let target = parent.join(name);
    let published = temporary.persist_noclobber(&target).map_err(|e| e.error)?;
    let bytes = published.metadata()?.len();
    #[cfg(unix)]
    {
        fs::File::open(&parent)?.sync_all()?;
    }
    Ok(BackupInfo {
        path: target.to_string_lossy().into_owned(),
        bytes,
        schema_version: version,
    })
}

pub(super) fn migrate_v1(conn: &mut Connection, root: &Path) -> Result<()> {
    let backups = root.join("backups");
    if !backups.exists() {
        fs::create_dir(&backups)?;
    }
    ensure_plain_path(&backups)?;
    // Pin the backup and migration to the same read snapshot. A concurrent writer makes
    // the read-to-write upgrade fail atomically instead of migrating an unbacked-up state.
    let tx = conn.transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 1 {
        return Err(Error::UnsupportedSchema);
    }
    copy_database(
        &tx,
        &backups,
        &format!("pre-migration-v1-{}.alicent-backup", Uuid::new_v4()),
        &AtomicBool::new(false),
    )?;
    tx.execute_batch(include_str!("schema-v2.sql"))?;
    tx.commit()?;
    Ok(())
}

pub(super) fn migrate_v2(conn: &mut Connection, root: &Path) -> Result<()> {
    let backups = root.join("backups");
    if !backups.exists() {
        fs::create_dir(&backups)?;
    }
    ensure_plain_path(&backups)?;
    let tx = conn.transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 2 {
        return Err(Error::UnsupportedSchema);
    }
    copy_database(
        &tx,
        &backups,
        &format!("pre-migration-v2-{}.alicent-backup", Uuid::new_v4()),
        &AtomicBool::new(false),
    )?;
    tx.execute_batch(include_str!("schema-v3.sql"))?;
    tx.commit()?;
    Ok(())
}

pub(super) fn migrate_v3(conn: &mut Connection, root: &Path) -> Result<()> {
    let backups = root.join("backups");
    if !backups.exists() {
        fs::create_dir(&backups)?;
    }
    ensure_plain_path(&backups)?;
    let tx = conn.transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 3 {
        return Err(Error::UnsupportedSchema);
    }
    copy_database(
        &tx,
        &backups,
        &format!("pre-migration-v3-{}.alicent-backup", Uuid::new_v4()),
        &AtomicBool::new(false),
    )?;
    tx.execute_batch(include_str!("schema-v4.sql"))?;
    tx.commit()?;
    Ok(())
}

impl Repository {
    /// Consistent live backup through SQLite Backup API; never raw-copy an open database.
    pub fn backup_to(&self, directory: &Path, cancelled: &AtomicBool) -> Result<BackupInfo> {
        copy_database(
            &self.conn,
            directory,
            &format!("{}-{}.alicent-backup", self.project()?.id, Uuid::new_v4()),
            cancelled,
        )
    }
    /// Recover to a NEW directory. The backup and any existing project are never overwritten.
    pub fn restore_backup(backup: &Path, parent: &Path, cancelled: &AtomicBool) -> Result<Self> {
        ensure_plain_path(backup)?;
        ensure_plain_path(parent)?;
        // Standalone exported backups must not resolve attacker-controlled sidecar links.
        for suffix in ["-wal", "-shm", "-journal"] {
            let sidecar = PathBuf::from(format!("{}{suffix}", backup.to_string_lossy()));
            if fs::symlink_metadata(&sidecar).is_ok() {
                ensure_plain_path(&sidecar)?;
            }
        }
        let source = Connection::open_with_flags(
            backup,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        source.execute_batch("PRAGMA trusted_schema=OFF; BEGIN;")?;
        validate_database(&source)?;
        let parent = parent.canonicalize()?;
        let staging = tempfile::Builder::new()
            .prefix(".alicent-restore-")
            .tempdir_in(&parent)?;
        copy_database(&source, staging.path(), "project.sqlite3", cancelled)?;
        source.execute_batch("COMMIT;")?;
        drop(source);
        // Validate/migrate the isolated copy before making it discoverable as a project.
        let staged = Self::open(staging.path())?;
        drop(staged);
        if cancelled.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let root = parent.join(format!("{}.alicent", Uuid::new_v4()));
        if fs::symlink_metadata(&root).is_ok() {
            return Err(Error::UnsafePath);
        }
        fs::rename(staging.path(), &root)?;
        Self::open(&root)
    }
}
