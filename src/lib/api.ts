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
  WorkspaceGrant,
  WorkspaceListing,
  WorkspaceFile,
  WorkspaceWriteResult,
  WorkspaceSearchResult
} from "./types";

export const api = {
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
