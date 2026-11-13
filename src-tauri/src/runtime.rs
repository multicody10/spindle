use crate::domain::ModelConfiguration;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use zip::ZipArchive;

const VERSION: &str = "b12347";
const PORT: u16 = 12345;
const BASE_URL: &str = "http://127.0.0.1:12345";
const MAX_DOWNLOAD_BYTES: u64 = 450 * 1024 * 1024;
const MAX_TOTAL_DOWNLOAD_BYTES: u64 = 800 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 2_048;
const MAX_LOG_LINES: usize = 400;
const CUDA_RUNTIME_DLLS: [&str; 3] = ["cudart64_12.dll", "cublas64_12.dll", "cublasLt64_12.dll"];
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy)]
struct RuntimeAsset {
    name: &'static str,
    sha256: &'static str,
    size_bytes: u64,
}

static CPU_ASSETS: [RuntimeAsset; 1] = [RuntimeAsset {
    name: "llama-b12347-bin-win-cpu-x64.zip",
    sha256: "476a30ac458f4881051a26c541707440de4aa744d878c739669289cbe137aa09",
    size_bytes: 25_097_360,
}];

static VULKAN_ASSETS: [RuntimeAsset; 1] = [RuntimeAsset {
    name: "llama-b12347-bin-win-vulkan-x64.zip",
    sha256: "7dccbc28c03dc929687df80894d736c9fd91c71b88537f7f1a7d2eefb85a86c5",
    size_bytes: 38_089_944,
}];

static CUDA_ASSETS: [RuntimeAsset; 2] = [
    RuntimeAsset {
        name: "llama-b12347-bin-win-cuda-12.4-x64.zip",
        sha256: "990e0bb9519e26c8bbd2c0de6e01e59ec596c2f3fe7309376ec416c2be56ca3d",
        size_bytes: 318_975_553,
    },
    RuntimeAsset {
        name: "cudart-llama-bin-win-cuda-12.4-x64.zip",
        sha256: "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
        size_bytes: 391_443_627,
    },
];

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeBackend {
    Cpu,
    Vulkan,
    Cuda,
}

impl RuntimeBackend {
    fn assets(self) -> &'static [RuntimeAsset] {
        match self {
            Self::Cpu => &CPU_ASSETS,
            Self::Vulkan => &VULKAN_ASSETS,
            Self::Cuda => &CUDA_ASSETS,
        }
    }

    fn asset(self) -> &'static str {
        match self {
            Self::Cpu => "llama-b12347-bin-win-cpu-x64.zip",
            Self::Vulkan => "llama-b12347-bin-win-vulkan-x64.zip",
            Self::Cuda => {
                "llama-b12347-bin-win-cuda-12.4-x64.zip + cudart-llama-bin-win-cuda-12.4-x64.zip"
            }
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Vulkan => "vulkan",
            Self::Cuda => "cuda",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendInstallStatus {
    pub backend: RuntimeBackend,
    pub installed: bool,
    pub version: &'static str,
    pub asset: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedSettings {
    pub model_path: Option<String>,
    pub context_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u64>,
    pub min_p: Option<f64>,
    pub repeat_penalty: Option<f64>,
    pub build_info: Option<String>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub version: &'static str,
    pub running: bool,
    pub process_id: Option<u32>,
    pub backend: Option<RuntimeBackend>,
    pub model_path: Option<String>,
    pub installed_backends: Vec<BackendInstallStatus>,
    pub logs: Vec<String>,
    pub applied_settings: Option<AppliedSettings>,
}

struct ProcessState {
    child: Child,
    backend: RuntimeBackend,
    model_path: String,
}

#[derive(Default)]
pub struct ManagedRuntime {
    process: Mutex<Option<ProcessState>>,
    logs: Arc<Mutex<VecDeque<String>>>,
}

impl Drop for ManagedRuntime {
    fn drop(&mut self) {
        if let Ok(state) = self.process.get_mut() {
            if let Some(process) = state.as_mut() {
                let _ = process.child.kill();
                let _ = process.child.wait();
            }
        }
    }
}

impl ManagedRuntime {
    fn push_log(&self, line: impl Into<String>) {
        push_log_to(&self.logs, line.into());
    }

    fn clear_logs(&self) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.clear();
        }
    }

    fn take_snapshot(
        &self,
    ) -> (
        bool,
        Option<u32>,
        Option<RuntimeBackend>,
        Option<String>,
        Vec<String>,
    ) {
        let mut running = false;
        let mut process_id = None;
        let mut backend = None;
        let mut model_path = None;

        if let Ok(mut state) = self.process.lock() {
            if let Some(process) = state.as_mut() {
                match process.child.try_wait() {
                    Ok(None) => {
                        running = true;
                        process_id = Some(process.child.id());
                        backend = Some(process.backend);
                        model_path = Some(process.model_path.clone());
                    }
                    Ok(Some(status)) => {
                        self.push_log(format!("llama-server exited with status {status}."));
                        *state = None;
                    }
                    Err(error) => {
                        self.push_log(format!("Could not inspect llama-server process: {error}"))
                    }
                }
            }
        }

        let logs = self
            .logs
            .lock()
            .map(|items| items.iter().cloned().collect())
            .unwrap_or_default();
        (running, process_id, backend, model_path, logs)
    }

    fn stop_process(&self) -> Result<(), String> {
        let mut state = self
            .process
            .lock()
            .map_err(|_| "The runtime process state is unavailable.".to_string())?;
        let Some(mut process) = state.take() else {
            return Ok(());
        };
        match process.child.try_wait() {
            Ok(Some(status)) => {
                self.push_log(format!("llama-server had already exited ({status})."))
            }
            Ok(None) => {
                self.push_log("Stopping llama-server.");
                process
                    .child
                    .kill()
                    .map_err(|error| format!("Could not stop llama-server: {error}"))?;
                let _ = process.child.wait();
                self.push_log("llama-server stopped.");
            }
            Err(error) => {
                return Err(format!(
                    "Could not inspect llama-server before stopping: {error}"
                ))
            }
        }
        Ok(())
    }
}

