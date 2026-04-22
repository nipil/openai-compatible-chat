use crate::config::load_model_info_map;
use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use portable::{EnrichedModel, ModelType};
use regex::Regex;
use tracing::{info, warn};

pub const COMPATIBLE_MODEL_TYPES: &[ModelType] = &[
    ModelType::Chat,
    ModelType::Instruct,
    ModelType::Multimodal,
    ModelType::Reasoning,
];

// ── API ───────────────────────────────────────────────────────────────────────

pub async fn list_models(client: &Client<OpenAIConfig>) -> Result<Vec<String>> {
    // TODO: deduplicate ? just in case ?
    client
        .models()
        .list()
        .await
        .map(|r| r.data.into_iter().map(|m| m.id).collect())
        .map_err(|e| anyhow!("Failed to list models: {e}"))
}

// ── Filtering / sorting ───────────────────────────────────────────────────────

pub fn enriched_models_from_ids(
    ids: Vec<String>,
    reject_patterns: Vec<Regex>,
) -> Result<Vec<EnrichedModel>> {
    // Load discardable information
    let mut model_info_map = load_model_info_map()?;

    let mut models: Vec<EnrichedModel> = ids
        .into_iter()
        // do not keep ids that match any of the reject patterns
        .filter(|id| !reject_patterns.iter().any(|r| r.is_match(id)))
        .filter_map(|id| {
            // Check that we have metadata for the model, otherwise ignore it
            let Some(model_info) = model_info_map.remove(&id) else {
                warn!(model = id, "No metadata available, update required");
                return None;
            };
            // Drop models whose type is known but not in the compatible set.
            if !COMPATIBLE_MODEL_TYPES.contains(&model_info.model_type) {
                info!(
                    model = id,
                    model_type = model_info.model_type.to_string(),
                    "Incompatible model",
                );
                return None;
            }
            Some(EnrichedModel {
                id,
                info: model_info,
            })
        })
        .collect();

    // For this simple "one-shot" sort, implementing Ord/PartialEq/Eq is not needed
    models.sort_by(|a, b| a.info.family.cmp(&b.info.family).then(a.id.cmp(&b.id)));
    Ok(models)
}
