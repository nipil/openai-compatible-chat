// Helpers to go from crossterm::style::Color → the two formats reedline needs.

use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use chrono::Local;
use crossterm::style::Color as CrosstermColor;
use nu_ansi_term::{Color as NuColor, Style};
use portable::Theme;
use reedline::{
    DefaultHinter, Emacs, KeyCode, KeyModifiers, Prompt, PromptEditMode, PromptHistorySearch,
    PromptHistorySearchStatus, Reedline, ReedlineEvent, ValidationResult, Validator,
    default_emacs_keybindings,
};

use crate::cli::prompt::PromptState;
use crate::cli::themes::ConsoleColors;

pub(crate) const ANSI_COLOR_RESET: &str = "\x1b[0m";
const LEFT_PROMPT_END: &str = "\u{276F}"; // the good looking unicode '>'
const RIGHT_PROMPT_END: &str = "\u{276E}"; // the good looking unicode '<'

/// Produces a foreground ANSI escape string from a crossterm Color.
/// Use this inside `render_prompt_*` methods that return `Cow<str>`.
fn crossterm_to_ansi_foreground(crossterm_color: CrosstermColor) -> String {
    match crossterm_color {
        CrosstermColor::Black => "\x1b[30m".into(),
        CrosstermColor::DarkGrey => "\x1b[90m".into(),
        CrosstermColor::Red => "\x1b[91m".into(),
        CrosstermColor::DarkRed => "\x1b[31m".into(),
        CrosstermColor::Green => "\x1b[92m".into(),
        CrosstermColor::DarkGreen => "\x1b[32m".into(),
        CrosstermColor::Yellow => "\x1b[93m".into(),
        CrosstermColor::DarkYellow => "\x1b[33m".into(),
        CrosstermColor::Blue => "\x1b[94m".into(),
        CrosstermColor::DarkBlue => "\x1b[34m".into(),
        CrosstermColor::Magenta => "\x1b[95m".into(),
        CrosstermColor::DarkMagenta => "\x1b[35m".into(),
        CrosstermColor::Cyan => "\x1b[96m".into(),
        CrosstermColor::DarkCyan => "\x1b[36m".into(),
        CrosstermColor::White => "\x1b[97m".into(),
        CrosstermColor::Grey => "\x1b[37m".into(),
        CrosstermColor::Reset => "\x1b[39m".into(),
        CrosstermColor::AnsiValue(n) => format!("\x1b[38;5;{n}m"),
        CrosstermColor::Rgb { r, g, b } => format!("\x1b[38;2;{r};{g};{b}m"),
    }
}

/// Converts a crossterm Color to nu_ansi_term::Color.
/// Use this for DefaultHinter / Highlighter, which require nu_ansi_term styling.
pub(crate) fn crossterm_to_nu(crossterm_color: CrosstermColor) -> nu_ansi_term::Color {
    match crossterm_color {
        CrosstermColor::Black => NuColor::Black,
        CrosstermColor::DarkGrey => NuColor::DarkGray,
        CrosstermColor::Red => NuColor::LightRed,
        CrosstermColor::DarkRed => NuColor::Red,
        CrosstermColor::Green => NuColor::LightGreen,
        CrosstermColor::DarkGreen => NuColor::Green,
        CrosstermColor::Yellow => NuColor::LightYellow,
        CrosstermColor::DarkYellow => NuColor::Yellow,
        CrosstermColor::Blue => NuColor::LightBlue,
        CrosstermColor::DarkBlue => NuColor::Blue,
        CrosstermColor::Magenta => NuColor::LightMagenta,
        CrosstermColor::DarkMagenta => NuColor::Purple,
        CrosstermColor::Cyan => NuColor::LightCyan,
        CrosstermColor::DarkCyan => NuColor::Cyan,
        CrosstermColor::White => NuColor::White,
        CrosstermColor::Grey => NuColor::LightGray,
        CrosstermColor::AnsiValue(n) => NuColor::Fixed(n),
        CrosstermColor::Rgb { r, g, b } => NuColor::Rgb(r, g, b),
        CrosstermColor::Reset => NuColor::Default,
    }
}

// Regroup updatable prompt state and the immutable theme enum and crossterm colors
#[derive(Clone)]
pub(crate) struct AppPrompt {
    pub(crate) colors: Arc<ConsoleColors>,
    pub(crate) refresh_ms: Arc<u64>,
    pub(crate) state: Arc<RwLock<PromptState>>,
    pub(crate) theme: Arc<Theme>,
}

