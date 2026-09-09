ALTER TABLE documents ADD COLUMN order_key INTEGER NOT NULL DEFAULT 0;
WITH ranked(id, ordinal) AS (
  SELECT id, row_number() OVER (
    PARTITION BY parent_id
    ORDER BY kind='folder' DESC, title, id
  )
  FROM documents
)
UPDATE documents
SET order_key = (SELECT ordinal * 1024 FROM ranked WHERE ranked.id=documents.id);
CREATE INDEX documents_order ON documents(parent_id, order_key, id);
UPDATE project SET schema_version=4;
PRAGMA user_version=4;
