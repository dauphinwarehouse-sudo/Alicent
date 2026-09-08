CREATE TABLE project (
  id TEXT PRIMARY KEY, title TEXT NOT NULL, schema_version INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE documents (
  id TEXT PRIMARY KEY, parent_id TEXT REFERENCES documents(id), title TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('folder','scene','note')),
  content TEXT NOT NULL DEFAULT '', revision INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX documents_parent ON documents(parent_id, title, id);
CREATE TABLE versions (
  document_id TEXT NOT NULL REFERENCES documents(id), revision INTEGER NOT NULL,
  content TEXT NOT NULL, actor TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY(document_id, revision)
);
CREATE TABLE operations (
  id TEXT PRIMARY KEY, document_id TEXT NOT NULL REFERENCES documents(id),
  protocol_version INTEGER NOT NULL DEFAULT 1, actor TEXT NOT NULL,
  kind TEXT NOT NULL, before_hash TEXT, after_hash TEXT NOT NULL,
  revision INTEGER NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE receipts (command_id TEXT PRIMARY KEY, payload_hash TEXT NOT NULL, result TEXT NOT NULL);
CREATE VIRTUAL TABLE document_fts USING fts5(title, content, content='documents', content_rowid='rowid', tokenize='unicode61');
CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
  INSERT INTO document_fts(rowid,title,content) VALUES(new.rowid,new.title,new.content);
END;
CREATE TRIGGER documents_au AFTER UPDATE ON documents BEGIN
  INSERT INTO document_fts(document_fts,rowid,title,content) VALUES('delete',old.rowid,old.title,old.content);
  INSERT INTO document_fts(rowid,title,content) VALUES(new.rowid,new.title,new.content);
END;
PRAGMA user_version = 1;
