use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SKILLS: usize = 64;
const MAX_MANIFEST_BYTES: u64 = 24 * 1024;
const MAX_RESOURCE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    pub name: String,
    pub description: String,
    pub source: String,
    pub manifest_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedSkill {
    pub name: String,
    pub source: String,
    pub manifest: String,
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn parse_frontmatter(text: &str) -> Option<(String, String)> {
    let mut lines = text
        .replace("\r\n", "\n")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.first()?.trim() != "---" {
        return None;
    }
    lines.remove(0);
    let mut name = String::new();
    let mut description = String::new();
    let mut description_block = false;
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if description_block {
            if line.starts_with(' ') || line.starts_with('\t') {
                if !line.trim().is_empty() {
                    if !description.is_empty() {
                        description.push(' ');
                    }
                    description.push_str(line.trim());
                }
                continue;
            }
            description_block = false;
        }
        if let Some(value) = line.strip_prefix("name:") {
            name = value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_ascii_lowercase();
        } else if let Some(value) = line.strip_prefix("description:") {
            let value = value.trim();
            if value.is_empty() || value == ">" || value == "|" {
                description_block = true;
            } else {
                description = value.trim_matches('"').trim_matches('\'').to_string();
            }
        }
    }
    if valid_name(&name) && !description.trim().is_empty() {
        Some((name, description.chars().take(1_024).collect()))
    } else {
        None
    }
}

fn roots(project_root: &Path) -> Vec<(PathBuf, String)> {
    let mut output = vec![
        (
            project_root.join(".agents").join("skills"),
            "Project .agents".to_string(),
        ),
        (
            project_root.join(".rivet").join("skills"),
            "Project Rivet compatible".to_string(),
        ),
        (
            project_root.join(".spindle").join("skills"),
            "Project Spindle".to_string(),
        ),
    ];
    if let Some(home) = home_dir() {
        output.extend([
            (home.join(".agents").join("skills"), "Global .agents".into()),
            (
                home.join(".codex").join("skills"),
                "Codex compatible".into(),
            ),
            (
                home.join(".claude").join("skills"),
                "Claude compatible".into(),
            ),
            (
                home.join(".lmstudio").join("skills"),
                "LM Studio compatible".into(),
            ),
        ]);
    }
    if let Some(extra) = std::env::var_os("SPINDLE_SKILLS_DIR") {
        output
            .extend(std::env::split_paths(&extra).map(|path| (path, "SPINDLE_SKILLS_DIR".into())));
    }
    if let Some(extra) = std::env::var_os("RIVET_SKILLS_DIR") {
        output.extend(
            std::env::split_paths(&extra).map(|path| (path, "RIVET_SKILLS_DIR compatible".into())),
        );
    }
    output
}

pub fn discover(project_root: &Path) -> Result<Vec<AgentSkill>, String> {
    let mut found = BTreeMap::<String, AgentSkill>::new();
    for (root, source) in roots(project_root) {
        let entries = match fs::read_dir(&root) {
            Ok(value) => value,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if found.len() >= MAX_SKILLS {
                break;
            }
            let directory = entry.path();
            let metadata = match fs::symlink_metadata(&directory) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let folder = directory
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or_default();
            if !valid_name(folder) {
                continue;
            }
            let manifest_path = directory.join("SKILL.md");
            let metadata = match fs::symlink_metadata(&manifest_path) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > MAX_MANIFEST_BYTES
            {
                continue;
            }
            let content = match fs::read_to_string(&manifest_path) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let Some((name, description)) = parse_frontmatter(&content) else {
                continue;
            };
            if name != folder {
                continue;
            }
            found.entry(name.clone()).or_insert(AgentSkill {
                name,
                description,
                source: source.clone(),
                manifest_path: manifest_path.to_string_lossy().into_owned(),
            });
        }
    }
    Ok(found.into_values().collect())
}

pub fn load(project_root: &Path, name: &str) -> Result<LoadedSkill, String> {
    let requested = name.trim().to_ascii_lowercase();
    if !valid_name(&requested) {
        return Err("Invalid Agent Skill name.".into());
    }
    let skill = discover(project_root)?
        .into_iter()
        .find(|skill| skill.name == requested)
        .ok_or_else(|| "The requested Agent Skill is not installed.".to_string())?;
    let path = PathBuf::from(&skill.manifest_path);
    let root = path
        .parent()
        .ok_or_else(|| "Agent Skill directory is invalid.".to_string())?;
    let canonical_root = fs::canonicalize(root)
        .map_err(|_| "Could not resolve Agent Skill directory.".to_string())?;
    let canonical_manifest = fs::canonicalize(&path)
        .map_err(|_| "Could not resolve Agent Skill manifest.".to_string())?;
    if !canonical_manifest.starts_with(&canonical_root) || !canonical_manifest.is_file() {
        return Err("Agent Skill manifest escaped its directory.".into());
    }
    let bytes = fs::read(&canonical_manifest)
        .map_err(|_| "Could not read Agent Skill manifest.".to_string())?;
    if bytes.len() > MAX_RESOURCE_BYTES {
        return Err("Agent Skill manifest exceeded its read limit.".into());
    }
    let manifest = String::from_utf8(bytes)
        .map_err(|_| "Agent Skill manifest is not UTF-8 text.".to_string())?;
    Ok(LoadedSkill {
        name: skill.name,
        source: skill.source,
        manifest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_skill_frontmatter() {
        let input = "---\nname: repo-review\ndescription: Review repositories.\n---\n# Review\n";
        assert_eq!(parse_frontmatter(input).unwrap().0, "repo-review");
    }

    #[test]
    fn rejects_untrusted_or_invalid_skill_names() {
        assert!(!valid_name("../escape"));
        assert!(!valid_name("UpperCase"));
        assert!(!valid_name("bad--name"));
        assert!(valid_name("repo-review"));
    }

    #[test]
    fn load_is_scoped_to_a_discovered_manifest() {
        let root = std::env::temp_dir().join(format!("spindle-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let skill = root.join(".agents").join("skills").join("repo-review");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: repo-review\ndescription: Review repositories.\n---\n# Instructions\nUse tests.\n").unwrap();
        assert!(load(&root, "repo-review")
            .unwrap()
            .manifest
            .contains("Use tests."));
        fs::remove_dir_all(&root).unwrap();
    }
}
