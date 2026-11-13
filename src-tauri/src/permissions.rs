use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_AUDIT_EVENTS: usize = 512;
const MAX_RESOURCE_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionScope {
    ProjectRead,
    ProjectWrite,
    ProcessExecution,
    ExternalNetwork,
    McpTools,
    ParallelAgents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOperation {
    WorkspaceRead,
    FileChangePropose,
    FileChangeApply,
    FileChangeUndo,
    ProcessExecution,
    ExternalNetwork,
    McpToolCall,
    SpawnParallelAgent,
}

impl PermissionOperation {
    pub fn required_scope(self) -> PermissionScope {
        match self {
            Self::WorkspaceRead => PermissionScope::ProjectRead,
            Self::FileChangePropose | Self::FileChangeApply | Self::FileChangeUndo => {
                PermissionScope::ProjectWrite
            }
            Self::ProcessExecution => PermissionScope::ProcessExecution,
            Self::ExternalNetwork => PermissionScope::ExternalNetwork,
            Self::McpToolCall => PermissionScope::McpTools,
            Self::SpawnParallelAgent => PermissionScope::ParallelAgents,
        }
    }

    fn for_scope(scope: PermissionScope) -> Self {
        match scope {
            PermissionScope::ProjectRead => Self::WorkspaceRead,
            PermissionScope::ProjectWrite => Self::FileChangePropose,
            PermissionScope::ProcessExecution => Self::ProcessExecution,
            PermissionScope::ExternalNetwork => Self::ExternalNetwork,
            PermissionScope::McpTools => Self::McpToolCall,
            PermissionScope::ParallelAgents => Self::SpawnParallelAgent,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceRead => "workspace_read",
            Self::FileChangePropose => "file_change_propose",
            Self::FileChangeApply => "file_change_apply",
            Self::FileChangeUndo => "file_change_undo",
            Self::ProcessExecution => "process_execution",
            Self::ExternalNetwork => "external_network",
            Self::McpToolCall => "mcp_tool_call",
            Self::SpawnParallelAgent => "spawn_parallel_agent",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermissions {
    pub session_id: String,
    pub project_id: String,
    pub scopes: Vec<PermissionScope>,
    pub expires_on_restart: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionAuditEvent {
    pub session_id: String,
    pub project_id: String,
    pub operation: String,
    pub scope: Option<PermissionScope>,
    pub resource: String,
    pub allowed: bool,
    pub reason: Option<String>,
    pub timestamp_unix_ms: u64,
}

#[derive(Default)]
struct BrokerState {
    sessions: BTreeMap<String, SessionPermissions>,
    audit: VecDeque<PermissionAuditEvent>,
    audit_database: Option<PathBuf>,
}

#[derive(Clone, Default)]
pub struct PermissionBroker {
    state: Arc<Mutex<BrokerState>>,
}

impl PermissionBroker {
    pub fn set_audit_database(&self, path: PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create audit database directory: {error}"))?;
        }
        let connection = rusqlite::Connection::open(&path)
            .map_err(|error| format!("Could not open permission audit database: {error}"))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS permission_audit_events (
               event_id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL,
               timestamp_unix_ms INTEGER NOT NULL,
               event_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_permission_audit_session
               ON permission_audit_events(session_id,event_id DESC);"
        ).map_err(|error| format!("Could not initialize permission audit storage: {error}"))?;
        self.state.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?
            .audit_database = Some(path);
        Ok(())
    }

    pub fn register_session(
        &self,
        session_id: String,
        project_id: String,
    ) -> Result<SessionPermissions, String> {
        if session_id.is_empty()
            || session_id.len() > 128
            || project_id.is_empty()
            || project_id.len() > MAX_RESOURCE_BYTES
        {
            return Err("The project permission identity is invalid.".into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        if let Some(existing) = state.sessions.get(&session_id) {
            if existing.project_id != project_id {
                return Err(
                    "The permission session is already bound to a different project.".into(),
                );
            }
            return Ok(existing.clone());
        }
        let permissions = SessionPermissions {
            session_id: session_id.clone(),
            project_id: project_id.clone(),
            scopes: vec![PermissionScope::ProjectRead],
            expires_on_restart: true,
        };
        Self::record(
            &mut state,
            PermissionAuditEvent {
                session_id: session_id.clone(),
                project_id,
                operation: "session_registered".into(),
                scope: Some(PermissionScope::ProjectRead),
                resource: String::new(),
                allowed: true,
                reason: None,
                timestamp_unix_ms: now_ms(),
            },
        )?;
        state.sessions.insert(session_id, permissions.clone());
        Ok(permissions)
    }

    pub fn status(&self, session_id: &str) -> Result<SessionPermissions, String> {
        self.state
            .lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| "No permissions have been registered for this session.".into())
    }

    pub fn grant(
        &self,
        session_id: &str,
        requested: &[PermissionScope],
    ) -> Result<SessionPermissions, String> {
        if requested.is_empty() || requested.len() > 6 {
            return Err("Select at least one supported permission scope.".into());
        }
        let mut state = self.state.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        let existing = state.sessions.get(session_id).cloned().ok_or_else(|| {
            "The project session is not registered with the permission broker.".to_string()
        })?;
        let before = existing.scopes.iter().copied().collect::<BTreeSet<_>>();
        let mut scopes = before.clone();
        scopes.extend(requested.iter().copied());
        let mut next = existing.clone();
        next.scopes = scopes.into_iter().collect();
        for scope in requested.iter().copied().filter(|scope| !before.contains(scope)) {
            Self::record(&mut state, PermissionAuditEvent {
                session_id: session_id.to_owned(), project_id: existing.project_id.clone(),
                operation: "scope_granted".into(), scope: Some(scope), resource: String::new(),
                allowed: true, reason: None, timestamp_unix_ms: now_ms(),
            })?;
        }
        state.sessions.insert(session_id.to_owned(), next.clone());
        Ok(next)
    }

    pub fn revoke(
        &self,
        session_id: &str,
        requested: &[PermissionScope],
    ) -> Result<SessionPermissions, String> {
        let mut state = self.state.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        let existing = state.sessions.get(session_id).cloned().ok_or_else(|| {
            "The project session is not registered with the permission broker.".to_string()
        })?;
        let revoke = requested.iter().copied().collect::<BTreeSet<_>>();
        let before = existing.scopes.iter().copied().collect::<BTreeSet<_>>();
        let mut next = existing.clone();
        next.scopes.retain(|scope| *scope == PermissionScope::ProjectRead || !revoke.contains(scope));
        for scope in revoke.into_iter().filter(|scope| *scope != PermissionScope::ProjectRead && before.contains(scope)) {
            Self::record(&mut state, PermissionAuditEvent {
                session_id: session_id.to_owned(), project_id: existing.project_id.clone(),
                operation: "scope_revoked".into(), scope: Some(scope), resource: String::new(),
                allowed: true, reason: None, timestamp_unix_ms: now_ms(),
            })?;
        }
        state.sessions.insert(session_id.to_owned(), next.clone());
        Ok(next)
    }

    pub fn revoke_all(&self, session_id: &str) -> Result<(), String> {
        let mut state = self.state.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        if let Some(session) = state.sessions.get(session_id).cloned() {
            Self::record(&mut state, PermissionAuditEvent {
                session_id: session_id.to_owned(), project_id: session.project_id,
                operation: "session_revoked".into(), scope: None, resource: String::new(),
                allowed: true, reason: None, timestamp_unix_ms: now_ms(),
            })?;
            state.sessions.remove(session_id);
        }
        Ok(())
    }

    pub fn authorize(
        &self,
        session_id: &str,
        operation: PermissionOperation,
        resource: &str,
    ) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        let session = state
            .sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| "The project session has no active permission grant.".to_string())?;
        let scope = operation.required_scope();
        let invalid_resource =
            resource.len() > MAX_RESOURCE_BYTES || resource.chars().any(char::is_control);
        let allowed = !invalid_resource && session.scopes.contains(&scope);
        let reason = if invalid_resource {
            Some("The requested resource identifier is invalid.".to_string())
        } else if !allowed {
            Some(format!(
                "The session has not been granted the {scope:?} permission."
            ))
        } else {
            None
        };
        Self::record(
            &mut state,
            PermissionAuditEvent {
                session_id: session_id.to_owned(),
                project_id: session.project_id,
                operation: operation.as_str().into(),
                scope: Some(scope),
                resource: resource.chars().take(MAX_RESOURCE_BYTES).collect(),
                allowed,
                reason: reason.clone(),
                timestamp_unix_ms: now_ms(),
            },
        )?;
        match reason {
            Some(message) => Err(message),
            None => Ok(()),
        }
    }

    pub fn audit_log(&self, session_id: &str) -> Result<Vec<PermissionAuditEvent>, String> {
        let state = self.state.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?;
        if let Some(path) = state.audit_database.as_ref() {
            let connection = rusqlite::Connection::open(path)
                .map_err(|error| format!("Could not read permission audit storage: {error}"))?;
            let mut statement = connection.prepare(
                "SELECT event_json FROM permission_audit_events WHERE session_id=?1 ORDER BY event_id DESC LIMIT 100"
            ).map_err(|error| error.to_string())?;
            let rows = statement.query_map([session_id], |row| row.get::<_, String>(0))
                .map_err(|error| error.to_string())?;
            let mut events = rows.map(|row| {
                let raw = row.map_err(|error| error.to_string())?;
                serde_json::from_str::<PermissionAuditEvent>(&raw)
                    .map_err(|error| format!("A stored permission audit record is invalid: {error}"))
            }).collect::<Result<Vec<_>, String>>()?;
            events.reverse();
            return Ok(events);
        }
        Ok(state.audit.iter().filter(|event| event.session_id == session_id)
            .rev().take(100).cloned().collect::<Vec<_>>().into_iter().rev().collect())
    }

    pub fn require(&self, session_id: &str, scope: PermissionScope) -> Result<(), String> {
        self.authorize(session_id, PermissionOperation::for_scope(scope), "")
    }

    fn record(state: &mut BrokerState, event: PermissionAuditEvent) -> Result<(), String> {
        if let Some(path) = state.audit_database.as_ref() {
            let connection = rusqlite::Connection::open(path)
                .map_err(|error| format!("Permission decision was not recorded; operation denied: {error}"))?;
            let serialized = serde_json::to_string(&event)
                .map_err(|error| format!("Could not serialize permission audit event: {error}"))?;
            connection.execute(
                "INSERT INTO permission_audit_events(session_id,timestamp_unix_ms,event_json) VALUES (?1,?2,?3)",
                rusqlite::params![event.session_id, event.timestamp_unix_ms as i64, serialized],
            ).map_err(|error| format!("Permission decision was not recorded; operation denied: {error}"))?;
            connection.execute(
                "DELETE FROM permission_audit_events WHERE event_id IN (
                   SELECT event_id FROM permission_audit_events ORDER BY event_id DESC LIMIT -1 OFFSET 10000
                 )", [],
            ).map_err(|error| format!("Could not enforce permission audit retention: {error}"))?;
        }
        if state.audit.len() >= MAX_AUDIT_EVENTS { state.audit.pop_front(); }
        state.audit.push_back(event);
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_start_read_only_and_privileges_require_a_grant() {
        let broker = PermissionBroker::default();
        let state = broker.register_session("s1".into(), "p1".into()).unwrap();
        assert_eq!(state.scopes, vec![PermissionScope::ProjectRead]);
        broker.require("s1", PermissionScope::ProjectRead).unwrap();
        assert!(broker.require("s1", PermissionScope::ProjectWrite).is_err());
        let state = broker
            .grant(
                "s1",
                &[
                    PermissionScope::ProjectWrite,
                    PermissionScope::ProcessExecution,
                ],
            )
            .unwrap();
        assert!(state.scopes.contains(&PermissionScope::ProjectRead));
        assert!(state.scopes.contains(&PermissionScope::ProjectWrite));
        assert!(state.expires_on_restart);
    }

    #[test]
    fn revoking_elevated_scopes_never_removes_the_baseline_read_scope() {
        let broker = PermissionBroker::default();
        broker.register_session("s1".into(), "p1".into()).unwrap();
        broker
            .grant(
                "s1",
                &[PermissionScope::ProjectWrite, PermissionScope::McpTools],
            )
            .unwrap();
        let state = broker
            .revoke(
                "s1",
                &[
                    PermissionScope::ProjectRead,
                    PermissionScope::ProjectWrite,
                    PermissionScope::McpTools,
                ],
            )
            .unwrap();
        assert_eq!(state.scopes, vec![PermissionScope::ProjectRead]);
    }

    #[test]
    fn permission_audit_history_survives_broker_recreation() {
        let path = std::env::temp_dir().join(format!(
            "spindle-permission-audit-{}-{}.sqlite3", std::process::id(), now_ms()
        ));
        let first = PermissionBroker::default();
        first.set_audit_database(path.clone()).unwrap();
        first.register_session("durable-session".into(), "project-1".into()).unwrap();
        first.grant("durable-session", &[PermissionScope::ProjectWrite]).unwrap();
        assert!(first.authorize(
            "durable-session", PermissionOperation::ProcessExecution, "task_validation"
        ).is_err());
        drop(first);

        let second = PermissionBroker::default();
        second.set_audit_database(path.clone()).unwrap();
        let events = second.audit_log("durable-session").unwrap();
        assert!(events.iter().any(|event| event.operation == "session_registered"));
        assert!(events.iter().any(|event| event.operation == "scope_granted"));
        assert!(events.iter().any(|event| event.operation == "process_execution" && !event.allowed));
        drop(second);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite3-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite3-shm"));
    }

    #[test]
    fn authorization_is_operation_scoped_and_audited() {
        let broker = PermissionBroker::default();
        broker
            .register_session("s1".into(), "project-root".into())
            .unwrap();
        assert!(broker
            .authorize("s1", PermissionOperation::FileChangeApply, "src/main.rs")
            .is_err());
        broker
            .grant("s1", &[PermissionScope::ProjectWrite])
            .unwrap();
        broker
            .authorize("s1", PermissionOperation::FileChangeApply, "src/main.rs")
            .unwrap();
        assert!(broker
            .authorize("s1", PermissionOperation::ProcessExecution, "cargo test")
            .is_err());
        let log = broker.audit_log("s1").unwrap();
        assert!(log.iter().any(|event| {
            event.operation == "file_change_apply"
                && event.resource == "src/main.rs"
                && event.allowed
        }));
        assert!(log
            .iter()
            .any(|event| { event.operation == "process_execution" && !event.allowed }));
    }

    #[test]
    fn revocation_removes_the_session_and_its_authority() {
        let broker = PermissionBroker::default();
        broker.register_session("s1".into(), "p1".into()).unwrap();
        broker
            .grant("s1", &[PermissionScope::ProjectWrite])
            .unwrap();
        broker.revoke_all("s1").unwrap();
        assert!(broker
            .authorize("s1", PermissionOperation::FileChangeApply, "a.rs")
            .is_err());
    }

    #[test]
    fn project_identity_accepts_normal_windows_paths() {
        let broker = PermissionBroker::default();
        assert!(broker
            .register_session("s1".into(), r"C:\Users\Test\Documents\Project".into())
            .is_ok());
    }
}
