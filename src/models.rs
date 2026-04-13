use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig, error::OpenAIError};
use regex::Regex;

use crate::config::{Exclusion, Mapping, ModelMeta};

pub const ALLOWED_TYPES: &[&str] = &["chat", "multimodal", "reasoning", "instruct"];

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct EnrichedModel {
    pub id: String,
    pub family: String,
    pub model_type: Option<String>,
    pub max_tokens: Option<u32>,
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
    mapping: &Mapping,
    excluded: &[String],
    filters: &[Regex],
) -> Vec<EnrichedModel> {
    let mut models: Vec<EnrichedModel> = ids
        .into_iter()
        .filter(|id| !excluded.contains(id))
        .filter(|id| !filters.iter().any(|r| r.is_match(id)))
        .filter_map(|id| {
            let meta: Option<&ModelMeta> = mapping.get(&id);
            let ty = meta.and_then(|m| m.model_type.clone());
            // Drop models whose type is known but not in the allowed set.
            if let Some(ref t) = ty {
                if !ALLOWED_TYPES.contains(&t.as_str()) {
                    return None;
                }
            }
            Some(EnrichedModel {
                family: meta
                    .and_then(|m| m.family.clone())
                    .unwrap_or_else(|| "zzz".into()),
                max_tokens: meta.and_then(|m| m.max_tokens),
                model_type: ty,
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
    mapping: &Mapping,
    excl: &Exclusion,
    filters: &[Regex],
) -> Option<String> {
    if excl.excluded_models.contains(&id.to_string()) {
        return Some("excluded (previously marked as not allowed)".into());
    }
    if filters.iter().any(|r| r.is_match(id)) {
        return Some("filtered out by exclude_model_name_regex".into());
    }
    if let Some(meta) = mapping.get(id) {
        if let Some(ref t) = meta.model_type {
            if !ALLOWED_TYPES.contains(&t.as_str()) {
                return Some(format!("filtered out (type={t} not supported)"));
            }
        }
    }
    None
}
