use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::openai::ModelType;

pub type EnrichedModels = HashMap<String, ModelInfo>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub(crate) description: String,
    pub(crate) family: String,

    #[serde(rename = "type")]
    pub model_type: ModelType,

    pub(crate) context_window: Option<u32>,
    pub(crate) release: Option<String>,
}

pub(crate) struct EnrichedModel {
    pub(crate) id: String,
    pub(crate) info: ModelInfo,
}

impl EnrichedModel {
    pub(crate) fn new(id: String, info: ModelInfo) -> Self {
        EnrichedModel { id, info }
    }
}

pub const COMPATIBLE_MODEL_TYPES: &[ModelType] = &[
    ModelType::Chat,
    ModelType::Instruct,
    ModelType::Multimodal,
    ModelType::Reasoning,
];
