use serde::Serialize;
use serde_json::Value;
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
    path.to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase()
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
            let linked = file_type.is_symlink() || is_reparse_or_symlink(&path);
            if file_type.is_dir() {
                // Never descend through directory symlinks, junctions, or reparse points.
                if linked {
                    continue;
                }
                let name = entry.file_name();
                if !ignored_directory(&name.to_string_lossy()) && depth < MAX_DEPTH {
                    pending.push((path, depth + 1));
                }
                continue;
            }

            if !path
                .extension()
                .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("gguf"))
            {
                continue;
            }

            // Hugging Face snapshots and some model managers expose GGUF files as
            // file symlinks. Follow file links for metadata only, but never recurse
            // through linked directories.
            let metadata = if linked {
                match fs::metadata(&path) {
                    Ok(metadata) if metadata.is_file() => metadata,
                    _ => continue,
                }
            } else {
                match entry.metadata() {
                    Ok(metadata) if metadata.is_file() => metadata,
                    _ => continue,
                }
            };

            let canonical_target = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !seen.insert(path_key(&canonical_target)) {
                continue;
            }

            // Keep the visible .gguf snapshot path rather than the symlink target,
            // whose Hugging Face blob filename has no extension.
            models.push(ModelLibraryEntry {
                file_name: entry.file_name().to_string_lossy().into_owned(),
                path: path.to_string_lossy().into_owned(),
                size_bytes: metadata.len(),
                source: source.to_owned(),
            });
        }
    }
}

fn read_text_path(path: &Path, relative_to: &Path) -> Option<PathBuf> {
    let value = fs::read_to_string(path).ok()?;
    let value = value.trim().trim_start_matches('\u{feff}').trim();
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    Some(if path.is_absolute() {
        path
    } else {
        relative_to.join(path)
    })
}

fn read_json_path_setting(path: &Path, key: &str, relative_to: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(path).ok()?;
    let contents = contents.trim_start_matches('\u{feff}');
    let document: Value = serde_json::from_str(contents).ok()?;
    let configured = document
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    Some(if configured.is_absolute() {
        configured
    } else {
        relative_to.join(configured)
    })
}

#[derive(Default)]
struct ModelDirectoryOverrides {
    lm_studio_home: Option<PathBuf>,
    lm_studio_models: Option<PathBuf>,
    gpt4all_models: Option<PathBuf>,
    jan_data_directory: Option<PathBuf>,
    jan_models: Option<PathBuf>,
    koboldcpp_models: Option<PathBuf>,
    huggingface_home: Option<PathBuf>,
    huggingface_cache: Option<PathBuf>,
}

fn add_existing_fallback(roots: &mut Vec<(String, PathBuf)>, name: &str, path: PathBuf) {
    if path.is_dir() {
        roots.push((name.to_owned(), path));
    }
}

