use crate::domain::{ModelConfiguration, OptimizationProfile, SavedPreset};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or(0)
}

fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Spindle data directory: {error}"))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("Could not open Spindle database: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS schema_migrations (
           version INTEGER PRIMARY KEY,
           applied_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS model_configurations (
           model_path TEXT NOT NULL,
           profile TEXT NOT NULL,
           configuration_json TEXT NOT NULL,
           updated_at INTEGER NOT NULL,
           PRIMARY KEY (model_path, profile)
         );
         CREATE TABLE IF NOT EXISTS model_presets (
           name TEXT PRIMARY KEY,
           configuration_json TEXT NOT NULL,
           updated_at INTEGER NOT NULL
         );
         INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, unixepoch());",
        )
        .map_err(|error| format!("Could not migrate Spindle database: {error}"))?;
    Ok(connection)
}

pub fn save_configuration(
    database: PathBuf,
    configuration: ModelConfiguration,
) -> Result<(), String> {
    let serialized = serde_json::to_string(&configuration).map_err(|error| error.to_string())?;
    let connection = open(&database)?;
    connection
        .execute(
            "INSERT INTO model_configurations(model_path, profile, configuration_json, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(model_path, profile) DO UPDATE SET
           configuration_json=excluded.configuration_json,
           updated_at=excluded.updated_at",
            params![
                configuration.model_path,
                configuration.profile.as_str(),
                serialized,
                now()
            ],
        )
        .map_err(|error| format!("Could not save model configuration: {error}"))?;
    Ok(())
}

pub fn load_configuration(
    database: PathBuf,
    model_path: String,
    profile: OptimizationProfile,
) -> Result<Option<ModelConfiguration>, String> {
    let connection = open(&database)?;
    let raw: Option<String> = connection.query_row(
        "SELECT configuration_json FROM model_configurations WHERE model_path=?1 AND profile=?2",
        params![model_path, profile.as_str()],
        |row| row.get(0),
    ).optional().map_err(|error| format!("Could not load model configuration: {error}"))?;
    raw.map(|text| {
        serde_json::from_str(&text)
            .map_err(|error| format!("Saved model configuration is invalid: {error}"))
    })
    .transpose()
}

pub fn save_preset(
    database: PathBuf,
    name: String,
    configuration: ModelConfiguration,
) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err("Preset names must contain 1 to 80 characters.".into());
    }
    let serialized = serde_json::to_string(&configuration).map_err(|error| error.to_string())?;
    let connection = open(&database)?;
    connection
        .execute(
            "INSERT INTO model_presets(name, configuration_json, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET
           configuration_json=excluded.configuration_json,
           updated_at=excluded.updated_at",
            params![name, serialized, now()],
        )
        .map_err(|error| format!("Could not save preset: {error}"))?;
    Ok(())
}

pub fn list_presets(database: PathBuf) -> Result<Vec<SavedPreset>, String> {
    let connection = open(&database)?;
    let mut statement = connection.prepare("SELECT name, configuration_json, updated_at FROM model_presets ORDER BY name COLLATE NOCASE")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            let name: String = row.get(0)?;
            let configuration: String = row.get(1)?;
            let updated_at: i64 = row.get(2)?;
            Ok((name, configuration, updated_at))
        })
        .map_err(|error| error.to_string())?;
    let mut presets = Vec::new();
    for row in rows {
        let (name, raw, updated_at) = row.map_err(|error| error.to_string())?;
        let configuration = serde_json::from_str(&raw)
            .map_err(|error| format!("Preset {name} is invalid: {error}"))?;
        presets.push(SavedPreset {
            name,
            configuration,
            updated_at,
        });
    }
    Ok(presets)
}

pub fn delete_preset(database: PathBuf, name: String) -> Result<(), String> {
    let connection = open(&database)?;
    connection
        .execute("DELETE FROM model_presets WHERE name=?1", params![name])
        .map_err(|error| format!("Could not delete preset: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_settings_and_presets_round_trip_through_sqlite() {
        let directory = std::env::temp_dir().join(format!(
            "spindle-persistence-{}-{}",
            std::process::id(),
            now()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let database = directory.join("settings.sqlite3");
        let configuration = ModelConfiguration::new(
            "C:\\Models\\sample.gguf".into(),
            OptimizationProfile::Automatic,
        );
        let expected = serde_json::to_string(&configuration).unwrap();

        save_configuration(database.clone(), configuration.clone()).unwrap();
        let loaded = load_configuration(
            database.clone(),
            configuration.model_path.clone(),
            OptimizationProfile::Automatic,
        )
        .unwrap()
        .unwrap();
        assert_eq!(serde_json::to_string(&loaded).unwrap(), expected);

        save_preset(database.clone(), "Coding".into(), configuration.clone()).unwrap();
        let presets = list_presets(database.clone()).unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "Coding");
        assert_eq!(
            serde_json::to_string(&presets[0].configuration).unwrap(),
            expected
        );

        delete_preset(database, "Coding".into()).unwrap();
        assert!(list_presets(directory.join("settings.sqlite3"))
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
