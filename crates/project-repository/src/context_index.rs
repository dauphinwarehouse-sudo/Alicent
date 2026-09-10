//! Stable, bounded lexical context index stored inside the project database.
#![forbid(unsafe_code)]

use crate::{uuid_column, Error, Repository, Result};
use rusqlite::{params, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use uuid::Uuid;

pub const MIN_CHUNK_BYTES: usize = 768;
pub const TARGET_CHUNK_BYTES: usize = 2 * 1024;
pub const MAX_CHUNK_BYTES: usize = 4 * 1024;
pub const MAX_CONTEXT_REFRESH_DOCUMENTS: u32 = 32;
pub const MAX_CONTEXT_RESULTS: u32 = 50;
const MAX_CONTEXT_QUERY_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextRefresh {
    pub refreshed_documents: usize,
    pub indexed_chunks: usize,
    pub inserted_chunks: usize,
    pub reused_chunks: usize,
    pub removed_chunks: usize,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextChunk {
    pub document_id: Uuid,
    pub chunk_id: String,
    pub source_revision: i64,
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
    pub source_revision: i64,
    pub document_title: String,
    pub ordinal: usize,
    pub byte_start: usize,
    pub byte_end: usize,
    pub content: String,
    pub rank: f64,
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

#[derive(Debug)]
struct RawChunk {
    chunk_id: String,
    content_hash: String,
}

#[derive(Debug, Default)]
struct DocumentRefresh {
    chunk_count: usize,
    inserted_chunks: usize,
    reused_chunks: usize,
    removed_chunks: usize,
}

impl Repository {
    /// Refreshes at most `limit` stale active scenes/notes in one transaction.
    /// Archived, excluded and folder rows can never become candidates.
    pub fn refresh_context_index(&mut self, limit: u32) -> Result<ContextRefresh> {
        if !(1..=MAX_CONTEXT_REFRESH_DOCUMENTS).contains(&limit) {
            return Err(Error::InvalidContextRefreshLimit);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let candidate_limit = i64::from(limit) + 1;
        let mut candidates = {
            let mut statement = tx.prepare(
                "SELECT d.id
                 FROM documents d
                 LEFT JOIN context_index_documents i ON i.document_id=d.id
                 WHERE d.archived_at IS NULL
                   AND d.kind IN ('scene','note')
                   AND d.ai_context_excluded=0
                   AND (i.document_id IS NULL OR i.source_revision!=d.revision)
                 ORDER BY d.updated_at,d.id
                 LIMIT ?1",
            )?;
            statement
                .query_map([candidate_limit], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let has_more = candidates.len() > limit as usize;
        candidates.truncate(limit as usize);

        let mut result = ContextRefresh {
            refreshed_documents: 0,
            indexed_chunks: 0,
            inserted_chunks: 0,
            reused_chunks: 0,
            removed_chunks: 0,
            has_more,
        };
        for document_id in candidates {
            let (title, source_revision, content): (String, i64, String) = tx.query_row(
                "SELECT title,revision,content FROM documents
                 WHERE id=?1 AND archived_at IS NULL
                   AND kind IN ('scene','note') AND ai_context_excluded=0",
                [&document_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let document_id = Uuid::parse_str(&document_id).map_err(|_| Error::Integrity)?;
            let refreshed = refresh_document(
                &tx,
                document_id,
                source_revision,
                &title,
                &content,
            )?;
            result.refreshed_documents += 1;
            result.indexed_chunks += refreshed.chunk_count;
            result.inserted_chunks += refreshed.inserted_chunks;
            result.reused_chunks += refreshed.reused_chunks;
            result.removed_chunks += refreshed.removed_chunks;
        }
        tx.commit()?;
        Ok(result)
    }

    /// Returns only fresh chunks from eligible documents and always excludes the current document.
    pub fn retrieve_context(
        &self,
        query: &str,
        limit: u32,
        current_document: Uuid,
    ) -> Result<Vec<ContextHit>> {
        if !(1..=MAX_CONTEXT_RESULTS).contains(&limit) {
            return Err(Error::InvalidContextResultLimit);
        }
        if query.len() > MAX_CONTEXT_QUERY_BYTES {
            return Err(Error::ContextQueryTooLarge);
        }
        let expression = literal_fts_expression(query);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            "SELECT f.document_id,f.chunk_id,f.source_revision,d.title,c.ordinal,
                    c.byte_start,c.byte_end,c.content,
                    bm25(context_chunk_fts,0.0,0.0,0.0,5.0,1.0)
             FROM context_chunk_fts f
             JOIN context_index_documents i ON i.document_id=f.document_id
             JOIN context_chunks c
               ON c.document_id=f.document_id AND c.chunk_id=f.chunk_id
             JOIN documents d ON d.id=f.document_id
             WHERE context_chunk_fts MATCH ?1
               AND f.document_id!=?2
               AND d.archived_at IS NULL
               AND d.kind IN ('scene','note')
               AND d.ai_context_excluded=0
               AND i.source_revision=d.revision
               AND c.source_revision=d.revision
               AND f.source_revision=d.revision
             ORDER BY bm25(context_chunk_fts,0.0,0.0,0.0,5.0,1.0),
                      f.document_id,c.ordinal
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![expression, current_document.to_string(), limit],
            |row| {
                Ok(ContextHit {
                    document_id: uuid_column(row, 0)?,
                    chunk_id: row.get(1)?,
                    source_revision: row.get(2)?,
                    document_title: row.get(3)?,
                    ordinal: row.get::<_, i64>(4)? as usize,
                    byte_start: row.get::<_, i64>(5)? as usize,
                    byte_end: row.get::<_, i64>(6)? as usize,
                    content: row.get(7)?,
                    rank: row.get(8)?,
                })
            },
        )?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn indexed_context_revision(&self, document_id: Uuid) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT source_revision FROM context_index_documents WHERE document_id=?1",
                [document_id.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn context_chunks_for_document(&self, document_id: Uuid) -> Result<Vec<ContextChunk>> {
        let mut statement = self.conn.prepare(
            "SELECT chunk_id,source_revision,ordinal,byte_start,byte_end,
                    content_hash,token_estimate,content
             FROM context_chunks WHERE document_id=?1 ORDER BY ordinal",
        )?;
        let rows = statement.query_map([document_id.to_string()], |row| {
            Ok(ContextChunk {
                document_id,
                chunk_id: row.get(0)?,
                source_revision: row.get(1)?,
                ordinal: row.get::<_, i64>(2)? as usize,
                byte_start: row.get::<_, i64>(3)? as usize,
                byte_end: row.get::<_, i64>(4)? as usize,
                content_hash: row.get(5)?,
                token_estimate: row.get::<_, i64>(6)? as usize,
                content: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

fn refresh_document(
    tx: &Transaction<'_>,
    document_id: Uuid,
    source_revision: i64,
    title: &str,
    content: &str,
) -> Result<DocumentRefresh> {
    let drafts = split_document(content);
    let old_chunks = raw_chunks(tx, document_id)?;
    let old_ids = old_chunks
        .iter()
        .map(|chunk| chunk.chunk_id.clone())
        .collect::<BTreeSet<_>>();
    let mut reusable = BTreeMap::<String, VecDeque<String>>::new();
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
                stable_chunk_id(document_id, &draft.content_hash, *duplicate_ordinal)
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
    let document_key = document_id.to_string();

    tx.execute(
        "DELETE FROM context_chunk_fts WHERE document_id=?1",
        [&document_key],
    )?;
    tx.execute(
        "INSERT INTO context_index_documents(document_id,source_revision,content_hash)
         VALUES(?1,?2,?3)
         ON CONFLICT(document_id) DO UPDATE SET
           source_revision=excluded.source_revision,
           content_hash=excluded.content_hash,
           indexed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",
        params![document_key, source_revision, digest(content.as_bytes())],
    )?;
    tx.execute(
        "UPDATE context_chunks SET ordinal=ordinal+1000000000 WHERE document_id=?1",
        [&document_key],
    )?;
    for chunk in &old_chunks {
        if !new_ids.contains(&chunk.chunk_id) {
            tx.execute(
                "DELETE FROM context_chunks WHERE document_id=?1 AND chunk_id=?2",
                params![document_key, chunk.chunk_id],
            )?;
        }
    }
    for chunk in &new_chunks {
        let chunk_content = &content[chunk.byte_start..chunk.byte_end];
        tx.execute(
            "INSERT INTO context_chunks(
               document_id,chunk_id,source_revision,ordinal,byte_start,byte_end,
               content_hash,token_estimate,content
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(document_id,chunk_id) DO UPDATE SET
               source_revision=excluded.source_revision,
               ordinal=excluded.ordinal,
               byte_start=excluded.byte_start,
               byte_end=excluded.byte_end,
               content_hash=excluded.content_hash,
               token_estimate=excluded.token_estimate,
               content=excluded.content",
            params![
                document_key,
                chunk.chunk_id,
                source_revision,
                chunk.ordinal as i64,
                chunk.byte_start as i64,
                chunk.byte_end as i64,
                chunk.content_hash,
                chunk.token_estimate as i64,
                chunk_content
            ],
        )?;
        tx.execute(
            "INSERT INTO context_chunk_fts(
               document_id,chunk_id,source_revision,title,content
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                document_key,
                chunk.chunk_id,
                source_revision,
                title,
                chunk_content
            ],
        )?;
    }

    Ok(DocumentRefresh {
        chunk_count: new_chunks.len(),
        inserted_chunks,
        reused_chunks,
        removed_chunks,
    })
}

fn raw_chunks(tx: &Transaction<'_>, document_id: Uuid) -> Result<Vec<RawChunk>> {
    let mut statement = tx.prepare(
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
            .next_back();
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
    result.push(ChunkDraft {
        byte_start: start,
        byte_end: end,
        content_hash: digest(slice.as_bytes()),
        token_estimate: slice.chars().count().div_ceil(4),
    });
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

fn boundary_fingerprint(content: &str) -> u64 {
    content
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
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
