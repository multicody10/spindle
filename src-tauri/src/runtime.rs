use crate::domain::ModelConfiguration;
use crate::hash::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, Cursor, Read, Write},
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
const MAX_DOWNLOAD_BYTES: usize = 80 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 180 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 256;
const MAX_LOG_LINES: usize = 400;
const MAX_LOG_LINE_BYTES: usize = 8 * 1024;
const HEALTH_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuntimeBackend {
    Cpu,
    Vulkan,
}

impl RuntimeBackend {
    fn asset(self) -> &'static str {
        match self {
            Self::Cpu => "llama-b12347-bin-win-cpu-x64.zip",
            Self::Vulkan => "llama-b12347-bin-win-vulkan-x64.zip",
        }
    }

    fn sha256(self) -> &'static str {
        match self {
            Self::Cpu => "476a30ac458f4881051a26c541707440de4aa744d878c739669289cbe137aa09",
            Self::Vulkan => "7dccbc28c03dc929687df80894d736c9fd91c71b88537f7f1a7d2eefb85a86c5",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Vulkan => "vulkan",
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
    find_server(&directory)
        .ok_or_else(|| "The installed llama.cpp executable is missing.".to_string())
}

fn backend_status(
    app: &AppHandle,
    backend: RuntimeBackend,
) -> Result<BackendInstallStatus, String> {
    let path = install_dir(app, backend)?;
    Ok(BackendInstallStatus {
        backend,
        installed: find_server(&path).is_some(),
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

    let url = format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/{VERSION}/{}",
        backend.asset()
    );
    runtime.push_log(format!(
        "Downloading pinned llama.cpp {VERSION} ({})…",
        backend.id()
    ));

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the runtime download client.".to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| format!("Could not download llama.cpp: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "The llama.cpp download returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_DOWNLOAD_BYTES as u64)
    {
        return Err("The llama.cpp archive exceeded the download size limit.".into());
    }
    let mut response = response;
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_DOWNLOAD_BYTES as u64) as usize,
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the llama.cpp download.".to_string())?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_DOWNLOAD_BYTES {
            return Err("The llama.cpp archive exceeded the download size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }

    let digest = sha256_hex(&bytes);
    if digest != backend.sha256() {
        runtime.push_log("Runtime archive checksum verification failed.");
        return Err(
            "The downloaded llama.cpp archive failed SHA-256 verification. Nothing was installed."
                .into(),
        );
    }
    runtime.push_log("Runtime archive checksum verified. Extracting files…");

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

    let extraction = extract_archive(&bytes, &staging);
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    let server = find_server(&staging).ok_or_else(|| {
        let _ = fs::remove_dir_all(&staging);
        "The verified archive did not contain llama-server.exe.".to_string()
    })?;
    let marker = staging.join("spindle-runtime.json");
    let metadata = serde_json::json!({
        "provider": "llama.cpp",
        "version": VERSION,
        "backend": backend.id(),
        "asset": backend.asset(),
        "sha256": digest
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

fn extract_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| "The downloaded runtime archive is not a valid ZIP file.".to_string())?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("The runtime archive contains too many entries.".into());
    }

    let mut extracted = 0u64;
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
        extracted = extracted.saturating_add(size);
        if size > MAX_DOWNLOAD_BYTES as u64 || extracted > MAX_EXTRACTED_BYTES {
            return Err("The extracted runtime exceeded the uncompressed size limit.".into());
        }
        let output = destination.join(relative);
        let parent = output
            .parent()
            .ok_or_else(|| "The runtime archive contains an invalid path.".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create a runtime directory: {error}"))?;
        let mut file = File::create(&output)
            .map_err(|error| format!("Could not extract a runtime file: {error}"))?;
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
    crate::validate_model_constraints(&configuration)?;
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
            if backend == RuntimeBackend::Vulkan && configuration.runtime.kv_cache_on_gpu {
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

    #[test]
    fn runtime_assets_are_pinned_and_distinct() {
        assert_ne!(
            RuntimeBackend::Cpu.sha256(),
            RuntimeBackend::Vulkan.sha256()
        );
        assert!(RuntimeBackend::Cpu.asset().contains(VERSION));
        assert!(RuntimeBackend::Vulkan.asset().contains(VERSION));
        assert_eq!(RuntimeBackend::Cpu.sha256().len(), 64);
        assert_eq!(RuntimeBackend::Vulkan.sha256().len(), 64);
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
        assert!(extract_archive(bytes.get_ref(), &root).is_err());
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
        assert!(extract_archive(bytes.get_ref(), &root).is_ok());
        assert!(find_server(&root).is_none());
        let _ = fs::remove_dir_all(root);
    }
}
