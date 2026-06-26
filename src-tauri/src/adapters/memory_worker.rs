//! Background "memory agent" worker.
//!
//! Polls the `memory_signals` queue, groups unprocessed signals by feature, and
//! asks the user-configured LLM to distill them into typed project memories.
//! Each candidate memory is embedded and deduplicated (in-process cosine) against
//! existing memories before being written with `source = agent` and a confidence
//! score (auto-apply trust model). Capturing a signal is free; this is where the
//! one deliberate, opt-in, direct-to-provider LLM call happens.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::application::memory as mem_app;
use crate::domain::ids::ProjectId;
use crate::domain::memory::{
    cosine_similarity, MemorySignal, MemorySource, MemoryType, ProjectMemoryEntry,
};
use crate::ports::db::AppSettingsRepository;
use crate::ports::memory::ProjectMemoryPort;
use crate::ports::memory_llm::{ChatMessage, MemoryLlmPort};
use crate::ports::memory_signals::MemorySignalsPort;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(45);
const BATCH: usize = 64;
const MAX_ATTEMPTS: i64 = 3;
const EXISTING_LIMIT: usize = 200;
/// Cosine similarity above which a new candidate is considered a duplicate of an
/// existing memory.
const DEDUP_THRESHOLD: f32 = 0.90;

const SYSTEM_PROMPT: &str = "You are a memory distiller for a software project. \
You are given raw signals captured during automated coding runs (human gate \
feedback, step failures/retries, and agent run summaries), plus the project's \
existing memories. Extract durable, reusable knowledge a future coding agent \
would benefit from: conventions, lessons learned, decisions, user preferences, \
and stable facts. Ignore one-off noise and anything already captured by an \
existing memory. Respond with ONLY a JSON array (no prose, no code fences). Each \
element: {\"memory_type\": one of \
[\"convention\",\"lesson\",\"decision\",\"preference\",\"fact\"], \"statement\": \
a single concise imperative sentence, \"confidence\": a number 0..1}. Return an \
empty array [] if nothing is worth remembering.";

#[derive(Deserialize)]
struct ExtractedMemory {
    memory_type: String,
    statement: String,
    #[serde(default = "default_confidence")]
    confidence: f64,
}

fn default_confidence() -> f64 {
    0.6
}

/// Spawn the background memory worker. No-op tick when the memory agent is
/// disabled or not fully configured.
pub fn start_memory_worker(
    app_settings: Arc<dyn AppSettingsRepository>,
    signals: Arc<dyn MemorySignalsPort>,
    memory: Arc<dyn ProjectMemoryPort>,
    memory_llm: Arc<dyn MemoryLlmPort>,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        interval.tick().await; // skip immediate first tick
        loop {
            interval.tick().await;
            if let Err(e) = tick(&*app_settings, &*signals, &*memory, &*memory_llm).await {
                eprintln!("[MemoryWorker] tick error: {}", e);
            }
        }
    });
}

async fn tick(
    app_settings: &dyn AppSettingsRepository,
    signals: &dyn MemorySignalsPort,
    memory: &dyn ProjectMemoryPort,
    memory_llm: &dyn MemoryLlmPort,
) -> Result<(), String> {
    let config = mem_app::load_config(app_settings);
    if !config.is_usable() {
        return Ok(());
    }
    let pending = signals.take_unprocessed(BATCH, MAX_ATTEMPTS)?;
    if pending.is_empty() {
        return Ok(());
    }
    let api_key = mem_app::load_api_key();

    // Group by feature so each LLM call sees a coherent run's worth of signals.
    let mut groups: HashMap<String, Vec<MemorySignal>> = HashMap::new();
    for s in pending {
        groups.entry(s.feature_id.clone()).or_default().push(s);
    }

    for (_feature_id, group) in groups {
        let ids: Vec<String> = group.iter().map(|s| s.id.clone()).collect();
        let project_id = match group.first() {
            Some(s) => s.project_id.clone(),
            None => continue,
        };

        match process_group(
            memory,
            memory_llm,
            &config,
            api_key.as_deref(),
            &project_id,
            &group,
        )
        .await
        {
            Ok(()) => {
                signals.mark_processed(&ids, crate::paths::now_ms())?;
            }
            Err(e) => {
                eprintln!("[MemoryWorker] group failed (will retry): {}", e);
                signals.bump_attempts(&ids)?;
            }
        }
    }
    Ok(())
}

