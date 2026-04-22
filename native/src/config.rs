use crate::openai::ModelType;
use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::info;

pub const CONFIG_PATH: &str = "config.json";
pub const MAPPING_PATH: &str = "ai_model_info/openai.json";

// ── Configurations ────────────────────────────────────────────────────────────

pub type ModelInfoMap = HashMap<String, ModelInfo>;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    #[serde(with = "serde_regex")]
    pub exclude_model_name_regex: Vec<Regex>,
    #[serde(default)]
    pub prepend_system_prompt: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub description: String,
    pub family: String,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    pub context_window: Option<u32>,
    pub release: Option<String>,
}

// ── I/O helpers ──────────────────────────────────────────────────────────────

pub fn load_config() -> Result<Config> {
    info!(file = CONFIG_PATH, "Loading configuration file");
    let raw =
        fs::read_to_string(CONFIG_PATH).with_context(|| format!("Cannot read '{CONFIG_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON or REGEX in '{CONFIG_PATH}'"))
}

pub fn load_model_info_map() -> Result<ModelInfoMap> {
    info!(file = MAPPING_PATH, "Loading mappings file");
    if !Path::new(MAPPING_PATH).exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(MAPPING_PATH)
        .with_context(|| format!("Cannot read '{MAPPING_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{MAPPING_PATH}'"))
}
