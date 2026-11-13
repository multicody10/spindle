mod agent_planner;
mod benchmark;
mod capabilities;
mod chat;
mod domain;
mod gguf;
mod hardware;
mod model_library;
mod optimizer;
mod permissions;
mod persistence;
mod projects;
mod providers;
mod runtime;
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

#[tauri::command]
async fn delete_project(app: AppHandle, project_id: String) -> Result<(), String> {
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
    let permission_session_id = session_id.clone();
    with_project_database(&app, move |database| {
        projects::delete_session(database, session_id)
    })
    .await?;
    permissions.revoke_all(&permission_session_id)
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
    agent_planner::create_read_only_plan(app, database, runtime.inner(), input).await
}
#[tauri::command]
async fn run_model_benchmark(
    app: AppHandle,
    runtime: State<'_, chat::ChatRuntime>,
    input: benchmark::BenchmarkRequest,
) -> Result<benchmark::BenchmarkRun, String> {
    let database = database_path(&app)?;
    benchmark::run(app, database, runtime.inner(), input).await
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
) -> Result<workspace::WorkspaceGrant, String> {
    workspace.grant(path, allow_write)
}

#[tauri::command]
fn revoke_workspace_access(
    session_id: String,
    workspace: State<'_, workspace::WorkspaceManager>,
) -> Result<(), String> {
    workspace.revoke(&session_id)
}

#[tauri::command]
async fn list_workspace_directory(
    session_id: String,
    relative_path: String,
    workspace: State<'_, workspace::WorkspaceManager>,
) -> Result<workspace::WorkspaceListing, String> {
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
) -> Result<workspace::WorkspaceFile, String> {
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || manager.read_file(&session_id, &relative_path))
        .await
        .map_err(|error| format!("Workspace read task failed: {error}"))?
}

#[tauri::command]
async fn write_workspace_file(
    session_id: String,
    relative_path: String,
    content: String,
    expected_revision: String,
    workspace: State<'_, workspace::WorkspaceManager>,
) -> Result<workspace::WorkspaceWriteResult, String> {
    let manager = workspace.inner().clone();
    tokio::task::spawn_blocking(move || {
        manager.write_file(&session_id, &relative_path, content, &expected_revision)
    })
    .await
    .map_err(|error| format!("Workspace write task failed: {error}"))?
}

#[tauri::command]
async fn search_workspace(
    session_id: String,
    query: String,
    workspace: State<'_, workspace::WorkspaceManager>,
) -> Result<workspace::WorkspaceSearchResult, String> {
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
    request: chat::ChatRequest,
) -> Result<chat::ChatResult, String> {
    validate_chat_request(&request)?;
    let request_id = request.request_id.clone();
    let cancellation = runtime.begin(&request_id)?;
    let result = chat::run(app, request, cancellation).await;
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

fn validate_model_constraints(configuration: &ModelConfiguration) -> Result<(), String> {
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
    Ok(())
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
        .manage(runtime::ManagedRuntime::default())
        .manage(workspace::WorkspaceManager::default())
        .manage(permissions::PermissionBroker::default())
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
            write_workspace_file,
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
