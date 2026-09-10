use super::*;
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

const ORDER_STEP: i64 = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativePosition {
    First,
    Last,
    Before(Uuid),
    After(Uuid),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderedDocumentSummary {
    #[serde(flatten)]
    pub summary: DocumentSummary,
    pub order_key: i64,
}

fn parent_value(parent: Option<Uuid>) -> Option<String> {
    parent.map(|value| value.to_string())
}

fn ensure_active_folder(tx: &Transaction<'_>, parent: Option<Uuid>) -> Result<()> {
    if let Some(parent_id) = parent {
        let kind: Option<String> = tx
            .query_row(
                "SELECT kind FROM documents WHERE id=?1 AND archived_at IS NULL",
                [parent_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if kind.as_deref() != Some("folder") {
            return Err(Error::InvalidParent);
        }
    }
    Ok(())
}

fn sibling_bound(
    tx: &Transaction<'_>,
    parent: Option<Uuid>,
    moving_id: Option<Uuid>,
    aggregate: &str,
    predicate: &str,
    pivot: Option<i64>,
) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT {aggregate}(order_key) FROM documents \
         WHERE parent_id IS ?1 AND id!=COALESCE(?2,'') {predicate}"
    );
    let parent = parent_value(parent);
    let moving_id = moving_id.map(|id| id.to_string());
    let value = match pivot {
        Some(pivot) => tx.query_row(&sql, params![parent, moving_id, pivot], |row| row.get(0))?,
        None => tx.query_row(&sql, params![parent, moving_id], |row| row.get(0))?,
    };
    Ok(value)
}

fn rebalance(tx: &Transaction<'_>, parent: Option<Uuid>) -> Result<()> {
    let ids = {
        let mut stmt =
            tx.prepare("SELECT id FROM documents WHERE parent_id IS ?1 ORDER BY order_key,id")?;
        let rows = stmt.query_map([parent_value(parent)], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (index, id) in ids.iter().enumerate() {
        let temporary = -i64::try_from(index + 1).map_err(|_| Error::Integrity)?;
        tx.execute(
            "UPDATE documents SET order_key=?1 WHERE id=?2",
            params![temporary, id],
        )?;
    }
    for (index, id) in ids.iter().enumerate() {
        let ordinal = i64::try_from(index + 1).map_err(|_| Error::Integrity)?;
        let key = ordinal.checked_mul(ORDER_STEP).ok_or(Error::Integrity)?;
        tx.execute(
            "UPDATE documents SET order_key=?1 WHERE id=?2",
            params![key, id],
        )?;
    }
    Ok(())
}

fn candidate(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (None, None) => Some(ORDER_STEP),
        (None, Some(right)) if right > 1 => Some(if right > ORDER_STEP {
            right - ORDER_STEP
        } else {
            right / 2
        }),
        (Some(left), None) if left > 0 => left.checked_add(ORDER_STEP),
        (Some(left), Some(right)) if left > 0 && right > left && right - left > 1 => {
            Some(left + (right - left) / 2)
        }
        _ => None,
    }
}

fn calculate_order_key(
    tx: &Transaction<'_>,
    parent: Option<Uuid>,
    position: &RelativePosition,
    moving_id: Option<Uuid>,
    may_rebalance: bool,
) -> Result<i64> {
    let (left, right) = match position {
        RelativePosition::First => (None, sibling_bound(tx, parent, moving_id, "min", "", None)?),
        RelativePosition::Last => (sibling_bound(tx, parent, moving_id, "max", "", None)?, None),
        RelativePosition::Before(sibling_id) | RelativePosition::After(sibling_id) => {
            if Some(*sibling_id) == moving_id {
                return Err(Error::InvalidParent);
            }
            let sibling_key: i64 = tx
                .query_row(
                    "SELECT order_key FROM documents \
                     WHERE id=?1 AND parent_id IS ?2 AND archived_at IS NULL",
                    params![sibling_id.to_string(), parent_value(parent)],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or(Error::InvalidParent)?;
            match position {
                RelativePosition::Before(_) => (
                    sibling_bound(
                        tx,
                        parent,
                        moving_id,
                        "max",
                        "AND order_key<?3",
                        Some(sibling_key),
                    )?,
                    Some(sibling_key),
                ),
                RelativePosition::After(_) => (
                    Some(sibling_key),
                    sibling_bound(
                        tx,
                        parent,
                        moving_id,
                        "min",
                        "AND order_key>?3",
                        Some(sibling_key),
                    )?,
                ),
                RelativePosition::First | RelativePosition::Last => unreachable!(),
            }
        }
    };
    if let Some(key) = candidate(left, right) {
        return Ok(key);
    }
    if may_rebalance {
        rebalance(tx, parent)?;
        return calculate_order_key(tx, parent, position, moving_id, false);
    }
    Err(Error::Integrity)
}

pub(super) fn last_order_key(
    tx: &Transaction<'_>,
    parent: Option<Uuid>,
    moving_id: Option<Uuid>,
) -> Result<i64> {
    calculate_order_key(tx, parent, &RelativePosition::Last, moving_id, true)
}

impl Repository {
    pub fn create_document_at(
        &mut self,
        operation_id: Uuid,
        title: &str,
        kind: DocumentKind,
        parent: Option<Uuid>,
        position: RelativePosition,
    ) -> Result<Document> {
        validate_title(title)?;
        let title = title.trim();
        let payload_hash = hash(&format!(
            "create:{}",
            serde_json::to_string(&(title, kind.as_str(), parent, &position))?
        ));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [operation_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        ensure_active_folder(&tx, parent)?;
        let order_key = calculate_order_key(&tx, parent, &position, None, true)?;
        let id = Uuid::new_v4();
        tx.execute(
            "INSERT INTO documents(id,parent_id,title,kind,order_key) VALUES(?1,?2,?3,?4,?5)",
            params![
                id.to_string(),
                parent_value(parent),
                title,
                kind.as_str(),
                order_key
            ],
        )?;
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,0,'','user:local')",
            [id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,after_hash,revision) VALUES(?1,?2,'user:local','create',?3,0)",
            params![operation_id.to_string(), id.to_string(), hash("")],
        )?;
        let result: Document = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1",
            [id.to_string()],
            |row| {
                Ok(Document {
                    summary: summary(row)?,
                    content: row.get(8)?,
                })
            },
        )?;
        tx.execute(
            "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
            params![
                operation_id.to_string(),
                payload_hash,
                serde_json::to_string(&result)?
            ],
        )?;
        tx.commit()?;
        Ok(result)
    }

    pub fn create_document_with_operation_id(
        &mut self,
        operation_id: Uuid,
        title: &str,
        kind: DocumentKind,
        parent: Option<Uuid>,
    ) -> Result<Document> {
        self.create_document_at(operation_id, title, kind, parent, RelativePosition::Last)
    }

    pub fn list_ordered(
        &self,
        parent: Option<Uuid>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<OrderedDocumentSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidPagination);
        }
        let mut stmt = self.conn.prepare(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,order_key FROM documents WHERE parent_id IS ?1 AND archived_at IS NULL ORDER BY order_key,id LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(params![parent_value(parent), limit, offset], |row| {
            Ok(OrderedDocumentSummary {
                summary: summary(row)?,
                order_key: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn move_document_relative(
        &mut self,
        command: MoveDocument,
        position: RelativePosition,
    ) -> Result<Document> {
        let payload_hash = hash(&format!(
            "move_relative:{}:{}",
            serde_json::to_string(&command)?,
            serde_json::to_string(&position)?
        ));
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some((previous, result)) = tx
            .query_row(
                "SELECT payload_hash,result FROM receipts WHERE command_id=?1",
                [command.command_id.to_string()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
        {
            if previous != payload_hash {
                return Err(Error::CommandMismatch);
            }
            return Ok(serde_json::from_str(&result)?);
        }
        let (current, current_key): (Document, i64) = tx
            .query_row(
                "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content,order_key FROM documents WHERE id=?1 AND archived_at IS NULL",
                [command.document_id.to_string()],
                |row| Ok((Document { summary: summary(row)?, content: row.get(8)? }, row.get(9)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if current.summary.revision != command.expected_revision {
            return Err(Error::Conflict);
        }
        ensure_active_folder(&tx, command.parent_id)?;
        if let (DocumentKind::Folder, Some(parent_id)) = (&current.summary.kind, command.parent_id)
        {
            let creates_cycle: i64 = tx.query_row(
                "WITH RECURSIVE descendants(id) AS (SELECT id FROM documents WHERE id=?1 AND archived_at IS NULL UNION ALL SELECT d.id FROM documents d JOIN descendants p ON d.parent_id=p.id WHERE d.archived_at IS NULL) SELECT EXISTS(SELECT 1 FROM descendants WHERE id=?2)",
                params![command.document_id.to_string(), parent_id.to_string()],
                |row| row.get(0),
            )?;
            if creates_cycle != 0 {
                return Err(Error::InvalidParent);
            }
        }
        let order_key = calculate_order_key(
            &tx,
            command.parent_id,
            &position,
            Some(command.document_id),
            true,
        )?;
        if current.summary.parent_id == command.parent_id && current_key == order_key {
            tx.execute(
                "INSERT INTO receipts(command_id,payload_hash,result) VALUES(?1,?2,?3)",
                params![
                    command.command_id.to_string(),
                    payload_hash,
                    serde_json::to_string(&current)?
                ],
            )?;
            tx.commit()?;
            return Ok(current);
        }
        let revision = current.summary.revision + 1;
        let changed = tx.execute(
            "UPDATE documents SET parent_id=?1,order_key=?2,revision=?3,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?4 AND revision=?5 AND archived_at IS NULL",
            params![parent_value(command.parent_id), order_key, revision, command.document_id.to_string(), command.expected_revision],
        )?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.execute(
            "INSERT INTO versions(document_id,revision,content,actor) VALUES(?1,?2,?3,'user:local')",
            params![command.document_id.to_string(), revision, &current.content],
        )?;
        let before = format!("{:?}:{current_key}", current.summary.parent_id);
        let after = format!("{:?}:{order_key}", command.parent_id);
        tx.execute(
            "INSERT INTO operations(id,document_id,actor,kind,before_hash,after_hash,revision) VALUES(?1,?2,'user:local','move_relative',?3,?4,?5)",
            params![command.command_id.to_string(), command.document_id.to_string(), hash(&before), hash(&after), revision],
        )?;
        let result: Document = tx.query_row(
            "SELECT id,parent_id,title,kind,revision,updated_at,ai_context_excluded,ai_context_pinned,content FROM documents WHERE id=?1",
            [command.document_id.to_string()],
            |row| {
                Ok(Document {
                    summary: summary(row)?,
                    content: row.get(8)?,
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
}
