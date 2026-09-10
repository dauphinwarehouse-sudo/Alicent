//! Stable, bounded text chunks and a durable lexical context index prototype.
#![forbid(unsafe_code)]

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use uuid::Uuid;

pub const MIN_CHUNK_BYTES: usize = 768;
pub const TARGET_CHUNK_BYTES: usize = 2 * 1024;
pub const MAX_CHUNK_BYTES: usize = 4 * 1024;
pub const MAX_INDEXED_DOCUMENT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SEARCH_RESULTS: u32 = 50;

const INDEX_FORMAT_VERSION: i64 = 1;
const SCHEMA: &str = r#"
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS context_index_meta (
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  format_version INTEGER NOT NULL
);
INSERT OR IGNORE INTO context_index_meta(singleton,format_version) VALUES(1,1);
CREATE TABLE IF NOT EXISTS indexed_documents (
  document_id TEXT PRIMARY KEY,
  revision INTEGER NOT NULL CHECK(revision>=0),
  title TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS context_chunks (
  document_id TEXT NOT NULL REFERENCES indexed_documents(document_id) ON DELETE CASCADE,
  chunk_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK(ordinal>=0),
  byte_start INTEGER NOT NULL CHECK(byte_start>=0),
  byte_end INTEGER NOT NULL CHECK(byte_end>=byte_start),
  content_hash TEXT NOT NULL,
  token_estimate INTEGER NOT NULL CHECK(token_estimate>=0),
  content TEXT NOT NULL,
  PRIMARY KEY(document_id,chunk_id),
  UNIQUE(document_id,ordinal)
);
CREATE INDEX IF NOT EXISTS context_chunks_document
ON context_chunks(document_id,ordinal,chunk_id);
CREATE VIRTUAL TABLE IF NOT EXISTS context_chunk_fts USING fts5(
  document_id UNINDEXED,
  chunk_id UNINDEXED,
  title,
  content,
  tokenize='unicode61'
);
"#;

#[derive(Debug, thiserror::Error)]
pub enum ContextIndexError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error("context index format is not supported")]
    UnsupportedFormat,
    #[error("document revision must be non-negative")]
    InvalidRevision,
    #[error("the same revision was indexed with different content")]
    RevisionMismatch,
    #[error("an older document revision cannot replace a newer index entry")]
    StaleRevision,
    #[error("document title is invalid")]
    InvalidTitle,
    #[error("document exceeds the 8 MiB indexing limit")]
    DocumentTooLarge,
    #[error("search result limit must be between 1 and 50")]
    InvalidSearchLimit,
    #[error("search query exceeds the 1024-byte limit")]
    QueryTooLarge,
}

pub type Result<T> = std::result::Result<T, ContextIndexError>;

