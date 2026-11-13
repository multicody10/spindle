use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPermissions {
    pub session_id: String,
    pub project_id: String,
    pub scopes: Vec<PermissionScope>,
    pub expires_on_restart: bool,
}

#[derive(Clone, Default)]
pub struct PermissionBroker {
    sessions: Arc<Mutex<BTreeMap<String, SessionPermissions>>>,
}

impl PermissionBroker {
    pub fn register_session(&self, session_id: String, project_id: String) -> Result<SessionPermissions, String> {
        if session_id.is_empty() || session_id.len() > 128 || project_id.is_empty() || project_id.len() > 128 {
            return Err("The project permission identity is invalid.".into());
        }
        let mut sessions = self.sessions.lock().map_err(|_| "The permission broker is unavailable.".to_string())?;
        if let Some(existing) = sessions.get(&session_id) {
            if existing.project_id != project_id {
                return Err("The permission session is already bound to a different project.".into());
            }
            return Ok(existing.clone());
        }
        let state = SessionPermissions {
            session_id: session_id.clone(),
            project_id,
            scopes: vec![PermissionScope::ProjectRead],
            expires_on_restart: true,
        };
        sessions.insert(session_id, state.clone());
        Ok(state)
    }

    pub fn status(&self, session_id: &str) -> Result<SessionPermissions, String> {
        self.sessions.lock()
            .map_err(|_| "The permission broker is unavailable.".to_string())?
            .get(session_id).cloned()
            .ok_or_else(|| "No permissions have been registered for this session.".into())
    }

    pub fn grant(&self, session_id: &str, requested: &[PermissionScope]) -> Result<SessionPermissions, String> {
        if requested.is_empty() || requested.len() > 6 {
            return Err("Select at least one supported permission scope.".into());
        }
        let mut sessions = self.sessions.lock().map_err(|_| "The permission broker is unavailable.".to_string())?;
        let session = sessions.get_mut(session_id).ok_or_else(|| "The project session is not registered with the permission broker.".to_string())?;
        let mut scopes = session.scopes.iter().copied().collect::<BTreeSet<_>>();
        scopes.extend(requested.iter().copied());
        session.scopes = scopes.into_iter().collect();
        Ok(session.clone())
    }

    pub fn revoke(&self, session_id: &str, requested: &[PermissionScope]) -> Result<SessionPermissions, String> {
        let mut sessions = self.sessions.lock().map_err(|_| "The permission broker is unavailable.".to_string())?;
        let session = sessions.get_mut(session_id).ok_or_else(|| "The project session is not registered with the permission broker.".to_string())?;
        let revoke = requested.iter().copied().collect::<BTreeSet<_>>();
        session.scopes.retain(|scope| *scope == PermissionScope::ProjectRead || !revoke.contains(scope));
        Ok(session.clone())
    }

    pub fn revoke_all(&self, session_id: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|_| "The permission broker is unavailable.".to_string())?;
        sessions.remove(session_id);
        Ok(())
    }

    pub fn require(&self, session_id: &str, scope: PermissionScope) -> Result<(), String> {
        let sessions = self.sessions.lock().map_err(|_| "The permission broker is unavailable.".to_string())?;
        let session = sessions.get(session_id).ok_or_else(|| "The project session has no active permission grant.".to_string())?;
        if session.scopes.contains(&scope) {
            Ok(())
        } else {
            Err(format!("The project session has not been granted the {scope:?} permission."))
        }
    }
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
        let state = broker.grant("s1", &[PermissionScope::ProjectWrite, PermissionScope::ProcessExecution]).unwrap();
        assert!(state.scopes.contains(&PermissionScope::ProjectRead));
        assert!(state.scopes.contains(&PermissionScope::ProjectWrite));
        assert!(state.expires_on_restart);
    }

    #[test]
    fn revoking_elevated_scopes_never_removes_the_baseline_read_scope() {
        let broker = PermissionBroker::default();
        broker.register_session("s1".into(), "p1".into()).unwrap();
        broker.grant("s1", &[PermissionScope::ProjectWrite, PermissionScope::McpTools]).unwrap();
        let state = broker.revoke("s1", &[PermissionScope::ProjectRead, PermissionScope::ProjectWrite, PermissionScope::McpTools]).unwrap();
        assert_eq!(state.scopes, vec![PermissionScope::ProjectRead]);
    }
}
