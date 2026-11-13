use crate::domain::{
    HardwareProfile, ModelConfiguration, ModelMetadata, OptimizationProfile, OptimizationResult,
};

const GIB: u64 = 1024 * 1024 * 1024;
const COMPUTE_HEADROOM: u64 = 768 * 1024 * 1024;

fn preferred_context(profile: OptimizationProfile, hardware: &HardwareProfile, model: &ModelMetadata) -> usize {
    let model_limit = model.context_length.filter(|limit| *limit > 0).unwrap_or(8192).min(131072);
    let minimum_context = model_limit.min(2048);
    let ram_gib = hardware.ram_total_bytes / GIB;
    let target = match profile {
        OptimizationProfile::MemoryEfficient => 4096,
        OptimizationProfile::MaximumContext => model_limit,
        OptimizationProfile::MaximumPerformance if ram_gib >= 32 => 16384,
        OptimizationProfile::MaximumPerformance => 8192,
        OptimizationProfile::Balanced if ram_gib >= 32 => 16384,
        OptimizationProfile::Balanced => 8192,
        OptimizationProfile::Automatic if ram_gib >= 32 => 16384,
        OptimizationProfile::Automatic => 8192,
        OptimizationProfile::Custom => 4096,
    };
    target.min(model_limit).max(minimum_context)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= GIB { format!("{:.2} GiB", bytes as f64 / GIB as f64) }
    else { format!("{:.0} MiB", bytes as f64 / (1024.0 * 1024.0)) }
}

