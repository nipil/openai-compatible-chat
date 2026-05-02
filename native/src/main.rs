use std::env;
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use async_openai::Client as OpenAiClient;
use async_openai::config::OpenAIConfig;
use clap::{Parser, Subcommand};
use native::AppState;
use native::cli::{DEFAULT_CLI_REFRESH_INTERVAL_MS, run_cli};
use native::config::{ConfigManager, DEFAULT_MODEL_INFO_FILE_URL, ModelInfoManager};
use native::models::{COMPATIBLE_MODEL_TYPES, EnrichedModels};
use native::openai::list_models;
use native::web::run_web;
use portable::Theme;
use reqwest::Client as ReqwestClient;
use tracing::{info, warn};
use tracing_appender::rolling;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, fmt};

#[cfg(all(not(feature = "cli"), not(feature = "web")))]
compile_error!("At lease one of the main features should be enabled !");

const TRACE_LOG: &str = "trace.log";

#[derive(Parser)]
#[command(name = "chat", about = "openai-compatible-chat", version)]
struct Args {
    #[arg(long, short = 't')]
    api_timeout_sec: Option<u64>,

    #[arg(long, short = 'c')]
    config_file: Option<String>,

    #[arg(long, short = 'i')]
    info_file: Option<String>,

    #[arg(long, short = 'm')]
    model_lock: Option<String>,

    #[arg(long)]
    log_file: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    ModelInfo {
        #[command(subcommand)]
        command: ModelInfoCommands,
    },

    #[cfg(feature = "cli")]
    Cli {
        #[arg(long, default_value = Theme::Dark.as_ref(), value_parser = Theme::from_str)]
        theme: Theme,

        #[arg(long, default_value_t = DEFAULT_CLI_REFRESH_INTERVAL_MS)]
        refresh_ms: u64,
    },

    #[cfg(feature = "web")]
    Web {
        /// Port to listen on
        #[arg(short = 'p', long = "port")]
        port: u16,

        /// Path to serve the WASM/JS and static files from
        #[arg(long, short = 'd', default_value = "wasm/dist")]
        dist_wasm: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommands {
    SetKey,
    Show,
}

#[derive(Subcommand)]
enum ModelInfoCommands {
    Show,

    Update {
        /// Url to retrieve the model description from.
        /// OpenAI api does not give these info.
        /// See `ai_model_info` package in the project.
        #[arg(long, short = 'u', default_value = DEFAULT_MODEL_INFO_FILE_URL)]
        url: String,
    },
}

fn use_color() -> bool {
    // hard disable
    if env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // force enable
    if env::var("CLICOLOR_FORCE")
        .ok()
        .map(|v| v.trim() == "1")
        .unwrap_or(false)
    {
        return true;
    }
    // alternate disable
    if env::var("CLICOLOR")
        .ok()
        .map(|v| v.trim() == "0")
        .unwrap_or(false)
    {
        return false;
    }
    // default: only if stdout is a terminal
    atty::is(atty::Stream::Stdout)
}

async fn run() -> Result<ExitCode> {
    // Enable ANSI colour codes on legacy Windows consoles (cmd.exe).
    // No-op on Windows 10+ / modern terminals / all Unix systems.
    #[cfg(windows)]
    enable_ansi_support::enable_ansi_support().ok();

    // Clean Ctrl-C exit from anywhere in the program.
    tokio::spawn(async {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\nExiting.");
        std::process::exit(0);
    });

    // Parse arguments
    let args = Args::parse();

    // Build the optional file layer and keep the guard alive for the duration of main.
    // The guard must outlive the subscriber, so return it and bind it at the call site.
    let (_guard_file, file_layer) = match &args.log_file {
        Some(level_str) => {
            let file_appender = rolling::daily(".", TRACE_LOG);
            let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
            // file level must always explicitely defined
            let file_level: LevelFilter = level_str
                .parse()
                .map_err(|_| anyhow!("Invalid trace_file level '{level_str}'"))?;
            let layer = fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false)
                .with_target(true)
                .with_span_events(fmt::format::FmtSpan::CLOSE)
                .with_filter(file_level);

            (Some(guard), Some(layer))
        }
        None => (None, None),
    };

    // console level via RUST_LOG and defaults to ERROR
    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(use_color())
        .with_span_events(fmt::format::FmtSpan::CLOSE)
        .with_target(true)
        .with_filter(EnvFilter::from_default_env());

    // finally setup logging
    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    // process configuration commands
    if let Commands::Config { command } = &args.command {
        match command {
            ConfigCommands::SetKey => {
                ConfigManager::new(args.config_file.as_ref())?
                    .load_or_default()?
                    .set_key()?
                    .save()?;
                println!("Configuration API key updated.");
                return Ok(ExitCode::SUCCESS);
            }
            ConfigCommands::Show => {
                ConfigManager::new(args.config_file.as_ref())?
                    .load_or_default()?
                    .show()?;
                return Ok(ExitCode::SUCCESS);
            }
        }
    }

    // load configuration once
    let cfg = ConfigManager::new(args.config_file.as_ref())?
        .load()?
        .config
        .clone();

    // Check that we have something loaded
    if cfg.api_key.is_empty() {
        eprintln!("Empty api key ! Set it with `config set-key`");
        return Ok(ExitCode::FAILURE);
    }

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key)
        .with_api_base(cfg.base_url);