#[derive(Debug, Clone, Copy)]
pub struct IndexDocument<'a> {
    pub document_id: Uuid,
    pub revision: i64,
    pub title: &'a str,
    pub content: &'a str,
    pub excluded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexUpdateKind {
    Indexed,
    Unchanged,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexUpdate {
    pub kind: IndexUpdateKind,
    pub chunk_count: usize,
    pub inserted_chunks: usize,
    pub reused_chunks: usize,
    pub removed_chunks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    pub document_id: Uuid,
    pub chunk_id: String,
    pub ordinal: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content_hash: String,
    pub token_estimate: usize,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextHit {
    pub document_id: Uuid,
    pub chunk_id: String,
    pub document_title: String,
    pub ordinal: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content: String,
    pub rank: f64,
}

pub struct ContextIndex {
    conn: Connection,
}

impl ContextIndex {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch(SCHEMA)?;
        let version: i64 = conn.query_row(
            "SELECT format_version FROM context_index_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if version != INDEX_FORMAT_VERSION {
            return Err(ContextIndexError::UnsupportedFormat);
        }
        Ok(Self { conn })
    }

    pub fn indexed_revision(&self, document_id: Uuid) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT revision FROM indexed_documents WHERE document_id=?1",
                [document_id.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn upsert_document(&mut self, document: IndexDocument<'_>) -> Result<IndexUpdate> {
        validate_document(&document)?;
        if document.excluded {
            let removed_chunks = self.remove_document(document.document_id)?;
            return Ok(IndexUpdate {
                kind: IndexUpdateKind::Removed,
                chunk_count: 0,
                inserted_chunks: 0,
                reused_chunks: 0,
                removed_chunks,
            });
        }

        let content_hash = digest(document.content.as_bytes());
        let existing = self
            .conn
            .query_row(
                "SELECT revision,title,content_hash FROM indexed_documents WHERE document_id=?1",
                [document.document_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        if let Some((revision, title, previous_hash)) = existing {
            if document.revision < revision {
                return Err(ContextIndexError::StaleRevision);
            }
            if document.revision == revision {
                if title == document.title && previous_hash == content_hash {
                    let chunk_count = self.chunks_for_document(document.document_id)?.len();
                    return Ok(IndexUpdate {
                        kind: IndexUpdateKind::Unchanged,
                        chunk_count,
                        inserted_chunks: 0,
                        reused_chunks: chunk_count,
                        removed_chunks: 0,
                    });
                }
                return Err(ContextIndexError::RevisionMismatch);
            }
        }

        let drafts = split_document(document.content);
        let old_chunks = self.raw_chunks(document.document_id)?;
        let mut reusable = BTreeMap::<String, VecDeque<String>>::new();
        let old_ids = old_chunks
            .iter()
            .map(|chunk| chunk.chunk_id.clone())
            .collect::<BTreeSet<_>>();
        for chunk in &old_chunks {
            reusable
                .entry(chunk.content_hash.clone())
                .or_default()
                .push_back(chunk.chunk_id.clone());
        }

        let mut duplicate_ordinals = BTreeMap::<String, usize>::new();
        let mut new_chunks = Vec::with_capacity(drafts.len());
        let mut reused_ids = BTreeSet::new();
        for (ordinal, draft) in drafts.into_iter().enumerate() {
            let duplicate_ordinal = duplicate_ordinals
                .entry(draft.content_hash.clone())
                .or_default();
            let chunk_id = reusable
                .get_mut(&draft.content_hash)
                .and_then(VecDeque::pop_front)
                .unwrap_or_else(|| {
                    stable_chunk_id(
                        document.document_id,
                        &draft.content_hash,
                        *duplicate_ordinal,
                    )
                });
            *duplicate_ordinal += 1;
            if old_ids.contains(&chunk_id) {
                reused_ids.insert(chunk_id.clone());
            }
            new_chunks.push(PreparedChunk {
                chunk_id,
                ordinal,
                byte_start: draft.byte_start,
                byte_end: draft.byte_end,
                content_hash: draft.content_hash,
                token_estimate: draft.token_estimate,
            });
        }

        let new_ids = new_chunks
            .iter()
            .map(|chunk| chunk.chunk_id.clone())
            .collect::<BTreeSet<_>>();
        let removed_chunks = old_chunks
            .iter()
            .filter(|chunk| !new_ids.contains(&chunk.chunk_id))
            .count();
        let reused_chunks = reused_ids.len();
        let inserted_chunks = new_chunks.len().saturating_sub(reused_chunks);

        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO indexed_documents(document_id,revision,title,content_hash)
             VALUES(?1,?2,?3,?4)
             ON CONFLICT(document_id) DO UPDATE SET
               revision=excluded.revision,
               title=excluded.title,
               content_hash=excluded.content_hash,
               indexed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            params![
                document.document_id.to_string(),
                document.revision,
                document.title,
                content_hash
            ],
        )?;
        tx.execute(
            "UPDATE context_chunks SET ordinal=ordinal+1000000000 WHERE document_id=?1",
            [document.document_id.to_string()],
        )?;
        for chunk in &old_chunks {
            if !new_ids.contains(&chunk.chunk_id) {
                tx.execute(
                    "DELETE FROM context_chunk_fts WHERE document_id=?1 AND chunk_id=?2",
                    params![document.document_id.to_string(), chunk.chunk_id],
                )?;
                tx.execute(
                    "DELETE FROM context_chunks WHERE document_id=?1 AND chunk_id=?2",
                    params![document.document_id.to_string(), chunk.chunk_id],
                )?;
            }
        }
        for chunk in &new_chunks {
            let content = &document.content[chunk.byte_start..chunk.byte_end];
            tx.execute(
                "DELETE FROM context_chunk_fts WHERE document_id=?1 AND chunk_id=?2",
                params![document.document_id.to_string(), chunk.chunk_id],
            )?;
            tx.execute(
                "INSERT INTO context_chunks(
                   document_id,chunk_id,ordinal,byte_start,byte_end,
                   content_hash,token_estimate,content
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(document_id,chunk_id) DO UPDATE SET
                   ordinal=excluded.ordinal,
                   byte_start=excluded.byte_start,
                   byte_end=excluded.byte_end,
                   content_hash=excluded.content_hash,
                   token_estimate=excluded.token_estimate,
                   content=excluded.content",
                params![
                    document.document_id.to_string(),
                    chunk.chunk_id,
                    chunk.ordinal as i64,
                    chunk.byte_start as i64,
                    chunk.byte_end as i64,
                    chunk.content_hash,
                    chunk.token_estimate as i64,
                    content
                ],
            )?;
            tx.execute(
                "INSERT INTO context_chunk_fts(document_id,chunk_id,title,content)
                 VALUES(?1,?2,?3,?4)",
                params![
                    document.document_id.to_string(),
                    chunk.chunk_id,
                    document.title,
                    content
                ],
            )?;
        }
        tx.commit()?;

        Ok(IndexUpdate {
            kind: IndexUpdateKind::Indexed,
            chunk_count: new_chunks.len(),
            inserted_chunks,
            reused_chunks,
            removed_chunks,
        })
    }

    pub fn remove_document(&mut self, document_id: Uuid) -> Result<usize> {
        let removed_chunks = self.conn.query_row(
            "SELECT count(*) FROM context_chunks WHERE document_id=?1",
            [document_id.to_string()],
            |row| row.get::<_, i64>(0),
        )? as usize;
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM context_chunk_fts WHERE document_id=?1",
            [document_id.to_string()],
        )?;
        tx.execute(
            "DELETE FROM indexed_documents WHERE document_id=?1",
            [document_id.to_string()],
        )?;
        tx.commit()?;
        Ok(removed_chunks)
    }

    pub fn chunks_for_document(&self, document_id: Uuid) -> Result<Vec<ChunkRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT chunk_id,ordinal,byte_start,byte_end,content_hash,token_estimate,content
             FROM context_chunks WHERE document_id=?1 ORDER BY ordinal",
        )?;
        let rows = statement.query_map([document_id.to_string()], |row| {
            Ok(ChunkRecord {
                document_id,
                chunk_id: row.get(0)?,
                ordinal: row.get::<_, i64>(1)? as usize,
                byte_start: row.get::<_, i64>(2)? as usize,
                byte_end: row.get::<_, i64>(3)? as usize,
                content_hash: row.get(4)?,
                token_estimate: row.get::<_, i64>(5)? as usize,
                content: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn search(
        &self,
        query: &str,
        limit: u32,
        exclude_document: Option<Uuid>,
    ) -> Result<Vec<ContextHit>> {
        if !(1..=MAX_SEARCH_RESULTS).contains(&limit) {
            return Err(ContextIndexError::InvalidSearchLimit);
        }
        if query.len() > 1024 {
            return Err(ContextIndexError::QueryTooLarge);
        }
        let expression = literal_fts_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let excluded = exclude_document.map(|document_id| document_id.to_string());
        let mut statement = self.conn.prepare(
            "SELECT f.document_id,f.chunk_id,d.title,c.ordinal,c.byte_start,c.byte_end,
                    c.content,bm25(context_chunk_fts,0.0,0.0,5.0,1.0)
             FROM context_chunk_fts f
             JOIN context_chunks c
               ON c.document_id=f.document_id AND c.chunk_id=f.chunk_id
             JOIN indexed_documents d ON d.document_id=f.document_id
             WHERE context_chunk_fts MATCH ?1
               AND (?2 IS NULL OR f.document_id<>?2)
             ORDER BY bm25(context_chunk_fts,0.0,0.0,5.0,1.0),f.document_id,c.ordinal
             LIMIT ?3",
        )?;
        let rows = statement.query_map(params![expression, excluded, limit], |row| {
            let raw_document_id: String = row.get(0)?;
            let document_id = Uuid::parse_str(&raw_document_id).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(ContextHit {
                document_id,
                chunk_id: row.get(1)?,
                document_title: row.get(2)?,
                ordinal: row.get::<_, i64>(3)? as usize,
                byte_start: row.get::<_, i64>(4)? as usize,
                byte_end: row.get::<_, i64>(5)? as usize,
                content: row.get(6)?,
                rank: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn raw_chunks(&self, document_id: Uuid) -> Result<Vec<RawChunk>> {
        let mut statement = self.conn.prepare(
            "SELECT chunk_id,content_hash FROM context_chunks
             WHERE document_id=?1 ORDER BY ordinal",
        )?;
        let rows = statement.query_map([document_id.to_string()], |row| {
            Ok(RawChunk {
                chunk_id: row.get(0)?,
                content_hash: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

#[derive(Debug)]
struct RawChunk {
    chunk_id: String,
    content_hash: String,
}

#[derive(Debug)]
struct ChunkDraft {
    byte_start: usize,
    byte_end: usize,
    content_hash: String,
    token_estimate: usize,
}

#[derive(Debug)]
struct PreparedChunk {
    chunk_id: String,
    ordinal: usize,
    byte_start: usize,
    byte_end: usize,
    content_hash: String,
    token_estimate: usize,
}

fn validate_document(document: &IndexDocument<'_>) -> Result<()> {
    if document.revision < 0 {
        return Err(ContextIndexError::InvalidRevision);
    }
    if document.title.trim().is_empty()
        || document.title.trim() != document.title
        || document.title.chars().count() > 200
        || document.title.chars().any(char::is_control)
    {
        return Err(ContextIndexError::InvalidTitle);
    }
    if document.content.len() > MAX_INDEXED_DOCUMENT_BYTES {
        return Err(ContextIndexError::DocumentTooLarge);
    }
    Ok(())
}

fn split_document(content: &str) -> Vec<ChunkDraft> {
    if content.is_empty() {
        return Vec::new();
    }
    let paragraphs = paragraph_ranges(content);
    let mut result = Vec::new();
    let mut chunk_start = 0;
    let mut chunk_end = 0;
    let mut has_chunk = false;

    for (paragraph_start, paragraph_end) in paragraphs {
        let paragraph_len = paragraph_end - paragraph_start;
        if paragraph_len > MAX_CHUNK_BYTES {
            if has_chunk {
                push_chunk(content, chunk_start, chunk_end, &mut result);
                has_chunk = false;
            }
            split_large_range(content, paragraph_start, paragraph_end, &mut result);
            continue;
        }

        if !has_chunk {
            chunk_start = paragraph_start;
            chunk_end = paragraph_end;
            has_chunk = true;
        } else if paragraph_end - chunk_start > MAX_CHUNK_BYTES {
            push_chunk(content, chunk_start, chunk_end, &mut result);
            chunk_start = paragraph_start;
            chunk_end = paragraph_end;
        } else {
            chunk_end = paragraph_end;
        }

        let chunk_len = chunk_end - chunk_start;
        let boundary = boundary_fingerprint(&content[paragraph_start..paragraph_end]) & 7 == 0;
        if chunk_len >= MIN_CHUNK_BYTES && (chunk_len >= TARGET_CHUNK_BYTES || boundary) {
            push_chunk(content, chunk_start, chunk_end, &mut result);
            has_chunk = false;
        }
    }

    if has_chunk {
        push_chunk(content, chunk_start, chunk_end, &mut result);
    }
    result
}

fn paragraph_ranges(content: &str) -> Vec<(usize, usize)> {
    let bytes = content.as_bytes();
    let mut result = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let mut cursor = index + 1;
            while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\r') {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor] == b'\n' {
                let mut end = cursor + 1;
                while end < bytes.len() && matches!(bytes[end], b'\n' | b'\r') {
                    end += 1;
                }
                result.push((start, end));
                start = end;
                index = end;
                continue;
            }
        }
        index += 1;
    }
    if start < content.len() {
        result.push((start, content.len()));
    }
    result
}

fn split_large_range(
    content: &str,
    mut start: usize,
    end: usize,
    result: &mut Vec<ChunkDraft>,
) {
    while end - start > MAX_CHUNK_BYTES {
        let mut cut = start + MAX_CHUNK_BYTES;
        while !content.is_char_boundary(cut) {
            cut -= 1;
        }
        let preferred = content[start..cut]
            .char_indices()
            .filter(|&(offset, character)| {
                start + offset + character.len_utf8() >= start + TARGET_CHUNK_BYTES
                    && character.is_whitespace()
            })
            .map(|(offset, character)| start + offset + character.len_utf8())
            .last();
        if let Some(preferred) = preferred {
            cut = preferred;
        }
        push_chunk(content, start, cut, result);
        start = cut;
    }
    if start < end {
        push_chunk(content, start, end, result);
    }
}

fn push_chunk(content: &str, start: usize, end: usize, result: &mut Vec<ChunkDraft>) {
    let slice = &content[start..end];
    let character_count = slice.chars().count();
    result.push(ChunkDraft {
        byte_start: start,
        byte_end: end,
        content_hash: digest(slice.as_bytes()),
        token_estimate: character_count.div_ceil(4),
    });
}

fn boundary_fingerprint(content: &str) -> u64 {
    content
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn stable_chunk_id(document_id: Uuid, content_hash: &str, duplicate_ordinal: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"alicent-context-chunk-v1\0");
    hasher.update(document_id.as_bytes());
    hasher.update(content_hash.as_bytes());
    hasher.update((duplicate_ordinal as u64).to_le_bytes());
    format!("{:x}", hasher.finalize())
}

fn digest(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn literal_fts_expression(query: &str) -> String {
    query
        .split_whitespace()
        .take(16)
        .map(|term| term.chars().take(64).collect::<String>())
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}
