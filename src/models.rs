use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig, error::OpenAIError};
use dialoguer::FuzzySelect;
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

// ── Display / selection ───────────────────────────────────────────────────────

/// Renders the columnar model grid, then opens an interactive fuzzy-search
/// prompt. Returns the selected model ID.
pub fn select_model(models: &[EnrichedModel]) -> Result<String> {
    if models.len() == 1 {
        crate::display::log_info(&format!("Auto-selected: {}", models[0].id));
        return Ok(models[0].id.clone());
    }

    print_model_grid(models);

    let labels: Vec<String> = models
        .iter()
        .map(|m| match &m.model_type {
            Some(t) => format!("{} ({})", m.id, t),
            None => m.id.clone(),
        })
        .collect();

    let idx = FuzzySelect::new()
        .with_prompt("Select model")
        .items(&labels)
        .default(0)
        .interact()
        .map_err(|e| anyhow!("Selection failed: {e}"))?;

    Ok(models[idx].id.clone())
}

fn print_model_grid(models: &[EnrichedModel]) {
    let term_w = crossterm::terminal::size()
        .map(|(w, _)| w as usize)
        .unwrap_or(120);

    let labels: Vec<String> = models
        .iter()
        .enumerate()
        .map(|(i, m)| match &m.model_type {
            Some(t) => format!("{}. {} ({})", i + 1, m.id, t),
            None => format!("{}. {}", i + 1, m.id),
        })
        .collect();

    let col_w = labels.iter().map(|l| l.len()).max().unwrap_or(20) + 4;
    let cols = (term_w / col_w).max(1);
    let rows = labels.len().div_ceil(cols);

    for row in 0..rows {
        let mut line = String::new();
        for col in 0..cols {
            let idx = col * rows + row;
            if idx < labels.len() {
                let lbl = &labels[idx];
                line.push_str(lbl);
                (0..col_w.saturating_sub(lbl.len())).for_each(|_| line.push(' '));
            }
        }
        println!("{line}");
    }
    println!();
}
