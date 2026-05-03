use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::{fs, io};

use directories_next::ProjectDirs;
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use crate::models::EnrichedModels;

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

    #[error("Network request error `{0}`")]
    Update(#[from] reqwest::Error),

    #[error("invalid content in `{path}`")]
    InvalidContent {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("file {path} exists and overwrite was not forced")]
    NoClobber { path: PathBuf },

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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub api_key: String,
    pub base_url: String,

    #[serde(with = "serde_regex")]
    pub exclude_model_name_regex: Vec<Regex>,

    #[serde(default)]
    pub default_system_prompt: String,
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

// ── Config manager ────────────────────────────────────────────────────────────

pub struct ConfigManager {
    path: PathBuf,
    pub config: Config,
}

impl ConfigManager {
    pub fn new(file: Option<&String>) -> Result<Self, ConfigError> {
        let dirs = Directories::new().ok_or_else(|| ConfigError::Directories)?;

        let path = match file {
            Some(file) => Ok(PathBuf::from(file)),

            None => {
                let path = Path::new(DEFAULT_CONFIG_FILE_NAME);
                dirs.config_file(path).map_err(|e| ConfigError::io(path, e))
            }
        }?;

        Ok(Self {
            path,
            config: Config::default(),
        })
    }

    fn to_json(&self) -> Result<String, ConfigError> {
        serde_json::to_string_pretty(&self.config).map_err(|e| ConfigError::json(&self.path, e))
    }

    pub fn show(&self) -> Result<&Self, ConfigError> {
        eprintln!("Configuration : {file}", file = self.path.to_string_lossy());
        println!("{}", self.to_json()?);
        Ok(self)
    }

    pub fn save(&self) -> Result<&Self, ConfigError> {
        info!(
            file = %self.path.display(),
            "Saving configuration"
        );

        fs::write(&self.path, self.to_json()?).map_err(|e| ConfigError::io(&self.path, e))?;

        Ok(self)
    }

    pub fn load(&mut self) -> Result<&mut Self, ConfigError> {
        info!(
            file = %self.path.display(),
            "Loading configuration file"
        );

        let content =
            std::fs::read_to_string(&self.path).map_err(|e| ConfigError::io(&self.path, e))?;

        self.config =
            serde_json::from_str(&content).map_err(|e| ConfigError::json(&self.path, e))?;

        Ok(self)
    }

    pub fn load_or_default(&mut self) -> Result<&mut Self, ConfigError> {
        match self.load() {
            Err(ConfigError::Io { path, source }) if source.kind() == ErrorKind::NotFound => {
                warn!(
                    file = %path.to_string_lossy(),
                    "Configuration not found, using defaults"
                );

                self.config = Config::default();

                Ok(self)
            }

            Ok(_) => Ok(self),

            Err(e) => Err(e),
        }
    }

    pub fn set_key(&mut self) -> Result<&Self, ConfigError> {
        print!(
            "Configuration : {file}\nAPI key ? ",
            file = self.path.to_string_lossy()
        );
        let _ = io::stdout().flush();

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| ConfigError::io(&self.path, e))?;

        self.config.api_key = input.trim().into();

        println!("API key set in {file}", file = self.path.to_string_lossy());
        Ok(self)
    }
}

// ── ModelInfo manager ─────────────────────────────────────────────────────────

pub struct ModelInfoManager {
    path: PathBuf,
    pub enriched_models: EnrichedModels,
}

impl ModelInfoManager {
    pub fn new(file: Option<&String>) -> Result<Self, ConfigError> {
        let dirs = Directories::new().ok_or_else(|| ConfigError::Directories)?;

        let path = match file {
            Some(file) => Ok(PathBuf::from(file)),

            None => {
                let path = Path::new(DEFAULT_MODEL_INFO_FILE_NAME);
                dirs.data_file(path).map_err(|e| ConfigError::io(path, e))
            }
        }?;

        Ok(Self {
            path,
            enriched_models: EnrichedModels::default(),
        })
    }

    fn to_json(&self) -> Result<String, ConfigError> {
        serde_json::to_string_pretty(&self.enriched_models)
            .map_err(|e| ConfigError::json(&self.path, e))
    }

    pub fn show(&self) -> Result<&Self, ConfigError> {
        eprintln!("Model info : {file}", file = self.path.to_string_lossy());
        println!("{}", self.to_json()?);
        Ok(self)
    }

    pub fn save(&self) -> Result<&Self, ConfigError> {
        info!(
            file = %self.path.display(),
            "Saving model info"
        );

        fs::write(&self.path, self.to_json()?).map_err(|e| ConfigError::io(&self.path, e))?;

        Ok(self)
    }

    pub fn load(&mut self) -> Result<&mut Self, ConfigError> {
        info!(
            file = %self.path.display(),
            "Loading model info"
        );

        let content =
            std::fs::read_to_string(&self.path).map_err(|e| ConfigError::io(&self.path, e))?;

        self.enriched_models = serde_json::from_str::<EnrichedModels>(&content)
            .map_err(|e| ConfigError::json(&self.path, e))?;

        Ok(self)
    }

    pub fn load_or_default(&mut self) -> Result<&mut Self, ConfigError> {
        match self.load() {
            Err(ConfigError::Io { path, source }) if source.kind() == ErrorKind::NotFound => {
                warn!(
                    file = %path.to_string_lossy(),
                    "Model info not found, using defaults"
                );

                self.enriched_models = EnrichedModels::default();

                Ok(self)
            }

            Ok(_) => Ok(self),

            Err(e) => Err(e),
        }
    }

    pub async fn update(&mut self, client: &Client, url: &str) -> Result<&mut Self, ConfigError> {
        info!(url = url, "Updating model info");

        let enriched_models = client
            .get(url)
            .send()
            .await?
            .json::<EnrichedModels>()
            .await?;

        self.enriched_models = enriched_models;

        info!(
            count = self.enriched_models.len(),
            file = %self.path.display(),
            "Fetched model info"
        );

        return Ok(self);
    }
}

// ── XDG directories ───────────────────────────────────────────────────────────

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