fn push_log_to(logs: &Arc<Mutex<VecDeque<String>>>, line: String) {
    let sanitized = line
        .chars()
        .filter(|character| !character.is_control() || *character == '\t')
        .collect::<String>();
    let line = sanitized
        .chars()
        .take(MAX_LOG_LINE_BYTES)
        .collect::<String>();
    if line.trim().is_empty() {
        return;
    }
    if let Ok(mut entries) = logs.lock() {
        entries.push_back(line);
        while entries.len() > MAX_LOG_LINES {
            entries.pop_front();
        }
    }
}

fn runtime_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join("runtime").join("llama.cpp"))
        .map_err(|error| format!("Could not resolve the Spindle runtime directory: {error}"))
}

fn install_dir(app: &AppHandle, backend: RuntimeBackend) -> Result<PathBuf, String> {
    Ok(runtime_root(app)?.join(VERSION).join(backend.id()))
}

fn executable_path(app: &AppHandle, backend: RuntimeBackend) -> Result<PathBuf, String> {
    let directory = install_dir(app, backend)?;
    find_backend_server(&directory, backend).ok_or_else(|| {
        if backend == RuntimeBackend::Cuda {
            "The CUDA runtime installation is incomplete. Reinstall the CUDA backend and verify its runtime DLL bundle.".to_string()
        } else {
            "The installed llama.cpp executable is missing.".to_string()
        }
    })
}

fn backend_status(
    app: &AppHandle,
    backend: RuntimeBackend,
) -> Result<BackendInstallStatus, String> {
    let path = install_dir(app, backend)?;
    Ok(BackendInstallStatus {
        backend,
        installed: find_backend_server(&path, backend).is_some(),
        version: VERSION,
        asset: backend.asset(),
    })
}

