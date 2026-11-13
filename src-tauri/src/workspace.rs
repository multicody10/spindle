use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_PATH_BYTES: usize = 4 * 1024;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_DIRECTORY_ENTRIES: usize = 500;
const MAX_SEARCH_FILES: usize = 2_000;
const MAX_SEARCH_BYTES: usize = 16 * 1024 * 1024;
const MAX_SEARCH_FILE_BYTES: usize = 512 * 1024;
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_SEARCH_DEPTH: usize = 16;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);
static CHANGE_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGrant {
    pub session_id: String,
    pub root_path: String,
    pub can_read: bool,
    pub can_write: bool,
    pub temporary: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: String,
    pub size_bytes: Option<u64>,
    pub modified_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceListing {
    pub relative_path: String,
    pub entries: Vec<WorkspaceEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFile {
    pub relative_path: String,
    pub content: String,
    pub size_bytes: usize,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWriteResult {
    pub relative_path: String,
    pub bytes_written: usize,
    pub revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchMatch {
    pub relative_path: String,
    pub line_number: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSearchResult {
    pub matches: Vec<WorkspaceSearchMatch>,
    pub files_scanned: usize,
    pub bytes_scanned: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
struct WorkspaceSession {
    grant: WorkspaceGrant,
    root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceChangeProposal {
    pub id: String,
    pub relative_path: String,
    pub base_revision: String,
    pub proposed_revision: String,
    pub diff: String,
    pub added_lines: usize,
    pub removed_lines: usize,
    pub status: String,
    pub created_at_unix_ms: u64,
}

#[derive(Clone)]
struct StoredFileChange {
    session_id: String,
    proposal: WorkspaceChangeProposal,
    original_content: String,
    proposed_content: String,
}

#[derive(Clone, Default)]
pub struct WorkspaceManager {
    active: Arc<Mutex<Option<WorkspaceSession>>>,
    changes: Arc<Mutex<BTreeMap<String, StoredFileChange>>>,
}

impl WorkspaceManager {
    pub fn status(&self) -> Result<Option<WorkspaceGrant>, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        Ok(active.as_ref().map(|session| session.grant.clone()))
    }

    pub fn grant(&self, path: String, allow_write: bool) -> Result<WorkspaceGrant, String> {
        if path.trim().is_empty() || path.len() > MAX_PATH_BYTES {
            return Err("Choose a valid workspace directory path.".into());
        }

        let root = fs::canonicalize(PathBuf::from(path.trim()))
            .map_err(|error| format!("Could not resolve the workspace directory: {error}"))?;
        if !root.is_dir() {
            return Err("The selected workspace path is not a directory.".into());
        }
        if root.parent().is_none() {
            return Err("Grant a project directory, not a filesystem root.".into());
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let grant = WorkspaceGrant {
            session_id: format!("{:x}-{:x}-{:x}", std::process::id(), timestamp, counter),
            root_path: root.to_string_lossy().into_owned(),
            can_read: true,
            can_write: allow_write,
            temporary: true,
        };
        let session = WorkspaceSession {
            grant: grant.clone(),
            root,
        };

        let mut active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        let previous_id = active.as_ref().map(|value| value.grant.session_id.clone());
        *active = Some(session);
        drop(active);
        if let Some(previous_id) = previous_id {
            self.changes
                .lock()
                .map_err(|_| "The workspace change store is unavailable.".to_string())?
                .retain(|_, change| change.session_id != previous_id);
        }
        Ok(grant)
    }

    pub fn revoke(&self, session_id: &str) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        let matches = active
            .as_ref()
            .is_some_and(|session| session.grant.session_id == session_id);
        if !matches {
            return if active.is_some() {
                Err("The workspace session identifier is stale.".into())
            } else {
                Ok(())
            };
        }
        *active = None;
        drop(active);
        self.changes
            .lock()
            .map_err(|_| "The workspace change store is unavailable.".to_string())?
            .retain(|_, change| change.session_id != session_id);
        Ok(())
    }

    fn session(&self, session_id: &str, require_write: bool) -> Result<WorkspaceSession, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        let session = active
            .as_ref()
            .filter(|session| session.grant.session_id == session_id)
            .ok_or_else(|| {
                "Workspace access has expired or was revoked. Grant the directory again."
                    .to_string()
            })?;

        if !session.grant.can_read {
            return Err("This workspace session does not have read permission.".into());
        }
        if require_write && !session.grant.can_write {
            return Err("File writes are not permitted for this workspace session.".into());
        }
        Ok(session.clone())
    }

    pub fn list_directory(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> Result<WorkspaceListing, String> {
        let session = self.session(session_id, false)?;
        let directory = resolve_directory(&session, relative_path)?;
        let mut entries = Vec::new();
        let mut truncated = false;

        let iterator = fs::read_dir(&directory)
            .map_err(|error| format!("Could not list this workspace directory: {error}"))?;
        for entry in iterator {
            let entry =
                entry.map_err(|error| format!("Could not read a directory entry: {error}"))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("Could not inspect workspace entry: {error}"))?;
            if metadata.file_type().is_symlink() || is_sensitive_name(&name) {
                continue;
            }
            if name.starts_with(".spindle-tmp-") {
                continue;
            }

            let kind = if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                continue;
            };
            let relative = relative_display(&session.root, &path)?;
            entries.push(WorkspaceEntry {
                name,
                relative_path: relative,
                kind: kind.into(),
                size_bytes: metadata.is_file().then_some(metadata.len()),
                modified_unix_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64),
            });
            if entries.len() > MAX_DIRECTORY_ENTRIES {
                entries.pop();
                truncated = true;
                break;
            }
        }

        entries.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        Ok(WorkspaceListing {
            relative_path: relative_path.replace('\\', "/"),
            entries,
            truncated,
        })
    }

    pub fn read_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> Result<WorkspaceFile, String> {
        let session = self.session(session_id, false)?;
        let path = resolve_file(&session, relative_path)?;
        if is_sensitive_path(Path::new(relative_path)) {
            return Err(
                "This file is classified as secret material and cannot be read by workspace tools."
                    .into(),
            );
        }

        let metadata = fs::metadata(&path)
            .map_err(|error| format!("Could not inspect the selected workspace file: {error}"))?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err("The selected file exceeds the 1 MiB workspace read limit.".into());
        }
        let bytes = read_bounded(&path, MAX_FILE_BYTES)
            .map_err(|error| format!("Could not read the selected workspace file: {error}"))?;
        let revision = format!("{:x}", Sha256::digest(&bytes));
        let content = String::from_utf8(bytes).map_err(|_| {
            "The selected file is not valid UTF-8 text and cannot be opened in the editor."
                .to_string()
        })?;
        let size_bytes = content.len();
        Ok(WorkspaceFile {
            relative_path: relative_path.replace('\\', "/"),
            content,
            size_bytes,
            revision,
        })
    }

    pub fn stage_file_change(
        &self,
        session_id: &str,
        relative_path: &str,
        content: String,
        expected_revision: &str,
    ) -> Result<WorkspaceChangeProposal, String> {
        let session = self.session(session_id, true)?;
        if content.len() > MAX_FILE_BYTES || content.contains('\0') {
            return Err("Workspace changes must be valid UTF-8 text no larger than 1 MiB.".into());
        }
        if relative_path.trim().is_empty() || is_sensitive_path(Path::new(relative_path)) {
            return Err("A valid, non-sensitive project file path is required.".into());
        }
        validate_revision(expected_revision)?;
        let path = resolve_file(&session, relative_path)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect the selected workspace file: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_FILE_BYTES as u64 {
            return Err("Workspace changes are limited to regular text files no larger than 1 MiB.".into());
        }
        let original_bytes = read_bounded(&path, MAX_FILE_BYTES)
            .map_err(|error| format!("Could not read the current file revision: {error}"))?;
        let base_revision = format!("{:x}", Sha256::digest(&original_bytes));
        if base_revision != expected_revision {
            return Err("This file changed outside Spindle after it was opened. Reopen it before staging a change.".into());
        }
        let original_content = String::from_utf8(original_bytes)
            .map_err(|_| "The selected file is not valid UTF-8 text.".to_string())?;
        if original_content == content {
            return Err("The proposed file content is identical to the current file.".into());
        }

        let proposed_revision = format!("{:x}", Sha256::digest(content.as_bytes()));
        let (diff, added_lines, removed_lines) =
            build_diff(relative_path, &original_content, &content);
        let timestamp = now_unix_ms();
        let proposal = WorkspaceChangeProposal {
            id: format!(
                "change-{:x}-{:x}-{:x}",
                std::process::id(),
                timestamp,
                CHANGE_COUNTER.fetch_add(1, Ordering::Relaxed)
            ),
            relative_path: relative_path.replace('\\', "/"),
            base_revision,
            proposed_revision,
            diff,
            added_lines,
            removed_lines,
            status: "pending".into(),
            created_at_unix_ms: timestamp,
        };
        let stored = StoredFileChange {
            session_id: session_id.to_owned(),
            proposal: proposal.clone(),
            original_content,
            proposed_content: content,
        };
        let mut changes = self
            .changes
            .lock()
            .map_err(|_| "The workspace change store is unavailable.".to_string())?;
        if changes.values().any(|change| {
            change.session_id == session_id && change.proposal.status == "pending"
        }) {
            return Err("Review or discard the existing staged change before creating another.".into());
        }
        if changes.len() >= 100 {
            return Err("The workspace change history is full. Restart Spindle to clear its temporary history.".into());
        }
        changes.insert(proposal.id.clone(), stored);
        Ok(proposal)
    }

    pub fn apply_file_change(
        &self,
        session_id: &str,
        proposal_id: &str,
    ) -> Result<WorkspaceWriteResult, String> {
        let session = self.session(session_id, true)?;
        let mut changes = self
            .changes
            .lock()
            .map_err(|_| "The workspace change store is unavailable.".to_string())?;
        let change = changes
            .get_mut(proposal_id)
            .filter(|change| change.session_id == session_id)
            .ok_or_else(|| "The staged file change does not exist for this workspace session.".to_string())?;
        if change.proposal.status != "pending" {
            return Err("Only a pending file change can be applied.".into());
        }
        let path = resolve_file(&session, &change.proposal.relative_path)?;
        let current = read_bounded(&path, MAX_FILE_BYTES)
            .map_err(|error| format!("Could not verify the current file before applying the change: {error}"))?;
        let revision = format!("{:x}", Sha256::digest(&current));
        if revision != change.proposal.base_revision {
            return Err("The file changed after this preview was created. Discard the preview, reopen the file, and stage the change again.".into());
        }
        atomic_replace(
            &session.root,
            &path,
            change.proposed_content.as_bytes(),
            &change.proposal.base_revision,
        )?;
        change.proposal.status = "applied".into();
        Ok(WorkspaceWriteResult {
            relative_path: change.proposal.relative_path.clone(),
            bytes_written: change.proposed_content.len(),
            revision: change.proposal.proposed_revision.clone(),
        })
    }

    pub fn undo_file_change(
        &self,
        session_id: &str,
        proposal_id: &str,
    ) -> Result<WorkspaceWriteResult, String> {
        let session = self.session(session_id, true)?;
        let mut changes = self
            .changes
            .lock()
            .map_err(|_| "The workspace change store is unavailable.".to_string())?;
        let change = changes
            .get_mut(proposal_id)
            .filter(|change| change.session_id == session_id)
            .ok_or_else(|| "The applied file change does not exist for this workspace session.".to_string())?;
        if change.proposal.status != "applied" {
            return Err("Only an applied file change can be undone.".into());
        }
        let path = resolve_file(&session, &change.proposal.relative_path)?;
        let current = read_bounded(&path, MAX_FILE_BYTES)
            .map_err(|error| format!("Could not verify the file before undo: {error}"))?;
        let revision = format!("{:x}", Sha256::digest(&current));
        if revision != change.proposal.proposed_revision {
            return Err("The file changed after Spindle applied this change. Undo was refused to protect newer edits.".into());
        }
        atomic_replace(
            &session.root,
            &path,
            change.original_content.as_bytes(),
            &change.proposal.proposed_revision,
        )?;
        change.proposal.status = "undone".into();
        Ok(WorkspaceWriteResult {
            relative_path: change.proposal.relative_path.clone(),
            bytes_written: change.original_content.len(),
            revision: change.proposal.base_revision.clone(),
        })
    }

    pub fn cancel_file_change(&self, session_id: &str, proposal_id: &str) -> Result<(), String> {
        let _session = self.session(session_id, true)?;
        let mut changes = self
            .changes
            .lock()
            .map_err(|_| "The workspace change store is unavailable.".to_string())?;
        let change = changes
            .get_mut(proposal_id)
            .filter(|change| change.session_id == session_id)
            .ok_or_else(|| "The staged file change does not exist for this workspace session.".to_string())?;
        if change.proposal.status != "pending" {
            return Err("Only a pending file change can be discarded.".into());
        }
        change.proposal.status = "cancelled".into();
        Ok(())
    }

    pub fn search(&self, session_id: &str, query: &str) -> Result<WorkspaceSearchResult, String> {
        let session = self.session(session_id, false)?;
        let query = query.trim();
        if query.is_empty() || query.len() > 256 {
            return Err("Enter a search term between 1 and 256 bytes.".into());
        }
        let needle = query.to_lowercase();
        let mut pending = vec![(session.root.clone(), 0usize)];
        let mut matches = Vec::new();
        let mut files_scanned = 0usize;
        let mut bytes_scanned = 0usize;
        let mut truncated = false;

        while let Some((directory, depth)) = pending.pop() {
            if depth > MAX_SEARCH_DEPTH
                || files_scanned >= MAX_SEARCH_FILES
                || matches.len() >= MAX_SEARCH_RESULTS
                || bytes_scanned >= MAX_SEARCH_BYTES
            {
                truncated = true;
                break;
            }
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                if files_scanned >= MAX_SEARCH_FILES
                    || matches.len() >= MAX_SEARCH_RESULTS
                    || bytes_scanned >= MAX_SEARCH_BYTES
                {
                    truncated = true;
                    break;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_sensitive_name(&name) || name.starts_with(".spindle-tmp-") {
                    continue;
                }
                let path = entry.path();
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(_) => continue,
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                let canonical = match fs::canonicalize(&path) {
                    Ok(path) if path.starts_with(&session.root) => path,
                    _ => continue,
                };
                if metadata.is_dir() {
                    if !is_ignored_directory(&name) {
                        pending.push((canonical, depth + 1));
                    }
                    continue;
                }
                if !metadata.is_file() || metadata.len() > MAX_SEARCH_FILE_BYTES as u64 {
                    continue;
                }

                let relative = match relative_display(&session.root, &canonical) {
                    Ok(relative) => relative,
                    Err(_) => continue,
                };
                if is_sensitive_path(Path::new(&relative)) {
                    continue;
                }

                let remaining_bytes = MAX_SEARCH_BYTES.saturating_sub(bytes_scanned);
                if metadata.len() > remaining_bytes as u64 {
                    truncated = true;
                    break;
                }
                files_scanned += 1;
                let bytes =
                    match read_bounded(&canonical, remaining_bytes.min(MAX_SEARCH_FILE_BYTES)) {
                        Ok(bytes) => bytes,
                        Err(_) => {
                            truncated = true;
                            break;
                        }
                    };
                bytes_scanned = bytes_scanned.saturating_add(bytes.len());
                let text = match std::str::from_utf8(&bytes) {
                    Ok(text) if !text.contains('\0') => text,
                    _ => continue,
                };
                for (line_index, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(&needle) {
                        matches.push(WorkspaceSearchMatch {
                            relative_path: relative.clone(),
                            line_number: line_index + 1,
                            preview: line.chars().take(400).collect(),
                        });
                        if matches.len() >= MAX_SEARCH_RESULTS {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
        }

        Ok(WorkspaceSearchResult {
            matches,
            files_scanned,
            bytes_scanned,
            truncated,
        })
    }
}

fn validate_revision(revision: &str) -> Result<(), String> {
    if revision.len() != 64
        || !revision.chars().all(|character| character.is_ascii_hexdigit())
    {
        return Err("The file revision token is invalid. Reopen the file before staging a change.".into());
    }
    Ok(())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn build_diff(path: &str, original: &str, proposed: &str) -> (String, usize, usize) {
    const MAX_DIFF_LINES: usize = 300;
    const MAX_DIFF_BYTES: usize = 64 * 1024;
    let old_lines = original.split('\n').collect::<Vec<_>>();
    let new_lines = proposed.split('\n').collect::<Vec<_>>();
    let mut prefix = 0usize;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let removed = old_lines.len().saturating_sub(prefix + suffix);
    let added = new_lines.len().saturating_sub(prefix + suffix);
    let context_start = prefix.saturating_sub(3);
    let old_end = old_lines.len().saturating_sub(suffix);
    let new_end = new_lines.len().saturating_sub(suffix);
    let suffix_context = suffix.min(3);
    let mut output = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        context_start + 1,
        old_end.saturating_sub(context_start) + suffix_context,
        context_start + 1,
        new_end.saturating_sub(context_start) + suffix_context
    );
    for line in old_lines.iter().take(prefix).skip(context_start) {
        output.push(' ');
        output.push_str(line);
        output.push('\n');
    }
    for line in old_lines.iter().take(old_end).skip(prefix).take(MAX_DIFF_LINES) {
        output.push('-');
        output.push_str(line);
        output.push('\n');
    }
    for line in new_lines.iter().take(new_end).skip(prefix).take(MAX_DIFF_LINES) {
        output.push('+');
        output.push_str(line);
        output.push('\n');
    }
    if removed > MAX_DIFF_LINES || added > MAX_DIFF_LINES {
        output.push_str("... diff preview truncated; line counts above are exact ...\n");
    }
    let suffix_start = old_lines.len().saturating_sub(suffix);
    for line in old_lines.iter().skip(suffix_start).take(3) {
        output.push(' ');
        output.push_str(line);
        output.push('\n');
    }
    if output.len() > MAX_DIFF_BYTES {
        output = output.chars().take(MAX_DIFF_BYTES).collect();
        output.push_str("\n... diff preview truncated ...");
    }
    (output, added, removed)
}

fn atomic_replace(
    root: &Path,
    target: &Path,
    content: &[u8],
    expected_revision: &str,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or_else(|| "The selected workspace file has no parent directory.".to_string())?;
    let temporary = parent.join(format!(
        ".spindle-tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| format!("Could not create a temporary workspace file: {error}"))?;
        file.write_all(content)
            .map_err(|error| format!("Could not write the workspace file: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Could not flush the workspace file: {error}"))?;
        drop(file);

        let metadata = fs::symlink_metadata(target)
            .map_err(|error| format!("Could not revalidate the target file: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Workspace replacement is limited to regular files, not links or special files.".into());
        }
        let canonical = fs::canonicalize(target)
            .map_err(|error| format!("Could not revalidate the target path: {error}"))?;
        ensure_inside_root(root, &canonical)?;
        let current = read_bounded(&canonical, MAX_FILE_BYTES)
            .map_err(|error| format!("Could not recheck the target revision: {error}"))?;
        let current_revision = format!("{:x}", Sha256::digest(&current));
        if current_revision != expected_revision {
            return Err("The file changed immediately before replacement. The operation was aborted.".into());
        }
        fs::rename(&temporary, &canonical)
            .map_err(|error| format!("Could not atomically replace the workspace file: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(
        file.metadata()
            .map(|metadata| metadata.len().min(max_bytes as u64) as usize)
            .unwrap_or(0),
    );
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > max_bytes {
        return Err("The file grew beyond the permitted read limit.".into());
    }
    Ok(bytes)
}

fn validate_relative_path(value: &str, allow_root: bool) -> Result<PathBuf, String> {
    if value.len() > MAX_PATH_BYTES || value.contains('\0') {
        return Err("The workspace-relative path is invalid or too long.".into());
    }
    if value.trim().is_empty() {
        return if allow_root {
            Ok(PathBuf::new())
        } else {
            Err("A file path is required.".into())
        };
    }

    let path = Path::new(value);
    if path.is_absolute() {
        return Err("Workspace tools accept relative paths only.".into());
    }

    let mut clean = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() || part.contains(':') || part == "." || part == ".." {
                    return Err("The workspace path contains an invalid component.".into());
                }
                clean.push(part.as_ref());
            }
            _ => return Err("The workspace path contains an invalid component.".into()),
        }
    }

    if clean.as_os_str().is_empty() && !allow_root {
        return Err("A file path is required.".into());
    }
    Ok(clean)
}

fn resolve_directory(session: &WorkspaceSession, relative: &str) -> Result<PathBuf, String> {
    if is_sensitive_path(Path::new(relative)) {
        return Err("Sensitive credential directories are not exposed to workspace tools.".into());
    }
    let clean = validate_relative_path(relative, true)?;
    let mut current = session.root.clone();
    for component in clean.components() {
        let Component::Normal(part) = component else {
            return Err("The workspace path contains an invalid component.".into());
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Could not resolve the workspace directory: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Workspace paths may not traverse links or non-directory entries.".into());
        }
    }
    let canonical = fs::canonicalize(&current)
        .map_err(|error| format!("Could not resolve the workspace directory: {error}"))?;
    ensure_inside_root(&session.root, &canonical)?;
    Ok(canonical)
}

fn resolve_file(session: &WorkspaceSession, relative: &str) -> Result<PathBuf, String> {
    let clean = validate_relative_path(relative, false)?;
    let mut current = session.root.clone();
    for component in clean.components() {
        let Component::Normal(part) = component else {
            return Err("The workspace path contains an invalid component.".into());
        };
        current.push(part);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| format!("Could not resolve the workspace file: {error}"))?;
        if metadata.file_type().is_symlink() {
            return Err("Workspace tools do not follow symbolic links or reparse points.".into());
        }
    }
    let canonical = fs::canonicalize(&current)
        .map_err(|error| format!("Could not resolve the workspace file: {error}"))?;
    ensure_inside_root(&session.root, &canonical)?;
    let metadata = fs::metadata(&canonical)
        .map_err(|error| format!("Could not inspect the workspace file: {error}"))?;
    if !metadata.is_file() {
        return Err("The selected workspace path is not a regular file.".into());
    }
    Ok(canonical)
}

fn ensure_inside_root(root: &Path, path: &Path) -> Result<(), String> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err("The requested path resolves outside the granted workspace.".into())
    }
}

fn relative_display(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "A workspace entry escaped its granted root.".to_string())?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn is_sensitive_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with(".env")
        || matches!(
            name.as_str(),
            ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg"
        )
        || name == ".npmrc"
        || name == ".pypirc"
        || name == "id_rsa"
        || name == "id_ed25519"
        || name == "credentials"
        || name == "credentials.json"
        || name == "service-account.json"
        || name == "secrets.json"
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".asc")
}

fn is_sensitive_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            name.as_str(),
            ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg"
        ) || is_sensitive_name(&name)
    })
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
            | ".next"
            | ".venv"
            | "vendor"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workspace() -> (WorkspaceManager, PathBuf, WorkspaceGrant) {
        let root = std::env::temp_dir().join(format!(
            "spindle-workspace-test-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        let manager = WorkspaceManager::default();
        let grant = manager
            .grant(root.to_string_lossy().into_owned(), false)
            .unwrap();
        (manager, root, grant)
    }

    #[test]
    fn workspace_grants_are_temporary_and_revocable() {
        let (manager, root, grant) = test_workspace();
        assert!(grant.temporary);
        assert!(grant.can_read);
        assert!(!grant.can_write);
        manager.revoke(&grant.session_id).unwrap();
        assert!(manager.status().unwrap().is_none());
        assert!(manager.list_directory(&grant.session_id, "").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn read_only_grants_reject_writes() {
        let (manager, root, grant) = test_workspace();
        fs::write(root.join("notes.txt"), "original").unwrap();
        let opened = manager.read_file(&grant.session_id, "notes.txt").unwrap();
        let error = manager
            .stage_file_change(
                &grant.session_id,
                "notes.txt",
                "hello".into(),
                &opened.revision,
            )
            .unwrap_err();
        assert!(error.contains("not permitted"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn write_grants_allow_bounded_text_files() {
        let (manager, root, _) = test_workspace();
        let grant = manager
            .grant(root.to_string_lossy().into_owned(), true)
            .unwrap();
        fs::write(root.join("src-notes.txt"), "workspace text").unwrap();
        let file = manager
            .read_file(&grant.session_id, "src-notes.txt")
            .unwrap();
        let proposal = manager
            .stage_file_change(
                &grant.session_id,
                "src-notes.txt",
                "updated text".into(),
                &file.revision,
            )
            .unwrap();
        assert_eq!(proposal.status, "pending");
        assert!(proposal.diff.contains("-workspace text"));
        assert_eq!(fs::read_to_string(root.join("src-notes.txt")).unwrap(), "workspace text");
        let saved = manager.apply_file_change(&grant.session_id, &proposal.id).unwrap();
        assert_eq!(saved.revision.len(), 64);
        let updated = manager
            .read_file(&grant.session_id, "src-notes.txt")
            .unwrap();
        assert_eq!(updated.content, "updated text");
        manager.undo_file_change(&grant.session_id, &proposal.id).unwrap();
        assert_eq!(fs::read_to_string(root.join("src-notes.txt")).unwrap(), "workspace text");
        assert!(manager
            .stage_file_change(
                &grant.session_id,
                "../escape.txt",
                "no".into(),
                &"0".repeat(64),
            )
            .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_writes_when_the_file_changed_after_open() {
        let (manager, root, _) = test_workspace();
        let grant = manager
            .grant(root.to_string_lossy().into_owned(), true)
            .unwrap();
        fs::write(root.join("conflict.txt"), "original").unwrap();
        let opened = manager
            .read_file(&grant.session_id, "conflict.txt")
            .unwrap();
        fs::write(root.join("conflict.txt"), "external change").unwrap();
        let error = manager
            .stage_file_change(
                &grant.session_id,
                "conflict.txt",
                "my stale edit".into(),
                &opened.revision,
            )
            .unwrap_err();
        assert!(error.contains("changed outside Spindle"));
        assert_eq!(
            fs::read_to_string(root.join("conflict.txt")).unwrap(),
            "external change"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staged_changes_abort_on_external_edits_and_can_be_cancelled() {
        let (manager, root, _) = test_workspace();
        let grant = manager.grant(root.to_string_lossy().into_owned(), true).unwrap();
        fs::write(root.join("conflict.txt"), "original").unwrap();
        let opened = manager.read_file(&grant.session_id, "conflict.txt").unwrap();
        let proposal = manager
            .stage_file_change(&grant.session_id, "conflict.txt", "planned".into(), &opened.revision)
            .unwrap();
        fs::write(root.join("conflict.txt"), "newer external edit").unwrap();
        assert!(manager.apply_file_change(&grant.session_id, &proposal.id).is_err());
        assert_eq!(fs::read_to_string(root.join("conflict.txt")).unwrap(), "newer external edit");
        manager.cancel_file_change(&grant.session_id, &proposal.id).unwrap();
        assert!(manager.apply_file_change(&grant.session_id, &proposal.id).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn secret_files_are_hidden_and_cannot_be_read_or_written() {
        let (manager, root, _) = test_workspace();
        let grant = manager
            .grant(root.to_string_lossy().into_owned(), true)
            .unwrap();
        fs::write(root.join(".env.local"), "API_KEY=secret").unwrap();
        fs::create_dir_all(root.join(".ssh")).unwrap();
        fs::write(root.join(".ssh").join("id_rsa"), "private key").unwrap();
        assert!(manager.read_file(&grant.session_id, ".env.local").is_err());
        assert!(manager.read_file(&grant.session_id, ".ssh/id_rsa").is_err());
        assert!(manager
            .stage_file_change(
                &grant.session_id,
                ".env.local",
                "changed".into(),
                &"0".repeat(64),
            )
            .is_err());
        let listing = manager.list_directory(&grant.session_id, "").unwrap();
        assert!(listing.entries.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn path_validation_rejects_traversal_and_absolute_paths() {
        assert!(validate_relative_path("../outside.txt", false).is_err());
        assert!(validate_relative_path("src/../../outside.txt", false).is_err());
        assert!(validate_relative_path("C:\\\\Windows\\\\win.ini", false).is_err());
        assert!(validate_relative_path("", false).is_err());
        assert!(validate_relative_path("", true).is_ok());
    }

    #[test]
    fn search_is_literal_case_insensitive_and_bounded() {
        let (manager, root, grant) = test_workspace();
        fs::write(root.join("Example.TS"), "const Answer = 42;\n").unwrap();
        let result = manager.search(&grant.session_id, "answer").unwrap();
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].relative_path, "Example.TS");
        assert_eq!(result.matches[0].line_number, 1);
        let _ = fs::remove_dir_all(root);
    }
}