fn estimate_context_budget(
    model: &ModelMetadata,
    hardware: &HardwareProfile,
    profile: OptimizationProfile,
) -> (usize, usize, usize, bool, Vec<String>) {
    let mut notes = Vec::new();
    let ram_available = if hardware.ram_available_bytes > 0 {
        hardware.ram_available_bytes
    } else {
        hardware.ram_total_bytes.saturating_mul(3) / 5
    };
    let ram_reserve = (hardware.ram_total_bytes / 5).max(2 * GIB);
    let ram_budget = ram_available.saturating_sub(ram_reserve);

    let gpu = hardware.gpus.first();
    let gpu_available = gpu.map(|device| device.available_vram_bytes).unwrap_or(0);
    let gpu_total = gpu.map(|device| device.total_vram_bytes).unwrap_or(0);
    let gpu_reserve = (gpu_total / 5).max(1024 * 1024 * 1024);
    let gpu_budget = gpu_available.saturating_sub(gpu_reserve);

    let layers = model.layer_count.unwrap_or(0);
    let model_bytes = model.file_size_bytes;
    let slots = match profile {
        OptimizationProfile::MaximumPerformance if ram_available >= 32 * GIB && gpu_total >= 12 * GIB => 2,
        _ => 1,
    };
    let bytes_per_token = model.kv_bytes_per_token;
    if bytes_per_token.is_none() {
        notes.push("KV-cache size could not be derived from GGUF architecture metadata; context is capped at 4K and a 2 GiB memory reserve is applied.".into());
    }

    let target = preferred_context(profile, hardware, model);
    // Without architecture data, keep the initial recommendation small and reserve a
    // conservative memory allowance instead of treating an unknown KV cache as zero.
    let target = if bytes_per_token.is_none() { target.min(4096) } else { target };
    let minimum_context = model.context_length
        .filter(|limit| *limit > 0)
        .unwrap_or(8192)
        .min(2048);
    let mut candidate = target;
    let unknown_kv_reserve = if bytes_per_token.is_none() { 2 * GIB } else { 0 };

    let (context, gpu_layers, slots, kv_on_gpu) = loop {
        let kv_bytes = bytes_per_token
            .and_then(|per_token| per_token.checked_mul(candidate as u64))
            .and_then(|bytes| bytes.checked_mul(slots as u64));
        let kv_estimate = kv_bytes.unwrap_or(0);
        let kv_on_gpu = bytes_per_token.is_some()
            && gpu_budget > kv_estimate.saturating_add(COMPUTE_HEADROOM)
            && gpu_budget > 0;
        let gpu_kv = if kv_on_gpu { kv_estimate } else { 0 };
        let cpu_kv = if kv_on_gpu { 0 } else { kv_estimate };

        let gpu_weight_budget = gpu_budget
            .saturating_sub(gpu_kv)
            .saturating_sub(COMPUTE_HEADROOM)
            .saturating_sub(unknown_kv_reserve);
        let gpu_layers = if layers > 0 && model_bytes > 0 {
            ((gpu_weight_budget as f64 / model_bytes as f64) * layers as f64)
                .floor().clamp(0.0, layers as f64) as usize
        } else { 0 };
        let gpu_weight_bytes = if layers > 0 {
            model_bytes.saturating_mul(gpu_layers as u64) / layers as u64
        } else { 0 };
        let cpu_weight_bytes = model_bytes.saturating_sub(gpu_weight_bytes);
        let cpu_required = cpu_weight_bytes
            .saturating_add(cpu_kv)
            .saturating_add(COMPUTE_HEADROOM)
            .saturating_add(unknown_kv_reserve);

        if cpu_required <= ram_budget || candidate <= minimum_context {
            if cpu_required > ram_budget {
                notes.push("Estimated model weights and KV cache exceed the conservative available system-memory budget. Reduce context or use a smaller quantization.".into());
            }
            break (candidate, gpu_layers, slots, kv_on_gpu);
        }
        candidate = (candidate / 2).max(minimum_context);
    };
    if gpu.is_none() {
        notes.push("No supported GPU memory telemetry was found. GPU offload is set to zero rather than guessed.".into());
    } else if layers == 0 {
        notes.push("GGUF layer count is unavailable. Automatic GPU-layer estimation is disabled until model metadata can be read.".into());
    } else {
        notes.push(format!("Estimated GPU offload: {gpu_layers} of {layers} layers, reserving VRAM for the display, compute buffers, and the selected KV-cache placement."));
    }
    if let Some(per_token) = bytes_per_token {
        notes.push(format!(
            "Estimated KV-cache footprint at {context} tokens and {slots} sequence slot(s): {}.",
            format_bytes(per_token.saturating_mul(context as u64).saturating_mul(slots as u64))
        ));
    }
    notes.push("System-memory budgeting reserves at least 2 GiB; available memory can change while other applications run.".into());
    match model.context_length.filter(|limit| *limit > 0) {
        Some(limit) => notes.push(format!(
            "Context recommendation is capped by the model's declared context limit ({limit} tokens)."
        )),
        None => notes.push("The model does not declare a usable context limit in GGUF metadata; a conservative 8K fallback ceiling is used.".into()),
    }
    (context, gpu_layers, slots, kv_on_gpu, notes)
}