pub async fn status(app: &AppHandle, runtime: &ManagedRuntime) -> Result<RuntimeStatus, String> {
    let (running, process_id, backend, model_path, logs) = runtime.take_snapshot();
    let applied_settings = if running {
        match read_applied_settings().await {
            Ok(settings)
                if settings.model_path.as_deref() == model_path.as_deref()
                    && settings
                        .build_info
                        .as_deref()
                        .is_some_and(|info| info.starts_with(VERSION)) =>
            {
                Some(settings)
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(RuntimeStatus {
        version: VERSION,
        running,
        process_id,
        backend,
        model_path,
        installed_backends: vec![
            backend_status(app, RuntimeBackend::Cpu)?,
            backend_status(app, RuntimeBackend::Vulkan)?,
            backend_status(app, RuntimeBackend::Cuda)?,
        ],
        logs,
        applied_settings,
    })
}

pub async fn install(
    app: AppHandle,
    runtime: &ManagedRuntime,
    backend: RuntimeBackend,
) -> Result<BackendInstallStatus, String> {
    if runtime.take_snapshot().0 {
        return Err("Stop the active model before installing or replacing a runtime.".into());
    }

    let destination = install_dir(&app, backend)?;
    if find_server(&destination).is_some() {
        return backend_status(&app, backend);
    }

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(900))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if is_allowed_release_asset_redirect(attempt.previous(), attempt.url()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| "Could not initialize the runtime download client.".to_string())?;

    let root = runtime_root(&app)?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create the runtime directory: {error}"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staging = root.join(format!(".staging-{}-{nonce}", std::process::id()));
    fs::create_dir(&staging)
        .map_err(|error| format!("Could not create the runtime staging directory: {error}"))?;

    runtime.push_log(format!(
        "Downloading and verifying pinned llama.cpp {VERSION} ({})…",
        backend.id()
    ));
    let verified_assets =
        match download_and_extract_assets(&client, runtime, backend, &staging).await {
            Ok(assets) => assets,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
        };

    let server = find_backend_server(&staging, backend).ok_or_else(|| {
        let _ = fs::remove_dir_all(&staging);
        if backend == RuntimeBackend::Cuda {
            "The verified CUDA archives did not contain llama-server.exe and all three required CUDA runtime DLLs.".to_string()
        } else {
            "The verified archive did not contain llama-server.exe.".to_string()
        }
    })?;
    let marker = staging.join("spindle-runtime.json");
    let metadata = serde_json::json!({
        "provider": "llama.cpp",
        "version": VERSION,
        "backend": backend.id(),
        "assets": verified_assets
    });
    fs::write(
        marker,
        serde_json::to_vec_pretty(&metadata).unwrap_or_default(),
    )
    .map_err(|error| format!("Could not write runtime metadata: {error}"))?;

    let relative_server = server
        .strip_prefix(&staging)
        .map_err(|_| "The runtime executable escaped its staging directory.".to_string())?;
    let staged_server = staging.join(relative_server);
    if !staged_server.is_file() {
        let _ = fs::remove_dir_all(&staging);
        return Err("The extracted llama-server executable is missing.".into());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not prepare the runtime destination: {error}"))?;
    }
    if destination.exists() {
        fs::remove_dir_all(&destination)
            .map_err(|error| format!("Could not replace the previous runtime: {error}"))?;
    }
    fs::rename(&staging, &destination).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        format!("Could not move the verified runtime into place: {error}")
    })?;
    runtime.push_log(format!(
        "llama.cpp {VERSION} ({}) installed and verified.",
        backend.id()
    ));
    backend_status(&app, backend)
}

fn is_allowed_release_asset_redirect(previous: &[reqwest::Url], target: &reqwest::Url) -> bool {
    previous.len() == 1
        && previous[0].scheme() == "https"
        && previous[0].host_str() == Some("github.com")
        && previous[0].port_or_known_default() == Some(443)
        && target.scheme() == "https"
        && target.host_str() == Some("release-assets.githubusercontent.com")
        && target.port_or_known_default() == Some(443)
}

