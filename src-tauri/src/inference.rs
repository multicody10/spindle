use crate::{domain::RuntimeSettings, runtime::RuntimeBackend};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};

const MAX_PARALLEL_SLOTS: usize = 32;
const MAX_QUEUED_REQUESTS: usize = 64;
const MAX_RESERVED_SESSIONS: usize = MAX_PARALLEL_SLOTS + MAX_QUEUED_REQUESTS;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRuntimeStatus {
    pub model_loaded: bool,
    pub model_path: Option<String>,
    pub backend: Option<RuntimeBackend>,
    pub parallel_slots: usize,
    pub active_requests: usize,
    pub queued_requests: usize,
    pub available_slots: usize,
    pub configuration_fingerprint: Option<String>,
    pub assigned_sessions: usize,
    pub slot_evictions: u64,
}

struct LoadedModel {
    model_path: String,
    backend: RuntimeBackend,
    parallel_slots: usize,
    configuration_fingerprint: String,
}

#[derive(Debug, Clone, Copy)]
struct SessionSlot {
    id: usize,
    last_used: u64,
}

#[derive(Default)]
struct SchedulerState {
    model: Option<LoadedModel>,
    generation: u64,
    accepting_requests: bool,
    reserved_sessions: HashSet<String>,
    active_sessions: HashMap<String, String>,
    queued_requests: usize,
    session_slots: HashMap<String, SessionSlot>,
    slot_owners: Vec<Option<String>>,
    dirty_slots: Vec<bool>,
    use_clock: u64,
    slot_evictions: u64,
}

struct SchedulerInner {
    state: Mutex<SchedulerState>,
    semaphore: Mutex<Arc<Semaphore>>,
}

#[derive(Clone)]
pub struct InferenceRuntimeManager {
    inner: Arc<SchedulerInner>,
}

impl Default for InferenceRuntimeManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(SchedulerInner {
                state: Mutex::new(SchedulerState::default()),
                semaphore: Mutex::new(Arc::new(Semaphore::new(1))),
            }),
        }
    }
}

pub struct InferenceSessionLease {
    inner: Arc<SchedulerInner>,
    session_id: String,
    slot_id: usize,
    needs_clear: bool,
    _permit: OwnedSemaphorePermit,
}

impl InferenceSessionLease {
    pub fn slot_id(&self) -> usize {
        self.slot_id
    }

    pub fn needs_clear(&self) -> bool {
        self.needs_clear
    }
}

impl Drop for InferenceSessionLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.active_sessions.remove(&self.session_id);
            state.reserved_sessions.remove(&self.session_id);
            state.use_clock = state.use_clock.wrapping_add(1);
            let last_used = state.use_clock;
            if let Some(slot) = state.session_slots.get_mut(&self.session_id) {
                slot.last_used = last_used;
            }
        }
    }
}

impl InferenceRuntimeManager {
    pub fn configure(
        &self,
        model_path: &str,
        backend: RuntimeBackend,
        settings: &RuntimeSettings,
        runtime_version: &str,
    ) -> Result<(), String> {
        if settings.parallel_slots == 0 || settings.parallel_slots > MAX_PARALLEL_SLOTS {
            return Err(format!(
                "The shared inference scheduler supports 1 to {MAX_PARALLEL_SLOTS} parallel slots."
            ));
        }

        let normalized_path = model_path.replace('/', "\\").to_ascii_lowercase();
        let fingerprint_payload =
            serde_json::to_vec(&(runtime_version, &normalized_path, backend, settings))
                .map_err(|_| "Could not fingerprint the loaded inference model.".to_string())?;
        let fingerprint = format!("{:x}", Sha256::digest(fingerprint_payload));

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        if !state.reserved_sessions.is_empty() || state.queued_requests > 0 {
            return Err(
                "The shared inference runtime still has active or queued session requests.".into(),
            );
        }
        let semaphore = Arc::new(Semaphore::new(settings.parallel_slots));
        *self
            .inner
            .semaphore
            .lock()
            .map_err(|_| "The inference scheduler semaphore is unavailable.".to_string())? =
            semaphore;
        state.generation = state.generation.wrapping_add(1);
        state.model = Some(LoadedModel {
            model_path: model_path.to_owned(),
            backend,
            parallel_slots: settings.parallel_slots,
            configuration_fingerprint: fingerprint,
        });
        state.session_slots.clear();
        state.slot_owners = vec![None; settings.parallel_slots];
        state.dirty_slots = vec![false; settings.parallel_slots];
        state.use_clock = 0;
        state.slot_evictions = 0;
        state.accepting_requests = true;
        Ok(())
    }

