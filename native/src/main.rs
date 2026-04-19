use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use clap::{Parser, Subcommand};
use portable::{Config, EnrichedModel, ModelInfo, ModelInfoMap};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[cfg(all(not(feature = "cli"), not(feature = "web")))]
compile_error!("At lease one of the main features should be enabled !");

#[cfg(feature = "cli")]
mod chat;
mod config;
#[cfg(feature = "cli")]
mod display;
mod models;
#[cfg(feature = "web")]
mod web;

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
    let args = Args::parse();

    // Load configuration once
    let cfg = config::load_config().map_err(|e| {
        #[cfg(feature = "cli")]
        display::log_error(&e.to_string());
        e
    })?;
    let mapping = config::load_model_info_map()?;
    let filters = models::compile_regex(&cfg.exclude_model_name_regex)?;

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(&cfg.api_key)
        .with_api_base(&cfg.base_url);
    let client = Client::with_config(oa_cfg);

    // check model arg for validity once
    let locked_model = match args.model {
        Some(ref m) => match models::test_model(&client, m).await {
            Ok(()) => args.model.clone(),
            Err(models::ModelError::Network(e)) => return Err(anyhow!("Network: {e}")),
            Err(_) => {
                eprintln!("Warning: model '{m}' unavailable, ignoring lock");
                None
            }
        },
        None => None,
    };

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli => {
            cli(locked_model, client, mapping, filters, cfg).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port } => {
            // wraps because shared between multiple request handlers within the web server
            let state = web::AppState {
                client: Arc::new(client),
                infos: Arc::new(mapping),
                filters: Arc::new(filters),
                system_prompt: Arc::new(cfg.prepend_system_prompt),
                locked_model: Arc::new(locked_model),
            };
            web(port, state).await?;
        }
    }
    Ok(())
}

#[cfg(feature = "web")]
async fn web(port: &u16, state: web::AppState) -> Result<()> {
    let app = web::router(state);
    let listen_addr = format!("localhost:{port}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    println!("Server listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(feature = "cli")]
async fn cli(
    locked_model: Option<String>,
    client: Client<OpenAIConfig>,
    mapping: HashMap<String, ModelInfo>,
    filters: Vec<regex::Regex>,
    config: Config,
) -> Result<()> {
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

    let mut forced: Option<String> = locked_model;

    // Fetch models once per run
    // TODO: move to main ?
    let enriched_models =
        models::filter_and_sort(models::list_models(&client).await?, &mapping, &filters);

    loop {
        // ── Resolve which model to use ──────────────────────────────────────
        let (model, from_arg) = match forced.take() {
            Some(id) => match models::test_model(&client, &id).await {
                Ok(()) => {
                    if let Some(reason) = models::explain_rejection(&id, &mapping, &filters) {
                        display::log_warning(&format!("Model '{id}' is {reason}"));
                        let m = display::select_model(&enriched_models)?;
                        (m, false)
                    } else {
                        display::log_info(&format!("Using model: {id}"));
                        (id, true)
                    }
                }
                Err(models::ModelError::NotAllowed) => {
                    // TODO: deduplicate from chat.rs:88
                    display::log_warning(&format!("Model '{id}' not allowed"));
                    (display::select_model(&enriched_models)?, false)
                }
                Err(_) => {
                    display::log_error(&format!("Model '{id}' is unavailable or does not exist"));
                    (display::select_model(&enriched_models)?, false)
                }
            },
            None => (display::select_model(&enriched_models)?, false),
        };

        // ── Run chat session ────────────────────────────────────────────────
        let outcome = chat::run(&client, &model, &enriched_models, &config).await?; // TODO: maybe more error once we stop swallowing them

        if let chat::ChatOutcome::ModelForbidden = outcome {}

        // Matches Python: exit after the session when --model was the trigger.
        if from_arg {
            return Ok(());
        }
    }
}
