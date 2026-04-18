use anyhow::{Context, Result};
use portable::{Config, Exclusion, ModelInfoMap};
use std::{collections::HashMap, fs, path::Path};
// use config::
pub const CONFIG_PATH: &str = "config.json";
pub const MAPPING_PATH: &str = "ai_model_info/openai.json";
pub const EXCLUSION_PATH: &str = "exclusion.json";

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

pub fn load_model_id_exclusion_list() -> Result<Exclusion> {
    if !Path::new(EXCLUSION_PATH).exists() {
        return Ok(Exclusion::default());
    }
    let raw = fs::read_to_string(EXCLUSION_PATH)
        .with_context(|| format!("Cannot read '{EXCLUSION_PATH}'"))?;
    serde_json::from_str(&raw).with_context(|| format!("Invalid JSON in '{EXCLUSION_PATH}'"))
}

pub fn save_model_id_exclusion_list(ex: &Exclusion) -> Result<()> {
    Ok(fs::write(
        EXCLUSION_PATH,
        serde_json::to_string_pretty(ex)?,
    )?)
}
