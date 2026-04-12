use anyhow::Result;
use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;

mod chat;
mod config;
mod display;
mod models;
mod tokens;

use config::{load_config, load_exclusion, load_mapping, save_exclusion};

#[derive(Parser)]
#[command(name = "chat", about = "Interactive LLM CLI", version)]
struct Args {
    #[arg(long, short = 'm')]
    model: Option<String>,
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

    let args = Args::parse();
    let config = load_config().map_err(|e| {
        display::log_error(&e.to_string());
        e
    })?;
    let mapping = load_mapping()?;
    let mut excl = load_exclusion()?;
    let filters = models::compile_regex(&config.exclude_model_name_regex)?;

    let client = Client::with_config(
        OpenAIConfig::new()
            .with_api_key(&config.api_key)
            .with_api_base(&config.base_url),
    );

    let mut forced: Option<String> = args.model;
    let mut model_cache: Option<Vec<models::EnrichedModel>> = None;

    loop {
        // ── Resolve which model to use ──────────────────────────────────────
        let (model, from_arg) = match forced.take() {
            Some(id) => match models::test_model(&client, &id).await {
                Ok(()) => {
                    if let Some(reason) = models::explain_rejection(&id, &mapping, &excl, &filters)
                    {
                        display::log_warning(&format!("Model '{id}' is {reason}"));
                        let m =
                            pick_from_list(&client, &mut model_cache, &mapping, &excl, &filters)
                                .await?;
                        (m, false)
                    } else {
                        display::log_info(&format!("Using model: {id}"));
                        (id, true)
                    }
                }
                Err(models::ModelError::NotAllowed) => {
                    display::log_warning(&format!("Model '{id}' not allowed → excluded"));
                    if !excl.excluded_models.contains(&id) {
                        excl.excluded_models.push(id);
                        save_exclusion(&excl)?;
                    }
                    let m = pick_from_list(&client, &mut model_cache, &mapping, &excl, &filters)
                        .await?;
                    (m, false)
                }
                Err(_) => {
                    display::log_error(&format!("Model '{id}' is unavailable or does not exist"));
                    let m = pick_from_list(&client, &mut model_cache, &mapping, &excl, &filters)
                        .await?;
                    (m, false)
                }
            },
            None => {
                let m =
                    pick_from_list(&client, &mut model_cache, &mapping, &excl, &filters).await?;
                (m, false)
            }
        };

        // ── Run chat session ────────────────────────────────────────────────
        let outcome =
            chat::run(&client, &model, model_cache.as_deref(), &mut excl, &config).await?;

        if let chat::ChatOutcome::ModelExcluded = outcome {
            save_exclusion(&excl)?;
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
    mapping: &config::Mapping,
    excl: &config::Exclusion,
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

    models::select_model(list)
}
