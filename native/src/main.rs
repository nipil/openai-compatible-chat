use anyhow::Result;
use async_openai::Client as OpenAiClient;
use async_openai::config::OpenAIConfig;
use clap::{Parser, Subcommand};
use native::cli::run_cli;
use native::config::load_config;
use native::models::EnrichedModels;
use native::openai::list_models;
use native::web::run_web;
use native::{AppState, config::load_model_info_map};
use reqwest::Client as ReqwestClient;
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(all(not(feature = "cli"), not(feature = "web")))]
compile_error!("At lease one of the main features should be enabled !");

#[derive(Parser)]
#[command(name = "chat", about = "Interactive LLM", version)]
struct Args {
    #[arg(long, short = 't', default_value_t = 3000)]
    api_timeout_ms: u64,

    #[arg(long, short = 'c', default_value = "config.json")]
    config_file: String,

    #[arg(long, short = 'i', default_value = "ai_model_info/openai.json")]
    info_file: String,

    #[arg(long, short = 'l')]
    lock_model: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// CLI subcommand
    #[cfg(feature = "cli")]
    Cli,

    /// Web subcommand
    #[cfg(feature = "web")]
    Web {
        /// Port to listen on
        #[arg(short = 'p', long = "port")]
        port: u16,

        /// Path to
        #[arg(long, short = 'd', default_value = "wasm/dist")]
        dist_wasm: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
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

    // Tracing configuration for logging
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt::layer().with_span_events(fmt::format::FmtSpan::CLOSE))
        .init();

    // Load configuration once
    let cfg = load_config(Path::new(&args.config_file)).map_err(|e| {
        error!(exc = e.to_string(), "Error loading configuration");
        e
    })?;

    // Load model information
    let mut info_map = load_model_info_map(Path::new(&args.info_file)).map_err(|e| {
        error!(exc = e.to_string(), "Error loading model information");
        e
    })?;

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key)
        .with_api_base(cfg.base_url);

    // Configure a shared http client
    let reqwest_client = ReqwestClient::builder()
        // TODO: configure proxy
        // TODO: configure ...
        .timeout(std::time::Duration::from_millis(args.api_timeout_ms))
        .build()?;

    // Build the OpenAI client from parts
    let openai_client = OpenAiClient::with_config(oa_cfg).with_http_client(reqwest_client);

    // Build the list of usable models
    let candidate_models: EnrichedModels = list_models(&openai_client)
        .await?
        .into_iter()
        // Constrain the model to the one specified, if any
        .filter(|id| {
            args.lock_model.as_ref().map_or(true, |lock_id| {
                let keep = lock_id == id;
                if !keep {
                    info!(
                        model = id,
                        lock = args.lock_model,
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
            info_map
                .remove(&id)
                .or_else(|| {
                    warn!(model = id, "No metadata available, update required");
                    None
                })
                .and_then(|info| Some((id, info)))
        })
        .collect();

    // Drop unused model information to free up memory before the actual run
    drop(info_map);

    // Finally assemble the state and provide it
    let state = AppState {
        openai_client: Arc::new(openai_client),
        prepend_system_prompt: Arc::new(cfg.prepend_system_prompt),
        candidate_models: Arc::new(candidate_models),
    };

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli => {
            run_cli(state).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port, dist_wasm } => {
            run_web(state, port, dist_wasm).await?;
        }
    }
    Ok(())
}
