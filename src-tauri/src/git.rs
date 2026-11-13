use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_GIT_OUTPUT: usize = 16 * 1024 * 1024;
const MAX_REVIEW_DIFF: usize = 2 * 1024 * 1024;
const MAX_APPLY_PATCH: usize = 8 * 1024 * 1024;
const MAX_HISTORY: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRepositoryStatus {
    pub root_path: String,
    pub head: String,
    pub branch: String,
    pub clean: bool,
    pub changed_files: usize,
    pub staged_files: usize,
    pub untracked_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitSummary {
    pub commit: String,
    pub short_commit: String,
    pub author: String,
    pub committed_at: i64,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskWorktree {
    pub task_id: String,
    pub project_id: String,
    pub path: String,
    pub base_commit: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskWorktreeDiff {
    pub worktree: TaskWorktree,
    pub status: GitRepositoryStatus,
    pub diff: String,
    pub diff_truncated: bool,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .unwrap_or_default().as_secs().min(i64::MAX as u64) as i64
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Spindle data directory: {error}"))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("Could not open task worktree database: {error}"))?;
    connection.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS task_worktrees (
           task_id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL,
           path TEXT NOT NULL UNIQUE,
           base_commit TEXT NOT NULL,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_task_worktrees_project ON task_worktrees(project_id);
         CREATE TABLE IF NOT EXISTS task_worktree_applications (
           task_id TEXT PRIMARY KEY,
           project_id TEXT NOT NULL,
           root_path TEXT NOT NULL,
           base_commit TEXT NOT NULL,
           patch TEXT NOT NULL,
           patch_sha256 TEXT NOT NULL,
           status TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_task_worktree_applications_status
           ON task_worktree_applications(status, updated_at);"
    ).map_err(|error| format!("Could not initialize task worktree storage: {error}"))?;
    Ok(connection)
}

fn git_executable() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let executable = "git.exe";
    #[cfg(not(windows))]
    let executable = "git";
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(executable);
        if candidate.is_file() {
            return candidate.canonicalize()
                .map_err(|error| format!("Could not resolve Git executable: {error}"));
        }
    }
    Err("Git was not found on PATH. Install Git and restart Spindle.".into())
}

fn run_git(root: &Path, args: &[&str]) -> Result<Output, String> {
    let hooks_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let hooks_config = format!("core.hooksPath={hooks_path}");
    let mut command = Command::new(git_executable()?);
    command.current_dir(root)
        .args([
            "-c", hooks_config.as_str(), "-c", "core.pager=cat",
            "-c", "credential.helper=", "-c", "core.fsmonitor=false",
            "--no-optional-locks",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_LFS_SKIP_SMUDGE", "1");
    crate::child_process::hide_console_window(&mut command);
    let output = command.output()
        .map_err(|error| format!("Could not start Git: {error}"))?;
    if output.stdout.len().saturating_add(output.stderr.len()) > MAX_GIT_OUTPUT {
        return Err("Git produced more than the permitted 2 MiB output limit.".into());
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git operation failed: {}", detail.chars().take(1_000).collect::<String>()));
    }
    Ok(output)
}

fn run_git_input(root: &Path, args: &[&str], input: &[u8]) -> Result<Output, String> {
    let hooks_path = if cfg!(windows) { "NUL" } else { "/dev/null" };
    let hooks_config = format!("core.hooksPath={hooks_path}");
    let mut command = Command::new(git_executable()?);
    command.current_dir(root)
        .args([
            "-c", hooks_config.as_str(), "-c", "core.pager=cat",
            "-c", "credential.helper=", "-c", "core.fsmonitor=false",
            "--no-optional-locks",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::child_process::hide_console_window(&mut command);
    let mut child = command.spawn()
        .map_err(|error| format!("Could not start Git: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input)
            .map_err(|error| format!("Could not send the reviewed patch to Git: {error}"))?;
    }
    let output = child.wait_with_output()
        .map_err(|error| format!("Could not collect Git's patch result: {error}"))?;
    if output.stdout.len().saturating_add(output.stderr.len()) > MAX_GIT_OUTPUT {
        return Err("Git produced more than the permitted 16 MiB output limit.".into());
    }
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Git patch operation failed: {}", detail.chars().take(1_000).collect::<String>()));
    }
    Ok(output)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(run_git(root, args)?.stdout)
        .map_err(|_| "Git returned output that was not valid UTF-8.".into())
}

fn repository_root(path: &Path) -> Result<PathBuf, String> {
    let selected = path.canonicalize()
        .map_err(|error| format!("Could not resolve project directory: {error}"))?;
    if !selected.is_dir() {
        return Err("The selected project path is not a directory.".into());
    }
    let root = git_text(&selected, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root.trim()).canonicalize()
        .map_err(|error| format!("Could not resolve Git repository root: {error}"))?;
    if root != selected {
        return Err("Git operations require the registered project directory to be the repository root.".into());
    }
    Ok(root)
}

pub fn status(project_root: &str) -> Result<GitRepositoryStatus, String> {
    let root = repository_root(Path::new(project_root))?;
    reject_configured_filters(&root)?;
    let head = git_text(&root, &["rev-parse", "HEAD"])?.trim().to_owned();
    let branch = git_text(&root, &["branch", "--show-current"])?.trim().to_owned();
    let raw = git_text(&root, &["status", "--porcelain=v1", "--branch", "--untracked-files=normal"])?;
    let mut changed = 0;
    let mut staged = 0;
    let mut untracked = 0;
    let mut status_branch = String::new();
    for line in raw.lines() {
        if let Some(value) = line.strip_prefix("## ") {
            status_branch = value.to_owned();
            continue;
        }
        let bytes = line.as_bytes();
        if bytes.len() < 2 { continue; }
        if bytes[0] != b' ' || bytes[1] != b' ' {
            changed += 1;
            if bytes[0] != b' ' { staged += 1; }
            if bytes[0] == b'?' && bytes[1] == b'?' { untracked += 1; }
        }
    }
    let display_branch = if branch.is_empty() {
        if status_branch.is_empty() { format!("detached@{}", &head[..head.len().min(8)]) } else { status_branch }
    } else { branch };
    Ok(GitRepositoryStatus {
        root_path: root.to_string_lossy().into_owned(),
        head,
        branch: display_branch,
        clean: changed == 0,
        changed_files: changed,
        staged_files: staged,
        untracked_files: untracked,
    })
}

pub fn history(project_root: &str, limit: usize) -> Result<Vec<GitCommitSummary>, String> {
    let root = repository_root(Path::new(project_root))?;
    let limit = limit.clamp(1, MAX_HISTORY);
    let count = format!("--max-count={limit}");
    let raw = git_text(&root, &[
        "log", &count, "--date=unix",
        "--pretty=format:%H%x09%h%x09%an%x09%at%x09%s",
    ])?;
    let mut commits = Vec::new();
    for line in raw.lines() {
        let mut parts = line.splitn(5, '\t');
        let (Some(commit), Some(short_commit), Some(author), Some(timestamp), Some(subject)) =
            (parts.next(), parts.next(), parts.next(), parts.next(), parts.next()) else { continue; };
        let Ok(committed_at) = timestamp.parse::<i64>() else { continue; };
        commits.push(GitCommitSummary {
            commit: commit.to_owned(),
            short_commit: short_commit.to_owned(),
            author: author.chars().take(120).collect(),
            committed_at,
            subject: subject.chars().take(240).collect(),
        });
    }
    Ok(commits)
}

fn reject_configured_filters(root: &Path) -> Result<(), String> {
    let mut command = Command::new(git_executable()?);
    command.current_dir(root)
        .args(["config", "--local", "--get-regexp", "^filter\\..*\\.(clean|smudge|process)$"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_TERMINAL_PROMPT", "0");
    crate::child_process::hide_console_window(&mut command);
    let output = command.output()
        .map_err(|error| format!("Could not inspect Git filter configuration: {error}"))?;
    if output.status.success() && !output.stdout.is_empty() {
        return Err("This repository configures external Git filters. Spindle refuses to check out a task worktree because filters can execute arbitrary programs in the application process. Disable the repository's filter drivers first.".into());
    }
    if output.status.code() != Some(1) && !output.status.success() {
        return Err("Could not safely inspect repository Git filter configuration.".into());
    }
    Ok(())
}

pub fn create_task_worktree(
    database_path: &Path,
    project_id: &str,
    project_root: &str,
    task_id: &str,
    storage_root: &Path,
) -> Result<TaskWorktree, String> {
    validate_task_id(task_id)?;
    let root = repository_root(Path::new(project_root))?;
    let connection = open_database(database_path)?;
    let task_state: Option<String> = connection.query_row(
        "SELECT state FROM agent_tasks WHERE id=?1 AND project_id=?2",
        params![task_id, project_id],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if task_state.as_deref() != Some("approved") {
        return Err("A task must have an explicitly approved plan before creating its isolated worktree.".into());
    }
    let existing: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_worktrees WHERE task_id=?1)",
        params![task_id], |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if existing {
        return Err("An isolated worktree already exists for this task.".into());
    }
    let repo_status = status(project_root)?;
    if !repo_status.clean {
        return Err("The project working tree is not clean. Commit, stash, or resolve its changes before creating an isolated task worktree.".into());
    }
    let target_root = storage_root.join("task-worktrees");
    std::fs::create_dir_all(&target_root)
        .map_err(|error| format!("Could not create isolated worktree storage: {error}"))?;
    let target_root = target_root.canonicalize()
        .map_err(|error| format!("Could not resolve isolated worktree storage: {error}"))?;
    let target = target_root.join(task_id);
    if target.exists() {
        return Err("An unregistered task worktree already exists at the expected path.".into());
    }
    let base_commit = repo_status.head;
    let target_arg = target.to_string_lossy().into_owned();
    run_git(&root, &["worktree", "add", "--detach", &target_arg, &base_commit])?;
    let worktree = TaskWorktree {
        task_id: task_id.to_owned(),
        project_id: project_id.to_owned(),
        path: target.to_string_lossy().into_owned(),
        base_commit,
        created_at: now(),
    };
    if let Err(error) = connection.execute(
        "INSERT INTO task_worktrees(task_id,project_id,path,base_commit,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![worktree.task_id, worktree.project_id, worktree.path, worktree.base_commit, worktree.created_at],
    ) {
        let _ = run_git(&root, &["worktree", "remove", "--force", &target_arg]);
        return Err(format!("Could not persist the isolated worktree record: {error}"));
    }
    Ok(worktree)
}

pub fn get_worktree(database_path: &Path, task_id: &str) -> Result<TaskWorktree, String> {
    validate_task_id(task_id)?;
    let connection = open_database(database_path)?;
    let worktree = connection.query_row(
        "SELECT task_id,project_id,path,base_commit,created_at FROM task_worktrees WHERE task_id=?1",
        params![task_id],
        |row| Ok(TaskWorktree {
            task_id: row.get(0)?, project_id: row.get(1)?, path: row.get(2)?,
            base_commit: row.get(3)?, created_at: row.get(4)?,
        }),
    ).optional().map_err(|error| error.to_string())?
      .ok_or_else(|| "This task does not have an isolated worktree.".to_string())?;
    let data_root = database_path.parent().ok_or_else(|| "Spindle database path has no parent.".to_string())?;
    let expected = data_root.join("task-worktrees").join(task_id).canonicalize()
        .map_err(|error| format!("Could not resolve the registered task worktree: {error}"))?;
    let stored = PathBuf::from(&worktree.path).canonicalize()
        .map_err(|error| format!("Could not resolve the stored task worktree: {error}"))?;
    if stored != expected {
        return Err("The stored task worktree path failed its application-data ownership check.".into());
    }
    Ok(worktree)
}

pub fn worktree_diff(database_path: &Path, task_id: &str) -> Result<TaskWorktreeDiff, String> {
    let worktree = get_worktree(database_path, task_id)?;
    let root = repository_root(Path::new(&worktree.path))?;
    reject_configured_filters(&root)?;

    let untracked = git_text(&root, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    for relative in untracked.split('\0').filter(|path| !path.is_empty()) {
        validate_review_path(relative)?;
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect an untracked task file: {error}"))?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_file() || metadata.len() > 1024 * 1024 {
            return Err(format!("Untracked task file '{relative}' is a link, special file, or exceeds the 1 MiB review limit."));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("Could not read untracked task file '{relative}': {error}"))?;
        std::str::from_utf8(&bytes)
            .map_err(|_| format!("Untracked task file '{relative}' is not UTF-8 text and cannot be reviewed."))?;
        run_git(&root, &["add", "-N", "--", relative])?;
    }

    let raw_paths = git_text(&root, &["diff", "--name-only", "-z", &worktree.base_commit, "--"])?;
    let mut paths = 0usize;
    for path in raw_paths.split('\0').filter(|path| !path.is_empty()) {
        validate_changed_file(&root, path)?;
        paths += 1;
        if paths > 500 {
            return Err("The task contains more than 500 changed paths. Review is capped at 500 files.".into());
        }
    }
    let status = status(&worktree.path)?;
    let output = run_git(&root, &[
        "diff", "--no-ext-diff", "--no-textconv", "--no-color", "--binary",
        "--unified=3", &worktree.base_commit, "--",
    ])?;
    let truncated = output.stdout.len() > MAX_REVIEW_DIFF;
    let diff = String::from_utf8_lossy(&output.stdout[..output.stdout.len().min(MAX_REVIEW_DIFF)])
        .into_owned();
    if diff.contains("GIT binary patch") || diff.contains("Binary files ") {
        return Err("Binary file changes are not supported by the text based review and apply workflow.".into());
    }
    Ok(TaskWorktreeDiff { worktree, status, diff, diff_truncated: truncated })
}

pub fn remove_worktree(
    database_path: &Path,
    project_root: &str,
    task_id: &str,
) -> Result<(), String> {
    let worktree = get_worktree(database_path, task_id)?;
    let root = repository_root(Path::new(project_root))?;
    let path = PathBuf::from(&worktree.path).canonicalize()
        .map_err(|error| format!("Could not resolve task worktree path: {error}"))?;
    let parent = path.parent().ok_or_else(|| "Task worktree path is invalid.".to_string())?;
    if !parent.ends_with("task-worktrees") || !path.ends_with(task_id) {
        return Err("The stored task worktree path failed its ownership check.".into());
    }
    let current_head = git_text(&path, &["rev-parse", "HEAD"])?;
    if current_head.trim() != worktree.base_commit {
        return Err("The task worktree contains commits beyond its recorded base. Preserve or integrate those commits before removing it.".into());
    }
    if !status(&worktree.path)?.clean {
        return Err("The task worktree contains changed or untracked files. Review or preserve them before removing it.".into());
    }
    let target = path.to_string_lossy().into_owned();
    run_git(&root, &["worktree", "remove", &target])?;
    let connection = open_database(database_path)?;
    connection.execute("DELETE FROM task_worktrees WHERE task_id=?1", params![task_id])
        .map_err(|error| format!("Could not clear task worktree metadata: {error}"))?;
    Ok(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskApplicationResult {
    pub task_id: String,
    pub status: String,
    pub changed_files: usize,
    pub patch_sha256: String,
    pub applied_at: i64,
}

fn validate_changed_file(root: &Path, relative: &str) -> Result<(), String> {
    validate_review_path(relative)?;
    let target = root.join(relative);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if is_reparse_or_symlink(&metadata) || !metadata.is_file() || metadata.len() > 1024 * 1024 {
                return Err(format!("Changed file '{relative}' is a link, special file, or exceeds the 1 MiB review limit."));
            }
            let canonical = target.canonicalize()
                .map_err(|error| format!("Could not resolve changed task file '{relative}': {error}"))?;
            if !canonical.starts_with(root) {
                return Err(format!("Changed file '{relative}' resolves outside the task worktree."));
            }
            let bytes = std::fs::read(&canonical)
                .map_err(|error| format!("Could not read changed task file '{relative}': {error}"))?;
            std::str::from_utf8(&bytes)
                .map_err(|_| format!("Changed file '{relative}' is not UTF-8 text and cannot be applied."))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = target.parent()
                .ok_or_else(|| format!("Deleted file '{relative}' has no parent directory."))?;
            let canonical_parent = parent.canonicalize()
                .map_err(|error| format!("Could not resolve parent of deleted file '{relative}': {error}"))?;
            if !canonical_parent.starts_with(root) {
                return Err(format!("Deleted file '{relative}' resolves outside the task worktree."));
            }
        }
        Err(error) => return Err(format!("Could not inspect changed task file '{relative}': {error}")),
    }
    Ok(())
}

fn validate_review_path(value: &str) -> Result<(), String> {
    let path = Path::new(value);
    if value.is_empty() || value.len() > 4096 || path.is_absolute() || value.chars().any(char::is_control) {
        return Err("A changed file path is invalid for review.".into());
    }
    let mut count = 0usize;
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            return Err("Changed file paths may not contain traversal, roots, or prefixes.".into());
        };
        let name = name.to_string_lossy().to_ascii_lowercase();
        if name == ".git" || name.starts_with(".env") || matches!(
            name.as_str(), ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg" | ".npmrc" |
            ".pypirc" | "id_rsa" | "id_ed25519" | "credentials" | "credentials.json" |
            "service-account.json" | "secrets.json"
        ) || [".pem", ".key", ".p12", ".pfx", ".asc"].iter().any(|suffix| name.ends_with(suffix)) {
            return Err("Secret or Git metadata paths are excluded from task review and apply.".into());
        }
        count += 1;
    }
    if count == 0 { return Err("A changed file path is required.".into()); }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub fn apply_worktree_changes(
    database_path: &Path,
    project_root: &str,
    task_id: &str,
) -> Result<TaskApplicationResult, String> {
    validate_task_id(task_id)?;
    let worktree = get_worktree(database_path, task_id)?;
    let root = repository_root(Path::new(project_root))?;
    let state_db = open_database(database_path)?;
    let task_state: String = state_db.query_row(
        "SELECT state FROM agent_tasks WHERE id=?1 AND project_id=?2",
        params![task_id, worktree.project_id],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?
        .ok_or_else(|| "The selected task no longer exists.".to_string())?;
    if task_state != "completed" {
        return Err("Only a completed agent task can be applied to the project working tree.".into());
    }
    let primary = status(project_root)?;
    if !primary.clean || primary.head != worktree.base_commit {
        return Err("The registered project must remain clean at the task's base commit. Commit or stash project changes, then recreate the task from the current HEAD before applying.".into());
    }
    let reviewed = worktree_diff(database_path, task_id)?;
    if reviewed.diff_truncated {
        return Err("The task diff exceeds the 2 MiB review limit. Reduce the change set before applying.".into());
    }
    if reviewed.diff.trim().is_empty() {
        return Err("The task has no changes to apply.".into());
    }
    if reviewed.diff.len() > MAX_APPLY_PATCH {
        return Err("The task patch exceeds the 8 MiB application limit.".into());
    }
    let changed_paths = git_text(Path::new(&worktree.path), &[
        "diff", "--name-only", "-z", &worktree.base_commit, "--",
    ])?;
    let mut path_count = 0usize;
    for path in changed_paths.split('\0').filter(|path| !path.is_empty()) {
        validate_review_path(path)?;
        path_count += 1;
        if path_count > 500 { return Err("The task contains more than 500 changed files.".into()); }
    }
    if path_count == 0 { return Err("No supported text file changes were found.".into()); }
    if reviewed.diff.contains("GIT binary patch") || reviewed.diff.contains("Binary files ") {
        return Err("Binary changes cannot be applied through the reviewed text workflow.".into());
    }

    let patch_sha256 = format!("{:x}", sha2::Sha256::digest(reviewed.diff.as_bytes()));
    let existing: Option<String> = state_db.query_row(
        "SELECT status FROM task_worktree_applications WHERE task_id=?1",
        params![task_id],
        |row| row.get(0),
    ).optional().map_err(|error| error.to_string())?;
    if matches!(existing.as_deref(), Some("applied" | "applying")) {
        return Err("This task already has an active application record. Undo it or inspect the project before applying again.".into());
    }

    run_git_input(&root, &["apply", "--check", "--binary", "--"], reviewed.diff.as_bytes())?;
    let timestamp = now();
    state_db.execute(
        "INSERT INTO task_worktree_applications(task_id,project_id,root_path,base_commit,patch,patch_sha256,status,created_at,updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,'applying',?7,?7)
         ON CONFLICT(task_id) DO UPDATE SET project_id=excluded.project_id,root_path=excluded.root_path,
         base_commit=excluded.base_commit,patch=excluded.patch,patch_sha256=excluded.patch_sha256,
         status='applying',created_at=excluded.created_at,updated_at=excluded.updated_at",
        params![task_id, worktree.project_id, root.to_string_lossy(), worktree.base_commit, reviewed.diff, patch_sha256, timestamp],
    ).map_err(|error| format!("Could not persist the task application journal: {error}"))?;

    if let Err(error) = run_git_input(&root, &["apply", "--binary", "--"], reviewed.diff.as_bytes()) {
        let _ = state_db.execute(
            "UPDATE task_worktree_applications SET status='failed',updated_at=?1 WHERE task_id=?2 AND status='applying'",
            params![now(), task_id],
        );
        return Err(error);
    }
    state_db.execute(
        "UPDATE task_worktree_applications SET status='applied',updated_at=?1 WHERE task_id=?2 AND patch_sha256=?3",
        params![now(), task_id, patch_sha256],
    ).map_err(|error| format!("The patch was applied, but its application journal could not be finalized: {error}"))?;
    Ok(TaskApplicationResult {
        task_id: task_id.to_owned(),
        status: "applied".into(),
        changed_files: path_count,
        patch_sha256,
        applied_at: now(),
    })
}

pub fn worktree_application_status(
    database_path: &Path,
    task_id: &str,
) -> Result<Option<TaskApplicationResult>, String> {
    let connection = open_database(database_path)?;
    connection.query_row(
        "SELECT task_id,status,patch_sha256,updated_at,patch
         FROM task_worktree_applications WHERE task_id=?1",
        params![task_id],
        |row| {
            let patch: String = row.get(4)?;
            Ok(TaskApplicationResult {
                task_id: row.get(0)?,
                status: row.get(1)?,
                patch_sha256: row.get(2)?,
                applied_at: row.get(3)?,
                changed_files: patch.matches("diff --git ").count(),
            })
        },
    ).optional().map_err(|error| error.to_string())
}

pub fn undo_worktree_application(
    database_path: &Path,
    task_id: &str,
) -> Result<TaskApplicationResult, String> {
    validate_task_id(task_id)?;
    let connection = open_database(database_path)?;
    let (root_path, patch, patch_sha, status_value): (String, String, String, String) = connection.query_row(
        "SELECT root_path,patch,patch_sha256,status FROM task_worktree_applications WHERE task_id=?1",
        params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).optional().map_err(|error| error.to_string())?
      .ok_or_else(|| "This task has no recorded project application to undo.".to_string())?;
    if status_value != "applied" {
        return Err("Only a completed task application can be undone.".into());
    }
    if format!("{:x}", sha2::Sha256::digest(patch.as_bytes())) != patch_sha {
        return Err("The stored application patch failed its integrity check. Undo was refused.".into());
    }
    let root = repository_root(Path::new(&root_path))?;
    run_git_input(&root, &["apply", "--reverse", "--check", "--binary", "--"], patch.as_bytes())?;
    run_git_input(&root, &["apply", "--reverse", "--binary", "--"], patch.as_bytes())?;
    connection.execute(
        "UPDATE task_worktree_applications SET status='undone',updated_at=?1 WHERE task_id=?2 AND status='applied'",
        params![now(), task_id],
    ).map_err(|error| format!("The reverse patch was applied, but the application journal could not be updated: {error}"))?;
    Ok(TaskApplicationResult {
        task_id: task_id.to_owned(),
        status: "undone".into(),
        changed_files: 0,
        patch_sha256: patch_sha,
        applied_at: now(),
    })
}

pub fn recover_worktree_applications(database_path: &Path) -> Result<usize, String> {
    let connection = open_database(database_path)?;
    let mut statement = connection.prepare(
        "SELECT task_id,root_path,patch,patch_sha256 FROM task_worktree_applications WHERE status='applying'",
    ).map_err(|error| error.to_string())?;
    let rows = statement.query_map([], |row| Ok((
        row.get::<_, String>(0)?, row.get::<_, String>(1)?,
        row.get::<_, String>(2)?, row.get::<_, String>(3)?
    ))).map_err(|error| error.to_string())?;
    let records = rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut recovered = 0usize;
    for (task_id, root_path, patch, patch_sha) in records {
        if format!("{:x}", sha2::Sha256::digest(patch.as_bytes())) != patch_sha {
            connection.execute(
                "UPDATE task_worktree_applications SET status='conflict',updated_at=?1 WHERE task_id=?2",
                params![now(), task_id],
            ).map_err(|error| error.to_string())?;
            continue;
        }
        let root = match repository_root(Path::new(&root_path)) {
            Ok(root) => root,
            Err(_) => {
                connection.execute(
                    "UPDATE task_worktree_applications SET status='conflict',updated_at=?1 WHERE task_id=?2 AND status='applying'",
                    params![now(), task_id],
                ).map_err(|error| error.to_string())?;
                recovered += 1;
                continue;
            }
        };
        let applied = run_git_input(&root, &["apply", "--reverse", "--check", "--binary", "--"], patch.as_bytes()).is_ok();
        let not_applied = run_git_input(&root, &["apply", "--check", "--binary", "--"], patch.as_bytes()).is_ok();
        let recovered_state = if applied { "applied" } else if not_applied { "failed" } else { "conflict" };
        connection.execute(
            "UPDATE task_worktree_applications SET status=?1,updated_at=?2 WHERE task_id=?3 AND status='applying'",
            params![recovered_state, now(), task_id],
        ).map_err(|error| error.to_string())?;
        recovered += 1;
    }
    Ok(recovered)
}

fn validate_task_id(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 160 ||
        !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("The task identifier is invalid for an isolated worktree.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ids_reject_path_syntax() {
        assert!(validate_task_id("task-123-abc").is_ok());
        assert!(validate_task_id("../outside").is_err());
        assert!(validate_task_id("C:\\outside").is_err());
        assert!(validate_task_id("").is_err());
    }

    #[test]
    fn review_paths_reject_traversal_and_secrets() {
        assert!(validate_review_path("src/main.rs").is_ok());
        assert!(validate_review_path("../outside").is_err());
        assert!(validate_review_path(".env.production").is_err());
        assert!(validate_review_path("config/credentials.json").is_err());
        assert!(validate_review_path(".git/config").is_err());
    }

    #[test]
    fn history_limit_is_bounded() {
        assert_eq!(1usize.clamp(1, MAX_HISTORY), 1);
        assert_eq!(500usize.clamp(1, MAX_HISTORY), MAX_HISTORY);
    }
}