fn default_model_roots(
    home_directory: Option<&Path>,
    local_app_data_directory: Option<&Path>,
    roaming_app_data_directory: Option<&Path>,
    overrides: ModelDirectoryOverrides,
) -> Vec<(String, PathBuf)> {
    let mut roots = Vec::new();

    // LM Studio supports a movable home directory, a legacy cache home, and a
    // newer .lmstudio home. The active models folder can also be changed in settings.
    if let Some(path) = overrides.lm_studio_models.clone() {
        roots.push(("LM Studio (configured)".to_owned(), path));
    }
    if let Some(home) = home_directory {
        let legacy_home = home.join(".cache").join("lm-studio");
        let pointer_home = read_text_path(&home.join(".lmstudio-home-pointer"), home);
        let selected_home = overrides
            .lm_studio_home
            .clone()
            .or(pointer_home)
            .unwrap_or_else(|| {
                if legacy_home.is_dir() {
                    legacy_home.clone()
                } else {
                    home.join(".lmstudio")
                }
            });
        let configured_downloads = read_json_path_setting(
            &selected_home.join("settings.json"),
            "downloadsFolder",
            &selected_home,
        );
        roots.push((
            "LM Studio".to_owned(),
            configured_downloads.unwrap_or_else(|| selected_home.join("models")),
        ));

        // Keep both known homes discoverable when they contain older or separately
        // imported models, even if the active home now points somewhere else.
        add_existing_fallback(
            &mut roots,
            "LM Studio (legacy cache)",
            legacy_home.join("models"),
        );
        add_existing_fallback(
            &mut roots,
            "LM Studio (current home)",
            home.join(".lmstudio").join("models"),
        );
    }

    // GPT4All Desktop's Windows download directory and the Python package's default
    // cache are separate locations and may both contain directly usable GGUF files.
    if let Some(path) = overrides.gpt4all_models.clone() {
        roots.push(("GPT4All (configured)".to_owned(), path));
    }
    if let Some(base) = local_app_data_directory {
        let desktop_models = base.join("nomic.ai").join("GPT4All");
        if overrides.gpt4all_models.is_some() {
            add_existing_fallback(&mut roots, "GPT4All Desktop", desktop_models);
        } else {
            roots.push(("GPT4All".to_owned(), desktop_models));
        }
    }
    if let Some(home) = home_directory {
        add_existing_fallback(
            &mut roots,
            "GPT4All Python cache",
            home.join(".cache").join("gpt4all"),
        );
    }

    // Jan currently stores llama.cpp GGUFs under llamacpp/models inside its data
    // directory. The data directory itself is user-configurable in Jan settings.
    if let Some(path) = overrides.jan_models.clone() {
        roots.push(("Jan (configured)".to_owned(), path));
    }
    if let Some(roaming) = roaming_app_data_directory {
        let default_jan_data = roaming.join("Jan").join("data");
        let settings_data = read_json_path_setting(
            &roaming.join("Jan").join("settings.json"),
            "data_folder",
            &roaming.join("Jan"),
        );
        let jan_data = overrides
            .jan_data_directory
            .clone()
            .or(settings_data)
            .unwrap_or(default_jan_data);
        roots.push(("Jan".to_owned(), jan_data.join("llamacpp").join("models")));
        add_existing_fallback(&mut roots, "Jan (legacy models)", jan_data.join("models"));
    }

    // KoboldCpp's Download Dir is selected in its UI. It has no single stable
    // per-user model directory, so users can expose the selected folder explicitly.
    if let Some(path) = overrides.koboldcpp_models {
        roots.push(("KoboldCpp (configured Download Dir)".to_owned(), path));
    }

    // Hugging Face stores real GGUF snapshot paths below models--*/snapshots/*.
    // HF_HUB_CACHE is the direct cache override; HF_HOME points one level above hub.
    if let Some(path) = overrides.huggingface_cache {
        roots.push(("Hugging Face Hub cache (configured)".to_owned(), path));
    } else if let Some(path) = overrides.huggingface_home {
        roots.push(("Hugging Face Hub cache".to_owned(), path.join("hub")));
    } else if let Some(home) = home_directory {
        roots.push((
            "Hugging Face Hub cache".to_owned(),
            home.join(".cache").join("huggingface").join("hub"),
        ));
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
            .then_with(|| {
                left.path
                    .to_ascii_lowercase()
                    .cmp(&right.path.to_ascii_lowercase())
            })
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
    let overrides = ModelDirectoryOverrides {
        lm_studio_home: env::var_os("LMSTUDIO_HOME").map(PathBuf::from),
        lm_studio_models: env::var_os("LMSTUDIO_MODELS_DIR").map(PathBuf::from),
        gpt4all_models: env::var_os("GPT4ALL_MODELS_DIR").map(PathBuf::from),
        jan_data_directory: env::var_os("JAN_DATA_DIR").map(PathBuf::from),
        jan_models: env::var_os("JAN_MODELS_DIR").map(PathBuf::from),
        koboldcpp_models: env::var_os("KOBOLDCPP_MODELS_DIR").map(PathBuf::from),
        huggingface_home: env::var_os("HF_HOME").map(PathBuf::from),
        huggingface_cache: env::var_os("HF_HUB_CACHE").map(PathBuf::from),
    };
    let roots = default_model_roots(
        home.as_deref(),
        local_app_data.as_deref(),
        roaming_app_data.as_deref(),
        overrides,
    );

    discover_from_roots(&documents_directory, roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_overrides() -> ModelDirectoryOverrides {
        ModelDirectoryOverrides::default()
    }

    #[test]
    fn discovers_gguf_models_in_all_supported_default_locations() {
        let temporary =
            env::temp_dir().join(format!("spindle-model-library-{}", std::process::id()));
        let documents = temporary.join("Documents");
        let home = temporary.join("Home");
        let local_app_data = temporary.join("Local");
        let roaming_app_data = temporary.join("Roaming");

        let spindle_models = documents.join("Spindle").join("Models");
        let lmstudio_legacy = home
            .join(".cache")
            .join("lm-studio")
            .join("models")
            .join("publisher")
            .join("model");
        let lmstudio_current = home
            .join(".lmstudio")
            .join("models")
            .join("publisher")
            .join("model");
        let gpt4all_desktop = local_app_data.join("nomic.ai").join("GPT4All");
        let gpt4all_python = home.join(".cache").join("gpt4all");
        let jan_models = roaming_app_data
            .join("Jan")
            .join("data")
            .join("llamacpp")
            .join("models")
            .join("org")
            .join("repo");
        let huggingface_models = home
            .join(".cache")
            .join("huggingface")
            .join("hub")
            .join("models--org--repo")
            .join("snapshots")
            .join("0123456789abcdef");
        let koboldcpp_models = temporary.join("koboldcpp-downloads");

        for directory in [
            &spindle_models,
            &lmstudio_legacy,
            &lmstudio_current,
            &gpt4all_desktop,
            &gpt4all_python,
            &jan_models,
            &huggingface_models,
            &koboldcpp_models,
        ] {
            fs::create_dir_all(directory).unwrap();
        }

        for (directory, filename) in [
            (&spindle_models, "spindle.gguf"),
            (&lmstudio_legacy, "lmstudio-legacy.gguf"),
            (&lmstudio_current, "lmstudio-current.gguf"),
            (&gpt4all_desktop, "gpt4all-desktop.gguf"),
            (&gpt4all_python, "gpt4all-python.gguf"),
            (&jan_models, "jan.gguf"),
            (&huggingface_models, "huggingface.gguf"),
            (&koboldcpp_models, "koboldcpp.gguf"),
        ] {
            fs::write(directory.join(filename), b"test model").unwrap();
        }

        let roots = default_model_roots(
            Some(&home),
            Some(&local_app_data),
            Some(&roaming_app_data),
            ModelDirectoryOverrides {
                koboldcpp_models: Some(koboldcpp_models.clone()),
                ..no_overrides()
            },
        );
        let result = discover_from_roots(&documents, roots).unwrap();

        assert_eq!(result.models.len(), 8);
        for filename in [
            "spindle.gguf",
            "lmstudio-legacy.gguf",
            "lmstudio-current.gguf",
            "gpt4all-desktop.gguf",
            "gpt4all-python.gguf",
            "jan.gguf",
            "huggingface.gguf",
            "koboldcpp.gguf",
        ] {
            assert!(result
                .models
                .iter()
                .any(|model| model.file_name == filename));
        }

        let jan = result
            .models
            .iter()
            .find(|model| model.file_name == "jan.gguf")
            .unwrap();
        assert!(jan.path.contains("llamacpp"));
        let managed = Path::new(&result.managed_directory);
        assert_eq!(
            managed.file_name().and_then(|name| name.to_str()),
            Some("Models")
        );
        assert_eq!(
            managed
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str()),
            Some("Spindle")
        );

        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn honors_lm_studio_pointer_and_downloads_folder_setting() {
        let temporary =
            env::temp_dir().join(format!("spindle-lmstudio-settings-{}", std::process::id()));
        let home = temporary.join("Home");
        let custom_home = temporary.join("LM Studio Home");
        let custom_models = temporary.join("External Models");
        fs::create_dir_all(&custom_home).unwrap();
        fs::create_dir_all(&custom_models).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".lmstudio-home-pointer"),
            custom_home.to_string_lossy().as_bytes(),
        )
        .unwrap();
        let settings = serde_json::json!({
            "downloadsFolder": custom_models.to_string_lossy()
        });
        fs::write(custom_home.join("settings.json"), settings.to_string()).unwrap();

        let roots = default_model_roots(Some(&home), None, None, no_overrides());
        assert!(roots
            .iter()
            .any(|(name, path)| { name == "LM Studio" && path == &custom_models }));

        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn honors_jan_custom_data_folder_setting() {
        let temporary =
            env::temp_dir().join(format!("spindle-jan-settings-{}", std::process::id()));
        let roaming = temporary.join("Roaming");
        let jan_config = roaming.join("Jan");
        let custom_data = temporary.join("Jan Data");
        fs::create_dir_all(&jan_config).unwrap();
        let settings = serde_json::json!({
            "data_folder": custom_data.to_string_lossy()
        });
        fs::write(jan_config.join("settings.json"), settings.to_string()).unwrap();

        let roots = default_model_roots(None, None, Some(&roaming), no_overrides());
        assert!(roots.iter().any(|(name, path)| {
            name == "Jan" && path == &custom_data.join("llamacpp").join("models")
        }));

        let _ = fs::remove_dir_all(temporary);
    }

    #[test]
    fn uses_configured_model_directories_when_supplied() {
        let temporary = env::temp_dir().join(format!(
            "spindle-model-library-custom-{}",
            std::process::id()
        ));
        let custom = temporary.join("shared-models");
        fs::create_dir_all(&custom).unwrap();
        fs::write(custom.join("custom.gguf"), b"model").unwrap();

        let roots = default_model_roots(
            None,
            None,
            None,
            ModelDirectoryOverrides {
                lm_studio_models: Some(custom.clone()),
                ..no_overrides()
            },
        );
        let result = discover_from_roots(&temporary.join("Documents"), roots).unwrap();
        assert!(result.models.iter().any(|model| {
            model.file_name == "custom.gguf" && model.source == "LM Studio (configured)"
        }));

        let _ = fs::remove_dir_all(temporary);
    }

    #[cfg(unix)]
    #[test]
    fn discovers_gguf_files_linked_from_hugging_face_snapshots() {
        use std::os::unix::fs::symlink;

        let temporary = env::temp_dir().join(format!("spindle-hf-symlink-{}", std::process::id()));
        let hub = temporary.join("hub");
        let snapshot = hub
            .join("models--org--repo")
            .join("snapshots")
            .join("0123456789abcdef");
        let blobs = hub.join("models--org--repo").join("blobs");
        fs::create_dir_all(&snapshot).unwrap();
        fs::create_dir_all(&blobs).unwrap();
        let blob = blobs.join("0123456789abcdef");
        fs::write(&blob, b"GGUF model bytes").unwrap();
        symlink(&blob, snapshot.join("model-Q4_K_M.gguf")).unwrap();

        let mut models = Vec::new();
        let mut seen = HashSet::new();
        let mut visited = 0;
        let mut truncated = false;
        scan_root(
            &hub,
            "Hugging Face Hub cache",
            &mut models,
            &mut seen,
            &mut visited,
            &mut truncated,
        );

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].file_name, "model-Q4_K_M.gguf");
        assert!(models[0].path.ends_with("model-Q4_K_M.gguf"));
        assert_eq!(models[0].size_bytes, b"GGUF model bytes".len() as u64);

        let _ = fs::remove_dir_all(temporary);
    }
}
