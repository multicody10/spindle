mod domain;
mod gguf;
mod hardware;
mod optimizer;
mod persistence;
mod providers;

use crate::domain::{
    AppInfo, ModelConfiguration, OptimizationProfile, OptimizationResult, SavedPreset,
};
use tauri::{AppHandle, Manager};

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
async fn local_provider_statuses() -> Result<Vec<providers::LocalProviderStatus>, String> {
    providers::discover().await
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
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            app_info,
            local_provider_statuses,
            hardware_profile,
            inspect_model,
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
