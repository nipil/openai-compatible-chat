use reedline::{
    DefaultPrompt, DefaultPromptSegment, EditCommand, Emacs, KeyCode, KeyModifiers, Reedline,
    ReedlineEvent, Signal, ValidationResult, Validator, default_emacs_keybindings,
};
use thiserror::Error;
use tracing::{debug, trace, warn};

#[derive(Error, Debug)]
pub enum PromptError {
    #[error("IO error {0}")]
    Input(#[from] std::io::Error),
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
