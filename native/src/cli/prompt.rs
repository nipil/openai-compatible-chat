use dialoguer::FuzzySelect;
use nu_ansi_term::Style;
use portable::{MessageRole, TokenUsage};
use reedline::{EditCommand, Signal};
use thiserror::Error;
use tokio::task::JoinError;
use tracing::{debug, error, info, trace, warn};

use crate::cli::reedline::{AppPrompt, build_reedline, crossterm_to_nu};
use crate::models::{EnrichedModel, EnrichedModels};

#[derive(Error, Debug)]
pub enum PromptError {
    #[error("IO error {0}")]
    Input(#[from] std::io::Error),

    #[error("Thread join error : {0}")]
    Join(#[from] JoinError),

    #[error("Fuzzy prompt failed : {0}")]
    Fuzzy(#[from] dialoguer::Error),

    #[error("Selection failed : {0}")]
    SelectionFailed(String),
}

// Wrap it in Arc<RwLock<>> so the editor loop can mutate it and the prompt
// can read it from &self without needing &mut.

/// Live state that can change between readline calls.
pub(crate) struct PromptState {
    pub(crate) selected_model: EnrichedModel,
    pub(crate) current_role: MessageRole,
    pub(crate) token_usage: TokenUsage,
}

impl PromptState {
    pub(crate) fn new(model: EnrichedModel) -> Self {
        Self {
            selected_model: model,
            current_role: MessageRole::System,
            token_usage: TokenUsage::default(),
        }
    }
}

/// Drops into the reedline editor, optionally pre-filled with `prefill`.
/// Returns the submitted text, or None on Ctrl+C / Ctrl+D.
pub(crate) async fn read_multiline(
    prompt: AppPrompt,
    prefill: Option<&str>,
) -> Result<Option<String>, PromptError> {
    let mut editor = build_reedline(Style::new().fg(crossterm_to_nu(prompt.colors.as_ref().meta))); // ghost text = meta color

    // Pre-fill: run edit commands against the buffer before handing
    // control to the user. InsertString handles embedded '\n' correctly,
    // so multiline prefill works out of the box.
    if let Some(prefill) = prefill {
        debug!("reedline prefill : {prefill:?}");
        editor.run_edit_commands(&[EditCommand::InsertString(String::from(prefill))]);
    }

    // prompt the user while not blocking the thread
    let res = non_blocking(move || editor.read_line(&prompt)).await??;

    let res = match res {
        Signal::Success(buf) => {
            trace!("reedline success: {buf:?}");
            Some(buf)
        }

        Signal::CtrlC => {
            trace!("reedline Ctrl-C");
            None
        }

        _ => {
            warn!("reedline non-exhaustive {res:?}");
            None
        }
    };

    Ok(res)
}

/// Opens an interactive fuzzy-search and returns the selected model ID.
pub(crate) async fn select_model(
    models: &EnrichedModels,
) -> Result<Option<EnrichedModel>, PromptError> {
    termimad::print_text("\n---\n");

    // Handle the simple cases
    let Some((model_id, model_info)) = models.iter().next() else {
        error!("No model available for selection");
        return Ok(None);
    };

    if models.len() == 1 {
        info!(model = model_id, "Auto-selected model");
        return Ok(Some(EnrichedModel::new(
            model_id.into(),
            model_info.clone(),
        )));
    }

    // Build sorted list of models and let user choose
    let mut choices: Vec<String> = models.keys().map(|k| k.clone()).collect();
    choices.sort();

    let index = non_blocking({
        // so that choices_dup can move yet choices stay available
        let choices_dup = choices.clone();

        move || {
            // The theme in fuzzyselect is not send+'static
            FuzzySelect::new()
                .with_prompt("Select model")
                .items(choices_dup)
                .default(0)
                .interact_opt()
        }
    })
    .await??;

    let Some(index) = index else {
        // When pressing escape, fuzzyselect returns Ok(None)
        // When pressing ctrl-c, fuzzyselect does NOT handle it, and the process dies
        return Ok(None); // no choice was made
    };

    // Look up the key from the index
    let Some(model_id) = choices.get(index) else {
        Err(PromptError::SelectionFailed(format!(
            "Selection failed: {index}"
        )))?
    };

    // Look up the info from the id
    let Some(model_info) = models.get(model_id) else {
        Err(PromptError::SelectionFailed(format!(
            "Selection failed: {model_id}"
        )))?
    };

    Ok(Some(EnrichedModel::new(
        model_id.into(),
        model_info.clone(),
    )))
}

/// Factored function to not block the main thread while reading the inputs
pub(crate) async fn non_blocking<F, T>(f: F) -> Result<T, PromptError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Ok(tokio::task::spawn_blocking(f).await?)
}
