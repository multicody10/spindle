use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_OUTPUT_BYTES: usize = 96 * 1024;
const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationCommand {
    CargoFmt,
    CargoCheck,
    CargoTest,
}

impl ValidationCommand {
    pub fn label(self) -> &'static str {
        match self {
            Self::CargoFmt => "cargo fmt --manifest-path <project manifest> --all -- --check",
            Self::CargoCheck => "cargo check --locked --manifest-path <project manifest> --all-targets",
            Self::CargoTest => "cargo test --locked --manifest-path <project manifest>",
        }
    }

    fn args(self, manifest: &str) -> Vec<String> {
        let args = match self {
            Self::CargoFmt => vec!["fmt", "--manifest-path", manifest, "--all", "--", "--check"],
            Self::CargoCheck => vec!["check", "--locked", "--manifest-path", manifest, "--all-targets"],
            Self::CargoTest => vec!["test", "--locked", "--manifest-path", manifest],
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
        return match rappct::supports_lpac() {
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
        };
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
    #[cfg(windows)]
    {
        run_validation_windows(worktree_path, command)
    }
    #[cfg(not(windows))]
    {
        let _ = (worktree_path, command);
        Err("The restricted execution worker is available only on Windows.".into())
    }
}

#[cfg(windows)]
fn run_validation_windows(
    worktree_path: &str,
    command: ValidationCommand,
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
    if matches!(command, ValidationCommand::CargoCheck | ValidationCommand::CargoTest) {
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
        suspended: false,
        join_job: Some(JobLimits {
            memory_bytes: Some(2 * 1024 * 1024 * 1024usize),
            cpu_rate_percent: Some(80),
            kill_on_job_close: true,
        }),
        startup_timeout: Some(Duration::from_secs(15)),
    };

    let started = Instant::now();
    let mut child = launch_in_container_with_io(&capabilities, &options)
        .map_err(|error| format!("The AppContainer worker could not be started; no unsandboxed fallback was attempted: {error}"))?;
    drop(child.stdin.take());
    let stdout = child.stdout.take().map(|mut stream| thread::spawn(move || read_capped(&mut stream, MAX_OUTPUT_BYTES)));
    let stderr = child.stderr.take().map(|mut stream| thread::spawn(move || read_capped(&mut stream, MAX_OUTPUT_BYTES)));
    let exit_code = child.wait(Some(EXECUTION_TIMEOUT))
        .map_err(|error| format!("The restricted worker timed out or failed; its Job Object was closed: {error}"))?;
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
        isolation: "Windows AppContainer + LPAC; no network capabilities; 2 GiB memory limit; 80% CPU cap; kill-on-job-close".into(),
    })
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
        assert_eq!(ValidationCommand::CargoFmt.args("src-tauri/Cargo.toml").join(" "), "fmt --manifest-path src-tauri/Cargo.toml --all -- --check");
        assert_eq!(ValidationCommand::CargoCheck.args("Cargo.toml").join(" "), "check --locked --manifest-path Cargo.toml --all-targets");
        assert_eq!(ValidationCommand::CargoTest.args("Cargo.toml").join(" "), "test --locked --manifest-path Cargo.toml");
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