async fn process_group(
    memory: &dyn ProjectMemoryPort,
    memory_llm: &dyn MemoryLlmPort,
    config: &crate::domain::memory::MemoryAgentConfig,
    api_key: Option<&str>,
    project_id: &ProjectId,
    group: &[MemorySignal],
) -> Result<(), String> {
    let existing = memory.memory_list(project_id, EXISTING_LIMIT)?;

    let user_prompt = build_user_prompt(group, &existing);
    let raw = memory_llm
        .chat(
            &config.chat_endpoint,
            &config.chat_model,
            api_key,
            vec![
                ChatMessage::system(SYSTEM_PROMPT),
                ChatMessage::user(user_prompt),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;

    let candidates = parse_candidates(&raw)?;
    if candidates.is_empty() {
        return Ok(());
    }

    for cand in candidates {
        let statement = cand.statement.trim().to_string();
        if statement.is_empty() {
            continue;
        }
        let memory_type = MemoryType::from_str(&cand.memory_type).unwrap_or(MemoryType::Fact);
        let confidence = cand.confidence.clamp(0.0, 1.0);

        let embedding = memory_llm
            .embed(
                config.embed_endpoint_or_chat(),
                &config.embed_model,
                api_key,
                vec![statement.clone()],
            )
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
            .ok_or_else(|| "embeddings returned no vector".to_string())?;

        let now = crate::paths::now_ms();

        // Dedup against existing memories that carry an embedding.
        let best = existing
            .iter()
            .filter_map(|e| {
                e.embedding
                    .as_ref()
                    .map(|v| (e, cosine_similarity(&embedding, v)))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1));

        if let Some((matched, sim)) = best {
            if sim >= DEDUP_THRESHOLD {
                // A human memory already covers this — don't clobber it.
                if matched.source == MemorySource::Human {
                    continue;
                }
                // Refresh the matching agent memory in place (merge).
                let merged = ProjectMemoryEntry {
                    id: matched.id.clone(),
                    project_id: project_id.clone(),
                    key: matched.key.clone(),
                    value: statement.clone(),
                    source: MemorySource::Agent,
                    confidence: matched.confidence.max(confidence),
                    memory_type: Some(memory_type),
                    statement: Some(statement.clone()),
                    embedding: Some(embedding.clone()),
                    embedding_model: Some(config.embed_model.clone()),
                    last_used_at: matched.last_used_at,
                    use_count: matched.use_count,
                    created_at: matched.created_at,
                    updated_at: now,
                };
                memory.memory_upsert(merged)?;
                continue;
            }
        }

        let entry = ProjectMemoryEntry {
            id: format!("pm-{}", crate::paths::new_id()),
            project_id: project_id.clone(),
            key: derive_key(memory_type, &statement),
            value: statement.clone(),
            source: MemorySource::Agent,
            confidence,
            memory_type: Some(memory_type),
            statement: Some(statement.clone()),
            embedding: Some(embedding),
            embedding_model: Some(config.embed_model.clone()),
            last_used_at: None,
            use_count: 0,
            created_at: now,
            updated_at: now,
        };
        memory.memory_upsert(entry)?;
    }

    Ok(())
}

fn build_user_prompt(group: &[MemorySignal], existing: &[ProjectMemoryEntry]) -> String {
    let mut out = String::new();
    out.push_str("## Existing memories\n");
    if existing.is_empty() {
        out.push_str("(none)\n");
    } else {
        for e in existing {
            let t = e.memory_type.map(|t| t.as_str()).unwrap_or("note");
            let body = e.statement.as_deref().unwrap_or(&e.value);
            out.push_str(&format!("- [{}] {}\n", t, body));
        }
    }
    out.push_str("\n## New signals from the latest run\n");
    for s in group {
        out.push_str(&format!("- ({}) {}\n", s.kind.as_str(), s.content));
    }
    out.push_str("\nReturn the JSON array of new memories now.");
    out
}

/// Extract a JSON array from a model response that may include prose or code
/// fences, then parse it.
fn parse_candidates(raw: &str) -> Result<Vec<ExtractedMemory>, String> {
    let start = raw.find('[');
    let end = raw.rfind(']');
    let slice = match (start, end) {
        (Some(s), Some(e)) if e > s => &raw[s..=e],
        _ => return Ok(Vec::new()),
    };
    serde_json::from_str::<Vec<ExtractedMemory>>(slice)
        .map_err(|e| format!("parse candidates: {}", e))
}

/// Build a short, stable-ish key label from a statement (legacy column is
/// NOT NULL; the semantic content lives in `statement`).
fn derive_key(memory_type: MemoryType, statement: &str) -> String {
    let slug: String = statement
        .chars()
        .take(48)
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    format!(
        "{}_{}",
        memory_type.as_str(),
        slug.trim_matches('_').to_lowercase()
    )
}
