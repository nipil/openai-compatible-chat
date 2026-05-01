use dialoguer::FuzzySelect;
use reedline::{
    DefaultPrompt, DefaultPromptSegment, EditCommand, Emacs, KeyCode, KeyModifiers, Reedline,
    ReedlineEvent, Signal, ValidationResult, Validator, default_emacs_keybindings,
};
use thiserror::Error;
use tokio::task::JoinError;
use tracing::{debug, error, info, trace, warn};

use crate::cli::display::DisplayError;
use crate::models::{EnrichedModel, EnrichedModels};

#[derive(Error, Debug)]
pub enum PromptError {
    #[error("IO error {0}")]
    Input(#[from] std::io::Error),
    #[error("Thread join error : {0}")]
    Join(#[from] JoinError),
}

/// Validator: Enter always inserts a newline, never submits ---
struct MultilineValidator;

impl Validator for MultilineValidator {
    fn validate(&self, _line: &str) -> ValidationResult {
        // Returning Incomplete means reedline inserts a newline on Enter.
        // Submission only happens via the explicit Ctrl+Enter binding below.
        ValidationResult::Incomplete
    }
}

/// Build the editor with custom keybindings ---
fn build_editor() -> Reedline {
    let mut keybindings = default_emacs_keybindings();

    // Ctrl+Enter → submit (the only way to leave the editor)
    keybindings.add_binding(KeyModifiers::CONTROL, KeyCode::Enter, ReedlineEvent::Submit);

    // Ctrl+C → abort (gives Signal::CtrlC so the caller can handle it)
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('c'),
        ReedlineEvent::CtrlC,
    );

    Reedline::create()
        .with_validator(Box::new(MultilineValidator))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
}

/// Drops into the reedline editor, optionally pre-filled with `prefill`.
/// Returns the submitted text, or None on Ctrl+C / Ctrl+D.
pub(crate) fn read_multiline(
    prompt: &str,
    prefill: Option<&str>,
) -> Result<Option<String>, PromptError> {
    let mut editor = build_editor();

    // Pre-fill: run edit commands against the buffer before handing
    // control to the user. InsertString handles embedded '\n' correctly,
    // so multiline prefill works out of the box.
    if let Some(prefill) = prefill {
        debug!("reedline prefill : {prefill:?}");
        editor.run_edit_commands(&[EditCommand::InsertString(String::from(prefill))]);
    }

    // A simple two-segment prompt:
    //   first line
    //   continuation
    let prompt = DefaultPrompt::new(
        DefaultPromptSegment::Basic(prompt.to_string()),
        DefaultPromptSegment::Basic("· ".to_string()),
    );

    let res = editor.read_line(&prompt)?;

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
) -> Result<Option<EnrichedModel>, DisplayError> {
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

    let index = tokio::task::spawn_blocking({
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
    .await?
    .map_err(|e| DisplayError::SelectionFailed(format!("Selection failed: {e}")))?;

    let Some(index) = index else {
        return Ok(None); // no choice was made
    };

    // Look up the key from the index
    let Some(model_id) = choices.get(index) else {
        Err(DisplayError::SelectionFailed(format!(
            "Selection failed: {index}"
        )))?
    };

    // Look up the info from the id
    let Some(model_info) = models.get(model_id) else {
        Err(DisplayError::SelectionFailed(format!(
            "Selection failed: {model_id}"
        )))?
    };

    Ok(Some(EnrichedModel::new(
        model_id.into(),
        model_info.clone(),
    )))
}