async fn download_asset(
    client: &reqwest::Client,
    asset: &RuntimeAsset,
    destination: &Path,
) -> Result<(), String> {
    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{VERSION}/{}",
        asset.name
    );
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not download {}: {error}", asset.name))?;
    if !response.status().is_success() {
        return Err(format!(
            "The llama.cpp download for {} returned HTTP {}.",
            asset.name,
            response.status().as_u16()
        ));
    }
    let final_url = response.url();
    if final_url.scheme() != "https"
        || !matches!(
            final_url.host_str(),
            Some("github.com" | "release-assets.githubusercontent.com")
        )
    {
        return Err("The llama.cpp download resolved to an unexpected host.".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES || length != asset.size_bytes)
    {
        return Err(format!(
            "The {} download had an unexpected size.",
            asset.name
        ));
    }

    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create a temporary download file: {error}"))?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0u64;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| format!("Could not read the {} download.", asset.name))?
    {
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if downloaded > MAX_DOWNLOAD_BYTES || downloaded > asset.size_bytes {
            return Err(format!(
                "The {} archive exceeded its pinned download size.",
                asset.name
            ));
        }
        hasher.update(&chunk);
        output
            .write_all(&chunk)
            .map_err(|_| format!("Could not write the {} download to disk.", asset.name))?;
    }
    if downloaded != asset.size_bytes {
        return Err(format!(
            "The {} download was incomplete: expected {} bytes, received {}.",
            asset.name, asset.size_bytes, downloaded
        ));
    }
    output
        .flush()
        .and_then(|_| output.sync_all())
        .map_err(|_| format!("Could not finalize the {} download.", asset.name))?;
    let digest = format!("{:x}", hasher.finalize());
    if digest != asset.sha256 {
        return Err(format!(
            "The {} archive failed SHA-256 verification. Nothing was installed.",
            asset.name
        ));
    }
    Ok(())
}

async fn download_and_extract_assets(
    client: &reqwest::Client,
    runtime: &ManagedRuntime,
    backend: RuntimeBackend,
    staging: &Path,
) -> Result<Vec<Value>, String> {
    let assets = backend.assets();
    let total_download_bytes = assets.iter().map(|asset| asset.size_bytes).sum::<u64>();
    if total_download_bytes > MAX_TOTAL_DOWNLOAD_BYTES {
        return Err("The selected runtime exceeds the total download size limit.".into());
    }

    let mut extracted_bytes = 0u64;
    let mut verified_assets = Vec::with_capacity(assets.len());
    for (index, asset) in assets.iter().enumerate() {
        runtime.push_log(format!(
            "Downloading {} ({}/{})…",
            asset.name,
            index + 1,
            assets.len()
        ));
        let archive_path = staging.join(format!(".spindle-download-{index}.zip"));
        download_asset(client, asset, &archive_path).await?;
        runtime.push_log(format!("Verified SHA-256 for {}.", asset.name));

        let archive = File::open(&archive_path)
            .map_err(|_| format!("Could not open the verified {} archive.", asset.name))?;
        extract_archive(archive, staging, &mut extracted_bytes)?;
        fs::remove_file(&archive_path)
            .map_err(|_| format!("Could not remove the temporary {} archive.", asset.name))?;
        verified_assets.push(serde_json::json!({
            "name": asset.name,
            "sha256": asset.sha256,
            "sizeBytes": asset.size_bytes
        }));
    }
    runtime.push_log("All runtime archives were verified and extracted.");
    Ok(verified_assets)
}

fn extract_archive<R: Read + Seek>(
    reader: R,
    destination: &Path,
    extracted: &mut u64,
) -> Result<(), String> {
    let mut archive = ZipArchive::new(reader)
        .map_err(|_| "The downloaded runtime archive is not a valid ZIP file.".to_string())?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("The runtime archive contains too many entries.".into());
    }

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|_| "Could not read an entry from the runtime archive.".to_string())?;
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() {
            return Err("The runtime archive contains a symbolic link and was rejected.".into());
        }
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| "The runtime archive contains an unsafe file path.".to_string())?
            .to_path_buf();
        if relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err("The runtime archive contains an unsafe file path.".into());
        }
        let size = entry.size();
        *extracted = extracted.saturating_add(size);
        if size > MAX_EXTRACTED_BYTES || *extracted > MAX_EXTRACTED_BYTES {
            return Err("The extracted runtime exceeded the uncompressed size limit.".into());
        }
        let output = destination.join(relative);
        let parent = output
            .parent()
            .ok_or_else(|| "The runtime archive contains an invalid path.".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create a runtime directory: {error}"))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|_| {
                "The runtime archive contains a duplicate or unwritable file path.".to_string()
            })?;
        let copied = io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Could not extract a runtime file: {error}"))?;
        if copied != size {
            return Err("A runtime archive entry had an unexpected extracted size.".into());
        }
        file.flush()
            .map_err(|error| format!("Could not flush a runtime file: {error}"))?;
    }
    Ok(())
}

