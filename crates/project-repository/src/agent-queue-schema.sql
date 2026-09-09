CREATE TABLE IF NOT EXISTS queue_metadata (
  singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
  project_id TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS execution_profiles (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  max_steps INTEGER NOT NULL CHECK(max_steps > 0),
  max_tool_calls INTEGER NOT NULL CHECK(max_tool_calls > 0),
  max_input_bytes INTEGER NOT NULL CHECK(max_input_bytes > 0),
  max_output_bytes INTEGER NOT NULL CHECK(max_output_bytes > 0),
  max_cost_microusd INTEGER NOT NULL CHECK(max_cost_microusd >= 0),
  max_runtime_secs INTEGER NOT NULL CHECK(max_runtime_secs > 0),
  lease_secs INTEGER NOT NULL CHECK(lease_secs BETWEEN 5 AND 300),
  is_builtin INTEGER NOT NULL CHECK(is_builtin IN (0, 1)),
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS custom_agents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  instructions TEXT NOT NULL,
  profile_id TEXT NOT NULL REFERENCES execution_profiles(id),
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS custom_agents_created
  ON custom_agents(created_at, id);

CREATE TABLE IF NOT EXISTS agent_tasks (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL,
  agent_id TEXT NOT NULL REFERENCES custom_agents(id),
  profile_id TEXT NOT NULL REFERENCES execution_profiles(id),
  enqueue_key TEXT NOT NULL UNIQUE,
  request_hash TEXT NOT NULL,
  prompt TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN (
    'queued', 'planning', 'waiting_for_approval', 'running',
    'paused', 'completed', 'failed', 'cancelled'
  )),
  max_steps INTEGER NOT NULL,
  max_tool_calls INTEGER NOT NULL,
  max_input_bytes INTEGER NOT NULL,
  max_output_bytes INTEGER NOT NULL,
  max_cost_microusd INTEGER NOT NULL,
  lease_secs INTEGER NOT NULL,
  deadline_at INTEGER NOT NULL,
  used_steps INTEGER NOT NULL DEFAULT 0,
  used_tool_calls INTEGER NOT NULL DEFAULT 0,
  used_input_bytes INTEGER NOT NULL DEFAULT 0,
  used_output_bytes INTEGER NOT NULL DEFAULT 0,
  used_cost_microusd INTEGER NOT NULL DEFAULT 0,
  cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK(cancel_requested IN (0, 1)),
  failure_code TEXT,
  lease_owner TEXT,
  lease_token TEXT,
  lease_expires_at INTEGER,
  attempt INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS agent_tasks_ready
  ON agent_tasks(status, created_at, id);
CREATE INDEX IF NOT EXISTS agent_tasks_agent
  ON agent_tasks(agent_id, updated_at DESC, id);

CREATE TABLE IF NOT EXISTS queue_command_receipts (
  command_id TEXT PRIMARY KEY,
  payload_hash TEXT NOT NULL,
  result_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS queue_claim_receipts (
  claim_id TEXT PRIMARY KEY,
  worker_id TEXT NOT NULL,
  task_id TEXT REFERENCES agent_tasks(id),
  lease_token TEXT,
  lease_expires_at INTEGER,
  created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_dispatches (
  task_id TEXT NOT NULL REFERENCES agent_tasks(id),
  call_id TEXT NOT NULL,
  payload_hash TEXT NOT NULL,
  state TEXT NOT NULL CHECK(state IN (
    'dispatched', 'succeeded', 'failed', 'not_applied'
  )),
  dispatch_lease_token TEXT NOT NULL,
  side_effects_applied INTEGER NOT NULL DEFAULT 0 CHECK(side_effects_applied IN (0, 1)),
  output_hash TEXT,
  error_code TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY(task_id, call_id)
);

CREATE TABLE IF NOT EXISTS task_audit (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  task_id TEXT NOT NULL REFERENCES agent_tasks(id),
  event TEXT NOT NULL,
  at_epoch_secs INTEGER NOT NULL,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS task_audit_task
  ON task_audit(task_id, sequence);
