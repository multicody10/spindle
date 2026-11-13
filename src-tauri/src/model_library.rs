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

#[cfg(windows)]
fn is_reparse_or_symlink(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;

    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & 0x0400 != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(true)
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

            let path = entry.path();
            // Do not recurse through symlinks, junctions, or other reparse points.
            if file_type.is_symlink() || is_reparse_or_symlink(&path) {
                continue;
            }
            if file_type.is_dir() {
                let name = entry.file_name();
                if !ignored_directory(&name.to_string_lossy()) && depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }

            if !file_type.is_file()
                || !path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("gguf")
                })
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

fn default_model_roots(
    home_directory: Option<&Path>,
    local_app_data_directory: Option<&Path>,
    roaming_app_data_directory: Option<&Path>,
    lm_studio_override: Option<PathBuf>,
    gpt4all_override: Option<PathBuf>,
    jan_override: Option<PathBuf>,
    koboldcpp_override: Option<PathBuf>,
) -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();

    let lm_studio = lm_studio_override
        .clone()
        .or_else(|| home_directory.map(|home| home.join(".lmstudio").join("models")));
    if let Some(path) = lm_studio {
        roots.push((
            if lm_studio_override.is_some() { "LM Studio (configured)" } else { "LM Studio" }.into(),
            path,
        ));
    }

    let gpt4all = gpt4all_override
        .clone()
        .or_else(|| local_app_data_directory.map(|base| base.join("nomic.ai").join("GPT4All")));
    if let Some(path) = gpt4all {
        roots.push((
            if gpt4all_override.is_some() { "GPT4All (configured)" } else { "GPT4All" }.into(),
            path,
        ));
    }

    let jan = jan_override
        .clone()
        .or_else(|| roaming_app_data_directory.map(|base| base.join("Jan").join("data").join("models")));
    if let Some(path) = jan {
        roots.push((
            if jan_override.is_some() { "Jan (configured)" } else { "Jan" }.into(),
            path,
        ));
    }

    if let Some(path) = koboldcpp_override {
        roots.push(("KoboldCpp (configured)".to_owned(), path));
    }

    roots
}

fn discover_from_roots(
    documents_directory: &Path,
    roots: Vec<(String, PathBuf)>,
) -> Result<ModelLibrary, String> {
    let managed_directory = documents_directory.join("Spindle").join("Models");
    fs::create_dir_all(&managed_directory)
        .map_err(|error| format!("Could not create the Spindle model directory: {error}"))?;

    let mut all_roots = vec![("Spindle library".to_owned(), managed_directory.clone())];
    all_roots.extend(roots);

    let mut locations = Vec::new();
    let mut models = Vec::new();
    let mut seen_roots = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut visited_entries = 0usize;
    let mut truncated = false;

    for (name, root) in all_roots {
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
    let local_app_data = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let roaming_app_data = env::var_os("APPDATA").map(PathBuf::from);
    let roots = default_model_roots(
        home.as_deref(),
        local_app_data.as_deref(),
        roaming_app_data.as_deref(),
        env::var_os("LMSTUDIO_MODELS_DIR").map(PathBuf::from),
        env::var_os("GPT4ALL_MODELS_DIR").map(PathBuf::from),
        env::var_os("JAN_MODELS_DIR").map(PathBuf::from),
        env::var_os("KOBOLDCPP_MODELS_DIR").map(PathBuf::from),
    );

    discover_from_roots(&documents_directory, roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_spindle_library_and_discovers_common_gguf_locations() {
        let temporary = env::temp_dir().join(format!(
            "spindle-model-library-{}",
            std::process::id()
        ));
        let documents = temporary.join("Documents");
        let home = temporary.join("Home");
        let local_app_data = temporary.join("Local");
        let roaming_app_data = temporary.join("Roaming");

        let spindle_models = documents.join("Spindle").join("Models");
        let lmstudio_models = home.join(".lmstudio").join("models").join("publisher").join("model");
        let gpt4all_models = local_app_data.join("nomic.ai").join("GPT4All");
        let jan_models = roaming_app_data.join("Jan").join("data").join("models");

        for directory in [&spindle_models, &lmstudio_models, &gpt4all_models, &jan_models] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(spindle_models.join("spindle-model.gguf"), b"test").unwrap();
        fs::write(lmstudio_models.join("lmstudio-model.gguf"), b"test model").unwrap();
        fs::write(gpt4all_models.join("gpt4all-model.gguf"), b"test model").unwrap();
        fs::write(jan_models.join("jan-model.gguf"), b"test model").unwrap();
        fs::write(lmstudio_models.join("not-a-model.txt"), b"ignore").unwrap();

        let roots = default_model_roots(
            Some(&home),
            Some(&local_app_data),
            Some(&roaming_app_data),
            None,
            None,
            None,
            None,
        );
        let result = discover_from_roots(&documents, roots).unwrap();

        assert_eq!(result.models.len(), 4);
        let managed = Path::new(&result.managed_directory);
        assert_eq!(managed.file_name().and_then(|name| name.to_str()), Some("Models"));
        assert_eq!(
            managed.parent().and_then(Path::file_name).and_then(|name| name.to_str()),
            Some("Spindle")
        );
        for (file_name, source) in [
            ("lmstudio-model.gguf", "LM Studio"),
            ("gpt4all-model.gguf", "GPT4All"),
            ("jan-model.gguf", "Jan"),
            ("spindle-model.gguf", "Spindle library"),
        ] {
            assert!(result.models.iter().any(|model| {
                model.file_name == file_name && model.source == source
            }));
        }

        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn uses_configured_model_directories_when_supplied() {
        let temporary = env::temp_dir().join(format!(
            "spindle-model-library-custom-{}",
            std::process::id()
        ));
        let documents = temporary.join("Documents");
        let custom = temporary.join("shared-models");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("custom.gguf"), b"model").unwrap();

        let roots = default_model_roots(None, None, None, Some(custom.clone()), None, None, None);
        let result = discover_from_roots(&documents, roots).unwrap();
        assert!(result.models.iter().any(|model| {
            model.file_name == "custom.gguf" && model.source == "LM Studio (configured)"
        }));

        let _ = fs::remove_dir_all(temporary);
    }
}
