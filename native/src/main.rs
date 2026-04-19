use crate::display::{log_critical, log_warning};
use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use clap::{Parser, Subcommand};
use portable::EnrichedModel;
use std::sync::Arc;

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

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key)
        .with_api_base(cfg.base_url);
    let client = Client::with_config(oa_cfg);

    // Build models database once
    let allowed_models = models::enriched_models_from_ids(
        models::list_models(&client)
            .await?
            .into_iter()
            // handle locked model from command-line
            .filter(|id| args.model.as_ref().map_or(true, |lock_id| lock_id == id))
            .collect(),
        // TODO: compile regex using serde
        models::compile_regex(cfg.exclude_model_name_regex)?,
    )?;

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli => {
            cli(client, allowed_models, cfg.prepend_system_prompt).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port } => {
            web(client, allowed_models, port, cfg.prepend_system_prompt).await?;
        }
    }
    Ok(())
}

#[cfg(feature = "web")]
async fn web(
    client: Client<OpenAIConfig>,
    allowed_models: Vec<EnrichedModel>,
    port: &u16,
    prepend_system_prompt: String,
) -> Result<()> {
    let state = web::AppState {
        client: Arc::new(client),
        prepend_system_prompt: Arc::new(prepend_system_prompt),
        allowed_models: Arc::new(allowed_models),
    };
    let app = web::router(state);
    let listen_addr = format!("localhost:{port}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    println!("Server listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(feature = "cli")]
async fn cli(
    client: Client<OpenAIConfig>,
    allowed_models: Vec<EnrichedModel>,
    prepend_system_prompt: String,
) -> Result<()> {
    // Enable ANSI colour codes on legacy Windows consoles (cmd.exe).
    // No-op on Windows 10+ / modern terminals / all Unix systems.
    // TODO: move to main
    #[cfg(windows)]
    enable_ansi_support::enable_ansi_support().ok();

    // Clean Ctrl-C exit from anywhere in the program.
    // TODO: move to main
    tokio::spawn(async {
        tokio::signal::ctrl_c().await.ok();
        eprintln!("\nExiting.");
        std::process::exit(0);
    });

    loop {
        // ── Run chat session ────────────────────────────────────────────────
        let selected_index = display::select_model(&allowed_models)?; // TODO: test with an result<option> here
        match chat::run(
            &client,
            &allowed_models[selected_index],
            &prepend_system_prompt,
        )
        .await?
        {
            chat::ChatOutcome::ChatEnded => {
                display::log_info("Chat ended.");
                continue;
            }
            chat::ChatOutcome::ContextLimitReached => {
                log_warning("Context limit reached — starting a new conversation.");
                continue;
            }
            chat::ChatOutcome::ExitRequested => {
                display::log_info("Requested to quit");
                return Ok(());
            }
            chat::ChatOutcome::ModelForbidden => {
                if allowed_models.len() > 1 {
                    display::log_warning("Model is forbidden, choose another one.");
                    continue;
                } else {
                    log_critical("The only available model is forbidden, exiting.");
                    return Err(anyhow!("No more model available to use."));
                }
            }
        }
    }
}
