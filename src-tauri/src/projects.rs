use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_NAME_BYTES: usize = 160;
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_PROMPT_BYTES: usize = 32 * 1024;
const MAX_PLAN_STEPS: usize = 16;
const MAX_PATHS_PER_STEP: usize = 24;
static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn id(prefix: &str) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!(
        "{prefix}-{:x}-{timestamp:x}-{counter:x}",
        std::process::id()
    )
}

fn open(database: &Path) -> Result<Connection, String> {
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Spindle data directory: {error}"))?;
    }
    let connection = Connection::open(database)
        .map_err(|error| format!("Could not open Spindle database: {error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS schema_migrations (
           version INTEGER PRIMARY KEY,
           applied_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS projects (
           id TEXT PRIMARY KEY,
           name TEXT NOT NULL,
           root_path TEXT NOT NULL COLLATE NOCASE UNIQUE,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           last_opened_at INTEGER,
           archived_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_projects_updated ON projects(archived_at, updated_at DESC);
         CREATE TABLE IF NOT EXISTS sessions (
           id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
           title TEXT NOT NULL,
           provider_id TEXT,
           model_id TEXT,
           status TEXT NOT NULL DEFAULT 'idle',
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project_id, updated_at DESC);
         CREATE TABLE IF NOT EXISTS session_messages (
           id TEXT PRIMARY KEY,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           sequence INTEGER NOT NULL,
           role TEXT NOT NULL CHECK(role IN ('user','assistant','system')),
           content TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           UNIQUE(session_id, sequence)
         );
         CREATE INDEX IF NOT EXISTS idx_messages_session ON session_messages(session_id, sequence);
         CREATE TABLE IF NOT EXISTS agent_tasks (
           id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
           session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
           title TEXT NOT NULL,
           prompt TEXT NOT NULL,
           state TEXT NOT NULL,
           plan_json TEXT,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           approved_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_tasks_session ON agent_tasks(session_id, updated_at DESC);
         CREATE INDEX IF NOT EXISTS idx_tasks_project ON agent_tasks(project_id, updated_at DESC);
         CREATE TABLE IF NOT EXISTS task_steps (
           id TEXT PRIMARY KEY,
           task_id TEXT NOT NULL REFERENCES agent_tasks(id) ON DELETE CASCADE,
           sequence INTEGER NOT NULL,
           title TEXT NOT NULL,
           description TEXT NOT NULL,
           target_paths_json TEXT NOT NULL,
           validation TEXT NOT NULL,
           risk TEXT NOT NULL,
           status TEXT NOT NULL DEFAULT 'pending',
           UNIQUE(task_id, sequence)
         );
         CREATE INDEX IF NOT EXISTS idx_task_steps_task ON task_steps(task_id, sequence);
         CREATE TABLE IF NOT EXISTS agent_events (
           id TEXT PRIMARY KEY,
           task_id TEXT NOT NULL REFERENCES agent_tasks(id) ON DELETE CASCADE,
           sequence INTEGER NOT NULL,
           kind TEXT NOT NULL,
           payload_json TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           UNIQUE(task_id, sequence)
         );
         CREATE INDEX IF NOT EXISTS idx_agent_events_task ON agent_events(task_id, sequence);
         CREATE TABLE IF NOT EXISTS project_memories (
           id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
           kind TEXT NOT NULL,
           memory_key TEXT NOT NULL,
           content TEXT NOT NULL,
           source_path TEXT NOT NULL DEFAULT '',
           source_revision TEXT NOT NULL DEFAULT '',
           confidence REAL NOT NULL DEFAULT 0.7,
           importance REAL NOT NULL DEFAULT 0.5,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           last_used_at INTEGER NOT NULL DEFAULT 0,
           archived INTEGER NOT NULL DEFAULT 0,
           UNIQUE(project_id, kind, memory_key)
         );
         CREATE INDEX IF NOT EXISTS idx_project_memories_active
           ON project_memories(project_id, archived, importance DESC, updated_at DESC);
         INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (2, unixepoch());",
        )
        .map_err(|error| format!("Could not migrate project and agent storage: {error}"))?;
    Ok(connection)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_opened_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMessage {
    pub id: String,
    pub session_id: String,
    pub sequence: i64,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlanStep {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub target_paths: Vec<String>,
    #[serde(default)]
    pub validation: String,
    #[serde(default)]
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlan {
    pub summary: String,
    pub steps: Vec<AgentPlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStep {
    pub id: String,
    pub task_id: String,
    pub sequence: i64,
    pub title: String,
    pub description: String,
    pub target_paths: Vec<String>,
    pub validation: String,
    pub risk: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTask {
    pub id: String,
    pub project_id: String,
    pub session_id: String,
    pub title: String,
    pub prompt: String,
    pub state: String,
    pub plan: Option<Value>,
    pub created_at: i64,
    pub updated_at: i64,
    pub approved_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDetail {
    pub task: AgentTask,
    pub steps: Vec<TaskStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: String,
    pub task_id: String,
    pub sequence: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMemory {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub memory_key: String,
    pub content: String,
    pub source_path: String,
    pub source_revision: String,
    pub confidence: f64,
    pub importance: f64,
    pub updated_at: i64,
}

fn project_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        root_path: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        last_opened_at: row.get(5)?,
    })
}

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        provider_id: row.get(3)?,
        model_id: row.get(4)?,
        status: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentTask> {
    let raw: Option<String> = row.get(6)?;
    let plan = raw.and_then(|value| serde_json::from_str(&value).ok());
    Ok(AgentTask {
        id: row.get(0)?,
        project_id: row.get(1)?,
        session_id: row.get(2)?,
        title: row.get(3)?,
        prompt: row.get(4)?,
        state: row.get(5)?,
        plan,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        approved_at: row.get(9)?,
    })
}

fn validate_name(value: &str) -> Result<String, String> {
    let name = value.trim();
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err("Names must contain between 1 and 160 UTF-8 bytes.".into());
    }
    if name.chars().any(char::is_control) {
        return Err("Names cannot contain control characters.".into());
    }
    Ok(name.to_owned())
}

fn canonical_project_root(value: &str) -> Result<PathBuf, String> {
    if value.trim().is_empty() || value.len() > 4_096 {
        return Err("Choose a valid project directory.".into());
    }
    let root = std::fs::canonicalize(PathBuf::from(value.trim()))
        .map_err(|error| format!("Could not resolve the project directory: {error}"))?;
    if !root.is_dir() || root.parent().is_none() {
        return Err("Choose a project directory rather than a filesystem root or file.".into());
    }
    Ok(root)
}

pub fn list_projects(database: PathBuf) -> Result<Vec<Project>, String> {
    let connection = open(&database)?;
    let mut statement = connection
        .prepare(
            "SELECT id,name,root_path,created_at,updated_at,last_opened_at
         FROM projects WHERE archived_at IS NULL
         ORDER BY COALESCE(last_opened_at, updated_at) DESC, name COLLATE NOCASE",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], project_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn get_project(database: PathBuf, project_id: String) -> Result<Project, String> {
    let connection = open(&database)?;
    connection
        .query_row(
            "SELECT id,name,root_path,created_at,updated_at,last_opened_at
         FROM projects WHERE id=?1 AND archived_at IS NULL",
            params![project_id],
            project_from_row,
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "The selected project no longer exists or has been archived.".into())
}

pub fn create_project(
    database: PathBuf,
    root_path: String,
    name: String,
) -> Result<Project, String> {
    let root = canonical_project_root(&root_path)?;
    let name = validate_name(&name)?;
    let root_path = root.to_string_lossy().into_owned();
    let connection = open(&database)?;
    let exists: Option<String> = connection
        .query_row(
            "SELECT id FROM projects WHERE root_path=?1 COLLATE NOCASE AND archived_at IS NULL",
            params![root_path],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if exists.is_some() {
        return Err("This directory is already registered as a Spindle project.".into());
    }
    let timestamp = now();
    let project = Project {
        id: id("project"),
        name,
        root_path,
        created_at: timestamp,
        updated_at: timestamp,
        last_opened_at: Some(timestamp),
    };
    connection
        .execute(
            "INSERT INTO projects(id,name,root_path,created_at,updated_at,last_opened_at)
         VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                project.id,
                project.name,
                project.root_path,
                timestamp,
                timestamp,
                timestamp
            ],
        )
        .map_err(|error| format!("Could not create project: {error}"))?;
    Ok(project)
}

pub fn rename_project(
    database: PathBuf,
    project_id: String,
    name: String,
) -> Result<Project, String> {
    let name = validate_name(&name)?;
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE projects SET name=?1,updated_at=?2 WHERE id=?3 AND archived_at IS NULL",
            params![name, now(), project_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected project no longer exists.".into());
    }
    get_project(database, project_id)
}

pub fn archive_project(database: PathBuf, project_id: String) -> Result<(), String> {
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE projects SET archived_at=?1,updated_at=?1 WHERE id=?2 AND archived_at IS NULL",
            params![now(), project_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected project no longer exists.".into());
    }
    Ok(())
}

pub fn delete_project(database: PathBuf, project_id: String) -> Result<(), String> {
    let connection = open(&database)?;
    let changed = connection
        .execute("DELETE FROM projects WHERE id=?1", params![project_id])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected project no longer exists.".into());
    }
    Ok(())
}

pub fn touch_project(database: PathBuf, project_id: String) -> Result<Project, String> {
    let connection = open(&database)?;
    let changed = connection.execute(
        "UPDATE projects SET last_opened_at=?1,updated_at=?1 WHERE id=?2 AND archived_at IS NULL",
        params![now(), project_id],
    ).map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected project no longer exists.".into());
    }
    get_project(database, project_id)
}

pub fn list_sessions(database: PathBuf, project_id: String) -> Result<Vec<Session>, String> {
    let connection = open(&database)?;
    let mut statement = connection.prepare(
        "SELECT s.id,s.project_id,s.title,s.provider_id,s.model_id,s.status,s.created_at,s.updated_at
         FROM sessions s INNER JOIN projects p ON p.id=s.project_id
         WHERE s.project_id=?1 AND p.archived_at IS NULL
         ORDER BY s.updated_at DESC",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id], session_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn create_session(
    database: PathBuf,
    project_id: String,
    title: String,
) -> Result<Session, String> {
    let title = validate_name(&title)?;
    let connection = open(&database)?;
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id=?1 AND archived_at IS NULL)",
            params![project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("Open an active project before creating a session.".into());
    }
    let timestamp = now();
    let session = Session {
        id: id("session"),
        project_id,
        title,
        provider_id: None,
        model_id: None,
        status: "idle".into(),
        created_at: timestamp,
        updated_at: timestamp,
    };
    connection.execute(
        "INSERT INTO sessions(id,project_id,title,provider_id,model_id,status,created_at,updated_at)
         VALUES (?1,?2,?3,NULL,NULL,'idle',?4,?4)",
        params![session.id, session.project_id, session.title, timestamp],
    ).map_err(|error| format!("Could not create session: {error}"))?;
    Ok(session)
}

pub fn rename_session(
    database: PathBuf,
    session_id: String,
    title: String,
) -> Result<Session, String> {
    let title = validate_name(&title)?;
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE sessions SET title=?1,updated_at=?2 WHERE id=?3",
            params![title, now(), session_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected session no longer exists.".into());
    }
    get_session(database, session_id)
}

pub fn get_session(database: PathBuf, session_id: String) -> Result<Session, String> {
    let connection = open(&database)?;
    connection.query_row(
        "SELECT s.id,s.project_id,s.title,s.provider_id,s.model_id,s.status,s.created_at,s.updated_at
         FROM sessions s INNER JOIN projects p ON p.id=s.project_id
         WHERE s.id=?1 AND p.archived_at IS NULL",
        params![session_id],
        session_from_row,
    ).optional().map_err(|error| error.to_string())?
      .ok_or_else(|| "The selected session no longer exists.".into())
}

pub fn update_session_model(
    database: PathBuf,
    session_id: String,
    provider_id: String,
    model_id: String,
) -> Result<Session, String> {
    if !matches!(provider_id.as_str(), "ollama" | "lm-studio" | "llama-cpp") {
        return Err("Select a supported local inference provider.".into());
    }
    if model_id.trim().is_empty() || model_id.len() > 256 {
        return Err("Select a model with a valid identifier.".into());
    }
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE sessions SET provider_id=?1,model_id=?2,updated_at=?3 WHERE id=?4",
            params![provider_id, model_id.trim(), now(), session_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected session no longer exists.".into());
    }
    get_session(database, session_id)
}

pub fn delete_session(database: PathBuf, session_id: String) -> Result<(), String> {
    let connection = open(&database)?;
    let changed = connection
        .execute("DELETE FROM sessions WHERE id=?1", params![session_id])
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected session no longer exists.".into());
    }
    Ok(())
}

pub fn list_messages(database: PathBuf, session_id: String) -> Result<Vec<SessionMessage>, String> {
    let connection = open(&database)?;
    let mut statement = connection
        .prepare(
            "SELECT m.id,m.session_id,m.sequence,m.role,m.content,m.created_at
         FROM (
           SELECT * FROM session_messages
           WHERE session_id=?1 ORDER BY sequence DESC LIMIT 128
         ) m INNER JOIN sessions s ON s.id=m.session_id
         ORDER BY m.sequence ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![session_id], |row| {
            Ok(SessionMessage {
                id: row.get(0)?,
                session_id: row.get(1)?,
                sequence: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn append_message(
    database: PathBuf,
    session_id: String,
    role: String,
    content: String,
) -> Result<SessionMessage, String> {
    if !matches!(role.as_str(), "user" | "assistant" | "system") {
        return Err("Only user, assistant, and system messages can be persisted.".into());
    }
    if content.trim().is_empty() || content.len() > MAX_MESSAGE_BYTES {
        return Err("Messages must contain 1 to 65,536 UTF-8 bytes.".into());
    }
    let mut connection = open(&database)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !exists {
        return Err("The selected session no longer exists.".into());
    }
    let count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM session_messages WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count >= 2_000 {
        return Err("This session has reached its 2,000-message retention limit.".into());
    }
    let sequence: i64 = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM session_messages WHERE session_id=?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let message = SessionMessage {
        id: id("message"),
        session_id: session_id.clone(),
        sequence,
        role: role.clone(),
        content,
        created_at: now(),
    };
    transaction
        .execute(
            "INSERT INTO session_messages(id,session_id,sequence,role,content,created_at)
         VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                message.id,
                message.session_id,
                message.sequence,
                message.role,
                message.content,
                message.created_at
            ],
        )
        .map_err(|error| format!("Could not persist the message: {error}"))?;
    if role == "user" {
        let title: Option<String> = transaction
            .query_row(
                "SELECT title FROM sessions WHERE id=?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if title.as_deref() == Some("New session") {
            let compact = message
                .content
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            let next_title = compact.chars().take(72).collect::<String>();
            transaction
                .execute(
                    "UPDATE sessions SET title=?1,updated_at=?2 WHERE id=?3",
                    params![next_title, message.created_at, session_id],
                )
                .map_err(|error| error.to_string())?;
        } else {
            transaction
                .execute(
                    "UPDATE sessions SET updated_at=?1 WHERE id=?2",
                    params![message.created_at, session_id],
                )
                .map_err(|error| error.to_string())?;
        }
    } else {
        transaction
            .execute(
                "UPDATE sessions SET updated_at=?1 WHERE id=?2",
                params![message.created_at, session_id],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(message)
}

pub fn create_task(
    database: PathBuf,
    project_id: String,
    session_id: String,
    prompt: String,
) -> Result<AgentTask, String> {
    if prompt.trim().is_empty() || prompt.len() > MAX_PROMPT_BYTES {
        return Err("Task prompts must contain 1 to 32,768 UTF-8 bytes.".into());
    }
    let title = prompt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(80)
        .collect::<String>();
    let connection = open(&database)?;
    let valid: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND project_id=?2)",
            params![session_id, project_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !valid {
        return Err("The task session does not belong to the selected project.".into());
    }
    let timestamp = now();
    let task = AgentTask {
        id: id("task"),
        project_id,
        session_id,
        title,
        prompt,
        state: "planning".into(),
        plan: None,
        created_at: timestamp,
        updated_at: timestamp,
        approved_at: None,
    };
    connection.execute(
        "INSERT INTO agent_tasks(id,project_id,session_id,title,prompt,state,plan_json,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,'planning',NULL,?6,?6)",
        params![task.id, task.project_id, task.session_id, task.title, task.prompt, timestamp],
    ).map_err(|error| format!("Could not create agent task: {error}"))?;
    append_event_inner(
        &connection,
        &task.id,
        "task_created",
        &serde_json::json!({"promptBytes": task.prompt.len()}),
    )?;
    Ok(task)
}

fn append_event_inner(
    connection: &Connection,
    task_id: &str,
    kind: &str,
    payload: &Value,
) -> Result<(), String> {
    let sequence: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence),0)+1 FROM agent_events WHERE task_id=?1",
            params![task_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "INSERT INTO agent_events(id,task_id,sequence,kind,payload_json,created_at)
         VALUES (?1,?2,?3,?4,?5,?6)",
            params![
                id("event"),
                task_id,
                sequence,
                kind,
                payload.to_string(),
                now()
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn append_task_event(
    database: PathBuf,
    task_id: String,
    kind: String,
    payload: Value,
) -> Result<(), String> {
    if kind.trim().is_empty() || kind.len() > 64 {
        return Err("Agent event kind is invalid.".into());
    }
    let connection = open(&database)?;
    append_event_inner(&connection, &task_id, &kind, &payload)
}

pub fn save_task_plan(
    database: PathBuf,
    task_id: String,
    plan: AgentPlan,
) -> Result<TaskDetail, String> {
    if plan.summary.trim().is_empty()
        || plan.summary.len() > 4_000
        || plan.steps.is_empty()
        || plan.steps.len() > MAX_PLAN_STEPS
    {
        return Err("The planner returned an empty or oversized plan.".into());
    }
    let serialized = serde_json::to_string(&plan).map_err(|error| error.to_string())?;
    let mut connection = open(&database)?;
    let transaction = connection
        .transaction()
        .map_err(|error| error.to_string())?;
    let changed = transaction
        .execute(
            "UPDATE agent_tasks SET plan_json=?1,state='awaiting_approval',updated_at=?2
         WHERE id=?3 AND state='planning'",
            params![serialized, now(), task_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The task is no longer in the planning state.".into());
    }
    transaction
        .execute("DELETE FROM task_steps WHERE task_id=?1", params![task_id])
        .map_err(|error| error.to_string())?;
    for (index, step) in plan.steps.iter().enumerate() {
        let title = validate_name(&step.title)?;
        if step.description.trim().is_empty()
            || step.description.len() > 4_000
            || step.validation.len() > 1_000
            || step.target_paths.len() > MAX_PATHS_PER_STEP
        {
            return Err("A planner step exceeded the supported size limits.".into());
        }
        let target_paths =
            serde_json::to_string(&step.target_paths).map_err(|error| error.to_string())?;
        transaction.execute(
            "INSERT INTO task_steps(id,task_id,sequence,title,description,target_paths_json,validation,risk,status)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'pending')",
            params![id("step"), task_id, index as i64, title, step.description, target_paths, step.validation, step.risk],
        ).map_err(|error| error.to_string())?;
    }
    append_event_inner(
        &transaction,
        &task_id,
        "plan_created",
        &serde_json::json!({"steps": plan.steps.len()}),
    )?;
    transaction.commit().map_err(|error| error.to_string())?;
    task_detail(database, task_id)
}

pub fn fail_task(database: PathBuf, task_id: String, reason: String) -> Result<(), String> {
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE agent_tasks SET state='failed',updated_at=?1 WHERE id=?2 AND state='planning'",
            params![now(), task_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The task is no longer in the planning state.".into());
    }
    append_event_inner(
        &connection,
        &task_id,
        "planning_failed",
        &serde_json::json!({"reason": reason.chars().take(800).collect::<String>()}),
    )
}

pub fn approve_task_plan(database: PathBuf, task_id: String) -> Result<AgentTask, String> {
    let connection = open(&database)?;
    let timestamp = now();
    let changed = connection
        .execute(
            "UPDATE agent_tasks SET state='approved',approved_at=?1,updated_at=?1
         WHERE id=?2 AND state='awaiting_approval' AND plan_json IS NOT NULL",
            params![timestamp, task_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("Only a complete plan awaiting approval can be approved.".into());
    }
    append_event_inner(
        &connection,
        &task_id,
        "plan_approved",
        &serde_json::json!({"approvedAt": timestamp}),
    )?;
    get_task(database, task_id)
}

pub fn reject_task_plan(database: PathBuf, task_id: String) -> Result<AgentTask, String> {
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE agent_tasks SET state='rejected',updated_at=?1
         WHERE id=?2 AND state IN ('planning','awaiting_approval')",
            params![now(), task_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected task cannot be rejected in its current state.".into());
    }
    append_event_inner(&connection, &task_id, "plan_rejected", &Value::Null)?;
    get_task(database, task_id)
}

pub fn get_task(database: PathBuf, task_id: String) -> Result<AgentTask, String> {
    let connection = open(&database)?;
    connection.query_row(
        "SELECT id,project_id,session_id,title,prompt,state,plan_json,created_at,updated_at,approved_at
         FROM agent_tasks WHERE id=?1",
        params![task_id],
        task_from_row,
    ).optional().map_err(|error| error.to_string())?
      .ok_or_else(|| "The selected task no longer exists.".into())
}

pub fn list_tasks(database: PathBuf, session_id: String) -> Result<Vec<AgentTask>, String> {
    let connection = open(&database)?;
    let mut statement = connection.prepare(
        "SELECT id,project_id,session_id,title,prompt,state,plan_json,created_at,updated_at,approved_at
         FROM agent_tasks WHERE session_id=?1 ORDER BY updated_at DESC LIMIT 100",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![session_id], task_from_row)
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

pub fn task_detail(database: PathBuf, task_id: String) -> Result<TaskDetail, String> {
    let task = get_task(database.clone(), task_id.clone())?;
    let connection = open(&database)?;
    let mut statement = connection
        .prepare(
            "SELECT id,task_id,sequence,title,description,target_paths_json,validation,risk,status
         FROM task_steps WHERE task_id=?1 ORDER BY sequence",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_id], |row| {
            let raw: String = row.get(5)?;
            Ok(TaskStep {
                id: row.get(0)?,
                task_id: row.get(1)?,
                sequence: row.get(2)?,
                title: row.get(3)?,
                description: row.get(4)?,
                target_paths: serde_json::from_str(&raw).unwrap_or_default(),
                validation: row.get(6)?,
                risk: row.get(7)?,
                status: row.get(8)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let steps = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(TaskDetail { task, steps })
}

pub fn list_task_events(database: PathBuf, task_id: String) -> Result<Vec<AgentEvent>, String> {
    let connection = open(&database)?;
    let mut statement = connection
        .prepare(
            "SELECT id,task_id,sequence,kind,payload_json,created_at
         FROM agent_events WHERE task_id=?1 ORDER BY sequence",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![task_id], |row| {
            let raw: String = row.get(4)?;
            Ok(AgentEvent {
                id: row.get(0)?,
                task_id: row.get(1)?,
                sequence: row.get(2)?,
                kind: row.get(3)?,
                payload: serde_json::from_str(&raw).unwrap_or(Value::Null),
                created_at: row.get(5)?,
            })
        })
        .map_err(|error| error.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProjectMemoryInput {
    pub project_id: String,
    pub kind: String,
    pub memory_key: String,
    pub content: String,
    pub source_path: String,
    pub source_revision: String,
    pub confidence: f64,
    pub importance: f64,
}

pub fn save_project_memory(database: PathBuf, input: SaveProjectMemoryInput) -> Result<(), String> {
    if !matches!(
        input.kind.as_str(),
        "convention"
            | "user_preference"
            | "architecture_decision"
            | "task_outcome"
            | "failure_solution"
    ) {
        return Err("Unsupported project memory category.".into());
    }
    let key = validate_name(&input.memory_key)?;
    if input.content.trim().is_empty()
        || input.content.len() > 8_192
        || input.source_path.len() > 4_096
        || input.source_revision.len() > 256
    {
        return Err("Project memory content or source metadata exceeded its limits.".into());
    }
    if !input.confidence.is_finite()
        || !(0.0..=1.0).contains(&input.confidence)
        || !input.importance.is_finite()
        || !(0.0..=1.0).contains(&input.importance)
    {
        return Err("Memory confidence and importance must be between 0 and 1.".into());
    }
    let connection = open(&database)?;
    connection.execute(
        "INSERT INTO project_memories(id,project_id,kind,memory_key,content,source_path,source_revision,confidence,importance,created_at,updated_at,last_used_at,archived)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,0,0)
         ON CONFLICT(project_id,kind,memory_key) DO UPDATE SET
           content=excluded.content,source_path=excluded.source_path,
           source_revision=excluded.source_revision,confidence=excluded.confidence,
           importance=excluded.importance,updated_at=excluded.updated_at,archived=0",
        params![
            id("memory"), input.project_id, input.kind, key, input.content, input.source_path,
            input.source_revision, input.confidence, input.importance, now()
        ],
    )
    .map_err(|error| format!("Could not save project memory: {error}"))?;
    Ok(())
}

pub fn search_project_memory(
    database: PathBuf,
    project_id: String,
    query: String,
    limit: usize,
) -> Result<Vec<ProjectMemory>, String> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() || query.len() > 2_048 {
        return Err("Enter a project memory query between 1 and 2,048 bytes.".into());
    }
    let terms = query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .filter(|term| term.len() >= 2)
        .take(24)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let connection = open(&database)?;
    let mut statement = connection.prepare(
        "SELECT id,project_id,kind,memory_key,content,source_path,source_revision,confidence,importance,updated_at
         FROM project_memories WHERE project_id=?1 AND archived=0
         ORDER BY importance DESC,updated_at DESC LIMIT 300",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok(ProjectMemory {
                id: row.get(0)?,
                project_id: row.get(1)?,
                kind: row.get(2)?,
                memory_key: row.get(3)?,
                content: row.get(4)?,
                source_path: row.get(5)?,
                source_revision: row.get(6)?,
                confidence: row.get(7)?,
                importance: row.get(8)?,
                updated_at: row.get(9)?,
            })
        })
        .map_err(|error| error.to_string())?;
    let cap = limit.clamp(1, 12);
    let mut ranked = Vec::new();
    for item in rows {
        let memory = item.map_err(|error| error.to_string())?;
        let haystack = format!("{} {} {}", memory.kind, memory.memory_key, memory.content)
            .to_ascii_lowercase();
        let mut score = 0usize;
        for term in &terms {
            if haystack.contains(term) {
                score += 1;
            }
        }
        if score > 0 || haystack.contains(&query) {
            ranked.push((score, memory.importance, memory.updated_at, memory));
        }
    }
    ranked.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.total_cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
    });
    let results = ranked
        .into_iter()
        .take(cap)
        .map(|(_, _, _, memory)| memory)
        .collect::<Vec<_>>();
    Ok(results)
}

pub fn archive_project_memory(
    database: PathBuf,
    project_id: String,
    memory_id: String,
) -> Result<(), String> {
    let connection = open(&database)?;
    let changed = connection
        .execute(
            "UPDATE project_memories SET archived=1,updated_at=?1 WHERE id=?2 AND project_id=?3",
            params![now(), memory_id, project_id],
        )
        .map_err(|error| error.to_string())?;
    if changed == 0 {
        return Err("The selected project memory no longer exists.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_database() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("spindle-projects-{}", id("test")));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("app").join("spindle.sqlite3");
        (root, database)
    }

    #[test]
    fn projects_sessions_messages_and_tasks_survive_reopening() {
        let (root, database) = test_database();
        let project_root = root.join("project");
        fs::create_dir_all(&project_root).unwrap();
        let project = create_project(
            database.clone(),
            project_root.display().to_string(),
            "Example".into(),
        )
        .unwrap();
        let session =
            create_session(database.clone(), project.id.clone(), "New session".into()).unwrap();
        append_message(
            database.clone(),
            session.id.clone(),
            "user".into(),
            "Fix authentication.".into(),
        )
        .unwrap();
        append_message(
            database.clone(),
            session.id.clone(),
            "assistant".into(),
            "I will inspect the auth flow.".into(),
        )
        .unwrap();

        let reloaded = list_messages(database.clone(), session.id.clone()).unwrap();
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded[0].content, "Fix authentication.");
        assert_eq!(
            get_session(database.clone(), session.id.clone())
                .unwrap()
                .title,
            "Fix authentication."
        );

        let task = create_task(
            database.clone(),
            project.id.clone(),
            session.id.clone(),
            "Plan the auth fix.".into(),
        )
        .unwrap();
        let detail = save_task_plan(
            database.clone(),
            task.id.clone(),
            AgentPlan {
                summary: "Inspect and verify the authentication flow.".into(),
                steps: vec![AgentPlanStep {
                    title: "Inspect authentication".into(),
                    description: "Read the authentication entry points and tests.".into(),
                    target_paths: vec!["src/auth.ts".into()],
                    validation: "Identify existing tests.".into(),
                    risk: "low".into(),
                }],
            },
        )
        .unwrap();
        assert_eq!(detail.steps.len(), 1);
        assert_eq!(detail.task.state, "awaiting_approval");
        assert_eq!(
            approve_task_plan(database.clone(), task.id.clone())
                .unwrap()
                .state,
            "approved"
        );
        assert_eq!(
            list_task_events(database.clone(), task.id.clone())
                .unwrap()
                .len(),
            3
        );
        archive_project(database.clone(), project.id).unwrap();
        assert!(list_projects(database.clone()).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_names_prompts_and_memory_scores() {
        assert!(validate_name("   ").is_err());
        assert!(validate_name(&"x".repeat(MAX_NAME_BYTES + 1)).is_err());
        assert!(validate_name("ok").is_ok());

        let (root, database) = test_database();
        let project_root = root.join("project");
        fs::create_dir_all(&project_root).unwrap();
        let project = create_project(
            database.clone(),
            project_root.display().to_string(),
            "Example".into(),
        )
        .unwrap();
        save_project_memory(
            database.clone(),
            SaveProjectMemoryInput {
                project_id: project.id.clone(),
                kind: "architecture_decision".into(),
                memory_key: "auth-flow".into(),
                content: "Authentication lives in src/auth.ts".into(),
                source_path: "src/auth.ts".into(),
                source_revision: "rev1".into(),
                confidence: 0.9,
                importance: 0.8,
            },
        )
        .unwrap();
        let results = search_project_memory(
            database,
            project.id,
            "authentication auth".into(),
            5,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