fn find_backend_server(root: &Path, backend: RuntimeBackend) -> Option<PathBuf> {
    let server = find_server(root)?;
    if backend == RuntimeBackend::Cuda {
        let directory = server.parent()?;
        if !CUDA_RUNTIME_DLLS
            .iter()
            .all(|name| directory.join(name).is_file())
        {
            return None;
        }
    }
    Some(server)
}

fn find_server(root: &Path) -> Option<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .eq_ignore_ascii_case("llama-server.exe")
            }) {
                return Some(path);
            }
        }
    }
    None
}

fn capture_output<R: Read + Send + 'static>(
    reader: R,
    logs: Arc<Mutex<VecDeque<String>>>,
    prefix: &'static str,
) {
    thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 2048];
        let mut line = Vec::with_capacity(512);
        loop {
            let count = match reader.read(&mut chunk) {
                Ok(0) => {
                    if !line.is_empty() {
                        push_log_to(
                            &logs,
                            format!("{prefix} {}", String::from_utf8_lossy(&line)),
                        );
                    }
                    break;
                }
                Ok(count) => count,
                Err(error) => {
                    push_log_to(&logs, format!("{prefix} log stream ended: {error}"));
                    break;
                }
            };
            for byte in &chunk[..count] {
                if *byte == b'\n' || line.len() >= MAX_LOG_LINE_BYTES {
                    push_log_to(
                        &logs,
                        format!("{prefix} {}", String::from_utf8_lossy(&line)),
                    );
                    line.clear();
                } else if *byte != b'\r' {
                    line.push(*byte);
                }
            }
        }
    });
}

