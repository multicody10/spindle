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

export type ChatProviderId = "ollama" | "lm-studio" | "llama-cpp";

export type RuntimeBackend = "cpu" | "vulkan" | "cuda";

export interface BackendInstallStatus {
  backend: RuntimeBackend;
  installed: boolean;
  version: string;
  asset: string;
}

export interface AppliedRuntimeSettings {
  modelPath: string | null;
  contextTokens: number | null;
  temperature: number | null;
  topP: number | null;
  topK: number | null;
  minP: number | null;
  repeatPenalty: number | null;
  buildInfo: string | null;
  note: string;
}

export interface InferenceRuntimeStatus {
  modelLoaded: boolean;
  modelPath: string | null;
  backend: RuntimeBackend | null;
  parallelSlots: number;
  activeRequests: number;
  queuedRequests: number;
  availableSlots: number;
  configurationFingerprint: string | null;
  assignedSessions: number;
  slotEvictions: number;
}

export interface RuntimeStatus {
  version: string;
  running: boolean;
  processId: number | null;
  backend: RuntimeBackend | null;
  modelPath: string | null;
  installedBackends: BackendInstallStatus[];
  logs: string[];
  appliedSettings: AppliedRuntimeSettings | null;
  inference: InferenceRuntimeStatus;
}

export interface WorkspaceGrant {
  sessionId: string;
  rootPath: string;
  canRead: boolean;
  canWrite: boolean;
  temporary: boolean;
}

export interface WorkspaceEntry {
  name: string;
  relativePath: string;
  kind: "directory" | "file";
  sizeBytes: number | null;
  modifiedUnixMs: number | null;
}

export interface WorkspaceListing {
  relativePath: string;
  entries: WorkspaceEntry[];
  truncated: boolean;
}

export interface WorkspaceFile {
  relativePath: string;
  content: string;
  sizeBytes: number;
  revision: string;
}

export interface WorkspaceWriteResult {
  relativePath: string;
  bytesWritten: number;
  revision: string;
}
export interface WorkspaceChangeProposal {
  id: string;
  relativePath: string;
  baseRevision: string;
  proposedRevision: string;
  diff: string;
  addedLines: number;
  removedLines: number;
  status: "pending" | "applied" | "undone" | "cancelled";
  createdAtUnixMs: number;
}
export interface PermissionAuditEvent {
  sessionId: string;
  projectId: string;
  operation: string;
  scope: PermissionScope | null;
  resource: string;
  allowed: boolean;
  reason: string | null;
  timestampUnixMs: number;
}

export interface WorkspaceSearchMatch {
  relativePath: string;
  lineNumber: number;
  preview: string;
}

export interface WorkspaceSearchResult {
  matches: WorkspaceSearchMatch[];
  filesScanned: number;
  bytesScanned: number;
  truncated: boolean;
}
export type ChatMessageRole = "system" | "user" | "assistant";

export interface ChatMessage {
  role: ChatMessageRole;
  content: string;
}

export interface ChatRequest {
  requestId: string;
  sessionId: string;
  providerId: ChatProviderId;
  model: string;
  messages: ChatMessage[];
  contextTokens: number;
  generation: GenerationSettings;
}

export interface ChatDelta {
  requestId: string;
  delta: string;
}

export interface ChatThinkingDelta {
  requestId: string;
  delta: string;
}

