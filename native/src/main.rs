use anyhow::Result;
use async_openai::{Client, config::OpenAIConfig};
use clap::{Parser, Subcommand};
use native::cli::display::log_error;
use native::cli::run_cli;
use native::config::load_config;
use native::models::{enriched_models_from_ids, list_models};
use native::web::run_web;

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

    // Load configuration once
    let cfg = load_config().map_err(|e| {
        #[cfg(feature = "cli")]
        log_error(&e.to_string());
        e
    })?;

    // Create shared components
    let oa_cfg = OpenAIConfig::new()
        .with_api_key(cfg.api_key)
        .with_api_base(cfg.base_url);
    let client = Client::with_config(oa_cfg);

    // Build models database once
    let allowed_models = enriched_models_from_ids(
        list_models(&client)
            .await?
            .into_iter()
            // handle locked model from command-line
            .filter(|id| args.model.as_ref().map_or(true, |lock_id| lock_id == id))
            .collect(),
        cfg.exclude_model_name_regex,
    )?;

    #[cfg(all(feature = "cli", feature = "web"))]
    match &args.command {
        #[cfg(feature = "cli")]
        Commands::Cli => {
            run_cli(client, allowed_models, cfg.prepend_system_prompt).await?;
        }
        #[cfg(feature = "web")]
        Commands::Web { port } => {
            run_web(client, allowed_models, port, cfg.prepend_system_prompt).await?;
        }
    }
    Ok(())
}
