use super::*;
use serde::{de::DeserializeOwned, Serialize};

const CAUTIOUS: &str = "00000000-0000-0000-0000-000000000001";
const TASK_SELECT: &str = "SELECT id,project_id,agent_id,profile_id,status,prompt,max_steps,max_tool_calls,max_input_bytes,max_output_bytes,max_cost_microusd,deadline_at,used_steps,used_tool_calls,used_input_bytes,used_output_bytes,used_cost_microusd,failure_code,attempt,created_at,updated_at FROM agent_tasks";

fn init(conn: &Connection, now: i64) -> Result<()> {
    conn.execute_batch(include_str!("agent-queue-schema.sql"))?;
    for p in [
        ExecutionProfile {
            id: CAUTIOUS.into(),
            name: "Осторожный".into(),
            max_steps: 8,
            max_tool_calls: 4,
            max_input_bytes: 262_144,
            max_output_bytes: 262_144,
            max_cost_microusd: 500_000,
            max_runtime_secs: 900,
            lease_secs: 30,
        },
        ExecutionProfile {
            id: "00000000-0000-0000-0000-000000000002".into(),
            name: "Стандартный".into(),
            max_steps: 24,
            max_tool_calls: 12,
            max_input_bytes: 1_048_576,
            max_output_bytes: 1_048_576,
            max_cost_microusd: 2_000_000,
            max_runtime_secs: 1_800,
            lease_secs: 30,
        },
        ExecutionProfile {
            id: "00000000-0000-0000-0000-000000000003".into(),
            name: "Расширенный".into(),
            max_steps: 64,
            max_tool_calls: 32,
            max_input_bytes: 4_194_304,
            max_output_bytes: 4_194_304,
            max_cost_microusd: 8_000_000,
            max_runtime_secs: 3_600,
            lease_secs: 60,
        },
    ] {
        conn.execute("INSERT OR IGNORE INTO execution_profiles(id,name,max_steps,max_tool_calls,max_input_bytes,max_output_bytes,max_cost_microusd,max_runtime_secs,lease_secs,is_builtin,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,1,?10)", params![p.id,p.name,p.max_steps,p.max_tool_calls,p.max_input_bytes,p.max_output_bytes,p.max_cost_microusd,p.max_runtime_secs,p.lease_secs,now])?;
    }
    Ok(())
}
fn profile(row: &Row<'_>) -> rusqlite::Result<ExecutionProfile> {
    Ok(ExecutionProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        max_steps: row.get(2)?,
        max_tool_calls: row.get(3)?,
        max_input_bytes: row.get(4)?,
        max_output_bytes: row.get(5)?,
        max_cost_microusd: row.get(6)?,
        max_runtime_secs: row.get(7)?,
        lease_secs: row.get(8)?,
    })
}
fn status(value: &str) -> rusqlite::Result<TaskStatus> {
    match value {
        "queued" => Ok(TaskStatus::Queued),
        "planning" => Ok(TaskStatus::Planning),
        "waiting_for_approval" => Ok(TaskStatus::WaitingForApproval),
        "running" => Ok(TaskStatus::Running),
        "paused" => Ok(TaskStatus::Paused),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn task_row(row: &Row<'_>) -> rusqlite::Result<AgentTask> {
    Ok(AgentTask {
        id: uuid_column(row, 0)?,
        project_id: uuid_column(row, 1)?,
        agent_id: uuid_column(row, 2)?,
        profile_id: row.get(3)?,
        status: status(&row.get::<_, String>(4)?)?,
        prompt: row.get(5)?,
        budget: TaskBudget {
            max_steps: row.get(6)?,
            max_tool_calls: row.get(7)?,
            max_input_bytes: row.get(8)?,
            max_output_bytes: row.get(9)?,
            max_cost_microusd: row.get(10)?,
            deadline_at: row.get(11)?,
        },
        usage: TaskUsage {
            steps: row.get(12)?,
            tool_calls: row.get(13)?,
            input_bytes: row.get(14)?,
            output_bytes: row.get(15)?,
            cost_microusd: row.get(16)?,
        },
        failure_code: row.get(17)?,
        attempt: row.get(18)?,
        created_at: row.get(19)?,
        updated_at: row.get(20)?,
    })
}
fn get_task(tx: &rusqlite::Transaction<'_>, id: Uuid) -> Result<AgentTask> {
    let sql = format!("{TASK_SELECT} WHERE id=?1");
    tx.query_row(&sql, [id.to_string()], task_row)
        .optional()?
        .ok_or(Error::NotFound)
}
fn cmd_hash<T: Serialize>(kind: &str, value: &T) -> Result<String> {
    Ok(hash(&format!("{kind}:{}", serde_json::to_string(value)?)))
}
fn receipt<T: DeserializeOwned>(
    tx: &rusqlite::Transaction<'_>,
    id: Uuid,
    payload: &str,
) -> Result<Option<T>> {
    let old: Option<(String, String)> = tx
        .query_row(
            "SELECT payload_hash,result_json FROM queue_command_receipts WHERE command_id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match old {
        Some((h, _)) if h != payload => Err(Error::CommandMismatch),
        Some((_, v)) => Ok(Some(serde_json::from_str(&v)?)),
        None => Ok(None),
    }
}
fn save_receipt<T: Serialize>(
    tx: &rusqlite::Transaction<'_>,
    id: Uuid,
    payload: &str,
    value: &T,
) -> Result<()> {
    tx.execute(
        "INSERT INTO queue_command_receipts(command_id,payload_hash,result_json) VALUES(?1,?2,?3)",
        params![id.to_string(), payload, serde_json::to_string(value)?],
    )?;
    Ok(())
}
fn audit(
    tx: &rusqlite::Transaction<'_>,
    id: Uuid,
    event: &str,
    now: i64,
    detail: Option<&str>,
) -> Result<()> {
    tx.execute(
        "INSERT INTO task_audit(task_id,event,at_epoch_secs,detail) VALUES(?1,?2,?3,?4)",
        params![id.to_string(), event, now, detail],
    )?;
    Ok(())
}
fn recover(tx: &rusqlite::Transaction<'_>, now: i64) -> Result<()> {
    let ids = {
        let mut s=tx.prepare("SELECT id FROM agent_tasks WHERE status IN ('planning','waiting_for_approval','running') AND COALESCE(lease_expires_at,0)<=?1")?;
        s.query_map([now], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for raw in ids {
        let uncertain: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_dispatches WHERE task_id=?1 AND state='dispatched')",
            [&raw],
            |r| r.get(0),
        )?;
        let reason = if uncertain {
            "uncertain_tool_effect"
        } else {
            "lease_expired"
        };
        tx.execute("UPDATE agent_tasks SET status='paused',failure_code=?1,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?2 WHERE id=?3",params![reason,now,raw])?;
        audit(
            tx,
            Uuid::parse_str(&raw).map_err(|_| Error::Integrity)?,
            "recovered_paused",
            now,
            Some(reason),
        )?;
    }
    tx.execute("UPDATE agent_tasks SET status='failed',failure_code='deadline_exhausted',lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE status NOT IN ('completed','failed','cancelled') AND deadline_at<=?1",[now])?;
    Ok(())
}
fn live(tx: &rusqlite::Transaction<'_>, id: Uuid, token: &str, now: i64) -> Result<TaskStatus> {
    let (s, t, e): (String, Option<String>, Option<i64>) = tx
        .query_row(
            "SELECT status,lease_token,lease_expires_at FROM agent_tasks WHERE id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    if t.as_deref() != Some(token) || e.is_none_or(|x| x <= now) {
        return Err(Error::Conflict);
    }
    status(&s).map_err(Into::into)
}
fn valid_code(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl Repository {
    pub fn agent_execution_profiles(&mut self, now: i64) -> Result<Vec<ExecutionProfile>> {
        init(&self.conn, now)?;
        let mut s=self.conn.prepare("SELECT id,name,max_steps,max_tool_calls,max_input_bytes,max_output_bytes,max_cost_microusd,max_runtime_secs,lease_secs FROM execution_profiles ORDER BY max_steps,id")?;
        Ok(s.query_map([], profile)?
            .collect::<std::result::Result<_, _>>()?)
    }
    pub fn agent_custom_agents(&mut self, now: i64) -> Result<Vec<CustomAgent>> {
        init(&self.conn, now)?;
        let mut s=self.conn.prepare("SELECT id,name,instructions,profile_id,created_at FROM custom_agents ORDER BY created_at,id")?;
        Ok(s.query_map([], |r| {
            Ok(CustomAgent {
                id: uuid_column(r, 0)?,
                name: r.get(1)?,
                instructions: r.get(2)?,
                profile_id: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?)
    }
    pub fn agent_create(&mut self, c: CreateAgentCommand, now: i64) -> Result<CustomAgent> {
        init(&self.conn, now)?;
        validate_title(&c.name)?;
        if c.instructions.trim().is_empty()
            || c.instructions.len() > 65_536
            || c.instructions.contains('\0')
        {
            return Err(Error::Integrity);
        }
        let h = cmd_hash("agent_create", &c)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(v) = receipt(&tx, c.command_id, &h)? {
            return Ok(v);
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_profiles WHERE id=?1)",
            [&c.profile_id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        let v = CustomAgent {
            id: c.agent_id,
            name: c.name.trim().into(),
            instructions: c.instructions.trim().into(),
            profile_id: c.profile_id,
            created_at: now,
        };
        tx.execute("INSERT INTO custom_agents(id,name,instructions,profile_id,created_at) VALUES(?1,?2,?3,?4,?5)",params![v.id.to_string(),v.name,v.instructions,v.profile_id,v.created_at])?;
        save_receipt(&tx, c.command_id, &h, &v)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_enqueue(&mut self, c: EnqueueAgentTask, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        if c.prompt.trim().is_empty() || c.prompt.len() > 32_768 || c.prompt.contains('\0') {
            return Err(Error::Integrity);
        }
        let h = cmd_hash("agent_enqueue", &c)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(v) = receipt(&tx, c.command_id, &h)? {
            return Ok(v);
        }
        let p=tx.query_row("SELECT p.id,p.name,p.max_steps,p.max_tool_calls,p.max_input_bytes,p.max_output_bytes,p.max_cost_microusd,p.max_runtime_secs,p.lease_secs FROM custom_agents a JOIN execution_profiles p ON p.id=a.profile_id WHERE a.id=?1",[c.agent_id.to_string()],profile).optional()?.ok_or(Error::NotFound)?;
        let project: Uuid = tx.query_row("SELECT id FROM project", [], |r| uuid_column(r, 0))?;
        let deadline = now
            .checked_add(p.max_runtime_secs)
            .ok_or(Error::Integrity)?;
        tx.execute("INSERT INTO agent_tasks(id,project_id,agent_id,profile_id,enqueue_key,request_hash,prompt,status,max_steps,max_tool_calls,max_input_bytes,max_output_bytes,max_cost_microusd,lease_secs,deadline_at,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8,?9,?10,?11,?12,?13,?14,?15,?15)",params![c.task_id.to_string(),project.to_string(),c.agent_id.to_string(),p.id,c.command_id.to_string(),h,c.prompt.trim(),p.max_steps,p.max_tool_calls,p.max_input_bytes,p.max_output_bytes,p.max_cost_microusd,p.lease_secs,deadline,now])?;
        audit(&tx, c.task_id, "enqueued", now, Some("profile_snapshot"))?;
        let v = get_task(&tx, c.task_id)?;
        save_receipt(&tx, c.command_id, &h, &v)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_tasks(&mut self, now: i64) -> Result<Vec<AgentTask>> {
        init(&self.conn, now)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        let sql = format!("{TASK_SELECT} ORDER BY updated_at DESC,id LIMIT 200");
        let v = {
            let mut s = tx.prepare(&sql)?;
            s.query_map([], task_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_cancel(&mut self, c: AgentTaskCommand, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let h = cmd_hash("agent_cancel", &c)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(v) = receipt(&tx, c.command_id, &h)? {
            return Ok(v);
        }
        let current = get_task(&tx, c.task_id)?;
        if matches!(current.status, TaskStatus::Completed | TaskStatus::Failed) {
            return Err(Error::Domain(DomainError::InvalidTransition));
        }
        if current.status != TaskStatus::Cancelled {
            current.status.transition(TaskStatus::Cancelled)?;
            tx.execute("UPDATE agent_tasks SET status='cancelled',cancel_requested=1,failure_code=NULL,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE id=?2",params![now,c.task_id.to_string()])?;
            audit(&tx, c.task_id, "cancelled", now, None)?;
        }
        let v = get_task(&tx, c.task_id)?;
        save_receipt(&tx, c.command_id, &h, &v)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_resume(&mut self, c: AgentTaskCommand, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let h = cmd_hash("agent_resume", &c)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(v) = receipt(&tx, c.command_id, &h)? {
            return Ok(v);
        }
        let current = get_task(&tx, c.task_id)?;
        if current.status != TaskStatus::Paused {
            return Err(Error::Domain(DomainError::InvalidTransition));
        }
        let uncertain: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_dispatches WHERE task_id=?1 AND state='dispatched')",
            [c.task_id.to_string()],
            |r| r.get(0),
        )?;
        if uncertain {
            return Err(Error::Conflict);
        }
        current.status.transition(TaskStatus::Queued)?;
        tx.execute("UPDATE agent_tasks SET status='queued',failure_code=NULL,cancel_requested=0,updated_at=?1 WHERE id=?2",params![now,c.task_id.to_string()])?;
        audit(&tx, c.task_id, "resumed", now, None)?;
        let v = get_task(&tx, c.task_id)?;
        save_receipt(&tx, c.command_id, &h, &v)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_claim(
        &mut self,
        worker: &str,
        claim_id: Uuid,
        now: i64,
    ) -> Result<Option<AgentTaskClaim>> {
        init(&self.conn, now)?;
        if worker.is_empty() || worker.len() > 128 || worker.contains('\0') {
            return Err(Error::Integrity);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        let old:Option<(String,Option<String>,Option<String>,Option<i64>)>=tx.query_row("SELECT worker_id,task_id,lease_token,lease_expires_at FROM queue_claim_receipts WHERE claim_id=?1",[claim_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((w, id, t, e)) = old {
            if w != worker {
                return Err(Error::CommandMismatch);
            }
            return match (id, t, e) {
                (Some(id), Some(lease_token), Some(lease_expires_at)) => Ok(Some(AgentTaskClaim {
                    task: get_task(&tx, Uuid::parse_str(&id).map_err(|_| Error::Integrity)?)?,
                    lease_token,
                    lease_expires_at,
                })),
                (None, None, None) => Ok(None),
                _ => Err(Error::Integrity),
            };
        }
        let id:Option<String>=tx.query_row("SELECT id FROM agent_tasks WHERE status='queued' AND deadline_at>?1 ORDER BY created_at,id LIMIT 1",[now],|r|r.get(0)).optional()?;
        let Some(id) = id else {
            tx.execute(
                "INSERT INTO queue_claim_receipts(claim_id,worker_id,created_at) VALUES(?1,?2,?3)",
                params![claim_id.to_string(), worker, now],
            )?;
            tx.commit()?;
            return Ok(None);
        };
        let lease: i64 = tx.query_row(
            "SELECT lease_secs FROM agent_tasks WHERE id=?1",
            [&id],
            |r| r.get(0),
        )?;
        let expires = now.checked_add(lease).ok_or(Error::Integrity)?;
        let token = Uuid::new_v4().to_string();
        tx.execute("UPDATE agent_tasks SET status='planning',lease_owner=?1,lease_token=?2,lease_expires_at=?3,attempt=attempt+1,failure_code=NULL,updated_at=?4 WHERE id=?5",params![worker,token,expires,now,id])?;
        tx.execute("INSERT INTO queue_claim_receipts(claim_id,worker_id,task_id,lease_token,lease_expires_at,created_at) VALUES(?1,?2,?3,?4,?5,?6)",params![claim_id.to_string(),worker,id,token,expires,now])?;
        let task_id = Uuid::parse_str(&id).map_err(|_| Error::Integrity)?;
        audit(&tx, task_id, "claimed", now, Some("lease_acquired"))?;
        let v = AgentTaskClaim {
            task: get_task(&tx, task_id)?,
            lease_token: token,
            lease_expires_at: expires,
        };
        tx.commit()?;
        Ok(Some(v))
    }
    pub fn agent_start(&mut self, id: Uuid, token: &str, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        let s = live(&tx, id, token, now)?;
        if s != TaskStatus::Running {
            s.transition(TaskStatus::Running)?;
            tx.execute(
                "UPDATE agent_tasks SET status='running',updated_at=?1 WHERE id=?2",
                params![now, id.to_string()],
            )?;
            audit(&tx, id, "running", now, None)?;
        }
        let v = get_task(&tx, id)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_pause(
        &mut self,
        id: Uuid,
        token: &str,
        reason: &str,
        now: i64,
    ) -> Result<AgentTask> {
        init(&self.conn, now)?;
        if !valid_code(reason) {
            return Err(Error::Integrity);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        live(&tx, id, token, now)?.transition(TaskStatus::Paused)?;
        tx.execute("UPDATE agent_tasks SET status='paused',failure_code=?1,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?2 WHERE id=?3",params![reason,now,id.to_string()])?;
        audit(&tx, id, "paused", now, Some(reason))?;
        let v = get_task(&tx, id)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_record_step(&mut self, id: Uuid, token: &str, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        live(&tx, id, token, now)?;
        let current = get_task(&tx, id)?;
        if current.usage.steps >= current.budget.max_steps {
            tx.execute("UPDATE agent_tasks SET status='failed',failure_code='budget_steps',lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE id=?2",params![now,id.to_string()])?;
            audit(&tx, id, "budget_exhausted", now, Some("steps"))?;
        } else {
            tx.execute(
                "UPDATE agent_tasks SET used_steps=used_steps+1,updated_at=?1 WHERE id=?2",
                params![now, id.to_string()],
            )?;
            audit(&tx, id, "step_recorded", now, None)?;
        }
        let v = get_task(&tx, id)?;
        tx.commit()?;
        Ok(v)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn agent_begin_tool_dispatch(
        &mut self,
        id: Uuid,
        token: &str,
        call: &str,
        payload: &str,
        input: i64,
        cost: i64,
        now: i64,
    ) -> Result<ToolDispatchDecision> {
        init(&self.conn, now)?;
        if call.is_empty()
            || call.len() > 160
            || payload.len() != 64
            || !payload.bytes().all(|b| b.is_ascii_hexdigit())
            || input < 0
            || cost < 0
        {
            return Err(Error::Integrity);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        if live(&tx, id, token, now)? != TaskStatus::Running {
            return Err(Error::Domain(DomainError::InvalidTransition));
        }
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT payload_hash,state FROM tool_dispatches WHERE task_id=?1 AND call_id=?2",
                params![id.to_string(), call],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((h, s)) = &old {
            if h != payload {
                return Err(Error::CommandMismatch);
            }
            if s != "not_applied" {
                audit(&tx, id, "tool_suppressed", now, Some(s))?;
                tx.commit()?;
                return Ok(ToolDispatchDecision {
                    execute: false,
                    reason: Some(format!("already_{s}")),
                });
            }
        }
        let t = get_task(&tx, id)?;
        let reason = if t.usage.tool_calls >= t.budget.max_tool_calls {
            Some("budget_tool_calls")
        } else if t.usage.input_bytes.saturating_add(input) > t.budget.max_input_bytes {
            Some("budget_input_bytes")
        } else if t.usage.cost_microusd.saturating_add(cost) > t.budget.max_cost_microusd {
            Some("budget_cost")
        } else {
            None
        };
        if let Some(reason) = reason {
            tx.execute("UPDATE agent_tasks SET status='failed',failure_code=?1,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?2 WHERE id=?3",params![reason,now,id.to_string()])?;
            audit(&tx, id, "budget_exhausted", now, Some(reason))?;
            tx.commit()?;
            return Ok(ToolDispatchDecision {
                execute: false,
                reason: Some(reason.into()),
            });
        }
        if old.is_some() {
            tx.execute("UPDATE tool_dispatches SET state='dispatched',dispatch_lease_token=?1,side_effects_applied=0,output_hash=NULL,error_code=NULL,updated_at=?2 WHERE task_id=?3 AND call_id=?4",params![token,now,id.to_string(),call])?;
        } else {
            tx.execute("INSERT INTO tool_dispatches(task_id,call_id,payload_hash,state,dispatch_lease_token,created_at,updated_at) VALUES(?1,?2,?3,'dispatched',?4,?5,?5)",params![id.to_string(),call,payload,token,now])?;
        }
        tx.execute("UPDATE agent_tasks SET used_tool_calls=used_tool_calls+1,used_input_bytes=used_input_bytes+?1,used_cost_microusd=used_cost_microusd+?2,updated_at=?3 WHERE id=?4",params![input,cost,now,id.to_string()])?;
        audit(&tx, id, "tool_dispatched", now, None)?;
        tx.commit()?;
        Ok(ToolDispatchDecision {
            execute: true,
            reason: None,
        })
    }
    pub fn agent_finish_tool_dispatch(
        &mut self,
        id: Uuid,
        token: &str,
        call: &str,
        output: i64,
        succeeded: bool,
        now: i64,
    ) -> Result<AgentTask> {
        init(&self.conn, now)?;
        if output < 0 {
            return Err(Error::Integrity);
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (state,owner):(String,String)=tx.query_row("SELECT state,dispatch_lease_token FROM tool_dispatches WHERE task_id=?1 AND call_id=?2",params![id.to_string(),call],|r|Ok((r.get(0)?,r.get(1)?))).optional()?.ok_or(Error::NotFound)?;
        if state != "dispatched" {
            return get_task(&tx, id);
        }
        if owner != token {
            return Err(Error::Conflict);
        }
        let t = get_task(&tx, id)?;
        let output_over = t.usage.output_bytes.saturating_add(output) > t.budget.max_output_bytes;
        let failed = !succeeded || output_over;
        let code = if output_over {
            "budget_output_bytes"
        } else if !succeeded {
            "executor_failed"
        } else {
            "succeeded"
        };
        tx.execute("UPDATE tool_dispatches SET state=?1,side_effects_applied=1,error_code=?2,updated_at=?3 WHERE task_id=?4 AND call_id=?5",params![if failed{"failed"}else{"succeeded"},if failed{Some(code)}else{None},now,id.to_string(),call])?;
        if output_over || !succeeded {
            tx.execute("UPDATE agent_tasks SET status='failed',failure_code=?1,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?2 WHERE id=?3",params![code,now,id.to_string()])?;
        } else {
            tx.execute("UPDATE agent_tasks SET used_output_bytes=used_output_bytes+?1,updated_at=?2 WHERE id=?3",params![output,now,id.to_string()])?;
        }
        audit(&tx, id, "tool_completed", now, Some(code))?;
        let v = get_task(&tx, id)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_resolve_tool_effect(
        &mut self,
        c: AgentTaskCommand,
        call: &str,
        applied: bool,
        now: i64,
    ) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let h = cmd_hash("resolve_tool", &(c.clone(), call, applied))?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(v) = receipt(&tx, c.command_id, &h)? {
            return Ok(v);
        }
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM tool_dispatches WHERE task_id=?1 AND call_id=?2",
                params![c.task_id.to_string(), call],
                |r| r.get(0),
            )
            .optional()?;
        if state.as_deref() != Some("dispatched") {
            return Err(Error::Conflict);
        }
        tx.execute("UPDATE tool_dispatches SET state=?1,side_effects_applied=?2,error_code='operator_resolved',updated_at=?3 WHERE task_id=?4 AND call_id=?5",params![if applied{"succeeded"}else{"not_applied"},applied,now,c.task_id.to_string(),call])?;
        tx.execute("UPDATE agent_tasks SET status='paused',failure_code=?1,lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?2 WHERE id=?3",params![if applied{"resolved_applied"}else{"resolved_not_applied"},now,c.task_id.to_string()])?;
        audit(
            &tx,
            c.task_id,
            "tool_resolved",
            now,
            Some(if applied { "applied" } else { "not_applied" }),
        )?;
        let v = get_task(&tx, c.task_id)?;
        save_receipt(&tx, c.command_id, &h, &v)?;
        tx.commit()?;
        Ok(v)
    }
    pub fn agent_complete(&mut self, id: Uuid, token: &str, now: i64) -> Result<AgentTask> {
        init(&self.conn, now)?;
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        recover(&tx, now)?;
        live(&tx, id, token, now)?.transition(TaskStatus::Completed)?;
        let uncertain: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tool_dispatches WHERE task_id=?1 AND state='dispatched')",
            [id.to_string()],
            |r| r.get(0),
        )?;
        if uncertain {
            return Err(Error::Conflict);
        }
        tx.execute("UPDATE agent_tasks SET status='completed',lease_owner=NULL,lease_token=NULL,lease_expires_at=NULL,updated_at=?1 WHERE id=?2",params![now,id.to_string()])?;
        audit(&tx, id, "completed", now, None)?;
        let v = get_task(&tx, id)?;
        tx.commit()?;
        Ok(v)
    }
    #[cfg(test)]
    fn agent_audit_count(&self, id: Uuid) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM task_audit WHERE task_id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    fn setup() -> (tempfile::TempDir, Repository, CustomAgent) {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = Repository::create(dir.path(), "Queue test").unwrap();
        let command = CreateAgentCommand {
            command_id: Uuid::new_v4(),
            agent_id: Uuid::new_v4(),
            name: "Редактор".into(),
            instructions: "Проверяй текст безопасно".into(),
            profile_id: CAUTIOUS.into(),
        };
        let agent = repo.agent_create(command, 1).unwrap();
        (dir, repo, agent)
    }
    fn enqueue(repo: &mut Repository, agent: Uuid, now: i64) -> (EnqueueAgentTask, AgentTask) {
        let command = EnqueueAgentTask {
            command_id: Uuid::new_v4(),
            task_id: Uuid::new_v4(),
            agent_id: agent,
            prompt: "Проверь главу".into(),
        };
        let task = repo.agent_enqueue(command.clone(), now).unwrap();
        (command, task)
    }
    #[test]
    fn enqueue_and_claim_are_idempotent() {
        let (_dir, mut repo, agent) = setup();
        let (command, task) = enqueue(&mut repo, agent.id, 2);
        assert_eq!(repo.agent_enqueue(command, 3).unwrap().id, task.id);
        let claim_id = Uuid::new_v4();
        let first = repo.agent_claim("worker-a", claim_id, 4).unwrap().unwrap();
        let repeated = repo.agent_claim("worker-a", claim_id, 5).unwrap().unwrap();
        assert_eq!(first.lease_token, repeated.lease_token);
        assert_eq!(first.task.id, repeated.task.id);
    }
    #[test]
    fn crash_reopen_never_redispatches_an_uncertain_effect() {
        let (_dir, mut repo, agent) = setup();
        let root = repo.root().to_path_buf();
        let (_, task) = enqueue(&mut repo, agent.id, 2);
        let claim = repo
            .agent_claim("worker-a", Uuid::new_v4(), 3)
            .unwrap()
            .unwrap();
        repo.agent_start(task.id, &claim.lease_token, 4).unwrap();
        assert!(
            repo.agent_begin_tool_dispatch(
                task.id,
                &claim.lease_token,
                "call-1",
                &"a".repeat(64),
                1,
                1,
                5
            )
            .unwrap()
            .execute
        );
        drop(repo);
        let mut reopened = Repository::open(&root).unwrap();
        let recovered = reopened
            .agent_tasks(claim.lease_expires_at + 1)
            .unwrap()
            .remove(0);
        assert_eq!(recovered.status, TaskStatus::Paused);
        assert_eq!(
            recovered.failure_code.as_deref(),
            Some("uncertain_tool_effect")
        );
        assert!(reopened
            .agent_resume(
                AgentTaskCommand {
                    command_id: Uuid::new_v4(),
                    task_id: task.id
                },
                claim.lease_expires_at + 2
            )
            .is_err());
        reopened
            .agent_resolve_tool_effect(
                AgentTaskCommand {
                    command_id: Uuid::new_v4(),
                    task_id: task.id,
                },
                "call-1",
                true,
                claim.lease_expires_at + 3,
            )
            .unwrap();
        reopened
            .agent_resume(
                AgentTaskCommand {
                    command_id: Uuid::new_v4(),
                    task_id: task.id,
                },
                claim.lease_expires_at + 4,
            )
            .unwrap();
        let next = reopened
            .agent_claim("worker-b", Uuid::new_v4(), claim.lease_expires_at + 5)
            .unwrap()
            .unwrap();
        reopened
            .agent_start(task.id, &next.lease_token, claim.lease_expires_at + 6)
            .unwrap();
        let replay = reopened
            .agent_begin_tool_dispatch(
                task.id,
                &next.lease_token,
                "call-1",
                &"a".repeat(64),
                1,
                1,
                claim.lease_expires_at + 7,
            )
            .unwrap();
        assert!(!replay.execute);
        assert_eq!(replay.reason.as_deref(), Some("already_succeeded"));
    }
    #[test]
    fn cancellation_and_resume_persist() {
        let (_dir, mut repo, agent) = setup();
        let root = repo.root().to_path_buf();
        let (_, cancelled) = enqueue(&mut repo, agent.id, 2);
        let command = AgentTaskCommand {
            command_id: Uuid::new_v4(),
            task_id: cancelled.id,
        };
        assert_eq!(
            repo.agent_cancel(command.clone(), 3).unwrap().status,
            TaskStatus::Cancelled
        );
        assert_eq!(
            repo.agent_cancel(command, 4).unwrap().status,
            TaskStatus::Cancelled
        );
        let (_, pausable) = enqueue(&mut repo, agent.id, 5);
        let claim = repo
            .agent_claim("worker", Uuid::new_v4(), 6)
            .unwrap()
            .unwrap();
        repo.agent_start(pausable.id, &claim.lease_token, 7)
            .unwrap();
        repo.agent_pause(pausable.id, &claim.lease_token, "operator_pause", 8)
            .unwrap();
        assert_eq!(
            repo.agent_resume(
                AgentTaskCommand {
                    command_id: Uuid::new_v4(),
                    task_id: pausable.id
                },
                9
            )
            .unwrap()
            .status,
            TaskStatus::Queued
        );
        drop(repo);
        let mut reopened = Repository::open(&root).unwrap();
        let states = reopened.agent_tasks(10).unwrap();
        assert!(states
            .iter()
            .any(|t| t.id == cancelled.id && t.status == TaskStatus::Cancelled));
        assert!(states
            .iter()
            .any(|t| t.id == pausable.id && t.status == TaskStatus::Queued));
    }
    #[test]
    fn persisted_step_budget_stops_the_task() {
        let (_dir, mut repo, agent) = setup();
        let (_, task) = enqueue(&mut repo, agent.id, 2);
        let claim = repo
            .agent_claim("worker", Uuid::new_v4(), 3)
            .unwrap()
            .unwrap();
        repo.agent_start(task.id, &claim.lease_token, 4).unwrap();
        for step in 0..task.budget.max_steps {
            let current = repo
                .agent_record_step(task.id, &claim.lease_token, 5 + step)
                .unwrap();
            assert_eq!(current.status, TaskStatus::Running);
        }
        let exhausted = repo
            .agent_record_step(task.id, &claim.lease_token, 20)
            .unwrap();
        assert_eq!(exhausted.status, TaskStatus::Failed);
        assert_eq!(exhausted.failure_code.as_deref(), Some("budget_steps"));
        assert!(repo.agent_audit_count(task.id).unwrap() > task.budget.max_steps);
    }
    #[test]
    fn two_concurrent_agents_claim_without_lost_writes() {
        let (_dir, mut repo, agent) = setup();
        let root = repo.root().to_path_buf();
        let (_, one) = enqueue(&mut repo, agent.id, 2);
        let (_, two) = enqueue(&mut repo, agent.id, 3);
        drop(repo);
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for worker in ["one", "two"] {
            let root = root.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let mut repo = Repository::open(&root).unwrap();
                barrier.wait();
                repo.agent_claim(worker, Uuid::new_v4(), 4)
                    .unwrap()
                    .unwrap()
                    .task
                    .id
            }));
        }
        let a = handles.remove(0).join().unwrap();
        let b = handles.remove(0).join().unwrap();
        assert_ne!(a, b);
        assert!([one.id, two.id].contains(&a));
        assert!([one.id, two.id].contains(&b));
        let mut repo = Repository::open(&root).unwrap();
        let tasks = repo.agent_tasks(5).unwrap();
        assert_eq!(
            tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Planning)
                .count(),
            2
        );
    }
}