export interface Project {
  id: string; name: string; rootPath: string; createdAt: number; updatedAt: number; lastOpenedAt: number | null;
}
export interface ProjectSession {
  id: string; projectId: string; title: string; providerId: ChatProviderId | null; modelId: string | null;
  status: string; createdAt: number; updatedAt: number;
}
export interface PersistedSessionMessage {
  id: string; sessionId: string; sequence: number; role: "user" | "assistant" | "system"; content: string; createdAt: number;
}
export interface AgentPlanStep { title: string; description: string; targetPaths: string[]; validation: string; risk: string; }
export interface AgentPlan { summary: string; steps: AgentPlanStep[]; }
export interface AgentTask {
  id: string; projectId: string; sessionId: string; title: string; prompt: string; state: string;
  plan: AgentPlan | null; createdAt: number; updatedAt: number; approvedAt: number | null;
}
export interface AgentTaskStep extends AgentPlanStep { id: string; taskId: string; sequence: number; status: string; }
export interface AgentTaskDetail { task: AgentTask; steps: AgentTaskStep[]; }
export interface AgentTaskEvent { id: string; taskId: string; sequence: number; kind: string; payload: unknown; createdAt: number; }
export interface AgentRunRequest {
  taskId: string;
  providerId: string;
  model: string;
  contextTokens: number;
  generation: GenerationSettings;
}
export interface AgentRunSummary {
  taskId: string;
  state: string;
  finalResponse: string;
  turns: number;
  toolCalls: number;
}
export interface AgentRunEvent {
  taskId: string;
  kind: string;
  payload: unknown;
  createdAt: number;
}
export interface TaskApplicationResult {
  taskId: string;
  status: "applied" | "undone" | "failed" | "conflict" | "applying";
  changedFiles: number;
  patchSha256: string;
  appliedAt: number;
}
export interface ProjectMemory {
  id: string; projectId: string; kind: string; memoryKey: string; content: string; sourcePath: string;
  sourceRevision: string; confidence: number; importance: number; updatedAt: number;
}
export type CapabilitySupport = "supported" | "unsupported" | "provider_dependent" | "not_implemented";
export interface ProviderCapabilities {
  providerId: ChatProviderId; displayName: string; protocol: string; streaming: CapabilitySupport;
  cancellation: CapabilitySupport; usageMetrics: CapabilitySupport; toolCalling: CapabilitySupport;
  structuredOutput: CapabilitySupport; parallelSessions: CapabilitySupport; managedBySpindle: boolean;
}
export type PermissionScope = "project_read" | "project_write" | "process_execution" | "external_network" | "mcp_tools" | "parallel_agents";
export interface SessionPermissions { sessionId: string; projectId: string; scopes: PermissionScope[]; expiresOnRestart: boolean; }
export interface AgentSkill { name: string; description: string; source: string; manifestPath: string; }
export interface LoadedAgentSkill { name: string; source: string; manifest: string; }

export interface ChatToolCall {
  id: string;
  type: "function";
  function: { name: string; arguments: Record<string, unknown> };
}

export interface ChatResult {
  content: string;
  toolCalls: ChatToolCall[];
  providerId: string;
  model: string;
  durationMs: number;
  promptTokens: number | null;
  outputTokens: number | null;
  tokensPerSecond: number | null;
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

export interface ModelLibraryLocation {
  name: string;
  path: string;
  exists: boolean;
}

export interface ModelLibraryEntry {
  fileName: string;
  path: string;
  sizeBytes: number;
  source: string;
}

export interface ModelLibrary {
  managedDirectory: string;
  locations: ModelLibraryLocation[];
  models: ModelLibraryEntry[];
  truncated: boolean;
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

export interface BenchmarkSample {
  sequence: number;
  durationMs: number | null;
  outputTokens: number | null;
  tokensPerSecond: number | null;
  error: string | null;
}

export interface BenchmarkRun {
  id: string;
  providerId: string;
  modelId: string;
  contextTokens: number;
  requestedIterations: number;
  status: string;
  medianDurationMs: number | null;
  medianTokensPerSecond: number | null;
  startedAt: number;
  completedAt: number | null;
  samples: BenchmarkSample[];
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

export interface GitRepositoryStatus {
  rootPath: string; head: string; branch: string; clean: boolean;
  changedFiles: number; stagedFiles: number; untrackedFiles: number;
}
export interface GitCommitSummary {
  commit: string; shortCommit: string; author: string; committedAt: number; subject: string;
}
export interface TaskWorktree {
  taskId: string; projectId: string; path: string; baseCommit: string; createdAt: number;
}
export interface TaskWorktreeDiff {
  worktree: TaskWorktree; status: GitRepositoryStatus; diff: string; diffTruncated: boolean;
}
export interface SandboxStatus {
  supported: boolean; platform: string; isolation: string; network: string; reason: string | null;
}
export interface SandboxExecutionResult {
  command: string; exitCode: number; success: boolean; durationMs: number;
  stdout: string; stderr: string; outputTruncated: boolean; isolation: string;
}
export type ValidationCommand = "format" | "check" | "test";
