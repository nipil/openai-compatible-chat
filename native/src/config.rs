use std::path::{Path, PathBuf};
use std::{fs, io};

use directories_next::ProjectDirs;
use regex::Regex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;

use crate::models::EnrichedModels;
use crate::openai::ModelType;

const DEFAULT_CONFIG_FILE_NAME: &str = "config.json";
const DEFAULT_MODEL_INFO_FILE_NAME: &str = "openai.json";
const DEFAULT_OPENAI_API_BASE_URL: &str = "https://api.openai.com/v1";

pub const DEFAULT_MODEL_INFO_FILE_URL: &str = "https://raw.githubusercontent.com/nipil/openai-compatible-chat/refs/heads/main/ai_model_info/openai.json";

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to access `{path}`")]
    Io {
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

    #[error("failed to detect standard directories from home")]
    Directories,
}

impl ConfigError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }

    fn json(path: &Path, source: serde_json::Error) -> Self {
        Self::InvalidContent {
            path: path.to_path_buf(),
            source,
        }
    }
}

// ── Configurations ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,
    #[serde(with = "serde_regex")]
    pub exclude_model_name_regex: Vec<Regex>,
    #[serde(default)]
    pub default_system_prompt: String,
}

impl Config {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("Configuration should not fail to serialize")
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: Default::default(),
            base_url: String::from(DEFAULT_OPENAI_API_BASE_URL),
            exclude_model_name_regex: Default::default(),
            default_system_prompt: Default::default(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelInfo {
    pub(crate) description: String,
    pub(crate) family: String,
    #[serde(rename = "type")]
    pub model_type: ModelType,
    pub(crate) context_window: Option<u32>,
    pub(crate) release: Option<String>,
}

// ── I/O helpers ──────────────────────────────────────────────────────────────

// tracing macro specific: %=Display and ?=Debug
// Path does not implement Display
// There is a std::path::Display which provides .display()
// And this wrapper provides a thing that impl fmt::Display
// Every day you learn...

pub fn load_config(file: Option<String>) -> Result<Config, ConfigError> {
    let dirs = Directories::new().ok_or_else(|| ConfigError::Directories)?;

    let file = match file {
        Some(file) => PathBuf::from(file),
        None => {
            let path = Path::new(DEFAULT_CONFIG_FILE_NAME);
            dirs.config_file(path)
                .map_err(|e| ConfigError::io(path, e))?
        }
    };

    info!(
        file = %file.display(),
        "Loading configuration file"
    );

    let raw = std::fs::read_to_string(&file).map_err(|e| ConfigError::io(&file, e))?;
    let cfg = serde_json::from_str(&raw).map_err(|e| ConfigError::json(&file, e))?;

    Ok(cfg)
}

pub fn load_model_info_map(file: Option<String>) -> Result<EnrichedModels, ConfigError> {
    let dirs = Directories::new().ok_or_else(|| ConfigError::Directories)?;

    let file = match file {
        Some(file) => PathBuf::from(file),
        None => {
            let path = Path::new(DEFAULT_MODEL_INFO_FILE_NAME);
            dirs.data_file(path).map_err(|e| ConfigError::io(path, e))?
        }
    };

    tracing::info!(
        file = %file.display(),
        "Loading mappings file"
    );
    let raw = std::fs::read_to_string(&file).map_err(|e| ConfigError::io(&file, e))?;
    let models = serde_json::from_str(&raw).map_err(|e| ConfigError::json(&file, e))?;
    Ok(models)
}

// an impl to easily access XDG compliant folders
pub(crate) struct Directories {
    project: ProjectDirs,
}

impl Directories {
    /// Create a new instance for your app.
    /// The identifiers should be stable (reverse DNS style recommended).
    pub fn new() -> Option<Self> {
        let project = ProjectDirs::from("com.github", "nipil", "openai-compatible-chat")?;
        Some(Self { project })
    }

    /// Base config directory (XDG_CONFIG_HOME / %APPDATA%)
    fn config_dir(&self) -> &Path {
        self.project.config_dir()
    }

    /// Base data directory (XDG_DATA_HOME / %LOCALAPPDATA%)
    fn data_dir(&self) -> &Path {
        self.project.data_dir()
    }

    /// Get a full config file path (parent directory is ensured)
    pub fn config_file(&self, relative: &Path) -> std::io::Result<PathBuf> {
        let full = self.config_dir().join(relative);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(full)
    }

    /// Get a full data file path (parent directory is ensured)
    pub fn data_file(&self, relative: &Path) -> std::io::Result<PathBuf> {
        let full = self.data_dir().join(relative);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(full)
    }
}
