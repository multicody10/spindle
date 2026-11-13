use crate::chat::{self, ChatMessage, ChatProvider, ChatRequest, ChatRuntime};
use crate::{domain::GenerationSettings, inference::InferenceRuntimeManager};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::AppHandle;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
const MAX_ITERATIONS: usize = 5;
const MAX_OUTPUT_TOKENS: usize = 128;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

fn request_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let suffix = timestamp.wrapping_add(REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed))
        & 0x0000_ffff_ffff_ffff;
    format!("00000000-0000-4000-8000-{suffix:012x}")
}

fn open(database: &Path) -> Result<Connection, String> {
    if let Some(parent) = database.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create Spindle data directory: {error}"))?;
    }
    let connection = Connection::open(database)
        .map_err(|error| format!("Could not open benchmark database: {error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|error| error.to_string())?;
    connection.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE IF NOT EXISTS benchmark_runs (
           id TEXT PRIMARY KEY,
           provider_id TEXT NOT NULL,
           model_id TEXT NOT NULL,
           context_tokens INTEGER NOT NULL,
           requested_iterations INTEGER NOT NULL,
           status TEXT NOT NULL,
           median_duration_ms INTEGER,
           median_tokens_per_second REAL,
           started_at INTEGER NOT NULL,
           completed_at INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_benchmark_runs_started ON benchmark_runs(started_at DESC);
         CREATE TABLE IF NOT EXISTS benchmark_samples (
           id TEXT PRIMARY KEY,
           run_id TEXT NOT NULL REFERENCES benchmark_runs(id) ON DELETE CASCADE,
           sequence INTEGER NOT NULL,
           duration_ms INTEGER,
           output_tokens INTEGER,
           tokens_per_second REAL,
           error TEXT,
           created_at INTEGER NOT NULL,
           UNIQUE(run_id, sequence)
         );
         CREATE INDEX IF NOT EXISTS idx_benchmark_samples_run ON benchmark_samples(run_id, sequence);",
    ).map_err(|error| format!("Could not prepare benchmark storage: {error}"))?;
    Ok(connection)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkSample {
    pub sequence: usize,
    pub duration_ms: Option<u64>,
    pub output_tokens: Option<u64>,
    pub tokens_per_second: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRun {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub context_tokens: usize,
    pub requested_iterations: usize,
    pub status: String,
    pub median_duration_ms: Option<u64>,
    pub median_tokens_per_second: Option<f64>,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub samples: Vec<BenchmarkSample>,
}

fn median_u64(mut values: Vec<u64>) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn median_f64(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

fn begin_run(
    database: PathBuf,
    id: String,
    provider_id: String,
    model_id: String,
    context_tokens: usize,
    iterations: usize,
) -> Result<(), String> {
    let connection = open(&database)?;
    connection.execute(
        "INSERT INTO benchmark_runs(id,provider_id,model_id,context_tokens,requested_iterations,status,started_at)
         VALUES (?1,?2,?3,?4,?5,'running',?6)",
        params![id, provider_id, model_id, context_tokens as i64, iterations as i64, now()],
    ).map_err(|error| format!("Could not start benchmark record: {error}"))?;
    Ok(())
}

fn save_sample(database: PathBuf, run_id: String, sample: BenchmarkSample) -> Result<(), String> {
    let connection = open(&database)?;
    connection.execute(
        "INSERT INTO benchmark_samples(id,run_id,sequence,duration_ms,output_tokens,tokens_per_second,error,created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            format!("{}-{}", run_id, sample.sequence),
            run_id,
            sample.sequence as i64,
            sample.duration_ms.map(|v| v as i64),
            sample.output_tokens.map(|v| v as i64),
            sample.tokens_per_second,
            sample.error,
            now()
        ],
    ).map_err(|error| format!("Could not save benchmark sample: {error}"))?;
    Ok(())
}

fn finish_run(database: PathBuf, run_id: String) -> Result<(), String> {
    let connection = open(&database)?;
    let mut statement = connection.prepare(
        "SELECT duration_ms,tokens_per_second,error FROM benchmark_samples WHERE run_id=?1 ORDER BY sequence",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![run_id], |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?,
                row.get::<_, Option<f64>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut durations = Vec::new();
    let mut rates = Vec::new();
    let mut completed = 0usize;
    let mut failed = 0usize;
    for item in rows {
        let (duration, rate, error) = item.map_err(|error| error.to_string())?;
        if let Some(value) = duration {
            durations.push(value.max(0) as u64);
        }
        if let Some(value) = rate.filter(|v| v.is_finite() && *v >= 0.0) {
            rates.push(value);
        }
        if error.is_some() {
            failed += 1;
        } else {
            completed += 1;
        }
    }
    let status = if failed == 0 && completed > 0 {
        "completed"
    } else if completed > 0 {
        "partial"
    } else {
        "failed"
    };
    connection.execute(
        "UPDATE benchmark_runs SET status=?1,median_duration_ms=?2,median_tokens_per_second=?3,completed_at=?4 WHERE id=?5",
        params![status, median_u64(durations).map(|v| v as i64), median_f64(rates), now(), run_id],
    ).map_err(|error| format!("Could not finish benchmark record: {error}"))?;
    Ok(())
}

fn sample_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BenchmarkSample> {
    Ok(BenchmarkSample {
        sequence: row.get::<_, i64>(0)?.max(0) as usize,
        duration_ms: row
            .get::<_, Option<i64>>(1)?
            .map(|value| value.max(0) as u64),
        output_tokens: row
            .get::<_, Option<i64>>(2)?
            .map(|value| value.max(0) as u64),
        tokens_per_second: row.get(3)?,
        error: row.get(4)?,
    })
}

pub fn list_runs(database: PathBuf, limit: usize) -> Result<Vec<BenchmarkRun>, String> {
    let connection = open(&database)?;
    let mut statement = connection.prepare(
        "SELECT id,provider_id,model_id,context_tokens,requested_iterations,status,median_duration_ms,median_tokens_per_second,started_at,completed_at
         FROM benchmark_runs ORDER BY started_at DESC LIMIT ?1",
    ).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![limit.clamp(1, 50) as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<f64>>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<i64>>(9)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    for item in rows {
        let (
            id,
            provider_id,
            model_id,
            context_tokens,
            requested_iterations,
            status,
            median_duration_ms,
            median_tokens_per_second,
            started_at,
            completed_at,
        ) = item.map_err(|error| error.to_string())?;
        let mut samples_statement = connection
            .prepare(
                "SELECT sequence,duration_ms,output_tokens,tokens_per_second,error
             FROM benchmark_samples WHERE run_id=?1 ORDER BY sequence",
            )
            .map_err(|error| error.to_string())?;
        let samples = samples_statement
            .query_map(params![id], sample_from_row)
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        output.push(BenchmarkRun {
            id,
            provider_id,
            model_id,
            context_tokens: context_tokens.max(0) as usize,
            requested_iterations: requested_iterations.max(0) as usize,
            status,
            median_duration_ms: median_duration_ms.map(|v| v.max(0) as u64),
            median_tokens_per_second,
            started_at,
            completed_at,
            samples,
        });
    }
    Ok(output)
}

fn parse_provider(value: &str) -> Result<ChatProvider, String> {
    match value {
        "ollama" => Ok(ChatProvider::Ollama),
        "lm-studio" => Ok(ChatProvider::LmStudio),
        "llama-cpp" => Ok(ChatProvider::LlamaCpp),
        _ => Err("Select a supported local inference provider.".into()),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkRequest {
    pub provider_id: String,
    pub model_id: String,
    pub context_tokens: usize,
    pub generation: GenerationSettings,
    pub iterations: usize,
}

pub async fn run(
    app: AppHandle,
    database: PathBuf,
    runtime: &ChatRuntime,
    inference: &InferenceRuntimeManager,
    input: BenchmarkRequest,
) -> Result<BenchmarkRun, String> {
    let BenchmarkRequest {
        provider_id,
        model_id,
        context_tokens,
        mut generation,
        iterations,
    } = input;
    if iterations == 0 || iterations > MAX_ITERATIONS {
        return Err("Choose between 1 and 5 benchmark iterations.".into());
    }
    if model_id.trim().is_empty() || model_id.len() > 256 {
        return Err("Select a valid local model identifier.".into());
    }
    if !(4096..=131072).contains(&context_tokens) {
        return Err("Benchmark context must be between 4,096 and 131,072 tokens.".into());
    }
    let provider = parse_provider(&provider_id)?;
    generation.max_output_tokens = generation.max_output_tokens.clamp(16, MAX_OUTPUT_TOKENS);
    let run_id = request_id();
    let initial_run_id = run_id.clone();
    tokio::task::spawn_blocking({
        let database = database.clone();
        let provider_id = provider_id.clone();
        let model_id = model_id.clone();
        move || {
            begin_run(
                database,
                initial_run_id,
                provider_id,
                model_id,
                context_tokens,
                iterations,
            )
        }
    })
    .await
    .map_err(|error| format!("Benchmark initialization task failed: {error}"))??;

    for sequence in 0..iterations {
        let id = request_id();
        let request = ChatRequest {
            request_id: id.clone(),
            session_id: format!("benchmark-{run_id}"),
            provider_id: provider,
            model: model_id.clone(),
            messages: vec![
                ChatMessage { role: "system".into(), content: "You are running a local inference benchmark. Respond directly and concisely. Do not include a preamble.".into() },
                ChatMessage { role: "user".into(), content: "In four numbered points, explain how a hash table stores and retrieves a value. Keep each point to one sentence.".into() },
            ],
            context_tokens,
            generation: generation.clone(),
        };
        crate::validate_chat_request(&request)?;
        let cancellation = runtime.begin(&id)?;
        let result = chat::run(app.clone(), request, cancellation, inference).await;
        runtime.finish(&id);
        let sample = match result {
            Ok(result) => BenchmarkSample {
                sequence,
                duration_ms: Some(result.duration_ms),
                output_tokens: result.output_tokens,
                tokens_per_second: result.tokens_per_second,
                error: None,
            },
            Err(error) => BenchmarkSample {
                sequence,
                duration_ms: None,
                output_tokens: None,
                tokens_per_second: None,
                error: Some(error.chars().take(800).collect()),
            },
        };
        let failed = sample.error.is_some();
        tokio::task::spawn_blocking({
            let database = database.clone();
            let run_id = run_id.clone();
            move || save_sample(database, run_id, sample)
        })
        .await
        .map_err(|error| format!("Benchmark sample task failed: {error}"))??;
        if failed {
            break;
        }
    }
    tokio::task::spawn_blocking({
        let database = database.clone();
        let run_id = run_id.clone();
        move || finish_run(database, run_id)
    })
    .await
    .map_err(|error| format!("Benchmark finalization task failed: {error}"))??;
    let runs = list_runs(database, 50)?;
    runs.into_iter()
        .find(|run| run.id == run_id)
        .ok_or_else(|| "The benchmark completed but its result could not be reloaded.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_medians_are_stable_and_ignore_empty_samples() {
        assert_eq!(median_u64(vec![900, 300, 600]), Some(600));
        assert_eq!(median_u64(Vec::new()), None);
        assert_eq!(median_f64(vec![12.0, 4.0, 8.0]), Some(8.0));
        assert_eq!(median_f64(Vec::new()), None);
    }
}
