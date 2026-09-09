ALTER TABLE documents ADD COLUMN archived_at TEXT;
ALTER TABLE documents ADD COLUMN archive_root_id TEXT REFERENCES documents(id);
CREATE INDEX documents_archive ON documents(archived_at, archive_root_id, updated_at, id);
CREATE TABLE archive_journal (
  command_id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL REFERENCES documents(id),
  action TEXT NOT NULL CHECK(action IN ('archive','restore')),
  affected_count INTEGER NOT NULL CHECK(affected_count > 0),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
UPDATE project SET schema_version=3;
PRAGMA user_version=3;