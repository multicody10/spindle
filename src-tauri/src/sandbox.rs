use serde::{Deserialize, Serialize};
use std::{path::{Path, PathBuf}, sync::{atomic::{AtomicBool, Ordering}, Arc}};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 96 * 1024;
const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_WORKER_PROCESSES: u32 = 64;
const MAX_WORKER_PROCESS_MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;
const MAX_WORKER_JOB_MEMORY_BYTES: usize = 4 * 1024 * 1024 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCommand {
    Format,
    Check,
    Test,
}

impl ValidationCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::Format => "cargo fmt --manifest-path <project manifest> --all -- --check",
            Self::Check => "cargo check --locked --manifest-path <project manifest> --all-targets",
            Self::Test => "cargo test --locked --manifest-path <project manifest>",
        }
    }

    fn args(self, manifest: &str) -> Vec<String> {
        let args = match self {
            Self::Format => vec!["fmt", "--manifest-path", manifest, "--all", "--", "--check"],
            Self::Check => vec!["check", "--locked", "--manifest-path", manifest, "--all-targets"],
            Self::Test => vec!["test", "--locked", "--manifest-path", manifest],
        };
        args.into_iter().map(str::to_owned).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxStatus {
    pub supported: bool,
    pub platform: String,
    pub isolation: String,
    pub network: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxExecutionResult {
    pub command: String,
    pub exit_code: u32,
    pub success: bool,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub output_truncated: bool,
    pub isolation: String,
}

pub fn status() -> SandboxStatus {
    #[cfg(windows)]
    {
        match rappct::supports_lpac() {
            Ok(()) => SandboxStatus {
                supported: true,
                platform: "windows".into(),
                isolation: "AppContainer + LPAC + Job Object".into(),
                network: "No network capabilities granted".into(),
                reason: None,
            },
            Err(error) => SandboxStatus {
                supported: false,
                platform: "windows".into(),
                isolation: "unavailable".into(),
                network: "disabled".into(),
                reason: Some(format!("Windows LPAC is unavailable: {error}")),
            },
        }
    }
    #[cfg(not(windows))]
    {
        SandboxStatus {
            supported: false,
            platform: std::env::consts::OS.into(),
            isolation: "unavailable".into(),
            network: "disabled".into(),
            reason: Some("The restricted execution worker is currently Windows-only.".into()),
        }
    }
}

pub fn run_validation(
    worktree_path: &str,
    command: ValidationCommand,
) -> Result<SandboxExecutionResult, String> {
    run_validation_cancellable(worktree_path, command, None)
}

pub fn run_validation_cancellable(
    worktree_path: &str,
    command: ValidationCommand,
    cancellation: Option<Arc<AtomicBool>>,
) -> Result<SandboxExecutionResult, String> {
    #[cfg(windows)]
    {
        run_validation_windows(worktree_path, command, cancellation)
    }
    #[cfg(not(windows))]
    {
        let _ = (worktree_path, command, cancellation);
        Err("The restricted execution worker is available only on Windows.".into())
    }
}

#[cfg(windows)]
fn run_validation_windows(
    worktree_path: &str,
    command: ValidationCommand,
    cancellation: Option<Arc<AtomicBool>>,
) -> Result<SandboxExecutionResult, String> {
    use rappct::{
        acl::{grant_to_package, AccessMask, ResourcePath},
        launch::{JobLimits, LaunchOptions, StdioConfig, launch_in_container_with_io},
        AppContainerProfile, SecurityCapabilitiesBuilder,
    };
    use std::ffi::{OsStr, OsString};
    use std::os::windows::fs::MetadataExt;
    use std::thread;

    rappct::supports_lpac()
        .map_err(|error| format!("Windows LPAC is required; execution denied: {error}"))?;

    let worktree = PathBuf::from(worktree_path).canonicalize()
        .map_err(|error| format!("Could not resolve isolated task worktree: {error}"))?;
    let metadata = std::fs::symlink_metadata(&worktree)
        .map_err(|error| format!("Could not inspect isolated task worktree: {error}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 {
        return Err("The task worktree is not a regular directory.".into());
    }

    let manifest = if worktree.join("Cargo.toml").is_file() {
        worktree.join("Cargo.toml")
    } else if worktree.join("src-tauri").join("Cargo.toml").is_file() {
        worktree.join("src-tauri").join("Cargo.toml")
    } else {
        return Err("No supported Cargo.toml was found at the project root or src-tauri/Cargo.toml.".into());
    };
    let manifest = manifest.to_string_lossy().into_owned();
    let cargo = find_executable("cargo.exe")?;
    let rustc = find_executable("rustc.exe")?;
    let rustfmt = find_executable("rustfmt.exe")?;
    let rustup_home = rustup_home()?;
    let profile = AppContainerProfile::ensure(
        "com.spindle.agent.worker",
        "Spindle Restricted Agent Worker",
        Some("Isolated process identity for reviewed project validation"),
    ).map_err(|error| format!("Could not create the Windows AppContainer identity: {error}"))?;
    let capabilities = SecurityCapabilitiesBuilder::new(&profile.sid)
        .with_lpac_defaults()
        .lpac(true)
        .build()
        .map_err(|error| format!("Could not construct an LPAC security token: {error}"))?;

    let profile_home = profile.folder_path()
        .map_err(|error| format!("Could not resolve the worker profile directory: {error}"))?;
    let cargo_home = profile_home.join("cargo-home");
    let temp = profile_home.join("Temp");
    for path in [&profile_home, &cargo_home, &temp] {
        std::fs::create_dir_all(path)
            .map_err(|error| format!("Could not prepare the isolated worker profile: {error}"))?;
    }

    grant_to_package(
        ResourcePath::Directory(worktree.clone()), &profile.sid, AccessMask::GENERIC_ALL,
    ).map_err(|error| format!("Could not grant access to the isolated task worktree: {error}"))?;
    grant_to_package(
        ResourcePath::Directory(profile_home.clone()), &profile.sid, AccessMask::GENERIC_ALL,
    ).map_err(|error| format!("Could not grant access to the private worker profile: {error}"))?;
    grant_to_package(
        ResourcePath::Directory(rustup_home.clone()), &profile.sid, AccessMask::FILE_GENERIC_READ,
    ).map_err(|error| format!("Could not grant read-only access to the Rust toolchain: {error}"))?;

    let source_cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|path| PathBuf::from(path).join(".cargo")))
        .ok_or_else(|| "Could not resolve the user's Cargo cache directory.".to_string())?;
    for cache_name in ["registry", "git"] {
        let source = source_cargo_home.join(cache_name);
        let destination = cargo_home.join(cache_name);
        if source.is_dir() && !destination.exists() {
            let mut copied = 0u64;
            copy_cache_tree(&source, &destination, &mut copied)?;
        }
    }

    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    let system32 = system_root.join("System32");
    let mut path_entries = vec![
        cargo.parent().ok_or("Cargo executable path has no parent.")?.to_path_buf(),
        rustc.parent().ok_or("rustc executable path has no parent.")?.to_path_buf(),
        rustfmt.parent().ok_or("rustfmt executable path has no parent.")?.to_path_buf(),
        system32.clone(),
    ];
    if matches!(command, ValidationCommand::Check | ValidationCommand::Test) {
        if let Ok(linker) = find_executable("link.exe") {
            if let Some(parent) = linker.parent() {
                let parent = parent.to_path_buf();
                grant_to_package(
                    ResourcePath::Directory(parent.clone()), &profile.sid, AccessMask::FILE_GENERIC_READ,
                ).map_err(|error| format!("Could not grant read-only access to the linker: {error}"))?;
                path_entries.push(parent);
            }
        }
    }
    path_entries.sort();
    path_entries.dedup();
    for path in &path_entries {
        if path != &system32 {
            grant_to_package(
                ResourcePath::Directory(path.clone()), &profile.sid, AccessMask::FILE_GENERIC_READ,
            ).map_err(|error| format!("Could not grant read-only access to a toolchain directory: {error}"))?;
        }
    }
    let path_value = std::env::join_paths(path_entries)
        .map_err(|error| format!("Could not build the isolated worker PATH: {error}"))?;

    let mut env = vec![
        (OsString::from("SystemRoot"), system_root.as_os_str().to_owned()),
        (OsString::from("windir"), system_root.as_os_str().to_owned()),
        (OsString::from("ComSpec"), system32.join("cmd.exe").into_os_string()),
        (OsString::from("PATHEXT"), OsString::from(".COM;.EXE;.BAT;.CMD")),
        (OsString::from("PATH"), path_value),
        (OsString::from("USERPROFILE"), profile_home.as_os_str().to_owned()),
        (OsString::from("HOME"), profile_home.as_os_str().to_owned()),
        (OsString::from("TEMP"), temp.as_os_str().to_owned()),
        (OsString::from("TMP"), temp.as_os_str().to_owned()),
        (OsString::from("CARGO_HOME"), cargo_home.into_os_string()),
        (OsString::from("RUSTUP_HOME"), rustup_home.into_os_string()),
        (OsString::from("RUSTUP_AUTO_INSTALL"), OsString::from("0")),
        (OsString::from("CARGO_NET_OFFLINE"), OsString::from("true")),
        (OsString::from("CARGO_TARGET_DIR"), worktree.join("target").into_os_string()),
        (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
    ];
    for key in ["INCLUDE", "LIB", "LIBPATH"] {
        if let Some(value) = std::env::var_os(key) {
            for directory in std::env::split_paths(&value) {
                if directory.is_dir() {
                    let resolved = directory.canonicalize()
                        .map_err(|error| format!("Could not resolve the {key} toolchain directory: {error}"))?;
                    grant_to_package(
                        ResourcePath::Directory(resolved), &profile.sid, AccessMask::FILE_GENERIC_READ,
                    ).map_err(|error| format!("Could not grant read-only access to the {key} toolchain directory: {error}"))?;
                }
            }
            env.push((OsString::from(key), value));
        }
    }

    let command_args = command.args(&manifest);
    let command_line = std::iter::once(cargo.as_os_str())
        .chain(command_args.iter().map(OsStr::new))
        .map(quote_windows_argument)
        .collect::<Vec<_>>()
        .join(" ");
    let options = LaunchOptions {
        exe: cargo,
        cmdline: Some(command_line),
        cwd: Some(worktree.clone()),
        env: Some(env),
        stdio: StdioConfig::Pipe,
        // rappct assigns the still-suspended process to its Job Object before
        // returning. We apply and read back the complete job policy before the
        // primary thread is resumed.
        suspended: true,
        join_job: Some(JobLimits {
            memory_bytes: Some(MAX_WORKER_PROCESS_MEMORY_BYTES),
            cpu_rate_percent: Some(80),
            kill_on_job_close: true,
        }),
        startup_timeout: Some(Duration::from_secs(15)),
    };

    let started = Instant::now();
    let mut child = launch_in_container_with_io(&capabilities, &options)
        .map_err(|error| format!("The AppContainer worker could not be started; no unsandboxed fallback was attempted: {error}"))?;
    let job_handle = match child.job_guard.as_ref() {
        Some(guard) => guard.as_handle().0,
        None => {
            let terminated = terminate_worker_process(child.pid);
            let detail = if terminated {
                "the suspended worker was terminated"
            } else {
                "Windows did not confirm termination of the suspended process"
            };
            return Err(format!("The AppContainer launcher did not return its required Job Object guard; {detail}."));
        }
    };
    configure_worker_job(job_handle)?;
    resume_suspended_worker(child.pid)?;
    drop(child.stdin.take());
    let stdout = child.stdout.take().map(|mut stream| thread::spawn(move || read_capped(&mut stream, MAX_OUTPUT_BYTES)));
    let stderr = child.stderr.take().map(|mut stream| thread::spawn(move || read_capped(&mut stream, MAX_OUTPUT_BYTES)));
    let wait_thread = thread::spawn(move || child.wait(Some(EXECUTION_TIMEOUT)));
    let wait_started = Instant::now();
    let exit_code = loop {
        if wait_thread.is_finished() {
            break wait_thread.join()
                .map_err(|_| "The restricted worker wait thread failed.".to_string())?
                .map_err(|error| format!("The restricted worker timed out or failed; its Job Object was closed: {error}"))?;
        }
        if cancellation.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
            let terminated = unsafe { terminate_job_object(job_handle, 1223) };
            if terminated {
                let _ = wait_thread.join();
                return Err("Restricted validation was cancelled. The worker Job Object and its child processes were terminated.".into());
            }
            return Err("Restricted validation cancellation was requested, but Windows refused to terminate the worker Job Object. The configured ten-minute worker timeout remains active.".into());
        }
        if wait_started.elapsed() >= EXECUTION_TIMEOUT + Duration::from_secs(2) {
            let terminated = unsafe { terminate_job_object(job_handle, 1460) };
            if terminated {
                let _ = wait_thread.join();
                return Err("Restricted validation exceeded its ten-minute timeout. The worker Job Object and its child processes were terminated.".into());
            }
            return Err("Restricted validation exceeded its ten-minute timeout; worker termination could not be confirmed.".into());
        }
        thread::sleep(Duration::from_millis(40));
    };
    let (stdout, stdout_truncated) = join_output(stdout)?;
    let (stderr, stderr_truncated) = join_output(stderr)?;
    Ok(SandboxExecutionResult {
        command: command.label().into(),
        exit_code,
        success: exit_code == 0,
        duration_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        stdout: redact_output(&String::from_utf8_lossy(&stdout)),
        stderr: redact_output(&String::from_utf8_lossy(&stderr)),
        output_truncated: stdout_truncated || stderr_truncated,
        isolation: "Windows AppContainer + LPAC; no network capabilities; suspended launch; 64-process Job Object cap; 2 GiB per-process and 4 GiB aggregate memory caps; 80% CPU cap; no breakaway; kill-on-job-close".into(),
    })
}

#[cfg(windows)]
fn configure_worker_job(job: windows::Win32::Foundation::HANDLE) -> Result<(), String> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows::Win32::System::JobObjects::{
        QueryInformationJobObject, SetInformationJobObject,
        JobObjectExtendedLimitInformation,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_BREAKAWAY_OK,
        JOB_OBJECT_LIMIT_JOB_MEMORY,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK,
    };

    let required = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_JOB_MEMORY
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let mut requested = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    requested.BasicLimitInformation.LimitFlags = required;
    requested.BasicLimitInformation.ActiveProcessLimit = MAX_WORKER_PROCESSES;
    requested.ProcessMemoryLimit = MAX_WORKER_PROCESS_MEMORY_BYTES;
    requested.JobMemoryLimit = MAX_WORKER_JOB_MEMORY_BYTES;

    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &requested as *const _ as *const c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|error| format!("Could not enforce the restricted worker Job Object policy: {error}"))?;

    let mut effective = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    let mut returned_length = 0u32;
    unsafe {
        QueryInformationJobObject(
            Some(job),
            JobObjectExtendedLimitInformation,
            &mut effective as *mut _ as *mut c_void,
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            Some(&mut returned_length),
        )
    }
    .map_err(|error| format!("Could not verify the restricted worker Job Object policy: {error}"))?;

    let flags = effective.BasicLimitInformation.LimitFlags;
    let required_bits = required.0;
    let breakaway_bits =
        (JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK).0;
    if flags.0 & required_bits != required_bits
        || flags.0 & breakaway_bits != 0
        || effective.BasicLimitInformation.ActiveProcessLimit != MAX_WORKER_PROCESSES
        || effective.ProcessMemoryLimit != MAX_WORKER_PROCESS_MEMORY_BYTES
        || effective.JobMemoryLimit != MAX_WORKER_JOB_MEMORY_BYTES
        || returned_length < size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32
    {
        return Err("Windows did not retain the complete worker Job Object policy. The worker remains suspended and will be terminated.".into());
    }
    Ok(())
}

