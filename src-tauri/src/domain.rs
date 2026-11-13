use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationProfile {
    Automatic,
    MaximumPerformance,
    Balanced,
    MemoryEfficient,
    MaximumContext,
    Custom,
}

impl Default for OptimizationProfile {
    fn default() -> Self { Self::Automatic }
}

impl OptimizationProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::MaximumPerformance => "maximum_performance",
            Self::Balanced => "balanced",
            Self::MemoryEfficient => "memory_efficient",
            Self::MaximumContext => "maximum_context",
            Self::Custom => "custom",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuDevice {
    pub vendor: String,
    pub name: String,
    pub total_vram_bytes: u64,
    pub available_vram_bytes: u64,
    pub detection_method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub os: String,
    pub cpu_name: Option<String>,
    pub logical_threads: usize,
    pub physical_cores: Option<usize>,
    pub ram_total_bytes: u64,
    pub ram_available_bytes: u64,
    pub gpus: Vec<GpuDevice>,
    pub detection_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    pub path: String,
    pub file_size_bytes: u64,
    pub gguf_version: u32,
    pub tensor_count: u64,
    pub architecture: Option<String>,
    pub layer_count: Option<usize>,
    pub context_length: Option<usize>,
    pub embedding_length: Option<usize>,
    pub attention_heads: Option<usize>,
    pub key_value_heads: Option<usize>,
    pub key_length: Option<usize>,
    pub value_length: Option<usize>,
    pub file_type: Option<u32>,
    pub quantization: Option<String>,
    pub kv_bytes_per_token: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSettings {
    pub context_tokens: usize,
    pub gpu_layers: usize,
    pub threads: usize,
    pub batch_threads: usize,
    pub batch_size: usize,
    pub micro_batch_size: usize,
    pub parallel_slots: usize,
    pub flash_attention: bool,
    pub kv_cache_k_type: String,
    pub kv_cache_v_type: String,
    pub kv_cache_on_gpu: bool,
    pub use_memory_mapping: bool,
    pub lock_memory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationSettings {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: usize,
    pub min_p: f32,
    pub typical_p: f32,
    pub repeat_penalty: f32,
    pub repeat_last_n: i32,
    pub presence_penalty: f32,
    pub frequency_penalty: f32,
    pub max_output_tokens: usize,
    pub seed: Option<i64>,
    pub stop_sequences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfiguration {
    pub model_path: String,
    pub profile: OptimizationProfile,
    pub runtime: RuntimeSettings,
    pub generation: GenerationSettings,
    pub optimization_notes: Vec<String>,
    pub optimizer_revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResult {
    pub model: ModelMetadata,
    pub hardware: HardwareProfile,
    pub configuration: ModelConfiguration,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub updater_configured: bool,
    pub platform: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedPreset {
    pub name: String,
    pub configuration: ModelConfiguration,
    pub updated_at: i64,
}

impl ModelConfiguration {
    pub fn new(model_path: String, profile: OptimizationProfile) -> Self {
        Self {
            model_path,
            profile,
            runtime: RuntimeSettings {
                context_tokens: 4096,
                gpu_layers: 0,
                threads: 4,
                batch_threads: 4,
                batch_size: 128,
                micro_batch_size: 32,
                parallel_slots: 1,
                flash_attention: false,
                kv_cache_k_type: "f16".into(),
                kv_cache_v_type: "f16".into(),
                kv_cache_on_gpu: false,
                use_memory_mapping: true,
                lock_memory: false,
            },
            generation: GenerationSettings {
                temperature: 0.2,
                top_p: 0.95,
                top_k: 40,
                min_p: 0.0,
                typical_p: 1.0,
                repeat_penalty: 1.0,
                repeat_last_n: 64,
                presence_penalty: 0.0,
                frequency_penalty: 0.0,
                max_output_tokens: 4096,
                seed: None,
                stop_sequences: Vec::new(),
            },
            optimization_notes: Vec::new(),
            optimizer_revision: 1,
        }
    }
}