    pub fn ensure_idle(&self) -> Result<(), String> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        if !state.reserved_sessions.is_empty()
            || !state.active_sessions.is_empty()
            || state.queued_requests > 0
        {
            return Err(
                "The shared inference runtime has active or queued requests. Finish or cancel them before stopping or replacing the model."
                    .into(),
            );
        }
        Ok(())
    }

    pub fn begin_shutdown(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        if !state.reserved_sessions.is_empty()
            || !state.active_sessions.is_empty()
            || state.queued_requests > 0
        {
            return Err(
                "The shared inference runtime has active or queued requests. Finish or cancel them before stopping the model."
                    .into(),
            );
        }
        state.accepting_requests = false;
        Ok(())
    }

    pub fn resume(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.accepting_requests = state.model.is_some();
        }
    }

    pub fn clear(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        if state.accepting_requests
            || !state.reserved_sessions.is_empty()
            || !state.active_sessions.is_empty()
            || state.queued_requests > 0
        {
            return Err(
                "Stop the shared inference runtime before clearing it; active or queued requests must finish first."
                    .into(),
            );
        }
        *self
            .inner
            .semaphore
            .lock()
            .map_err(|_| "The inference scheduler semaphore is unavailable.".to_string())? =
            Arc::new(Semaphore::new(1));
        state.model = None;
        state.accepting_requests = false;
        state.session_slots.clear();
        state.slot_owners.clear();
        state.dirty_slots.clear();
        state.use_clock = 0;
        state.slot_evictions = 0;
        state.generation = state.generation.wrapping_add(1);
        Ok(());
    }

    pub fn invalidate(&self) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.model = None;
            state.accepting_requests = false;
            state.session_slots.clear();
            state.slot_owners.clear();
            state.dirty_slots.clear();
            state.use_clock = 0;
            state.slot_evictions = 0;
            state.generation = state.generation.wrapping_add(1);
            if let Ok(semaphore) = self.inner.semaphore.lock() {
                semaphore.close();
            }
        }
    }

    pub fn status(&self) -> InferenceRuntimeStatus {
        let Ok(state) = self.inner.state.lock() else {
            return InferenceRuntimeStatus::default();
        };
        let model = state.model.as_ref();
        let parallel_slots = model.map_or(0, |loaded| loaded.parallel_slots);
        let active_requests = state.active_sessions.len();
        InferenceRuntimeStatus {
            model_loaded: model.is_some() && state.accepting_requests,
            model_path: model.map(|loaded| loaded.model_path.clone()),
            backend: model.map(|loaded| loaded.backend),
            parallel_slots,
            active_requests,
            queued_requests: state.queued_requests,
            available_slots: parallel_slots.saturating_sub(active_requests),
            configuration_fingerprint: model.map(|loaded| loaded.configuration_fingerprint.clone()),
            assigned_sessions: state.session_slots.len(),
            slot_evictions: state.slot_evictions,
        }
    }

    pub fn mark_slot_cleared(&self, session_id: &str, slot_id: usize) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        let assignment = state
            .session_slots
            .get(session_id)
            .ok_or_else(|| "The session no longer owns an inference cache slot.".to_string())?;
        if assignment.id != slot_id || !state.active_sessions.contains_key(session_id) {
            return Err("The inference cache slot assignment changed before it could be cleared.".into());
        }
        let dirty = state
            .dirty_slots
            .get_mut(slot_id)
            .ok_or_else(|| "The assigned inference cache slot is invalid.".to_string())?;
        *dirty = false;
        Ok(())
    }

    pub async fn acquire_session(
        &self,
        session_id: &str,
        request_id: &str,
        cancellation: &Notify,
    ) -> Result<InferenceSessionLease, String> {
        if session_id.is_empty()
            || session_id.len() > 128
            || !session_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err("The inference session identifier is invalid.".into());
        }
        if request_id.is_empty() || request_id.len() > 128 {
            return Err("The inference request identifier is invalid.".into());
        }

        let (generation, semaphore) = {
            let mut state = self
                .inner
                .state
                .lock()
                .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
            if !state.accepting_requests || state.model.is_none() {
                return Err(
                    "Load a Spindle managed model before sending llama.cpp requests.".into(),
                );
            }
            if state.reserved_sessions.contains(session_id) {
                return Err("This session already has an inference request in progress.".into());
            }
            if state.reserved_sessions.len() >= MAX_RESERVED_SESSIONS {
                return Err(
                    "The shared inference scheduler has reached its session queue limit.".into(),
                );
            }
            if state.queued_requests >= MAX_QUEUED_REQUESTS {
                return Err("The shared inference request queue is full. Try again after a request finishes.".into());
            }
            state.reserved_sessions.insert(session_id.to_owned());
            state.queued_requests += 1;
            let semaphore = match self.inner.semaphore.lock() {
                Ok(value) => value.clone(),
                Err(_) => {
                    state.queued_requests = state.queued_requests.saturating_sub(1);
                    state.reserved_sessions.remove(session_id);
                    return Err("The inference scheduler semaphore is unavailable.".into());
                }
            };
            (state.generation, semaphore)
        };

        let permit = tokio::select! {
            biased;
            _ = cancellation.notified() => {
                self.release_queued_session(session_id);
                return Err("Generation cancelled while waiting for an inference slot.".into());
            }
            result = semaphore.acquire_owned() => match result {
                Ok(permit) => permit,
                Err(_) => {
                    self.release_queued_session(session_id);
                    return Err("The managed inference runtime stopped while this request was queued.".into());
                }
            }
        };

        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "The inference scheduler state is unavailable.".to_string())?;
        state.queued_requests = state.queued_requests.saturating_sub(1);
        if state.generation != generation || !state.accepting_requests || state.model.is_none() {
            state.reserved_sessions.remove(session_id);
            return Err(
                "The managed inference runtime changed before this request received a slot.".into(),
            );
        }

        state.use_clock = state.use_clock.wrapping_add(1);
        let last_used = state.use_clock;
        let slot_id = if let Some(existing) = state.session_slots.get_mut(session_id) {
            existing.last_used = last_used;
            existing.id
        } else {
            let free_slot = state.slot_owners.iter().position(Option::is_none);
            let (slot_id, evicted_owner) = if let Some(slot_id) = free_slot {
                (slot_id, None)
            } else {
                let candidate = state
                    .session_slots
                    .iter()
                    .filter(|(owner, _)| !state.active_sessions.contains_key(*owner))
                    .min_by_key(|(_, assignment)| assignment.last_used)
                    .map(|(owner, assignment)| (owner.clone(), assignment.id));
                let Some((owner, slot_id)) = candidate else {
                    state.reserved_sessions.remove(session_id);
                    return Err("No idle llama.cpp KV cache slot is available for this session.".into());
                };
                (slot_id, Some(owner))
            };

            if let Some(owner) = evicted_owner {
                state.session_slots.remove(&owner);
                state.slot_evictions = state.slot_evictions.saturating_add(1);
                if let Some(dirty) = state.dirty_slots.get_mut(slot_id) {
                    *dirty = true;
                }
            }
            let Some(owner) = state.slot_owners.get_mut(slot_id) else {
                state.reserved_sessions.remove(session_id);
                return Err("The selected llama.cpp KV cache slot is invalid.".into());
            };
            *owner = Some(session_id.to_owned());
            state.session_slots.insert(
                session_id.to_owned(),
                SessionSlot {
                    id: slot_id,
                    last_used,
                },
            );
            slot_id
        };
        let Some(needs_clear) = state.dirty_slots.get(slot_id).copied() else {
            state.reserved_sessions.remove(session_id);
            return Err("The assigned llama.cpp KV cache slot is invalid.".into());
        };
        state
            .active_sessions
            .insert(session_id.to_owned(), request_id.to_owned());
        drop(state);

        Ok(InferenceSessionLease {
            inner: self.inner.clone(),
            session_id: session_id.to_owned(),
            slot_id,
            needs_clear,
            _permit: permit,
        })
    }

    fn release_queued_session(&self, session_id: &str) {
        if let Ok(mut state) = self.inner.state.lock() {
            state.queued_requests = state.queued_requests.saturating_sub(1);
            state.reserved_sessions.remove(session_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ModelConfiguration, OptimizationProfile};
    use std::time::Duration;

    fn manager_with_slots(slots: usize) -> InferenceRuntimeManager {
        let manager = InferenceRuntimeManager::default();
        let mut settings =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic).runtime;
        settings.parallel_slots = slots;
        manager
            .configure("sample.gguf", RuntimeBackend::Cpu, &settings, "test-build")
            .unwrap();
        manager
    }

    #[tokio::test]
    async fn shares_loaded_model_across_parallel_sessions_and_queues_excess() {
        let manager = manager_with_slots(2);
        let cancelled = Notify::new();
        let first = manager
            .acquire_session("session-one", "request-one", &cancelled)
            .await
            .unwrap();
        let second = manager
            .acquire_session("session-two", "request-two", &cancelled)
            .await
            .unwrap();
        assert_eq!(manager.status().active_requests, 2);
        assert_eq!(manager.status().available_slots, 0);

        let waiting_manager = manager.clone();
        let waiting_cancel = Arc::new(Notify::new());
        let waiting_cancel_for_task = waiting_cancel.clone();
        let waiting = tokio::spawn(async move {
            waiting_manager
                .acquire_session("session-three", "request-three", &waiting_cancel_for_task)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager.status().queued_requests == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        drop(first);
        let third = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(manager.status().active_requests, 2);
        assert_eq!(manager.status().queued_requests, 0);
        drop((second, third));
        assert_eq!(manager.status().active_requests, 0);
    }

    #[tokio::test]
    async fn session_slots_survive_turns_and_lru_eviction_marks_slots_dirty() {
        let manager = manager_with_slots(2);
        let cancelled = Notify::new();

        let first = manager.acquire_session("session-one", "request-one", &cancelled).await.unwrap();
        assert_eq!(first.slot_id(), 0);
        assert!(!first.needs_clear());
        drop(first);

        let second = manager.acquire_session("session-two", "request-two", &cancelled).await.unwrap();
        assert_eq!(second.slot_id(), 1);
        assert!(!second.needs_clear());
        drop(second);

        let first_again = manager.acquire_session("session-one", "request-three", &cancelled).await.unwrap();
        assert_eq!(first_again.slot_id(), 0);
        assert!(!first_again.needs_clear());
        drop(first_again);

        let third = manager.acquire_session("session-three", "request-four", &cancelled).await.unwrap();
        assert_eq!(third.slot_id(), 1);
        assert!(third.needs_clear());
        assert_eq!(manager.status().assigned_sessions, 2);
        assert_eq!(manager.status().slot_evictions, 1);
        manager.mark_slot_cleared("session-three", 1).unwrap();
        drop(third);

        let third_again = manager.acquire_session("session-three", "request-five", &cancelled).await.unwrap();
        assert_eq!(third_again.slot_id(), 1);
        assert!(!third_again.needs_clear());
        drop(third_again);
    }

    #[tokio::test]
    async fn prevents_overlapping_generations_in_the_same_session() {
        let manager = manager_with_slots(2);
        let cancelled = Notify::new();
        let _lease = manager
            .acquire_session("same-session", "request-one", &cancelled)
            .await
            .unwrap();
        let error = match manager
            .acquire_session("same-session", "request-two", &cancelled)
            .await
        {
            Ok(_) => panic!("A second request for the same session must be rejected."),
            Err(error) => error,
        };
        assert!(error.contains("already has an inference request"));
    }

    #[tokio::test]
    async fn cancellation_removes_a_waiting_session_from_the_queue() {
        let manager = manager_with_slots(1);
        let cancelled = Notify::new();
        let _active = manager
            .acquire_session("active-session", "request-one", &cancelled)
            .await
            .unwrap();
        let waiting_manager = manager.clone();
        let waiting_cancel = Arc::new(Notify::new());
        let cancel_for_task = waiting_cancel.clone();
        let waiting = tokio::spawn(async move {
            waiting_manager
                .acquire_session("waiting-session", "request-two", &cancel_for_task)
                .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if manager.status().queued_requests == 1 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        waiting_cancel.notify_one();
        let error = match waiting.await.unwrap() {
            Ok(_) => panic!("A cancelled waiting session must not acquire a slot."),
            Err(error) => error,
        };
        assert!(error.contains("cancelled"));
        assert_eq!(manager.status().queued_requests, 0);
        assert_eq!(manager.status().active_requests, 1);
    }

    #[tokio::test]
    async fn shutdown_closes_admission_until_the_model_is_reconfigured() {
        let manager = manager_with_slots(1);
        let cancelled = Notify::new();
        manager.begin_shutdown().unwrap();
        let error = match manager
            .acquire_session("session", "request", &cancelled)
            .await
        {
            Ok(_) => panic!("A request must not be admitted during runtime shutdown."),
            Err(error) => error,
        };
        assert!(error.contains("Load a Spindle managed model"));
        manager.resume();
        let lease = manager
            .acquire_session("session", "request", &cancelled)
            .await
            .unwrap();
        drop(lease);
    }

    #[tokio::test]
    async fn runtime_cannot_be_cleared_while_a_session_lease_is_active() {
        let manager = manager_with_slots(1);
        let cancelled = Notify::new();
        let lease = manager
            .acquire_session("active-session", "request-one", &cancelled)
            .await
            .unwrap();
        assert!(manager.ensure_idle().is_err());
        drop(lease);
        assert!(manager.ensure_idle().is_ok());
        manager.begin_shutdown().unwrap();
        manager.clear().unwrap();
        assert!(!manager.status().model_loaded);
    }
}