#[cfg(windows)]
fn resume_suspended_worker(process_id: u32) -> Result<(), String> {
    use std::mem::size_of;
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_NO_MORE_FILES, HANDLE,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, THREADENTRY32, TH32CS_SNAPTHREAD,
    };
    use windows::Win32::System::Threading::{
        OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
    };

    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe { let _ = CloseHandle(self.0); }
        }
    }

    let snapshot = OwnedHandle(
        unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
            .map_err(|error| format!("Could not inspect suspended worker threads: {error}"))?,
    );
    let mut entry = THREADENTRY32::default();
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    unsafe { Thread32First(snapshot.0, &mut entry) }
        .map_err(|error| format!("Could not enumerate suspended worker threads: {error}"))?;

    let mut primary_thread_id = None;
    loop {
        if entry.th32OwnerProcessID == process_id {
            if primary_thread_id.replace(entry.th32ThreadID).is_some() {
                return Err("The suspended worker unexpectedly has multiple threads before initialization; refusing to resume it.".into());
            }
        }
        match unsafe { Thread32Next(snapshot.0, &mut entry) } {
            Ok(()) => {}
            Err(error) => {
                let last_error = unsafe { GetLastError() };
                if last_error == ERROR_NO_MORE_FILES {
                    break;
                }
                return Err(format!("Could not complete suspended worker thread enumeration: {error}"));
            }
        }
    }

    let thread_id = primary_thread_id
        .ok_or_else(|| "The suspended worker's primary thread could not be identified.".to_string())?;
    let thread = OwnedHandle(
        unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, thread_id) }
            .map_err(|error| format!("Could not open the suspended worker's primary thread: {error}"))?,
    );
    let previous_suspend_count = unsafe { ResumeThread(thread.0) };
    if previous_suspend_count != 1 {
        return Err(format!(
            "The worker primary thread had an unexpected suspend count ({previous_suspend_count}); the Job Object will terminate it instead of running with an unverified startup state."
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn terminate_worker_process(process_id: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let Ok(process) = (unsafe { OpenProcess(PROCESS_TERMINATE, false, process_id) }) else {
        return false;
    };
    let terminated = unsafe { TerminateProcess(process, 1) }.is_ok();
    let closed = unsafe { CloseHandle(process) }.is_ok();
    terminated && closed
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn TerminateJobObject(job: *mut std::ffi::c_void, exit_code: u32) -> i32;
}

#[cfg(windows)]
unsafe fn terminate_job_object(job: *mut std::ffi::c_void, exit_code: u32) -> bool {
    TerminateJobObject(job, exit_code) != 0
}

#[cfg(windows)]
fn find_executable(name: &str) -> Result<PathBuf, String> {
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return candidate.canonicalize()
                .map_err(|error| format!("Could not resolve {name}: {error}"));
        }
    }
    Err(format!("{name} was not found on PATH. Install the Rust toolchain and restart Spindle."))
}

#[cfg(windows)]
fn rustup_home() -> Result<PathBuf, String> {
    let path = std::env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".rustup")))
        .ok_or_else(|| "Could not resolve RUSTUP_HOME.".to_string())?;
    let path = path.canonicalize()
        .map_err(|error| format!("Could not resolve the Rust toolchain directory: {error}"))?;
    if !path.is_dir() { return Err("RUSTUP_HOME is not a directory.".into()); }
    Ok(path)
}