pub async fn start(
    app: AppHandle,
    runtime: &ManagedRuntime,
    model_path: String,
    configuration: ModelConfiguration,
    backend: RuntimeBackend,
) -> Result<RuntimeStatus, String> {
    if runtime.take_snapshot().0 {
        return Err("A managed llama.cpp process is already running.".into());
    }
    crate::validate_configuration(&configuration)?;
    let model_metadata = crate::validate_model_constraints(&configuration)?;
    if configuration.model_path != model_path {
        return Err("The selected model path does not match the saved configuration.".into());
    }
    let model = fs::canonicalize(&model_path)
        .map_err(|error| format!("Could not resolve the selected model file: {error}"))?;
    if !model.is_file()
        || !model
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
    {
        return Err("Select an existing GGUF model file before starting the runtime.".into());
    }
    let executable = executable_path(&app, backend)?;

    runtime.clear_logs();
    runtime.push_log(format!(
        "Validated GGUF v{}: {} tensor(s), {} bytes, architecture {}, quantization {}.",
        model_metadata.gguf_version,
        model_metadata.tensor_count,
        model_metadata.file_size_bytes,
        model_metadata.architecture.as_deref().unwrap_or("unknown"),
        model_metadata.quantization.as_deref().unwrap_or("unknown")
    ));
    runtime.push_log(format!("Starting llama.cpp {VERSION} ({})…", backend.id()));

    let mut command = Command::new(&executable);
    command
        .current_dir(executable.parent().unwrap_or(Path::new(".")))
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(PORT.to_string())
        .arg("--no-webui")
        .arg("--no-agent")
        .arg("--offline")
        .arg("--model")
        .arg(&model)
        .arg("--ctx-size")
        .arg(configuration.runtime.context_tokens.to_string())
        .arg("--threads")
        .arg(configuration.runtime.threads.to_string())
        .arg("--threads-batch")
        .arg(configuration.runtime.batch_threads.to_string())
        .arg("--batch-size")
        .arg(configuration.runtime.batch_size.to_string())
        .arg("--ubatch-size")
        .arg(configuration.runtime.micro_batch_size.to_string())
        .arg("--gpu-layers")
        .arg(if backend == RuntimeBackend::Cpu {
            "0".into()
        } else {
            configuration.runtime.gpu_layers.to_string()
        })
        .arg("--parallel")
        .arg(configuration.runtime.parallel_slots.to_string())
        .arg("--flash-attn")
        .arg(if configuration.runtime.flash_attention {
            "on"
        } else {
            "off"
        })
        .arg("--cache-type-k")
        .arg(&configuration.runtime.kv_cache_k_type)
        .arg("--cache-type-v")
        .arg(&configuration.runtime.kv_cache_v_type)
        .arg(
            if backend != RuntimeBackend::Cpu && configuration.runtime.kv_cache_on_gpu {
                "--kv-offload"
            } else {
                "--no-kv-offload"
            },
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    let load_mode = match (
        configuration.runtime.use_memory_mapping,
        configuration.runtime.lock_memory,
    ) {
        (true, true) => "mmap+mlock",
        (true, false) => "mmap",
        (false, true) => "mlock",
        (false, false) => "none",
    };
    command.arg("--load-mode").arg(load_mode);

    command.env_clear();
    #[cfg(windows)]
    {
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            let system_directory = PathBuf::from(&system_root).join("System32");
            command
                .env("SystemRoot", &system_root)
                .env("WINDIR", &system_root)
                .env("PATH", system_directory);
        }
        let temporary_directory = std::env::temp_dir();
        command
            .env("TEMP", &temporary_directory)
            .env("TMP", temporary_directory);
    }

    let mut child = command.spawn().map_err(|error| {
        format!("Could not launch the verified llama-server executable: {error}")
    })?;
    let process_id = child.id();
    if let Some(stdout) = child.stdout.take() {
        capture_output(stdout, runtime.logs.clone(), "[out]");
    }
    if let Some(stderr) = child.stderr.take() {
        capture_output(stderr, runtime.logs.clone(), "[err]");
    }

    {
        let mut state = runtime
            .process
            .lock()
            .map_err(|_| "The runtime process state is unavailable.".to_string())?;
        if state.is_some() {
            let _ = child.kill();
            return Err("Another runtime process started concurrently.".into());
        }
        *state = Some(ProcessState {
            child,
            backend,
            model_path: model.to_string_lossy().into_owned(),
        });
    }

    runtime.push_log(format!(
        "llama-server process {process_id} started. Waiting for model readiness…"
    ));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the runtime health client.".to_string())?;

    let expected_model_path = model.to_string_lossy().into_owned();
    let started = std::time::Instant::now();
    while started.elapsed() < HEALTH_TIMEOUT {
        if !runtime.take_snapshot().0 {
            return Err(
                "llama-server exited before the model became ready. Review the runtime logs."
                    .into(),
            );
        }
        if client
            .get(format!("{BASE_URL}/health"))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
            && read_applied_settings().await.is_ok_and(|settings| {
                settings.model_path.as_deref() == Some(expected_model_path.as_str())
                    && settings
                        .build_info
                        .as_deref()
                        .is_some_and(|info| info.starts_with(VERSION))
            })
        {
            runtime.push_log("Model loaded. The pinned llama.cpp process is ready for local chat.");
            return status(&app, runtime).await;
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }

    let _ = runtime.stop_process();
    Err("llama-server did not finish loading the model within 120 seconds. Review the runtime logs.".into())
}

pub fn stop(runtime: &ManagedRuntime) -> Result<RuntimeStatus, String> {
    runtime.stop_process()?;
    Ok(RuntimeStatus {
        version: VERSION,
        running: false,
        process_id: None,
        backend: None,
        model_path: None,
        installed_backends: Vec::new(),
        logs: runtime.take_snapshot().4,
        applied_settings: None,
    })
}

pub async fn read_applied_settings() -> Result<AppliedSettings, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the runtime settings client.".to_string())?;
    let mut response = client
        .get(format!("{BASE_URL}/props"))
        .send()
        .await
        .map_err(|_| "The managed llama.cpp server is not responding.".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "llama.cpp returned HTTP {} while reading settings.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > 1024 * 1024)
    {
        return Err("llama.cpp runtime settings exceeded the response size limit.".into());
    }
    let mut body = Vec::with_capacity(32 * 1024);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read llama.cpp runtime settings.".to_string())?
    {
        if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
            return Err("llama.cpp runtime settings exceeded the response size limit.".into());
        }
        body.extend_from_slice(&chunk);
    }
    let document: Value = serde_json::from_slice(&body)
        .map_err(|_| "llama.cpp returned invalid runtime settings.".to_string())?;
    let defaults = document.get("default_generation_settings");
    let params = defaults.and_then(|value| value.get("params"));
    Ok(AppliedSettings {
        model_path: document.get("model_path").and_then(Value::as_str).map(str::to_owned),
        context_tokens: defaults.and_then(|value| value.get("n_ctx")).and_then(Value::as_u64),
        temperature: params.and_then(|value| value.get("temperature")).and_then(Value::as_f64),
        top_p: params.and_then(|value| value.get("top_p")).and_then(Value::as_f64),
        top_k: params.and_then(|value| value.get("top_k")).and_then(Value::as_u64),
        min_p: params.and_then(|value| value.get("min_p")).and_then(Value::as_f64),
        repeat_penalty: params.and_then(|value| value.get("repeat_penalty")).and_then(Value::as_f64),
        build_info: document.get("build_info").and_then(Value::as_str).map(str::to_owned),
        note: "Values are read from llama.cpp /props. Hardware offload and some load-time options are not exposed by this endpoint.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn runtime_assets_are_pinned_and_distinct() {
        let backends = [
            RuntimeBackend::Cpu,
            RuntimeBackend::Vulkan,
            RuntimeBackend::Cuda,
        ];
        let mut names = std::collections::HashSet::new();
        for backend in backends {
            assert!(!backend.assets().is_empty());
            for asset in backend.assets() {
                assert!(asset.sha256.len() == 64);
                assert!(asset.size_bytes > 0 && asset.size_bytes <= MAX_DOWNLOAD_BYTES);
                assert!(names.insert(asset.name));
            }
        }
        assert!(RuntimeBackend::Cpu.asset().contains(VERSION));
        assert!(RuntimeBackend::Vulkan.asset().contains(VERSION));
        assert!(RuntimeBackend::Cuda.asset().contains(VERSION));
        assert_eq!(RuntimeBackend::Cuda.assets().len(), 2);
        assert_eq!(RuntimeBackend::Cuda.id(), "cuda");
    }

    #[test]
    fn cuda_runtime_requires_all_companion_dlls_beside_the_server() {
        let root = std::env::temp_dir().join(format!(
            "spindle-cuda-runtime-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("llama-server.exe"), b"test").unwrap();
        assert!(find_backend_server(&root, RuntimeBackend::Cuda).is_none());
        assert!(find_backend_server(&root, RuntimeBackend::Cpu).is_some());
        for name in CUDA_RUNTIME_DLLS {
            fs::write(root.join(name), b"dll").unwrap();
        }
        assert!(find_backend_server(&root, RuntimeBackend::Cuda).is_some());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn release_download_redirect_is_limited_to_github_asset_host() {
        let source = reqwest::Url::parse(
            "https://github.com/ggml-org/llama.cpp/releases/download/b12347/archive.zip",
        )
        .unwrap();
        let target = reqwest::Url::parse(
            "https://release-assets.githubusercontent.com/asset.zip?token=opaque",
        )
        .unwrap();
        let external = reqwest::Url::parse("https://example.com/archive.zip").unwrap();
        let http_target =
            reqwest::Url::parse("http://release-assets.githubusercontent.com/asset.zip").unwrap();
        assert!(is_allowed_release_asset_redirect(
            std::slice::from_ref(&source),
            &target
        ));
        assert!(!is_allowed_release_asset_redirect(
            std::slice::from_ref(&source),
            &external
        ));
        assert!(!is_allowed_release_asset_redirect(
            std::slice::from_ref(&source),
            &http_target
        ));
        assert!(!is_allowed_release_asset_redirect(
            &[source.clone(), target.clone()],
            &external
        ));
    }

    #[test]
    fn rejects_archive_path_traversal() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file("../outside.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"unsafe").unwrap();
            writer.finish().unwrap();
        }
        let root = std::env::temp_dir().join(format!("spindle-zip-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut extracted = 0;
        assert!(extract_archive(Cursor::new(bytes.into_inner()), &root, &mut extracted).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_archives_without_server_after_extraction() {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file("readme.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"not a runtime").unwrap();
            writer.finish().unwrap();
        }
        let root = std::env::temp_dir().join(format!("spindle-zip-empty-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut extracted = 0;
        assert!(extract_archive(Cursor::new(bytes.into_inner()), &root, &mut extracted).is_ok());
        assert!(find_server(&root).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