pub fn optimize(
    model: ModelMetadata,
    hardware: HardwareProfile,
    profile: OptimizationProfile,
) -> OptimizationResult {
    let mut warnings = Vec::new();
    let (context_tokens, gpu_layers, parallel_slots, kv_cache_on_gpu, mut notes) =
        estimate_context_budget(&model, &hardware, profile);

    let logical_threads = hardware.logical_threads.max(1);
    let (batch_size, micro_batch_size) = match profile {
        OptimizationProfile::MemoryEfficient => (64, 16),
        OptimizationProfile::MaximumPerformance if hardware.ram_total_bytes >= 32 * GIB => (512, 128),
        OptimizationProfile::MaximumPerformance => (256, 64),
        _ if hardware.ram_total_bytes < 16 * GIB => (128, 32),
        _ => (256, 64),
    };
    let threads = (logical_threads / 2).max(1).min(16);
    let batch_threads = logical_threads.max(threads).min(32);

    if model.file_size_bytes > hardware.ram_total_bytes.saturating_add(
        hardware.gpus.iter().map(|gpu| gpu.total_vram_bytes).sum::<u64>()
    ) {
        warnings.push("The model file is larger than the combined detected RAM and VRAM capacity. Loading is unlikely to succeed without additional memory or a smaller quantization.".into());
    }
    if profile == OptimizationProfile::Custom {
        notes.push("Custom profile selected: recommendations are informational; manual values should be preserved.".into());
    }
    if hardware.gpus.iter().any(|gpu| gpu.vendor == "NVIDIA") {
        notes.push("NVIDIA memory telemetry is available, but this build does not start llama.cpp or validate the installed CUDA/Vulkan backend.".into());
    }

    let mut configuration = ModelConfiguration::new(model.path.clone(), profile);
    configuration.runtime.context_tokens = context_tokens;
    configuration.generation.max_output_tokens = configuration.generation.max_output_tokens.min(context_tokens);
    configuration.runtime.gpu_layers = gpu_layers;
    configuration.runtime.threads = threads;
    configuration.runtime.batch_threads = batch_threads;
    configuration.runtime.batch_size = batch_size;
    configuration.runtime.micro_batch_size = micro_batch_size;
    configuration.runtime.parallel_slots = parallel_slots;
    configuration.runtime.kv_cache_on_gpu = kv_cache_on_gpu;
    configuration.optimization_notes = notes;

    OptimizationResult { model, hardware, configuration, warnings }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GpuDevice;

    fn sample_model() -> ModelMetadata {
        ModelMetadata {
            path: "sample.gguf".into(), file_size_bytes: 4 * GIB, gguf_version: 3,
            tensor_count: 100, architecture: Some("llama".into()), layer_count: Some(32),
            context_length: Some(32768), embedding_length: Some(4096), attention_heads: Some(32),
            key_value_heads: Some(8), key_length: Some(128), value_length: Some(128),
            file_type: Some(15), quantization: Some("Q4_K_M".into()), kv_bytes_per_token: Some(131_072),
        }
    }

    #[test]
    fn memory_limited_hardware_gets_conservative_settings() {
        let hardware = HardwareProfile {
            os: "windows".into(), cpu_name: Some("test".into()), logical_threads: 8, physical_cores: None,
            ram_total_bytes: 16 * GIB, ram_available_bytes: 10 * GIB,
            gpus: vec![GpuDevice { vendor: "NVIDIA".into(), name: "test gpu".into(), total_vram_bytes: 8 * GIB, available_vram_bytes: 7 * GIB, detection_method: "fixture".into() }],
            detection_notes: vec![],
        };
        let result = optimize(sample_model(), hardware, OptimizationProfile::Automatic);
        assert!(result.configuration.runtime.context_tokens <= 8192);
        assert!(result.configuration.runtime.gpu_layers <= 32);
        assert_eq!(result.configuration.runtime.parallel_slots, 1);
    }


    #[test]
    fn maximum_context_never_exceeds_a_small_model_limit() {
        let mut model = sample_model();
        model.context_length = Some(1024);
        let hardware = HardwareProfile {
            os: "windows".into(),
            cpu_name: None,
            logical_threads: 8,
            physical_cores: None,
            ram_total_bytes: 16 * GIB,
            ram_available_bytes: 12 * GIB,
            gpus: vec![],
            detection_notes: vec![],
        };
        let result = optimize(model, hardware, OptimizationProfile::MaximumContext);
        assert_eq!(result.configuration.runtime.context_tokens, 1024);
        assert_eq!(result.configuration.generation.max_output_tokens, 1024);
    }

    #[test]
    fn no_gpu_never_recommends_gpu_offload() {
        let hardware = HardwareProfile {
            os: "windows".into(), cpu_name: None, logical_threads: 4, physical_cores: None,
            ram_total_bytes: 8 * GIB, ram_available_bytes: 5 * GIB, gpus: vec![], detection_notes: vec![],
        };
        let result = optimize(sample_model(), hardware, OptimizationProfile::Balanced);
        assert_eq!(result.configuration.runtime.gpu_layers, 0);
        assert!(!result.configuration.runtime.kv_cache_on_gpu);
    }
}
