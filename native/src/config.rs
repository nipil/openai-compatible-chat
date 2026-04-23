use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::Deserialize;
use thiserror::Error;
use tracing::info;

use crate::models::EnrichedModels;
use crate::openai::ModelType;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to access `{path}`")]
    FileError {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid content in `{path}`")]
    InvalidContent {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl ConfigError {
    fn file(path: &Path, source: io::Error) -> Self {
        Self::FileError {
            path: path.to_path_buf(),
            source,
        }
    }

    fn invalid(path: &Path, source: serde_json::Error) -> Self {
        Self::InvalidContent {
            path: path.to_path_buf(),
            source,
        }
    }
}

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

// tracing macro specific: %=Display and ?=Debug
// Path does not implement Display
// There is a std::path::Display which provides .display()
// And this wrapper provides a thing that impl fmt::Display

pub fn load_config(file: &Path) -> Result<Config, ConfigError> {
    info!(
        file = %file.display(),
        "Loading configuration file"
    );
    let raw = std::fs::read_to_string(file).map_err(|e| ConfigError::file(file, e))?;
    let cfg = serde_json::from_str(&raw).map_err(|e| ConfigError::invalid(file, e))?;
    Ok(cfg)
}

pub fn load_model_info_map(file: &Path) -> Result<EnrichedModels, ConfigError> {
    tracing::info!(
        file = %file.display(),
        "Loading mappings file"
    );
    if !Path::new(file).exists() {
        return Ok(EnrichedModels::new());
    }
    let raw = std::fs::read_to_string(file).map_err(|e| ConfigError::file(file, e))?;
    let models = serde_json::from_str(&raw).map_err(|e| ConfigError::invalid(file, e))?;
    Ok(models)
}
