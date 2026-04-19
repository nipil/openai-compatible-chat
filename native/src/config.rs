use anyhow::{Context, Result};
use portable::{Config, ModelInfoMap};
use std::{collections::HashMap, fs, path::Path};

pub const CONFIG_PATH: &str = "config.json";
pub const MAPPING_PATH: &str = "ai_model_info/openai.json";

// ── I/O helpers ──────────────────────────────────────────────────────────────

pub fn load_config() -> Result<Config> {
    let raw =
        fs::read_to_string(CONFIG_PATH).with_context(|| format!("Cannot read '{CONFIG_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{CONFIG_PATH}'"))
}

pub fn load_model_info_map() -> Result<ModelInfoMap> {
    if !Path::new(MAPPING_PATH).exists() {
        return Ok(HashMap::new());
    }
    let raw = fs::read_to_string(MAPPING_PATH)
        .with_context(|| format!("Cannot read '{MAPPING_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{MAPPING_PATH}'"))
}
