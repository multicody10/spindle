mod agent_planner;
mod agent_runtime;
mod benchmark;
mod capabilities;
mod chat;
mod child_process;
mod domain;
mod gguf;
#[rustfmt::skip]
mod git;
mod hardware;
mod inference;
mod model_library;
mod optimizer;
#[rustfmt::skip]
mod permissions;
mod persistence;
mod projects;
mod providers;
mod runtime;
#[rustfmt::skip]
mod sandbox;
mod skills;
mod workspace;

use crate::domain::{
    AppInfo, ModelConfiguration, OptimizationProfile, OptimizationResult, SavedPreset,
};
use tauri::{AppHandle, Manager, State};

async fn with_project_database<T, F>(app: &AppHandle, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(std::path::PathBuf) -> Result<T, String> + Send + 'static,
{
    let database = database_path(app)?;
    tokio::task::spawn_blocking(move || operation(database))
        .await
        .map_err(|error| format!("Project database task failed: {error}"))?
}

#[rustfmt::skip]
#[tauri::command]
async fn git_repository_status(app: AppHandle, project_id: String) -> Result<git::GitRepositoryStatus, String> {
    let project = with_project_database(&app, move |database| projects::get_project(database, project_id)).await?;
    tokio::task::spawn_blocking(move || git::status(&project.root_path))
        .await.map_err(|error| format!("Git status task failed: {error}"))?
}

#[rustfmt::skip]
#[tauri::command]
async fn git_repository_history(app: AppHandle, project_id: String, limit: usize) -> Result<Vec<git::GitCommitSummary>, String> {
    let project = with_project_database(&app, move |database| projects::get_project(database, project_id)).await?;
    tokio::task::spawn_blocking(move || git::history(&project.root_path, limit))
        .await.map_err(|error| format!("Git history task failed: {error}"))?
}

#[rustfmt::skip]
#[tauri::command]
async fn create_agent_task_worktree(
    app: AppHandle,
    project_id: String,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<git::TaskWorktree, String> {
    let task_lookup = task_id.clone();
    let task = with_project_database(&app, move |database| projects::get_task(database, task_lookup)).await?;
    if task.project_id != project_id || task.state != "approved" {
        return Err("The approved task does not belong to this project.".into());
    }
    permissions.authorize(&task.session_id, permissions::PermissionOperation::FileChangePropose, "task_worktree:create")?;
    let project = with_project_database(&app, move |database| projects::get_project(database, project_id)).await?;
    let database = database_path(&app)?;
    let storage_root = app.path().app_data_dir().map_err(|error| format!("Could not resolve Spindle data directory: {error}"))?;
    let task_for_event = task_id.clone();
    let project_for_event = project.id.clone();
    let worktree = tokio::task::spawn_blocking(move || git::create_task_worktree(
        &database, &project.id, &project.root_path, &task_id, &storage_root,
    )).await.map_err(|error| format!("Task worktree creation failed: {error}"))??;
    let event_path = worktree.path.clone();
    let event_commit = worktree.base_commit.clone();
    with_project_database(&app, move |database| projects::append_task_event(
        database, task_for_event, "worktree_created".into(),
        serde_json::json!({"path": event_path, "baseCommit": event_commit, "projectId": project_for_event}),
    )).await?;
    Ok(worktree)
}

#[rustfmt::skip]
#[tauri::command]
async fn agent_task_worktree_diff(
    app: AppHandle,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<git::TaskWorktreeDiff, String> {
    let task_lookup = task_id.clone();
    let task = with_project_database(&app, move |database| projects::get_task(database, task_lookup)).await?;
    permissions.authorize(&task.session_id, permissions::PermissionOperation::WorkspaceRead, "task_worktree:diff")?;
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || git::worktree_diff(&database, &task_id))
        .await.map_err(|error| format!("Task worktree diff failed: {error}"))?
}

#[rustfmt::skip]
#[tauri::command]
async fn remove_agent_task_worktree(
    app: AppHandle,
    project_id: String,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<(), String> {
    let task_lookup = task_id.clone();
    let task = with_project_database(&app, move |database| projects::get_task(database, task_lookup)).await?;
    if task.project_id != project_id {
        return Err("The selected task does not belong to this project.".into());
    }
    permissions.authorize(&task.session_id, permissions::PermissionOperation::FileChangeUndo, "task_worktree:remove")?;
    let project = with_project_database(&app, move |database| projects::get_project(database, project_id)).await?;
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || git::remove_worktree(&database, &project.root_path, &task_id))
        .await.map_err(|error| format!("Task worktree removal failed: {error}"))?
}

#[rustfmt::skip]
#[tauri::command]
fn agent_sandbox_status() -> sandbox::SandboxStatus { sandbox::status() }

#[rustfmt::skip]
#[tauri::command]
async fn run_agent_task_validation(
    app: AppHandle,
    task_id: String,
    command: sandbox::ValidationCommand,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<sandbox::SandboxExecutionResult, String> {
    let task_lookup = task_id.clone();
    let task = with_project_database(&app, move |database| projects::get_task(database, task_lookup)).await?;
    if task.state != "approved" {
        return Err("Only a task with an explicitly approved plan can be validated.".into());
    }
    permissions.authorize(&task.session_id, permissions::PermissionOperation::ProcessExecution, "task_validation")?;
    let database = database_path(&app)?;
    let worktree = git::get_worktree(&database, &task_id)?;
    let task_event_id = task_id.clone();
    let command_label = command.label().to_owned();
    let worktree_path = worktree.path;
    let event_path = worktree_path.clone();
    with_project_database(&app, move |database| projects::append_task_event(
        database, task_event_id, "validation_started".into(),
        serde_json::json!({"command": command_label, "worktree": event_path}),
    )).await?;
    let result = tokio::task::spawn_blocking(move || sandbox::run_validation(&worktree_path, command))
        .await.map_err(|error| format!("Restricted validation task failed: {error}"))?;
    match result {
        Ok(output) => {
            let task_event_id = task_id.clone();
            let payload = serde_json::json!({
                "command": output.command.clone(), "exitCode": output.exit_code,
                "success": output.success, "durationMs": output.duration_ms,
                "outputTruncated": output.output_truncated, "isolation": output.isolation.clone()
            });
            with_project_database(&app, move |database| projects::append_task_event(
                database, task_event_id, "validation_finished".into(), payload,
            )).await?;
            Ok(output)
        }
        Err(error) => {
            let task_event_id = task_id;
            let reason = error.chars().take(800).collect::<String>();
            let _ = with_project_database(&app, move |database| projects::append_task_event(
                database, task_event_id, "validation_failed".into(), serde_json::json!({"reason": reason}),
            )).await;
            Err(error)
        }
    }
}

#[tauri::command]
async fn run_agent_task(
    app: AppHandle,
    input: agent_runtime::RunAgentTaskRequest,
    runtime: State<'_, agent_runtime::AgentRuntime>,
    chat_runtime: State<'_, chat::ChatRuntime>,
    managed_runtime: State<'_, runtime::ManagedRuntime>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<agent_runtime::AgentRunSummary, String> {
    if !agent_runtime::execution_worker_ready() {
        return Err(
            "Agent execution is disabled until model-authored tools run inside the authenticated Windows worker and the adversarial acceptance suite passes.".into()
        );
    }
    let database = database_path(&app)?;
    agent_runtime::run_task(
        app,
        database,
        chat_runtime.inner(),
        &managed_runtime.inference,
        permissions.inner(),
        runtime.inner(),
        input,
    )
    .await
}

#[tauri::command]
async fn cancel_agent_task(
    app: AppHandle,
    task_id: String,
    runtime: State<'_, agent_runtime::AgentRuntime>,
) -> Result<bool, String> {
    let lookup = task_id.clone();
    let _task =
        with_project_database(&app, move |database| projects::get_task(database, lookup)).await?;
    let cancelled = runtime.cancel(&task_id);
    if cancelled {
        with_project_database(&app, move |database| {
            projects::append_task_event(
                database,
                task_id,
                "cancellation_requested".into(),
                serde_json::json!({}),
            )
        })
        .await?;
    }
    Ok(cancelled)
}

#[tauri::command]
async fn apply_agent_task_changes(
    app: AppHandle,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<git::TaskApplicationResult, String> {
    let lookup = task_id.clone();
    let task =
        with_project_database(&app, move |database| projects::get_task(database, lookup)).await?;
    if task.state != "completed" {
        return Err("Only a completed agent task can be applied.".into());
    }
    permissions.authorize(
        &task.session_id,
        permissions::PermissionOperation::FileChangeApply,
        "agent_task:apply_changes",
    )?;
    let project = with_project_database(&app, move |database| {
        projects::get_project(database, task.project_id)
    })
    .await?;
    let database = database_path(&app)?;
    let result = tokio::task::spawn_blocking(move || {
        git::apply_worktree_changes(&database, &project.root_path, &task_id)
    })
    .await
    .map_err(|error| format!("Task change application failed: {error}"))??;
    let task_event_id = result.task_id.clone();
    let payload = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    with_project_database(&app, move |database| {
        projects::append_task_event(database, task_event_id, "changes_applied".into(), payload)
    })
    .await?;
    Ok(result)
}

#[tauri::command]
async fn undo_agent_task_application(
    app: AppHandle,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<git::TaskApplicationResult, String> {
    let lookup = task_id.clone();
    let task =
        with_project_database(&app, move |database| projects::get_task(database, lookup)).await?;
    permissions.authorize(
        &task.session_id,
        permissions::PermissionOperation::FileChangeUndo,
        "agent_task:undo_application",
    )?;
    let database = database_path(&app)?;
    let result =
        tokio::task::spawn_blocking(move || git::undo_worktree_application(&database, &task_id))
            .await
            .map_err(|error| format!("Task application undo failed: {error}"))??;
    let task_event_id = result.task_id.clone();
    let payload = serde_json::to_value(&result).map_err(|error| error.to_string())?;
    with_project_database(&app, move |database| {
        projects::append_task_event(database, task_event_id, "changes_undone".into(), payload)
    })
    .await?;
    Ok(result)
}

#[tauri::command]
async fn agent_task_application_status(
    app: AppHandle,
    task_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<Option<git::TaskApplicationResult>, String> {
    let lookup = task_id.clone();
    let task =
        with_project_database(&app, move |database| projects::get_task(database, lookup)).await?;
    permissions.authorize(
        &task.session_id,
        permissions::PermissionOperation::WorkspaceRead,
        "agent_task:application_status",
    )?;
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || git::worktree_application_status(&database, &task_id))
        .await
        .map_err(|error| format!("Task application status failed: {error}"))?
}

const TAURI_CONFIG: &str = include_str!("../tauri.conf.json");

fn database_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("spindle.sqlite3"))
        .map_err(|error| format!("Could not resolve Spindle application data directory: {error}"))
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Spindle".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        updater_configured: !TAURI_CONFIG.contains("REPLACE_WITH_GENERATED_TAURI_PUBLIC_KEY"),
        platform: std::env::consts::OS.into(),
    }
}

#[tauri::command]
async fn project_permission_status(
    app: AppHandle,
    session_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<permissions::SessionPermissions, String> {
    let session = with_project_database(&app, move |database| {
        projects::get_session(database, session_id)
    })
    .await?;
    permissions.register_session(session.id.clone(), session.project_id)?;
    permissions.status(&session.id)
}

#[tauri::command]
async fn grant_project_permissions(
    app: AppHandle,
    session_id: String,
    requested: Vec<permissions::PermissionScope>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<permissions::SessionPermissions, String> {
    let lookup_id = session_id.clone();
    let session = with_project_database(&app, move |database| {
        projects::get_session(database, lookup_id)
    })
    .await?;
    permissions.register_session(session.id.clone(), session.project_id)?;
    permissions.grant(&session_id, &requested)
}

#[tauri::command]
fn revoke_project_permissions(
    session_id: String,
    requested: Vec<permissions::PermissionScope>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<permissions::SessionPermissions, String> {
    permissions.revoke(&session_id, &requested)
}

#[tauri::command]
fn provider_capabilities() -> Vec<capabilities::ProviderCapabilities> {
    capabilities::registry()
}

#[tauri::command]
async fn list_projects(app: AppHandle) -> Result<Vec<projects::Project>, String> {
    with_project_database(&app, projects::list_projects).await
}

#[tauri::command]
async fn create_project(
    app: AppHandle,
    root_path: String,
    name: String,
) -> Result<projects::Project, String> {
    with_project_database(&app, move |database| {
        projects::create_project(database, root_path, name)
    })
    .await
}

#[tauri::command]
async fn rename_project(
    app: AppHandle,
    project_id: String,
    name: String,
) -> Result<projects::Project, String> {
    with_project_database(&app, move |database| {
        projects::rename_project(database, project_id, name)
    })
    .await
}

#[tauri::command]
async fn archive_project(app: AppHandle, project_id: String) -> Result<(), String> {
    with_project_database(&app, move |database| {
        projects::archive_project(database, project_id)
    })
    .await
}

async fn remove_session_task_artifacts(
    app: &AppHandle,
    database: &std::path::Path,
    session_id: &str,
    project_root: &str,
) -> Result<(), String> {
    let lookup = session_id.to_owned();
    let task_ids = with_project_database(app, move |database| {
        projects::list_task_ids(database, lookup)
    }).await?;
    let database = database.to_path_buf();
    let root = project_root.to_owned();
    tokio::task::spawn_blocking(move || {
        for task_id in task_ids {
            git::remove_task_artifacts_if_safe(&database, &root, &task_id)?;
        }
        Ok(())
    }).await.map_err(|error| format!("Task worktree cleanup failed: {error}"))?
}

#[tauri::command]
async fn delete_project(
    app: AppHandle,
    project_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<(), String> {
    let project_lookup = project_id.clone();
    let project = with_project_database(&app, move |database| {
        projects::get_project(database, project_lookup)
    }).await?;
    let project_for_sessions = project_id.clone();
    let sessions = with_project_database(&app, move |database| {
        projects::list_sessions(database, project_for_sessions)
    })
    .await?;
    // Worktrees live outside the project DB and do not cascade with task rows.
    // Remove only clean worktrees; refuse deletion rather than destroy unreviewed work.
    let database = database_path(&app)?;
    for session in &sessions {
        remove_session_task_artifacts(&app, &database, &session.id, &project.root_path).await?;
    }
    // Revoke in-memory grants before deleting the owning database records.
    for session in &sessions {
        permissions.revoke_all(&session.id)?;
    }
    with_project_database(&app, move |database| {
        projects::delete_project(database, project_id)
    })
    .await
}

#[tauri::command]
async fn list_sessions(
    app: AppHandle,
    project_id: String,
) -> Result<Vec<projects::Session>, String> {
    with_project_database(&app, move |database| {
        projects::list_sessions(database, project_id)
    })
    .await
}

#[tauri::command]
async fn create_session(
    app: AppHandle,
    project_id: String,
    title: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<projects::Session, String> {
    let session = with_project_database(&app, move |database| {
        let session = projects::create_session(database.clone(), project_id, title)?;
        let _ = projects::touch_project(database, session.project_id.clone())?;
        Ok(session)
    })
    .await?;
    permissions.register_session(session.id.clone(), session.project_id.clone())?;
    Ok(session)
}
#[tauri::command]
async fn rename_session(
    app: AppHandle,
    session_id: String,
    title: String,
) -> Result<projects::Session, String> {
    with_project_database(&app, move |database| {
        projects::rename_session(database, session_id, title)
    })
    .await
}

#[tauri::command]
async fn delete_session(
    app: AppHandle,
    session_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<(), String> {
    let lookup_id = session_id.clone();
    let session = with_project_database(&app, move |database| {
        projects::get_session(database, lookup_id)
    })
    .await?;
    let project_id = session.project_id.clone();
    let project = with_project_database(&app, move |database| {
        projects::get_project(database, project_id)
    }).await?;
    let database = database_path(&app)?;
    remove_session_task_artifacts(&app, &database, &session.id, &project.root_path).await?;
    permissions.revoke_all(&session.id)?;
    with_project_database(&app, move |database| {
        projects::delete_session(database, session_id)
    })
    .await
}
#[tauri::command]
async fn update_session_model(
    app: AppHandle,
    session_id: String,
    provider_id: String,
    model_id: String,
) -> Result<projects::Session, String> {
    with_project_database(&app, move |database| {
        projects::update_session_model(database, session_id, provider_id, model_id)
    })
    .await
}

#[tauri::command]
async fn list_session_messages(
    app: AppHandle,
    session_id: String,
) -> Result<Vec<projects::SessionMessage>, String> {
    with_project_database(&app, move |database| {
        projects::list_messages(database, session_id)
    })
    .await
}

#[tauri::command]
async fn append_session_message(
    app: AppHandle,
    session_id: String,
    role: String,
    content: String,
) -> Result<projects::SessionMessage, String> {
    with_project_database(&app, move |database| {
        projects::append_message(database, session_id, role, content)
    })
    .await
}

#[tauri::command]
async fn list_agent_tasks(
    app: AppHandle,
    session_id: String,
) -> Result<Vec<projects::AgentTask>, String> {
    with_project_database(&app, move |database| {
        projects::list_tasks(database, session_id)
    })
    .await
}

#[tauri::command]
async fn agent_task_detail(
    app: AppHandle,
    task_id: String,
) -> Result<projects::TaskDetail, String> {
    with_project_database(&app, move |database| {
        projects::task_detail(database, task_id)
    })
    .await
}

#[tauri::command]
async fn approve_agent_task_plan(
    app: AppHandle,
    task_id: String,
) -> Result<projects::AgentTask, String> {
    with_project_database(&app, move |database| {
        projects::approve_task_plan(database, task_id)
    })
    .await
}

#[tauri::command]
async fn reject_agent_task_plan(
    app: AppHandle,
    task_id: String,
) -> Result<projects::AgentTask, String> {
    with_project_database(&app, move |database| {
        projects::reject_task_plan(database, task_id)
    })
    .await
}

#[tauri::command]
async fn list_agent_task_events(
    app: AppHandle,
    task_id: String,
) -> Result<Vec<projects::AgentEvent>, String> {
    with_project_database(&app, move |database| {
        projects::list_task_events(database, task_id)
    })
    .await
}

#[tauri::command]
async fn save_project_memory(
    app: AppHandle,
    input: projects::SaveProjectMemoryInput,
) -> Result<(), String> {
    with_project_database(&app, move |database| {
        projects::save_project_memory(database, input)
    })
    .await
}
#[tauri::command]
async fn search_project_memory(
    app: AppHandle,
    project_id: String,
    query: String,
    limit: usize,
) -> Result<Vec<projects::ProjectMemory>, String> {
    with_project_database(&app, move |database| {
        projects::search_project_memory(database, project_id, query, limit)
    })
    .await
}

#[tauri::command]
async fn archive_project_memory(
    app: AppHandle,
    project_id: String,
    memory_id: String,
) -> Result<(), String> {
    with_project_database(&app, move |database| {
        projects::archive_project_memory(database, project_id, memory_id)
    })
    .await
}

#[tauri::command]
async fn list_agent_skills(
    app: AppHandle,
    project_id: String,
) -> Result<Vec<skills::AgentSkill>, String> {
    let project = with_project_database(&app, move |database| {
        projects::get_project(database, project_id)
    })
    .await?;
    tokio::task::spawn_blocking(move || skills::discover(std::path::Path::new(&project.root_path)))
        .await
        .map_err(|error| format!("Agent Skill discovery task failed: {error}"))?
}

#[tauri::command]
async fn load_agent_skill(
    app: AppHandle,
    project_id: String,
    name: String,
) -> Result<skills::LoadedSkill, String> {
    let project = with_project_database(&app, move |database| {
        projects::get_project(database, project_id)
    })
    .await?;
    tokio::task::spawn_blocking(move || {
        skills::load(std::path::Path::new(&project.root_path), &name)
    })
    .await
    .map_err(|error| format!("Agent Skill loading task failed: {error}"))?
}

#[tauri::command]
async fn plan_project_task(
    app: AppHandle,
    runtime: State<'_, chat::ChatRuntime>,
    managed_runtime: State<'_, runtime::ManagedRuntime>,
    permissions: State<'_, permissions::PermissionBroker>,
    input: agent_planner::PlanTaskRequest,
) -> Result<projects::TaskDetail, String> {
    let database = database_path(&app)?;
    let lookup_session = input.session_id.clone();
    let project_id = input.project_id.clone();
    let session = with_project_database(&app, move |database| {
        projects::get_session(database, lookup_session)
    })
    .await?;
    if session.project_id != project_id {
        return Err("The selected session does not belong to this project.".into());
    }
    permissions.register_session(session.id.clone(), session.project_id.clone())?;
    permissions.require(&session.id, permissions::PermissionScope::ProjectRead)?;
    agent_planner::create_read_only_plan(
        app,
        database,
        runtime.inner(),
        &managed_runtime.inference,
        input,
    )
    .await
}
#[tauri::command]
async fn run_model_benchmark(
    app: AppHandle,
    runtime: State<'_, chat::ChatRuntime>,
    managed_runtime: State<'_, runtime::ManagedRuntime>,
    input: benchmark::BenchmarkRequest,
) -> Result<benchmark::BenchmarkRun, String> {
    let database = database_path(&app)?;
    benchmark::run(
        app,
        database,
        runtime.inner(),
        &managed_runtime.inference,
        input,
    )
    .await
}
#[tauri::command]
async fn list_model_benchmarks(
    app: AppHandle,
    limit: usize,
) -> Result<Vec<benchmark::BenchmarkRun>, String> {
    with_project_database(&app, move |database| benchmark::list_runs(database, limit)).await
}

#[tauri::command]
async fn local_provider_statuses() -> Result<Vec<providers::LocalProviderStatus>, String> {
    providers::discover().await
}

#[tauri::command]
fn workspace_status(
    workspace: State<'_, workspace::WorkspaceManager>,
) -> Result<Option<workspace::WorkspaceGrant>, String> {
    workspace.status()
}

#[tauri::command]
fn grant_workspace_access(
    path: String,
    allow_write: bool,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceGrant, String> {
    let previous = workspace.status()?.map(|grant| grant.session_id);
    let grant = workspace.grant(path, allow_write)?;
    if let Some(previous_id) = previous {
        if previous_id != grant.session_id {
            let _ = permissions.revoke_all(&previous_id);
        }
    }
    let registration = permissions
        .register_session(grant.session_id.clone(), grant.root_path.clone())
        .and_then(|_| {
            if allow_write {
                permissions.grant(
                    &grant.session_id,
                    &[permissions::PermissionScope::ProjectWrite],
                )?;
            }
            Ok(())
        });
    if let Err(error) = registration {
        let _ = workspace.revoke(&grant.session_id);
        let _ = permissions.revoke_all(&grant.session_id);
        return Err(error);
    }
    Ok(grant)
}

#[tauri::command]
fn revoke_workspace_access(
    session_id: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<(), String> {
    workspace.revoke(&session_id)?;
    permissions.revoke_all(&session_id)
}

#[tauri::command]
fn permission_audit_log(
    session_id: String,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<Vec<permissions::PermissionAuditEvent>, String> {
    permissions.audit_log(&session_id)
}

#[tauri::command]
async fn list_workspace_directory(
    session_id: String,
    relative_path: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceListing, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::WorkspaceRead,
        &relative_path,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.list_directory(&session_id, &relative_path))
        .await
        .map_err(|error| format!("Workspace listing task failed: {error}"))?
}

#[tauri::command]
async fn read_workspace_file(
    session_id: String,
    relative_path: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceFile, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::WorkspaceRead,
        &relative_path,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.read_file(&session_id, &relative_path))
        .await
        .map_err(|error| format!("Workspace read task failed: {error}"))?
}

#[tauri::command]
async fn stage_workspace_file_change(
    session_id: String,
    relative_path: String,
    content: String,
    expected_revision: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceChangeProposal, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::FileChangePropose,
        &relative_path,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || {
        manager.stage_file_change(&session_id, &relative_path, content, &expected_revision)
    })
    .await
    .map_err(|error| format!("Workspace change staging task failed: {error}"))?
}

#[tauri::command]
async fn apply_workspace_file_change(
    session_id: String,
    proposal_id: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceWriteResult, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::FileChangeApply,
        &proposal_id,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.apply_file_change(&session_id, &proposal_id))
        .await
        .map_err(|error| format!("Workspace change apply task failed: {error}"))?
}

#[tauri::command]
async fn undo_workspace_file_change(
    session_id: String,
    proposal_id: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceWriteResult, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::FileChangeUndo,
        &proposal_id,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.undo_file_change(&session_id, &proposal_id))
        .await
        .map_err(|error| format!("Workspace change undo task failed: {error}"))?
}

#[tauri::command]
async fn cancel_workspace_file_change(
    session_id: String,
    proposal_id: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<(), String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::FileChangePropose,
        &proposal_id,
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.cancel_file_change(&session_id, &proposal_id))
        .await
        .map_err(|error| format!("Workspace change discard task failed: {error}"))?
}

#[tauri::command]
async fn search_workspace(
    session_id: String,
    query: String,
    workspace: State<'_, workspace::WorkspaceManager>,
    permissions: State<'_, permissions::PermissionBroker>,
) -> Result<workspace::WorkspaceSearchResult, String> {
    permissions.authorize(
        &session_id,
        permissions::PermissionOperation::WorkspaceRead,
        "",
    )?;
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.search(&session_id, &query))
        .await
        .map_err(|error| format!("Workspace search task failed: {error}"))?
}

#[tauri::command]
async fn managed_runtime_status(
    app: AppHandle,
    runtime: State<'_, runtime::ManagedRuntime>,
) -> Result<runtime::RuntimeStatus, String> {
    runtime::status(&app, &runtime).await
}

#[tauri::command]
async fn install_llama_runtime(
    app: AppHandle,
    runtime: State<'_, runtime::ManagedRuntime>,
    backend: runtime::RuntimeBackend,
) -> Result<runtime::BackendInstallStatus, String> {
    runtime::install(app, &runtime, backend).await
}

#[tauri::command]
async fn start_llama_runtime(
    app: AppHandle,
    runtime: State<'_, runtime::ManagedRuntime>,
    model_path: String,
    configuration: ModelConfiguration,
    backend: runtime::RuntimeBackend,
) -> Result<runtime::RuntimeStatus, String> {
    runtime::start(app, &runtime, model_path, configuration, backend).await
}

#[tauri::command]
async fn stop_llama_runtime(
    app: AppHandle,
    runtime: State<'_, runtime::ManagedRuntime>,
) -> Result<runtime::RuntimeStatus, String> {
    runtime::stop(&runtime)?;
    runtime::status(&app, &runtime).await
}

#[tauri::command]
async fn start_chat(
    app: AppHandle,
    runtime: State<'_, chat::ChatRuntime>,
    managed_runtime: State<'_, runtime::ManagedRuntime>,
    request: chat::ChatRequest,
) -> Result<chat::ChatResult, String> {
    validate_chat_request(&request)?;
    let request_id = request.request_id.clone();
    let cancellation = runtime.begin(&request_id)?;
    let result = chat::run(app, request, cancellation, &managed_runtime.inference).await;
    runtime.finish(&request_id);
    result
}

#[tauri::command]
fn cancel_chat(request_id: String, runtime: State<'_, chat::ChatRuntime>) -> bool {
    if !is_valid_chat_request_id(&request_id) {
        return false;
    }
    runtime.cancel(&request_id)
}

fn is_valid_chat_request_id(request_id: &str) -> bool {
    request_id.len() == 36
        && request_id
            .chars()
            .all(|character| character.is_ascii_hexdigit() || character == '-')
}

pub(crate) fn validate_chat_request(request: &chat::ChatRequest) -> Result<(), String> {
    if request.session_id.is_empty()
        || request.session_id.len() > 128
        || !request
            .session_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("The inference session identifier is invalid.".into());
    }
    if !is_valid_chat_request_id(&request.request_id) {
        return Err("The chat request identifier is invalid.".into());
    }
    if request.model.trim().is_empty() || request.model.len() > 256 {
        return Err("Select a model with a valid identifier.".into());
    }
    if request.messages.is_empty() || request.messages.len() > 64 {
        return Err("A chat must contain between 1 and 64 messages.".into());
    }
    let mut total_bytes = 0usize;
    for message in &request.messages {
        if !matches!(message.role.as_str(), "system" | "user" | "assistant") {
            return Err("Chat messages may only use system, user, or assistant roles.".into());
        }
        if message.content.trim().is_empty() || message.content.len() > 64 * 1024 {
            return Err("Each chat message must contain 1 to 65,536 bytes.".into());
        }
        total_bytes = total_bytes.saturating_add(message.content.len());
    }
    if total_bytes > 256 * 1024 {
        return Err("The combined chat history exceeds the 256 KiB request limit.".into());
    }
    if request
        .messages
        .last()
        .is_none_or(|message| message.role != "user")
    {
        return Err("The final chat message must come from the user.".into());
    }

    let generation = &request.generation;
    if request.context_tokens == 0
        || request.context_tokens > 131_072
        || generation.max_output_tokens == 0
        || generation.max_output_tokens > 32_768
        || generation.max_output_tokens > request.context_tokens
    {
        return Err("Context and output limits are outside the supported range.".into());
    }
    if !generation.temperature.is_finite()
        || !(0.0..=5.0).contains(&generation.temperature)
        || !generation.top_p.is_finite()
        || !(0.0..=1.0).contains(&generation.top_p)
        || !generation.min_p.is_finite()
        || !(0.0..=1.0).contains(&generation.min_p)
        || !generation.typical_p.is_finite()
        || !(0.0..=1.0).contains(&generation.typical_p)
        || !generation.repeat_penalty.is_finite()
        || !(0.1..=3.0).contains(&generation.repeat_penalty)
        || !generation.presence_penalty.is_finite()
        || !(-2.0..=2.0).contains(&generation.presence_penalty)
        || !generation.frequency_penalty.is_finite()
        || !(-2.0..=2.0).contains(&generation.frequency_penalty)
        || generation.top_k > 1_000
        || generation.repeat_last_n < -1
        || generation.repeat_last_n > 8_192
        || generation.stop_sequences.len() > 32
        || generation
            .stop_sequences
            .iter()
            .any(|sequence| sequence.len() > 512)
    {
        return Err("One or more generation settings are outside the supported range.".into());
    }

    Ok(())
}

#[tauri::command]
async fn hardware_profile() -> Result<domain::HardwareProfile, String> {
    tokio::task::spawn_blocking(hardware::detect)
        .await
        .map_err(|error| format!("Hardware inspection task failed: {error}"))
}

#[tauri::command]
async fn inspect_model(path: String) -> Result<domain::ModelMetadata, String> {
    tokio::task::spawn_blocking(move || gguf::inspect(&path))
        .await
        .map_err(|error| format!("Model inspection task failed: {error}"))?
}

#[tauri::command]
async fn list_model_library(app: AppHandle) -> Result<model_library::ModelLibrary, String> {
    let documents_directory = app
        .path()
        .document_dir()
        .map_err(|error| format!("Could not resolve the Documents directory: {error}"))?;
    tokio::task::spawn_blocking(move || model_library::discover(documents_directory))
        .await
        .map_err(|error| format!("Model library scan task failed: {error}"))?
}

#[tauri::command]
async fn optimize_model(
    path: String,
    profile: OptimizationProfile,
) -> Result<OptimizationResult, String> {
    tokio::task::spawn_blocking(move || {
        let model = gguf::inspect(&path)?;
        let hardware = hardware::detect();
        Ok(optimizer::optimize(model, hardware, profile))
    })
    .await
    .map_err(|error| format!("Optimization task failed: {error}"))?
}

#[tauri::command]
async fn load_model_configuration(
    app: AppHandle,
    model_path: String,
    profile: OptimizationProfile,
) -> Result<Option<ModelConfiguration>, String> {
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || {
        persistence::load_configuration(database, model_path, profile)
    })
    .await
    .map_err(|error| format!("Configuration read task failed: {error}"))?
}

#[tauri::command]
async fn save_model_configuration(
    app: AppHandle,
    configuration: ModelConfiguration,
) -> Result<(), String> {
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || {
        validate_configuration(&configuration)?;
        validate_model_constraints(&configuration)?;
        persistence::save_configuration(database, configuration)
    })
    .await
    .map_err(|error| format!("Configuration write task failed: {error}"))?
}

#[tauri::command]
async fn save_preset(
    app: AppHandle,
    name: String,
    configuration: ModelConfiguration,
) -> Result<(), String> {
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || {
        validate_configuration(&configuration)?;
        validate_model_constraints(&configuration)?;
        persistence::save_preset(database, name, configuration)
    })
    .await
    .map_err(|error| format!("Preset write task failed: {error}"))?
}

