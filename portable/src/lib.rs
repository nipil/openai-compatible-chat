use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Mapping = HashMap<String, ModelMeta>;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    #[serde(default)]
    pub exclude_model_name_regex: Vec<String>,
    #[serde(default)]
    pub prepend_system_prompt: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelMeta {
    pub family: Option<String>,
    #[serde(rename = "type")]
    pub model_type: Option<String>,
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Exclusion {
    #[serde(default)]
    pub excluded_models: Vec<String>,
}
