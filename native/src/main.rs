use anyhow::Result;
use async_openai::Client as OpenAiClient;
use async_openai::config::OpenAIConfig;
use clap::{Parser, Subcommand};
use native::AppState;
use native::cli::run_cli;
use native::config::load_config;
use native::models::enriched_models_from_ids;
use native::openai::list_models;
use native::web::run_web;
use reqwest::Client as ReqwestClient;
use std::sync::Arc;
use tracing::error;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(all(not(feature = "cli"), not(feature = "web")))]
compile_error!("At lease one of the main features should be enabled !");

#[derive(Parser)]
#[command(name = "chat", about = "Interactive LLM", version)]
struct Args {
    #[arg(long, short = 'm')]
    model: Option<String>,

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
    let cfg = load_config().map_err(|e| {
        error!(exc = e.to_string(), "Error loading configuration");
        e
    })?;

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key)
        .with_api_base(cfg.base_url);

    // Configure a shared http client
    let reqwest_client = ReqwestClient::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Build the OpenAI client from parts
    let openai_client = OpenAiClient::with_config(oa_cfg).with_http_client(reqwest_client);

    // models database once
    let allowed_models = enriched_models_from_ids(
        list_models(&openai_client)
            .await?
            .into_iter()
            // handle locked model from command-line
            .filter(|id| args.model.as_ref().map_or(true, |lock_id| lock_id == id))
            .collect(),
        cfg.exclude_model_name_regex,
    )?;

    // Finally assemble the state and provide it
    let state = AppState {
        openai_client: Arc::new(openai_client),
        prepend_system_prompt: Arc::new(cfg.prepend_system_prompt),
        allowed_models: Arc::new(allowed_models),
    };

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli => {
            run_cli(state).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port } => {
            run_web(state, port).await?;
        }
    }
    Ok(())
}