#[tauri::command]
async fn list_presets(app: AppHandle) -> Result<Vec<SavedPreset>, String> {
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || persistence::list_presets(database))
        .await
        .map_err(|error| format!("Preset read task failed: {error}"))?
}

#[tauri::command]
async fn delete_preset(app: AppHandle, name: String) -> Result<(), String> {
    let database = database_path(&app)?;
    tokio::task::spawn_blocking(move || persistence::delete_preset(database, name))
        .await
        .map_err(|error| format!("Preset delete task failed: {error}"))?
}

fn validate_model_constraints(
    configuration: &ModelConfiguration,
) -> Result<domain::ModelMetadata, String> {
    let metadata = gguf::inspect(&configuration.model_path)?;
    if let Some(limit) = metadata.context_length {
        if configuration.runtime.context_tokens > limit {
            return Err(format!(
                "Configured context length exceeds the model's declared limit of {limit} tokens."
            ));
        }
    }
    if let Some(layers) = metadata.layer_count {
        if configuration.runtime.gpu_layers > layers {
            return Err(format!(
                "GPU offload cannot exceed the model's {layers} layers."
            ));
        }
    }
    Ok(metadata)
}

fn validate_configuration(configuration: &ModelConfiguration) -> Result<(), String> {
    let runtime = &configuration.runtime;
    let generation = &configuration.generation;
    if runtime.context_tokens == 0 || runtime.context_tokens > 131072 {
        return Err("Context length must be between 1 and 131,072 tokens.".into());
    }
    if runtime.gpu_layers > 512 {
        return Err("GPU layer count must be between 0 and 512.".into());
    }
    if runtime.threads == 0
        || runtime.threads > 256
        || runtime.batch_threads == 0
        || runtime.batch_threads > 256
    {
        return Err("CPU thread counts must be between 1 and 256.".into());
    }
    if runtime.batch_size == 0
        || runtime.batch_size > 8192
        || runtime.micro_batch_size == 0
        || runtime.micro_batch_size > runtime.batch_size
    {
        return Err(
            "Batch sizes are outside the supported range or micro batch exceeds evaluation batch."
                .into(),
        );
    }
    if runtime.parallel_slots == 0 || runtime.parallel_slots > 32 {
        return Err("Parallel slots must be between 1 and 32.".into());
    }
    if generation.max_output_tokens > runtime.context_tokens {
        return Err("Maximum output tokens cannot exceed the configured context length.".into());
    }
    if !generation.temperature.is_finite()
        || !(0.0..=5.0).contains(&generation.temperature)
        || !generation.top_p.is_finite()
        || !(0.0..=1.0).contains(&generation.top_p)
        || !generation.min_p.is_finite()
        || !(0.0..=1.0).contains(&generation.min_p)
        || !generation.typical_p.is_finite()
        || !(0.0..=1.0).contains(&generation.typical_p)
        || !generation.repeat_penalty.is_finite()
        || !(0.1..=3.0).contains(&generation.repeat_penalty)
        || !generation.presence_penalty.is_finite()
        || !(-2.0..=2.0).contains(&generation.presence_penalty)
        || !generation.frequency_penalty.is_finite()
        || !(-2.0..=2.0).contains(&generation.frequency_penalty)
        || generation.max_output_tokens == 0
        || generation.max_output_tokens > 131072
        || generation.top_k > 1000
        || generation.repeat_last_n < -1
        || generation.repeat_last_n > 8192
        || generation.stop_sequences.len() > 32
        || generation
            .stop_sequences
            .iter()
            .any(|value| value.len() > 512)
    {
        return Err("One or more generation parameters are outside the supported range.".into());
    }
    if !matches!(runtime.kv_cache_k_type.as_str(), "f16" | "q8_0" | "q4_0")
        || !matches!(runtime.kv_cache_v_type.as_str(), "f16" | "q8_0" | "q4_0")
    {
        return Err("Unsupported KV-cache data type.".into());
    }
    if configuration.model_path.trim().is_empty() {
        return Err("A model path is required before saving a model configuration.".into());
    }
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .manage(chat::ChatRuntime::default())
        .manage(agent_runtime::AgentRuntime::default())
        .manage(runtime::ManagedRuntime::default())
        .manage(workspace::WorkspaceManager::default())
        .manage(permissions::PermissionBroker::default())
        .setup(|app| {
            let database = app
                .path()
                .app_data_dir()
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .join("spindle.sqlite3");
            app.state::<permissions::PermissionBroker>()
                .set_audit_database(database.clone())
                .map_err(std::io::Error::other)?;
            let interrupted = projects::recover_agent_runs(database.clone())
                .map_err(std::io::Error::other)?;
            let recovered = git::recover_worktree_applications(&database)
                .map_err(std::io::Error::other)?;
            if interrupted > 0 || recovered > 0 {
                eprintln!("Spindle recovered {interrupted} interrupted agent run(s) and {recovered} task application journal(s).");
            }
            Ok(())
        })
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            app_info,
            provider_capabilities,
            project_permission_status,
            grant_project_permissions,
            revoke_project_permissions,
            list_projects,
            create_project,
            rename_project,
            archive_project,
            delete_project,
            list_sessions,
            create_session,
            rename_session,
            delete_session,
            update_session_model,
            list_session_messages,
            append_session_message,
            list_agent_tasks,
            agent_task_detail,
            approve_agent_task_plan,
            reject_agent_task_plan,
            list_agent_task_events,
            save_project_memory,
            search_project_memory,
            archive_project_memory,
            list_agent_skills,
            load_agent_skill,
            plan_project_task,
            run_model_benchmark,
            list_model_benchmarks,
            local_provider_statuses,
            workspace_status,
            grant_workspace_access,
            revoke_workspace_access,
            list_workspace_directory,
            read_workspace_file,
            stage_workspace_file_change,
            apply_workspace_file_change,
            undo_workspace_file_change,
            cancel_workspace_file_change,
            permission_audit_log,
            git_repository_status,
            git_repository_history,
            create_agent_task_worktree,
            agent_task_worktree_diff,
            remove_agent_task_worktree,
            agent_sandbox_status,
            run_agent_task_validation,
            run_agent_task,
            cancel_agent_task,
            apply_agent_task_changes,
            undo_agent_task_application,
            agent_task_application_status,
            search_workspace,
            managed_runtime_status,
            install_llama_runtime,
            start_llama_runtime,
            stop_llama_runtime,
            start_chat,
            cancel_chat,
            hardware_profile,
            inspect_model,
            list_model_library,
            optimize_model,
            load_model_configuration,
            save_model_configuration,
            save_preset,
            list_presets,
            delete_preset
        ])
        .run(tauri::generate_context!())
        .expect("error while running Spindle");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_local_chat_requests() {
        let configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        let request = chat::ChatRequest {
            request_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            session_id: "123e4567-e89b-12d3-a456-426614174001".into(),
            provider_id: chat::ChatProvider::Ollama,
            model: "qwen3.5:9b".into(),
            messages: vec![chat::ChatMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            context_tokens: 4096,
            generation: configuration.generation,
        };
        assert!(validate_chat_request(&request).is_ok());
    }

    #[test]
    fn rejects_non_user_final_chat_message() {
        let configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        let request = chat::ChatRequest {
            request_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            session_id: "123e4567-e89b-12d3-a456-426614174001".into(),
            provider_id: chat::ChatProvider::Ollama,
            model: "qwen3.5:9b".into(),
            messages: vec![chat::ChatMessage {
                role: "assistant".into(),
                content: "Not a user turn".into(),
            }],
            context_tokens: 4096,
            generation: configuration.generation,
        };
        assert!(validate_chat_request(&request)
            .unwrap_err()
            .contains("final chat message"));
    }

    #[test]
    fn rejects_oversized_chat_history() {
        let configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        let request = chat::ChatRequest {
            request_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            session_id: "123e4567-e89b-12d3-a456-426614174001".into(),
            provider_id: chat::ChatProvider::Ollama,
            model: "qwen3.5:9b".into(),
            messages: vec![chat::ChatMessage {
                role: "user".into(),
                content: "x".repeat(65_537),
            }],
            context_tokens: 4096,
            generation: configuration.generation,
        };
        assert!(validate_chat_request(&request).is_err());
    }

    #[test]
    fn accepts_the_default_model_configuration() {
        let configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        assert!(validate_configuration(&configuration).is_ok());
    }

    #[test]
    fn rejects_output_budget_larger_than_context() {
        let mut configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        configuration.runtime.context_tokens = 2048;
        configuration.generation.max_output_tokens = 4096;
        let error = validate_configuration(&configuration).unwrap_err();
        assert!(error.contains("cannot exceed"));
    }

    #[test]
    fn rejects_micro_batch_larger_than_batch() {
        let mut configuration =
            ModelConfiguration::new("sample.gguf".into(), OptimizationProfile::Automatic);
        configuration.runtime.batch_size = 64;
        configuration.runtime.micro_batch_size = 128;
        assert!(validate_configuration(&configuration).is_err());
    }
}
