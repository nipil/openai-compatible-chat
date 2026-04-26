use std::collections::HashMap;

use crate::config::ModelInfo;
use crate::openai::ModelType;

pub type EnrichedModels = HashMap<String, ModelInfo>;

pub(crate) struct EnrichedModel<'a> {
    pub(crate) id: &'a str,
    pub(crate) info: &'a ModelInfo,
}

impl<'a> EnrichedModel<'a> {
    pub(crate) fn new(id: &'a str, info: &'a ModelInfo) -> Self {
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
