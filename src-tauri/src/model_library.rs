use serde::Serialize;
use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

const MAX_SCAN_ENTRIES: usize = 20_000;
const MAX_MODELS: usize = 500;
const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLibraryLocation {
    pub name: String,
    pub path: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLibraryEntry {
    pub file_name: String,
    pub path: String,
    pub size_bytes: u64,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelLibrary {
    pub managed_directory: String,
    pub locations: Vec<ModelLibraryLocation>,
    pub models: Vec<ModelLibraryEntry>,
    pub truncated: bool,
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_ascii_lowercase()
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | ".cache" | "cache" | "blobs" | "node_modules" | "target" | "build" | "dist"
    )
}

fn scan_root(
    root: &Path,
    source: &str,
    models: &mut Vec<ModelLibraryEntry>,
    seen: &mut HashSet<String>,
    visited_entries: &mut usize,
    truncated: &mut bool,
) {
    if !root.is_dir() || models.len() >= MAX_MODELS || *truncated {
        return;
    }

    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut pending = vec![(root, 0usize)];

    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_DEPTH || models.len() >= MAX_MODELS || *truncated {
            *truncated = true;
            break;
        }

        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries {
            *visited_entries += 1;
            if *visited_entries > MAX_SCAN_ENTRIES || models.len() >= MAX_MODELS {
                *truncated = true;
                return;
            }

            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };

            // Do not recurse through symlinks or other non-directory reparse entries.
            if file_type.is_symlink() {
                continue;
            }

            let path = entry.path();
            if file_type.is_dir() {
                let name = entry.file_name();
                if !ignored_directory(&name.to_string_lossy()) && depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }

            if !file_type.is_file()
                || !path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
            {
                continue;
            }

            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(path_key(&canonical)) {
                continue;
            }

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            models.push(ModelLibraryEntry {
                file_name: entry.file_name().to_string_lossy().into_owned(),
                path: canonical.to_string_lossy().into_owned(),
                size_bytes: metadata.len(),
                source: source.to_owned(),
            });
        }
    }
}

fn discover_from_paths(
    documents_directory: &Path,
    home_directory: Option<&Path>,
    configured_lmstudio_directory: Option<&Path>,
) -> Result<ModelLibrary, String> {
    let managed_directory = documents_directory.join("Spindle").join("Models");
    fs::create_dir_all(&managed_directory)
        .map_err(|error| format!("Could not create the Spindle model directory: {error}"))?;

    let mut roots = vec![("Spindle library".to_owned(), managed_directory.clone())];
    if let Some(path) = configured_lmstudio_directory {
        roots.push(("LM Studio (configured)".into(), path.to_path_buf()));
    } else if let Some(home) = home_directory {
        roots.push((
            "LM Studio".into(),
            home.join(".lmstudio").join("models"),
        ));
    }

    let mut locations = Vec::new();
    let mut models = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut visited_entries = 0usize;
    let mut truncated = false;

    for (name, root) in roots {
        let canonical = root.canonicalize().unwrap_or_else(|_| root.clone());
        let key = path_key(&canonical);
        let exists = root.is_dir();

        if seen_roots.insert(key) {
            locations.push(ModelLibraryLocation {
                name: name.clone(),
                path: root.to_string_lossy().into_owned(),
                exists,
            });
            if exists {
                scan_root(
                    &root,
                    &name,
                    &mut models,
                    &mut seen_models,
                    &mut visited_entries,
                    &mut truncated,
                );
            }
        }
    }

    models.sort_by(|left, right| {
        left.file_name
            .to_ascii_lowercase()
            .cmp(&right.file_name.to_ascii_lowercase())
            .then_with(|| left.path.to_ascii_lowercase().cmp(&right.path.to_ascii_lowercase()))
    });

    Ok(ModelLibrary {
        managed_directory: managed_directory.to_string_lossy().into_owned(),
        locations,
        models,
        truncated,
    })
}

pub fn discover(documents_directory: PathBuf) -> Result<ModelLibrary, String> {
    let home = env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from);
    let configured_lmstudio = env::var_os("LMSTUDIO_MODELS_DIR").map(PathBuf::from);

    discover_from_paths(
        &documents_directory,
        home.as_deref(),
        configured_lmstudio.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_spindle_library_and_discovers_lm_studio_gguf_models() {
        let temporary = env::temp_dir().join(format!(
            "spindle-model-library-{}",
            std::process::id()
        ));
        let documents = temporary.join("Documents");
        let home = temporary.join("Home");
        let spindle_models = documents.join("Spindle").join("Models");
        let lmstudio_models = home.join(".lmstudio").join("models").join("publisher").join("model");

        fs::create_dir_all(&spindle_models).unwrap();
        fs::create_dir_all(&lmstudio_models).unwrap();
        fs::write(spindle_models.join("spindle-model.gguf"), b"test").unwrap();
        fs::write(lmstudio_models.join("lmstudio-model.gguf"), b"test model").unwrap();
        fs::write(lmstudio_models.join("not-a-model.txt"), b"ignore").unwrap();

        let result = discover_from_paths(&documents, Some(&home), None).unwrap();
        assert_eq!(result.models.len(), 2);
        assert!(result.managed_directory.ends_with("Spindle\\Models"));
        assert!(result.models.iter().any(|model| {
            model.file_name == "lmstudio-model.gguf" && model.source == "LM Studio"
        }));
        assert!(result.models.iter().any(|model| {
            model.file_name == "spindle-model.gguf" && model.source == "Spindle library"
        }));

        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn uses_configured_lm_studio_directory_when_supplied() {
        let temporary = env::temp_dir().join(format!(
            "spindle-model-library-custom-{}",
            std::process::id()
        ));
        let documents = temporary.join("Documents");
        let custom = temporary.join("shared-models");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("custom.gguf"), b"model").unwrap();

        let result = discover_from_paths(&documents, None, Some(&custom)).unwrap();
        assert!(result.models.iter().any(|model| {
            model.file_name == "custom.gguf" && model.source == "LM Studio (configured)"
        }));

        let _ = fs::remove_dir_all(temporary);
    }
}
