use serde::Serialize;
use std::{
    fs::{self, OpenOptions},
    io::Write,
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
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWriteResult {
    pub relative_path: String,
    pub bytes_written: usize,
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

#[derive(Clone, Default)]
pub struct WorkspaceManager {
    active: Arc<Mutex<Option<WorkspaceSession>>>,
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
        *active = Some(session);
        Ok(grant)
    }

    pub fn revoke(&self, session_id: &str) -> Result<(), String> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        match active.as_ref() {
            Some(session) if session.grant.session_id == session_id => {
                *active = None;
                Ok(())
            }
            Some(_) => Err("The workspace session identifier is stale.".into()),
            None => Ok(()),
        }
    }

    fn session(&self, session_id: &str, require_write: bool) -> Result<WorkspaceSession, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "The workspace permission state is unavailable.".to_string())?;
        let session = active
            .as_ref()
            .filter(|session| session.grant.session_id == session_id)
            .ok_or_else(|| "Workspace access has expired or was revoked. Grant the directory again.".to_string())?;

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
            let entry = entry.map_err(|error| format!("Could not read a directory entry: {error}"))?;
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

    pub fn read_file(&self, session_id: &str, relative_path: &str) -> Result<WorkspaceFile, String> {
        let session = self.session(session_id, false)?;
        let path = resolve_file(&session, relative_path)?;
        if is_sensitive_path(Path::new(relative_path)) {
            return Err("This file is classified as secret material and cannot be read by workspace tools.".into());
        }

        let metadata = fs::metadata(&path)
            .map_err(|error| format!("Could not inspect the selected workspace file: {error}"))?;
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err("The selected file exceeds the 1 MiB workspace read limit.".into());
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("Could not read the selected workspace file: {error}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("The selected file exceeds the 1 MiB workspace read limit.".into());
        }
        let content = String::from_utf8(bytes)
            .map_err(|_| "The selected file is not valid UTF-8 text and cannot be opened in the editor.".to_string())?;
        let size_bytes = content.len();
        Ok(WorkspaceFile {
            relative_path: relative_path.replace('\\', "/"),
            content,
            size_bytes,
        })
    }

    pub fn write_file(
        &self,
        session_id: &str,
        relative_path: &str,
        content: String,
    ) -> Result<WorkspaceWriteResult, String> {
        let session = self.session(session_id, true)?;
        if content.len() > MAX_FILE_BYTES {
            return Err("Workspace file writes are limited to 1 MiB.".into());
        }
        if content.contains('\0') {
            return Err("Binary content is not accepted by the workspace text editor.".into());
        }
        if relative_path.trim().is_empty() || is_sensitive_path(Path::new(relative_path)) {
            return Err("A valid, non-sensitive project file path is required.".into());
        }

        let relative = validate_relative_path(relative_path, false)?;
        let parent_relative = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = resolve_directory(&session, &parent_relative.to_string_lossy())?;
        let file_name = relative
            .file_name()
            .ok_or_else(|| "A file name is required.".to_string())?;
        let target = parent.join(file_name);

        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err("Workspace writes are limited to regular files, not links or special files.".into());
                }
                let canonical = fs::canonicalize(&target)
                    .map_err(|error| format!("Could not resolve the target file: {error}"))?;
                ensure_inside_root(&session.root, &canonical)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not inspect the target file: {error}")),
        }

        let temporary_name = format!(
            ".spindle-tmp-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let temporary = parent.join(temporary_name);
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| format!("Could not create a temporary workspace file: {error}"))?;
            file.write_all(content.as_bytes())
                .map_err(|error| format!("Could not write the workspace file: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("Could not flush the workspace file: {error}"))?;
            drop(file);
            fs::rename(&temporary, &target)
                .map_err(|error| format!("Could not atomically replace the workspace file: {error}"))?;
            Ok(())
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;

        Ok(WorkspaceWriteResult {
            relative_path: relative_path.replace('\\', "/"),
            bytes_written: content.len(),
        })
    }

    pub fn search(
        &self,
        session_id: &str,
        query: &str,
    ) -> Result<WorkspaceSearchResult, String> {
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
            if depth > MAX_SEARCH_DEPTH || files_scanned >= MAX_SEARCH_FILES || matches.len() >= MAX_SEARCH_RESULTS || bytes_scanned >= MAX_SEARCH_BYTES {
                truncated = true;
                break;
            }
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                if files_scanned >= MAX_SEARCH_FILES || matches.len() >= MAX_SEARCH_RESULTS || bytes_scanned >= MAX_SEARCH_BYTES {
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

                files_scanned += 1;
                let bytes = match fs::read(&canonical) {
                    Ok(bytes) if bytes.len() <= MAX_SEARCH_FILE_BYTES => bytes,
                    _ => continue,
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
        || matches!(name.as_str(), ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg")
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
        matches!(name.as_str(), ".ssh" | ".aws" | ".azure" | ".kube" | ".gnupg")
            || is_sensitive_name(&name)
    })
}

fn is_ignored_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | "node_modules" | "target" | "dist" | "build" | "coverage" | ".next" | ".venv" | "vendor"
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
        let error = manager
            .write_file(&grant.session_id, "notes.txt", "hello".into())
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
        manager
            .write_file(&grant.session_id, "src-notes.txt", "workspace text".into())
            .unwrap();
        manager
            .write_file(&grant.session_id, "src-notes.txt", "updated text".into())
            .unwrap();
        let file = manager.read_file(&grant.session_id, "src-notes.txt").unwrap();
        assert_eq!(file.content, "updated text");
        assert!(manager
            .write_file(&grant.session_id, "../escape.txt", "no".into())
            .is_err());
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
            .write_file(&grant.session_id, ".env.local", "changed".into())
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
