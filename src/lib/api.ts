import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  HardwareProfile,
  LocalProviderStatus,
  ChatRequest,
  ChatResult,
  RuntimeBackend,
  RuntimeStatus,
  BackendInstallStatus,
  ModelConfiguration,
  ModelMetadata,
  ModelLibrary,
  OptimizationProfile,
  OptimizationResult,
  SavedPreset,
  Project,
  ProjectSession,
  PersistedSessionMessage,
  AgentTask,
  AgentTaskDetail,
  AgentTaskEvent,
  AgentSkill,
  LoadedAgentSkill,
  ProjectMemory,
  ProviderCapabilities,
  SessionPermissions,
  PermissionScope,
  GenerationSettings,
  WorkspaceGrant,
  WorkspaceListing,
  WorkspaceFile,
  WorkspaceWriteResult,
  WorkspaceSearchResult
} from "./types";

export const api = {
  providerCapabilities: () => invoke<ProviderCapabilities[]>("provider_capabilities"),
  projectPermissionStatus: (sessionId: string) => invoke<SessionPermissions>("project_permission_status", { sessionId }),
  grantProjectPermissions: (sessionId: string, requested: PermissionScope[]) => invoke<SessionPermissions>("grant_project_permissions", { sessionId, requested }),
  revokeProjectPermissions: (sessionId: string, requested: PermissionScope[]) => invoke<SessionPermissions>("revoke_project_permissions", { sessionId, requested }),
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (rootPath: string, name: string) => invoke<Project>("create_project", { rootPath, name }),
  renameProject: (projectId: string, name: string) => invoke<Project>("rename_project", { projectId, name }),
  archiveProject: (projectId: string) => invoke<void>("archive_project", { projectId }),
  deleteProject: (projectId: string) => invoke<void>("delete_project", { projectId }),
  listSessions: (projectId: string) => invoke<ProjectSession[]>("list_sessions", { projectId }),
  createSession: (projectId: string, title = "New session") => invoke<ProjectSession>("create_session", { projectId, title }),
  renameSession: (sessionId: string, title: string) => invoke<ProjectSession>("rename_session", { sessionId, title }),
  deleteSession: (sessionId: string) => invoke<void>("delete_session", { sessionId }),
  updateSessionModel: (sessionId: string, providerId: string, modelId: string) => invoke<ProjectSession>("update_session_model", { sessionId, providerId, modelId }),
  listSessionMessages: (sessionId: string) => invoke<PersistedSessionMessage[]>("list_session_messages", { sessionId }),
  appendSessionMessage: (sessionId: string, role: PersistedSessionMessage["role"], content: string) =>
    invoke<PersistedSessionMessage>("append_session_message", { sessionId, role, content }),
  listAgentTasks: (sessionId: string) => invoke<AgentTask[]>("list_agent_tasks", { sessionId }),
  agentTaskDetail: (taskId: string) => invoke<AgentTaskDetail>("agent_task_detail", { taskId }),
  approveAgentTaskPlan: (taskId: string) => invoke<AgentTask>("approve_agent_task_plan", { taskId }),
  rejectAgentTaskPlan: (taskId: string) => invoke<AgentTask>("reject_agent_task_plan", { taskId }),
  listAgentTaskEvents: (taskId: string) => invoke<AgentTaskEvent[]>("list_agent_task_events", { taskId }),
  planProjectTask: (
    projectId: string, sessionId: string, prompt: string, providerId: string,
    model: string, contextTokens: number, generation: GenerationSettings
  ) => invoke<AgentTaskDetail>("plan_project_task", { projectId, sessionId, prompt, providerId, model, contextTokens, generation }),
  listAgentSkills: (projectId: string) => invoke<AgentSkill[]>("list_agent_skills", { projectId }),
  loadAgentSkill: (projectId: string, name: string) => invoke<LoadedAgentSkill>("load_agent_skill", { projectId, name }),
  searchProjectMemory: (projectId: string, query: string, limit = 8) =>
    invoke<ProjectMemory[]>("search_project_memory", { projectId, query, limit }),
  appInfo: () => invoke<AppInfo>("app_info"),
  hardware: () => invoke<HardwareProfile>("hardware_profile"),
  localProviders: () => invoke<LocalProviderStatus[]>("local_provider_statuses"),
  startChat: (request: ChatRequest) => invoke<ChatResult>("start_chat", { request }),
  cancelChat: (requestId: string) => invoke<boolean>("cancel_chat", { requestId }),
  runtimeStatus: () => invoke<RuntimeStatus>("managed_runtime_status"),
  installRuntime: (backend: RuntimeBackend) =>
    invoke<BackendInstallStatus>("install_llama_runtime", { backend }),
  startRuntime: (modelPath: string, configuration: ModelConfiguration, backend: RuntimeBackend) =>
    invoke<RuntimeStatus>("start_llama_runtime", { modelPath, configuration, backend }),
  stopRuntime: () => invoke<RuntimeStatus>("stop_llama_runtime"),
  workspaceStatus: () => invoke<WorkspaceGrant | null>("workspace_status"),
  grantWorkspace: (path: string, allowWrite: boolean) =>
    invoke<WorkspaceGrant>("grant_workspace_access", { path, allowWrite }),
  revokeWorkspace: (sessionId: string) =>
    invoke<void>("revoke_workspace_access", { sessionId }),
  listWorkspace: (sessionId: string, relativePath: string) =>
    invoke<WorkspaceListing>("list_workspace_directory", { sessionId, relativePath }),
  readWorkspaceFile: (sessionId: string, relativePath: string) =>
    invoke<WorkspaceFile>("read_workspace_file", { sessionId, relativePath }),
  writeWorkspaceFile: (sessionId: string, relativePath: string, content: string, expectedRevision: string) =>
    invoke<WorkspaceWriteResult>("write_workspace_file", { sessionId, relativePath, content, expectedRevision }),
  searchWorkspace: (sessionId: string, query: string) =>
    invoke<WorkspaceSearchResult>("search_workspace", { sessionId, query }),
  inspectModel: (path: string) => invoke<ModelMetadata>("inspect_model", { path }),
  modelLibrary: () => invoke<ModelLibrary>("list_model_library"),
  optimizeModel: (path: string, profile: OptimizationProfile) =>
    invoke<OptimizationResult>("optimize_model", { path, profile }),
  loadConfiguration: (modelPath: string, profile: OptimizationProfile) =>
    invoke<ModelConfiguration | null>("load_model_configuration", { modelPath, profile }),
  saveConfiguration: (configuration: ModelConfiguration) =>
    invoke<void>("save_model_configuration", { configuration }),
  savePreset: (name: string, configuration: ModelConfiguration) =>
    invoke<void>("save_preset", { name, configuration }),
  listPresets: () => invoke<SavedPreset[]>("list_presets"),
  deletePreset: (name: string) => invoke<void>("delete_preset", { name })
};
