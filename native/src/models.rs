use crate::config::load_model_info_map;
use crate::display::{log_info, log_warning};
use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use portable::{EnrichedModel, ModelType};
use regex::Regex;

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
                log_warning(&format!(
                    "No metadata for model {id}, update your metadata file !"
                ));
                return None;
            };
            // Drop models whose type is known but not in the compatible set.
            if !COMPATIBLE_MODEL_TYPES.contains(&model_info.model_type) {
                log_info(&format!(
                    "Incompatible model '{id}' of type '{}' ignored",
                    model_info.model_type
                ));
                return None;
            }
            Some(EnrichedModel {
                id,
                info: model_info,
            })
        })
        .collect();

    // TODO: implement PartialEq on ModelInfo ?
    models.sort_by(|a, b| a.info.family.cmp(&b.info.family).then(a.id.cmp(&b.id)));
    Ok(models)
}
