use crate::models::EnrichedModels;
use crate::openai::ModelType;
use anyhow::{Context, Result}; // TODO: get rid of anyhow in lib, do thiserror
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::info;

// ── Configurations ────────────────────────────────────────────────────────────

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

pub fn load_config(config_file: &str) -> Result<Config> {
    info!(file = config_file, "Loading configuration file");
    let raw =
        fs::read_to_string(config_file).with_context(|| format!("Cannot read '{config_file}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON or REGEX in '{config_file}'"))
}

pub fn load_model_info_map(info_file: &str) -> Result<EnrichedModels> {
    info!(file = info_file, "Loading mappings file");
    if !Path::new(info_file).exists() {
        return Ok(HashMap::new());
    }
    let raw =
        fs::read_to_string(info_file).with_context(|| format!("Cannot read '{info_file}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{info_file}'"))
}