impl Prompt for AppPrompt {
    // Left segment
    fn render_prompt_left(&'_ self) -> Cow<'_, str> {
        // guard is dropped immediately
        let current_role = self
            .state
            .read()
            .expect("Prompt `state` RwLock should not be poisonned")
            .current_role
            .clone();

        Cow::Owned(format!(
            "{timestamp_color}{timestamp}{reset} {role_color}{role}{reset} ",
            reset = ANSI_COLOR_RESET,
            role = current_role,
            role_color = crossterm_to_ansi_foreground(self.colors.token_warn),
            timestamp = Local::now().format("%H:%M:%S").to_string(),
            timestamp_color = crossterm_to_ansi_foreground(self.colors.timestamp),
        ))
    }

    // Right segment
    fn render_prompt_right(&'_ self) -> Cow<'_, str> {
        // guard is dropped immediately on block exit
        let (model_id, current_tokens, context_window) = {
            let state = self
                .state
                .read()
                .expect("Prompt `state` RwLock should not be poisonned");
            let model_id = state.selected_model.id.clone();
            let context_window = state.selected_model.info.context_window;
            let current_tokens = u32::from(&state.token_usage);
            (model_id, current_tokens, context_window)
        };

        // Picks the right token color from the theme based on current usage
        let tok_color = match context_window {
            None => self.colors.token_medium,
            Some(context_window) => {
                let ratio = current_tokens as f32 / context_window.max(1) as f32;
                if ratio < 0.50 {
                    self.colors.token_low
                } else if ratio < 0.75 {
                    self.colors.token_medium
                } else if ratio < 0.90 {
                    self.colors.token_warn
                } else {
                    self.colors.token_critical
                }
            }
        };

        Cow::Owned(format!(
            "{chrome}{indicator} {token_color}{tokens} tok{reset} {tag_color}[{model_color}{model_id}{tag_color}]{reset}",
            chrome = crossterm_to_ansi_foreground(self.colors.chrome),
            indicator = RIGHT_PROMPT_END,
            model_color = crossterm_to_ansi_foreground(self.colors.model_name),
            model_id = model_id,
            reset = ANSI_COLOR_RESET,
            tag_color = crossterm_to_ansi_foreground(self.colors.tag),
            token_color = crossterm_to_ansi_foreground(tok_color),
            tokens = match current_tokens {
                ..1_000 => format!("{}", current_tokens),
                1_000..10_000 => format!("{:.1}k", current_tokens as f32 / 1_000 as f32),
                10_000.. => format!("{}k", current_tokens / 1_000),
            }
        ))
    }

    // The blinking ❯ / cursor indicator, styled with the accent color
    fn render_prompt_indicator(&'_ self, edit_mode: PromptEditMode) -> Cow<'_, str> {
        let glyph = match edit_mode {
            PromptEditMode::Default | PromptEditMode::Emacs => LEFT_PROMPT_END,
            PromptEditMode::Vi(vi_mode) => match vi_mode {
                reedline::PromptViMode::Normal => "·",
                reedline::PromptViMode::Insert => LEFT_PROMPT_END,
            },
            PromptEditMode::Custom(_) => LEFT_PROMPT_END,
        };
        Cow::Owned(format!(
            "{}{}{} ",
            crossterm_to_ansi_foreground(self.colors.accent),
            glyph,
            ANSI_COLOR_RESET
        ))
    }

    fn render_prompt_multiline_indicator(&'_ self) -> Cow<'_, str> {
        Cow::Owned(format!(
            "{}╰❯{} ",
            crossterm_to_ansi_foreground(self.colors.chrome),
            ANSI_COLOR_RESET
        ))
    }

    fn render_prompt_history_search_indicator(
        &'_ self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let indicator = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };

        Cow::Owned(format!(
            "{}(search: {}{}{}){} ",
            crossterm_to_ansi_foreground(self.colors.chrome),
            crossterm_to_ansi_foreground(self.colors.meta),
            indicator,
            history_search.term,
            ANSI_COLOR_RESET,
        ))
    }
}

/// Build the editor with custom keybindings ---
pub(crate) fn build_reedline(hinter_style: Style) -> Reedline {
    let mut keybindings = default_emacs_keybindings();

    // Ctrl+D → submit (standard Unix EOF, works everywhere)
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('d'),
        ReedlineEvent::Submit,
    );

    // Ctrl+C → abort (gives Signal::CtrlC so the caller can handle it)
    keybindings.add_binding(
        KeyModifiers::CONTROL,
        KeyCode::Char('c'),
        ReedlineEvent::CtrlC,
    );

    // create a reedline editor
    Reedline::create()
        .with_hinter(Box::new(DefaultHinter::default().with_style(hinter_style)))
        .with_validator(Box::new(MultilineValidator))
        .with_edit_mode(Box::new(Emacs::new(keybindings)))
}

/// Validator: Enter always inserts a newline, never submits ---
pub(crate) struct MultilineValidator;

impl Validator for MultilineValidator {
    fn validate(&self, _line: &str) -> ValidationResult {
        // Returning Incomplete means reedline inserts a newline on Enter.
        // Submission only happens via the explicit Ctrl+Enter binding below.
        ValidationResult::Incomplete
    }
}
