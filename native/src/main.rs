use anyhow::{Result, anyhow};
use async_openai::{Client, config::OpenAIConfig};
use clap::{Parser, Subcommand};
use portable::{Config, Exclusion, Mapping, ModelMeta};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

mod chat;
mod config;
mod display;
mod models;
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
    Cli,
    /// Web subcommand
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
        display::log_error(&e.to_string());
        e
    })?;
    let mapping = config::load_mapping()?;
    let exclusion = config::load_exclusion().unwrap_or_default();
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

    match &args.command {
        Commands::Cli => {
            cli(locked_model, client, mapping, exclusion, filters, cfg).await?;
        }
        Commands::Web { port } => {
            // wraps because shared between multiple request handlers within the web server
            let state = web::AppState {
                client: Arc::new(client),
                mapping: Arc::new(mapping),
                exclusion: Arc::new(RwLock::new(exclusion)), // RwLock handles mut
                filters: Arc::new(filters),
                system_prompt: Arc::new(cfg.prepend_system_prompt),
                locked_model: Arc::new(locked_model),
            };
            web(port, state).await?;
        }
    }
    Ok(())
}

async fn web(port: &u16, state: web::AppState) -> Result<()> {
    let app = web::router(state);
    let listen_addr = format!("localhost:{port}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    println!("Server listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn cli(
    locked_model: Option<String>,
    client: Client<OpenAIConfig>,
    mapping: HashMap<String, ModelMeta>,
    mut exclusion: Exclusion,
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
    let mut model_cache: Option<Vec<models::EnrichedModel>> = None;

    loop {
        // ── Resolve which model to use ──────────────────────────────────────
        let (model, from_arg) = match forced.take() {
            Some(id) => match models::test_model(&client, &id).await {
                Ok(()) => {
                    if let Some(reason) =
                        models::explain_rejection(&id, &mapping, &exclusion, &filters)
                    {
                        display::log_warning(&format!("Model '{id}' is {reason}"));
                        let m = pick_from_list(
                            &client,
                            &mut model_cache,
                            &mapping,
                            &exclusion,
                            &filters,
                        )
                        .await?;
                        (m, false)
                    } else {
                        display::log_info(&format!("Using model: {id}"));
                        (id, true)
                    }
                }
                Err(models::ModelError::NotAllowed) => {
                    display::log_warning(&format!("Model '{id}' not allowed → excluded"));
                    if !exclusion.excluded_models.contains(&id) {
                        exclusion.excluded_models.push(id);
                        config::save_exclusion(&exclusion)?;
                    }
                    let m =
                        pick_from_list(&client, &mut model_cache, &mapping, &exclusion, &filters)
                            .await?;
                    (m, false)
                }
                Err(_) => {
                    display::log_error(&format!("Model '{id}' is unavailable or does not exist"));
                    let m =
                        pick_from_list(&client, &mut model_cache, &mapping, &exclusion, &filters)
                            .await?;
                    (m, false)
                }
            },
            None => {
                let m = pick_from_list(&client, &mut model_cache, &mapping, &exclusion, &filters)
                    .await?;
                (m, false)
            }
        };

        // ── Run chat session ────────────────────────────────────────────────
        let outcome = chat::run(
            &client,
            &model,
            model_cache.as_deref(),
            &mut exclusion,
            &config,
        )
        .await?;

        if let chat::ChatOutcome::ModelExcluded = outcome {
            config::save_exclusion(&exclusion)?;
            model_cache = None; // Force a fresh listing next iteration.
        }

        // Matches Python: exit after the session when --model was the trigger.
        if from_arg {
            return Ok(());
        }
    }
}

/// Lazily populate `cache`, then run the interactive fuzzy model selector.
async fn pick_from_list(
    client: &Client<OpenAIConfig>,
    cache: &mut Option<Vec<models::EnrichedModel>>,
    mapping: &Mapping,
    excl: &Exclusion,
    filters: &[regex::Regex],
) -> Result<String> {
    if cache.is_none() {
        let raw = models::list_models(client).await?;
        *cache = Some(models::filter_and_sort(
            raw,
            mapping,
            &excl.excluded_models,
            filters,
        ));
    }

    let list = cache.as_ref().unwrap();
    if list.is_empty() {
        display::log_critical("No models available.");
        std::process::exit(1);
    }

    display::select_model(list)
}
