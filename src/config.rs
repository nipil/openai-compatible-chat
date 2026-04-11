use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

pub const CONFIG_PATH: &str = "config.json";
pub const MAPPING_PATH: &str = "mapping.json";
pub const EXCLUSION_PATH: &str = "exclusion.json";

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

// ── I/O helpers ──────────────────────────────────────────────────────────────

pub fn load_config() -> Result<Config> {
    let raw =
        fs::read_to_string(CONFIG_PATH).with_context(|| format!("Cannot read '{CONFIG_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{CONFIG_PATH}'"))
}

pub fn load_mapping() -> Result<Mapping> {
    if !Path::new(MAPPING_PATH).exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(MAPPING_PATH)
        .with_context(|| format!("Cannot read '{MAPPING_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{MAPPING_PATH}'"))
}

pub fn load_exclusion() -> Result<Exclusion> {
    if !Path::new(EXCLUSION_PATH).exists() {
        return Ok(Exclusion::default());
    }
    let raw = fs::read_to_string(EXCLUSION_PATH)
        .with_context(|| format!("Cannot read '{EXCLUSION_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{EXCLUSION_PATH}'"))
}

pub fn save_exclusion(ex: &Exclusion) -> Result<()> {
    Ok(fs::write(
        EXCLUSION_PATH,
        serde_json::to_string_pretty(ex)?,
    )?)
}
