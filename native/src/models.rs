use std::collections::HashMap;

use crate::config::ModelInfo;
use crate::openai::ModelType;

pub type EnrichedModels = HashMap<String, ModelInfo>;

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
