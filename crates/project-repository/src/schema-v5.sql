ALTER TABLE documents ADD COLUMN ai_context_excluded INTEGER NOT NULL DEFAULT 0
  CHECK(ai_context_excluded IN (0,1));
ALTER TABLE documents ADD COLUMN ai_context_pinned INTEGER NOT NULL DEFAULT 0
  CHECK(ai_context_pinned IN (0,1));
CREATE INDEX documents_ai_context
ON documents(ai_context_pinned, ai_context_excluded, updated_at, id);
CREATE TRIGGER documents_ai_context_insert
BEFORE INSERT ON documents
WHEN new.ai_context_excluded=1 AND new.ai_context_pinned=1
BEGIN
  SELECT RAISE(ABORT, 'invalid ai context policy');
END;
CREATE TRIGGER documents_ai_context_update
BEFORE UPDATE OF ai_context_excluded,ai_context_pinned ON documents
WHEN new.ai_context_excluded=1 AND new.ai_context_pinned=1
BEGIN
  SELECT RAISE(ABORT, 'invalid ai context policy');
END;
UPDATE project SET schema_version=5;
PRAGMA user_version=5;
CREATE TABLE context_index_documents (
  document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
  source_revision INTEGER NOT NULL CHECK(source_revision >= 0),
  content_hash TEXT NOT NULL,
  indexed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE context_chunks (
  document_id TEXT NOT NULL REFERENCES context_index_documents(document_id) ON DELETE CASCADE,
  chunk_id TEXT NOT NULL,
  source_revision INTEGER NOT NULL CHECK(source_revision >= 0),
  ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
  byte_start INTEGER NOT NULL CHECK(byte_start >= 0),
  byte_end INTEGER NOT NULL CHECK(byte_end > byte_start),
  content_hash TEXT NOT NULL,
  token_estimate INTEGER NOT NULL CHECK(token_estimate >= 0),
  content TEXT NOT NULL,
  PRIMARY KEY(document_id, chunk_id),
  UNIQUE(document_id, ordinal)
);
CREATE INDEX context_chunks_document
ON context_chunks(document_id, source_revision, ordinal, chunk_id);
CREATE VIRTUAL TABLE context_chunk_fts USING fts5(
  document_id UNINDEXED,
  chunk_id UNINDEXED,
  source_revision UNINDEXED,
  title,
  content,
  tokenize='unicode61'
);
CREATE TRIGGER context_index_content_invalidate
AFTER UPDATE OF content ON documents
WHEN old.content IS NOT new.content
BEGIN
  DELETE FROM context_chunk_fts WHERE document_id=new.id;
END;
CREATE TRIGGER context_index_metadata_update
AFTER UPDATE ON documents
WHEN old.content IS new.content
 AND new.archived_at IS NULL
 AND new.kind IN ('scene','note')
 AND new.ai_context_excluded=0
BEGIN
  UPDATE context_index_documents
  SET source_revision=new.revision
  WHERE document_id=new.id AND source_revision=old.revision;
  UPDATE context_chunks
  SET source_revision=new.revision
  WHERE document_id=new.id AND source_revision=old.revision;
  UPDATE context_chunk_fts
  SET source_revision=new.revision,title=new.title
  WHERE document_id=new.id AND source_revision=old.revision;
END;
CREATE TRIGGER context_index_ineligible_purge
AFTER UPDATE OF archived_at,ai_context_excluded,kind ON documents
WHEN new.archived_at IS NOT NULL
  OR new.kind='folder'
  OR new.ai_context_excluded=1
BEGIN
  DELETE FROM context_chunk_fts WHERE document_id=new.id;
  DELETE FROM context_index_documents WHERE document_id=new.id;
END;
CREATE TRIGGER context_index_document_delete
BEFORE DELETE ON documents
BEGIN
  DELETE FROM context_chunk_fts WHERE document_id=old.id;
END;
UPDATE project SET schema_version=6;
PRAGMA user_version=6;
