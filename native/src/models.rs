use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig, error::OpenAIError};
use portable::{Exclusion, ModelInfoMap, ModelType};
use regex::Regex;

pub const ALLOWED_TYPES: &[ModelType] = &[
    ModelType::Chat,
    ModelType::Instruct,
    ModelType::Multimodal,
    ModelType::Reasoning,
];

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EnrichedModel {
    pub id: String,
    pub family: String,
    pub model_type: ModelType,
    pub context_window: Option<u32>,
}

#[derive(Debug)]
pub enum ModelError {
    NotAllowed,
    NotFound,
    Network(String),
}

// ── API ───────────────────────────────────────────────────────────────────────

pub async fn test_model(
    client: &Client<OpenAIConfig>,
    id: &str,
) -> std::result::Result<(), ModelError> {
    match client.models().retrieve(id).await {
        Ok(_) => Ok(()),
        Err(OpenAIError::ApiError(e)) => {
            let msg = e.message.to_lowercase();
            // TODO: check API exact error to see if it works actually
            // TODO: thiserror
            if msg.contains("not allowed") || msg.contains("permission") {
                Err(ModelError::NotAllowed)
            } else {
                Err(ModelError::NotFound)
            }
        }
        Err(e) => Err(ModelError::Network(e.to_string())),
    }
}

pub async fn list_models(client: &Client<OpenAIConfig>) -> Result<Vec<String>> {
    client
        .models()
        .list()
        .await
        .map(|r| r.data.into_iter().map(|m| m.id).collect())
        .map_err(|e| anyhow!("Failed to list models: {e}"))
}

// ── Filtering / sorting ───────────────────────────────────────────────────────

pub fn compile_regex(patterns: &[String]) -> Result<Vec<Regex>> {
    patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| anyhow!("Invalid regex '{p}': {e}")))
        .collect()
}

pub fn filter_and_sort(
    ids: Vec<String>,
    models: &ModelInfoMap,
    excluded: &[String],
    filters: &[Regex],
) -> Vec<EnrichedModel> {
    let mut models: Vec<EnrichedModel> = ids
        .into_iter()
        .filter(|id| !excluded.contains(id))
        .filter(|id| !filters.iter().any(|r| r.is_match(id)))
        .filter_map(|id| {
            let model_info = models.get(&id)?;
            // Drop models whose type is known but not in the allowed set.
            if !ALLOWED_TYPES.contains(&model_info.model_type) {
                return None;
            }
            Some(EnrichedModel {
                family: model_info.family.clone(),
                context_window: model_info.context_window,
                model_type: model_info.model_type.clone(),
                id,
            })
        })
        .collect();

    models.sort_by(|a, b| a.family.cmp(&b.family).then(a.id.cmp(&b.id)));
    models
}

/// Returns a human-readable rejection reason, or `None` if the model passes.
pub fn explain_rejection(
    id: &str,
    models: &ModelInfoMap,
    excl: &Exclusion,
    filters: &[Regex],
) -> Option<String> {
    if excl.excluded_models.contains(&id.to_string()) {
        return Some("excluded (previously marked as not allowed)".into());
    }
    if filters.iter().any(|r| r.is_match(id)) {
        return Some("filtered out by exclude_model_name_regex".into());
    }
    if let Some(meta) = models.get(id) {
        if !ALLOWED_TYPES.contains(&meta.model_type) {
            return Some(format!(
                "filtered out (type={} not supported)",
                meta.model_type
            ));
        }
    }
    None
}
