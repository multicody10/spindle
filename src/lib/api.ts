import { invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  HardwareProfile,
  LocalProviderStatus,
  ChatRequest,
  ChatResult,
  ModelConfiguration,
  ModelMetadata,
  OptimizationProfile,
  OptimizationResult,
  SavedPreset
} from "./types";

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  hardware: () => invoke<HardwareProfile>("hardware_profile"),
  localProviders: () => invoke<LocalProviderStatus[]>("local_provider_statuses"),
  startChat: (request: ChatRequest) => invoke<ChatResult>("start_chat", { request }),
  cancelChat: (requestId: string) => invoke<boolean>("cancel_chat", { requestId }),
  inspectModel: (path: string) => invoke<ModelMetadata>("inspect_model", { path }),
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