#[cfg(windows)]
fn copy_cache_tree(source: &Path, destination: &Path, copied: &mut u64) -> Result<(), String> {
    use std::fs;
    use std::os::windows::fs::MetadataExt;
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create isolated Cargo cache: {error}"))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("Could not read Cargo cache: {error}"))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|error| format!("Could not inspect Cargo cache entry: {error}"))?;
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0 { continue; }
        if metadata.is_dir() {
            copy_cache_tree(&source_path, &target_path, copied)?;
        } else if metadata.is_file() {
            *copied = copied.saturating_add(metadata.len());
            if *copied > MAX_CACHE_BYTES {
                return Err("Cargo's offline cache exceeds the 2 GiB worker copy limit. Execution was not started.".into());
            }
            fs::copy(&source_path, &target_path)
                .map_err(|error| format!("Could not copy Cargo's offline cache: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn quote_windows_argument(value: &std::ffi::OsStr) -> String {
    let value = value.to_string_lossy();
    if !value.is_empty() && !value.chars().any(|character| character.is_whitespace() || character == '"') {
        return value.into_owned();
    }
    let mut output = String::from("\"");
    let mut backslashes = 0usize;
    for character in value.chars() {
        if character == '\\' {
            backslashes += 1;
        } else if character == '"' {
            output.push_str(&"\\".repeat(backslashes * 2 + 1));
            output.push('"');
            backslashes = 0;
        } else {
            output.push_str(&"\\".repeat(backslashes));
            backslashes = 0;
            output.push(character);
        }
    }
    output.push_str(&"\\".repeat(backslashes * 2));
    output.push('"');
    output
}

#[cfg(windows)]
fn read_capped(stream: &mut impl std::io::Read, limit: usize) -> (Vec<u8>, bool) {
    let mut output = Vec::with_capacity(limit.min(16 * 1024));
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        match std::io::Read::read(stream, &mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                let keep = limit.saturating_sub(output.len()).min(count);
                output.extend_from_slice(&buffer[..keep]);
                truncated |= keep < count;
            }
        }
    }
    (output, truncated)
}

#[cfg(windows)]
fn join_output(
    output: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>,
) -> Result<(Vec<u8>, bool), String> {
    match output {
        Some(handle) => handle.join().map_err(|_| "Could not collect restricted worker output.".to_string()),
        None => Ok((Vec::new(), false)),
    }
}

fn redact_output(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let rest = &input[index..];
        let lower = rest.to_ascii_lowercase();
        let marker = [
            "authorization: bearer ", "bearer ", "password=", "passwd=", "secret=",
            "api_key=", "apikey=", "access_token=", "refresh_token=",
        ].iter().find(|marker| lower.starts_with(**marker));
        if let Some(marker) = marker {
            output.push_str(&input[index..index + marker.len()]);
            index += marker.len();
            while index < bytes.len() && !bytes[index].is_ascii_whitespace()
                && !matches!(bytes[index], b';' | b',' | b'"' | b'\'')
            { index += 1; }
            output.push_str("[REDACTED]");
            continue;
        }
        let token_prefix = ["github_pat_", "ghp_", "gho_", "ghs_", "ghu_", "xoxb-", "xoxp-"];
        if let Some(prefix) = token_prefix.iter().find(|prefix| lower.starts_with(**prefix)) {
            output.push_str("[REDACTED]");
            index += prefix.len();
            while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-')) {
                index += 1;
            }
            continue;
        }
        let character = rest.chars().next().expect("non-empty remainder");
        output.push(character);
        index += character.len_utf8();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_commands_are_fixed_and_do_not_accept_arguments() {
        assert_eq!(ValidationCommand::Format.args("src-tauri/Cargo.toml").join(" "), "fmt --manifest-path src-tauri/Cargo.toml --all -- --check");
        assert_eq!(ValidationCommand::Check.args("Cargo.toml").join(" "), "check --locked --manifest-path Cargo.toml --all-targets");
        assert_eq!(ValidationCommand::Test.args("Cargo.toml").join(" "), "test --locked --manifest-path Cargo.toml");
    }

    #[cfg(windows)]
    #[test]
    fn worker_job_policy_is_applied_and_read_back_before_resume() {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::JobObjects::CreateJobObjectW;
        use windows::core::PCWSTR;

        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .expect("a private Windows Job Object should be creatable");
        let result = configure_worker_job(job);
        unsafe { CloseHandle(job) }.expect("the test Job Object should close");
        result.expect("the full process and memory policy must be retained by Windows");
    }

    #[test]
    fn output_redaction_removes_common_bearer_and_provider_tokens() {
        let value = "Authorization: Bearer abcdefghijkl password=hunter2 ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let redacted = redact_output(value);
        assert!(!redacted.contains("abcdefghijkl"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
        assert!(redacted.contains("[REDACTED]"));
    }
}
