export type OptimizationProfile =
  | "automatic"
  | "maximum_performance"
  | "balanced"
  | "memory_efficient"
  | "maximum_context"
  | "custom";

export interface GpuDevice {
  vendor: string;
  name: string;
  totalVramBytes: number;
  availableVramBytes: number;
  detectionMethod: string;
}

export interface HardwareProfile {
  os: string;
  cpuName: string | null;
  logicalThreads: number;
  physicalCores: number | null;
  ramTotalBytes: number;
  ramAvailableBytes: number;
  gpus: GpuDevice[];
  detectionNotes: string[];
}

export interface LocalProviderModel {
  id: string;
  parameterSize: string | null;
  quantization: string | null;
}

export interface LocalProviderStatus {
  id: string;
  name: string;
  endpoint: string;
  available: boolean;
  latencyMs: number | null;
  models: LocalProviderModel[];
  message: string;
}

export interface ModelMetadata {
  path: string;
  fileSizeBytes: number;
  ggufVersion: number;
  tensorCount: number;
  architecture: string | null;
  layerCount: number | null;
  contextLength: number | null;
  embeddingLength: number | null;
  attentionHeads: number | null;
  keyValueHeads: number | null;
  keyLength: number | null;
  valueLength: number | null;
  fileType: number | null;
  quantization: string | null;
  kvBytesPerToken: number | null;
}

export interface RuntimeSettings {
  contextTokens: number;
  gpuLayers: number;
  threads: number;
  batchThreads: number;
  batchSize: number;
  microBatchSize: number;
  parallelSlots: number;
  flashAttention: boolean;
  kvCacheKType: string;
  kvCacheVType: string;
  kvCacheOnGpu: boolean;
  useMemoryMapping: boolean;
  lockMemory: boolean;
}

export interface GenerationSettings {
  temperature: number;
  topP: number;
  topK: number;
  minP: number;
  typicalP: number;
  repeatPenalty: number;
  repeatLastN: number;
  presencePenalty: number;
  frequencyPenalty: number;
  maxOutputTokens: number;
  seed: number | null;
  stopSequences: string[];
}

export interface ModelConfiguration {
  modelPath: string;
  profile: OptimizationProfile;
  runtime: RuntimeSettings;
  generation: GenerationSettings;
  optimizationNotes: string[];
  optimizerRevision: number;
}

export interface OptimizationResult {
  model: ModelMetadata;
  hardware: HardwareProfile;
  configuration: ModelConfiguration;
  warnings: string[];
}

export interface AppInfo {
  name: string;
  version: string;
  updaterConfigured: boolean;
  platform: string;
}

export interface SavedPreset {
  name: string;
  configuration: ModelConfiguration;
  updatedAt: number;
}

export const DEFAULT_CONFIGURATION: ModelConfiguration = {
  modelPath: "",
  profile: "automatic",
  runtime: {
    contextTokens: 4096,
    gpuLayers: 0,
    threads: 4,
    batchThreads: 4,
    batchSize: 128,
    microBatchSize: 32,
    parallelSlots: 1,
    flashAttention: false,
    kvCacheKType: "f16",
    kvCacheVType: "f16",
    kvCacheOnGpu: false,
    useMemoryMapping: true,
    lockMemory: false
  },
  generation: {
    temperature: 0.2,
    topP: 0.95,
    topK: 40,
    minP: 0,
    typicalP: 1,
    repeatPenalty: 1,
    repeatLastN: 64,
    presencePenalty: 0,
    frequencyPenalty: 0,
    maxOutputTokens: 4096,
    seed: null,
    stopSequences: []
  },
  optimizationNotes: [],
  optimizerRevision: 1
};
