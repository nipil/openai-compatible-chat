use std::collections::HashMap;

use crate::config::ModelInfo;
use crate::openai::ModelType;

pub type EnrichedModels = HashMap<String, ModelInfo>;

pub struct EnrichedModel<'a> {
    pub id: &'a str,
    pub info: &'a ModelInfo,
}

impl<'a> EnrichedModel<'a> {
    pub fn new(id: &'a str, info: &'a ModelInfo) -> Self {
        EnrichedModel { id, info }
    }
}

pub const COMPATIBLE_MODEL_TYPES: &[ModelType] = &[
    ModelType::Chat,
    ModelType::Instruct,
    ModelType::Multimodal,
    ModelType::Reasoning,
];

// models.sort_by(|a, b| a.info.family.cmp(&b.info.family).then(a.id.cmp(&b.id)));
