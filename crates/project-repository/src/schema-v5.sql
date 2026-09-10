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