    // Configure a shared http client
    let reqwest_client = {
        // system proxy are handled/enabled by default for HTTP/HTTPS
        // https://docs.rs/reqwest/latest/reqwest/index.html#proxies
        let mut builder = ReqwestClient::builder();
        // Apply conditional timeout
        builder = match args.api_timeout_sec {
            None => builder,
            Some(seconds) => builder.timeout(std::time::Duration::from_secs(seconds)),
        };
        builder.build()?
    };

    // process model info commands
    if let Commands::ModelInfo { command } = &args.command {
        match command {
            ModelInfoCommands::Show => {
                ModelInfoManager::new(args.info_file.as_ref())?
                    .load_or_default()?
                    .show()?;
                return Ok(ExitCode::SUCCESS);
            }
            ModelInfoCommands::Update { url } => {
                ModelInfoManager::new(args.info_file.as_ref())?
                    .load_or_default()?
                    .update(&reqwest_client, url)
                    .await?
                    .save()?;
                println!("Model info updated.");
                return Ok(ExitCode::SUCCESS);
            }
        }
    }

    // Build the OpenAI client from parts
    let openai_client = OpenAiClient::with_config(oa_cfg).with_http_client(reqwest_client);

    // Load model information
    let mut enriched_models = ModelInfoManager::new(args.info_file.as_ref())?
        .load()?
        .enriched_models
        .clone();

    // Check that we have something loaded
    if enriched_models.is_empty() {
        eprintln!("Empty model info ! Get it with `model-info update`");
        return Ok(ExitCode::FAILURE);
    }

    // Build the list of usable models
    let available_models: EnrichedModels = list_models(&openai_client)
        .await?
        .into_iter()
        // Constrain the model to the one specified, if any
        .filter(|id| {
            args.model_lock.as_ref().map_or(true, |lock_id| {
                let keep = lock_id == id;
                if !keep {
                    info!(
                        model = id,
                        lock = args.model_lock,
                        "Ignore model due to lock",
                    );
                }
                keep
            })
        })
        // Do not keep ids that match any of the reject patterns
        .filter(|id| {
            !cfg.exclude_model_name_regex.iter().any(|r| {
                let reject = r.is_match(id);
                if reject {
                    info!(
                        model = id,
                        pattern = r.to_string(),
                        "Ignore model matching reject pattern",
                    );
                }
                reject
            })
        })
        // Extract additional information for the ones we know
        .filter_map(|id| {
            enriched_models
                .remove(&id)
                .or_else(|| {
                    warn!(model = id, "No metadata available, update required");
                    None
                })
                .and_then(|info| Some((id, info)))
        })
        // Only keep the ones with a compatible chat-like type
        .filter(|(id, info)| {
            let keep = COMPATIBLE_MODEL_TYPES.contains(&info.model_type);
            if !keep {
                info!(
                    "type" = &info.model_type.as_ref(),
                    model = id,
                    "Ignoring incompatible model"
                );
            }
            keep
        })
        .collect();

    // Drop unused model information to free up memory before the actual run
    drop(enriched_models);

    // Finally assemble the state and provide it
    let state = AppState {
        openai_client: Arc::new(openai_client),
        default_system_prompt: Arc::new(cfg.default_system_prompt),
        available_models: Arc::new(available_models),
    };

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli { theme, refresh_ms } => {
            run_cli(state, &theme, *refresh_ms).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port, dist_wasm } => {
            run_web(state, port, dist_wasm).await?;
        }
        _ => {}
    }
    Ok(ExitCode::from(0))
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => return code,
        Err(e) => {
            // still keeps the pretty printing of anyhow
            eprintln!("Error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
