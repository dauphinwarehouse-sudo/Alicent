ALTER TABLE project ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;
CREATE TABLE checkpoints (
  id TEXT PRIMARY KEY, name TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE checkpoint_documents (
  checkpoint_id TEXT NOT NULL REFERENCES checkpoints(id),
  document_id TEXT NOT NULL, revision INTEGER NOT NULL,
  PRIMARY KEY(checkpoint_id, document_id),
  FOREIGN KEY(document_id, revision) REFERENCES versions(document_id, revision)
);
CREATE TRIGGER project_rev_insert AFTER INSERT ON documents BEGIN
  UPDATE project SET revision=revision+1;
END;
CREATE TRIGGER project_rev_update AFTER UPDATE ON documents BEGIN
  UPDATE project SET revision=revision+1;
END;
UPDATE project SET schema_version=2;
PRAGMA user_version=2;
