use std::io::{Write, stdout};
use std::time::{Duration, Instant};

use chrono::Local;
use crossterm::style::Stylize as _; // .with(color), .bold(), .italic()
use crossterm::{cursor, execute, terminal};
use portable::{Theme, TokenUsage};
use termimad::crossterm::style::Attribute::*; // Bold, Italic, CrossedOut, Underlined
use termimad::crossterm::style::Color::*;
use termimad::crossterm::style::{Attributes, Color};
use termimad::{CompoundStyle, MadSkin, StyledChar, gray};
use thiserror::Error;
use tracing::warn;

use crate::models::EnrichedModel;

// ── Decoration characters ─────────────────────────────────────────────────────

const BULLET_CHAR: char = '●';
const HRULE_CHAR: char = '─';
const QUOTE_CHAR: char = '▐';
const SCROLLBAR_THUMB: char = '▐';
const SCROLLBAR_TRACK: char = '│';

// ── Live markdown ─────────────────────────────────────────────────────────────

const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const LIVE_UPDATE_HEIGHT_PERCENT: f32 = 0.6;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum DisplayError {
    #[error("Model not found : {0}")]
    ModelNotFound(String),
}

// ── Theme ─────────────────────────────────────────────────────────────────────

/// Every colour decision lives here.  The rest of the module refers only to
/// semantic role names — never to raw colour literals.
pub struct ConsoleTheme {
    // ── Markdown headings ─────────────────────────────────────────────────────
    /// H1 — most prominent heading
    pub heading_1: Color,
    /// H2
    pub heading_2: Color,
    /// H3
    pub heading_3: Color,
    /// H4 and below — progressively less prominent, same colour
    pub heading_n: Color,

    // ── Inline text styles ────────────────────────────────────────────────────
    /// **bold** spans
    pub strong: Color,
    /// *italic* / emphasis spans
    pub emphasis: Color,
    /// ~~strikethrough~~ / deleted text
    pub deleted: Color,

    // ── Code ─────────────────────────────────────────────────────────────────
    /// Foreground for both inline `code` and fenced code blocks
    pub code: Color,

    // ── Structural / decorative markdown elements ─────────────────────────────
    /// Bullet markers, horizontal rules, scrollbar thumb
    pub accent: Color,
    /// Blockquote bar, table borders
    pub border: Color,

    // ── Shell chrome ──────────────────────────────────────────────────────────
    /// Static banner decorators ("───", "description:", …)
    pub chrome: Color,
    /// Model identifier — shown in banner and prompt tag
    pub model_name: Color,
    /// Supplementary model info (description, family, release)
    pub meta: Color,
    /// [HH:MM:SS] timestamp in the user prompt
    pub timestamp: Color,
    /// Secondary bracketed tag, e.g. [model-id]
    pub tag: Color,
    /// Elapsed-time readout
    pub duration: Color,

    // ── Token-usage thresholds ────────────────────────────────────────────────
    /// < 50 % — unobtrusive / de-emphasised
    pub token_low: Color,
    /// 50 – 75 % — neutral
    pub token_medium: Color,
    /// 75 – 90 % — approaching limit
    pub token_warn: Color,
    /// ≥ 90 % — critical
    pub token_critical: Color,
}

impl ConsoleTheme {
    /// Build a console theme from a them enum
    pub fn new(theme: &Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::dark(), // FIXME: update when light is done
        }
    }

    /// Rich-inspired dark-terminal theme (the only built-in theme for now).
    fn dark() -> Self {
        Self {
            heading_1: Cyan,
            heading_2: Magenta,
            heading_3: Yellow,
            heading_n: White,

            strong: White,
            emphasis: Yellow,
            deleted: Red,

            code: Green,

            accent: Cyan,
            border: Blue,

            chrome: White,
            model_name: Cyan,
            meta: White,
            timestamp: White,
            tag: DarkGrey,
            duration: DarkGrey,

            token_low: DarkGrey,
            token_medium: White,
            token_warn: Yellow,
            token_critical: Red,
        }
    }
}

/// Streams partial markdown to the terminal with in-place re-rendering.
pub(crate) struct LiveMarkdown {
    skin: MadSkin,
    lines_on_screen: u16,
    term_width: u16,
    term_height: u16,
    last_render: Instant,
    disabled: bool,
}

impl LiveMarkdown {
    pub(crate) fn new(theme: &ConsoleTheme) -> Self {
        let (w, h) = terminal::size().unwrap_or((120, 40));
        Self {
            skin: make_skin(theme),
            lines_on_screen: 0,
            term_width: w,
            term_height: h,
            last_render: Instant::now() - Duration::from_secs(1),
            disabled: false,
        }
    }

    /// Call after every streamed chunk — throttled to `REFRESH_INTERVAL`.
    pub(crate) fn update(&mut self, text: &str) {
        if self.disabled {
            return;
        }
        if self.last_render.elapsed() < REFRESH_INTERVAL {
            return;
        }
        if let Err(e) = self.paint(text) {
            warn!("Failed to paint : {:?}", e);
        }
        self.last_render = Instant::now();
    }

    /// Force a final, unthrottled render and move the cursor past it.
    pub(crate) fn finish(&mut self, text: &str) {
        if !text.is_empty() {
            if let Err(e) = self.paint(text) {
                warn!("Failed to paint : {:?}", e);
            }
        }
        println!();
    }

    /// Erase the previous render and redraw.
    /// Disables live updates once content exceeds `LIVE_UPDATE_HEIGHT_PERCENT`
    /// of the terminal height to avoid thrashing on very long responses.
    fn paint(&mut self, text: &str) -> std::io::Result<()> {
        let rendered = format!("{}", self.skin.term_text(text));
        let lines = count_visual_lines(&rendered, self.term_width);

        // Disable live updates once the block becomes tall.
        let threshold = (self.term_height as f32 * LIVE_UPDATE_HEIGHT_PERCENT) as u16;
        if !self.disabled && lines > threshold {
            self.disabled = true;
            // Flush whatever we have left as plain text so nothing is lost.
            if self.lines_on_screen == 0 {
                print!("{rendered}");
                stdout().flush()?;
            }
            return Ok(());
        }

        // Erase the previous render.
        if self.lines_on_screen > 0 {
            execute!(
                stdout(),
                cursor::MoveUp(self.lines_on_screen),
                terminal::Clear(terminal::ClearType::FromCursorDown),
            )?;
        }

        // Print the content
        print!("{rendered}");
        stdout().flush()?;
        self.lines_on_screen = lines;
        Ok(())
    }
}

// ── Markdown skin ─────────────────────────────────────────────────────────────

/// Build a `MadSkin` driven entirely by the provided `Theme`.
fn make_skin(theme: &ConsoleTheme) -> MadSkin {
    let mut skin = MadSkin::default_dark();

    // Headings (h1, h2, h3, and h4-h8)
    skin.headers[0].compound_style = CompoundStyle::new(
        Some(theme.heading_1),
        None,
        Attributes::from(Bold) | Underlined,
    );
    skin.headers[1].compound_style = CompoundStyle::new(Some(theme.heading_2), None, Bold.into());
    skin.headers[2].compound_style = CompoundStyle::new(Some(theme.heading_3), None, Bold.into());
    for h in &mut skin.headers[3..] {
        h.compound_style = CompoundStyle::new(Some(theme.heading_n), None, Bold.into());
    }

    // Inline text styles
    skin.bold = CompoundStyle::new(Some(theme.strong), None, Bold.into());
    skin.italic = CompoundStyle::new(Some(theme.emphasis), None, Italic.into());
    skin.strikeout = CompoundStyle::new(Some(theme.deleted), None, CrossedOut.into());

    // Code  (gray(n) : 0 = near-black … 23 = near-white)
    skin.inline_code = CompoundStyle::new(Some(theme.code), Some(gray(2)), Bold.into());

    // Code block: same fg, slightly lighter dark background for contrast
    skin.code_block.compound_style =
        CompoundStyle::new(Some(theme.code), Some(gray(3)), Attributes::default());

    // Structural / decorative
    skin.bullet = StyledChar::new(
        CompoundStyle::new(Some(theme.accent), None, Bold.into()),
        BULLET_CHAR,
    );
    skin.quote_mark = StyledChar::new(
        CompoundStyle::new(Some(theme.border), None, Italic.into()),
        QUOTE_CHAR,
    );
    skin.horizontal_rule = StyledChar::new(
        CompoundStyle::new(Some(theme.accent), None, Attributes::default()),
        HRULE_CHAR,
    );
    skin.table.compound_style = CompoundStyle::new(Some(theme.border), None, Attributes::default());
    skin.scrollbar.thumb = StyledChar::new(
        CompoundStyle::new(Some(theme.accent), None, Attributes::default()),
        SCROLLBAR_THUMB,
    );
    skin.scrollbar.track = StyledChar::new(
        CompoundStyle::new(Some(gray(6)), None, Attributes::default()),
        SCROLLBAR_TRACK,
    );

    skin
}

// ── Live markdown display ─────────────────────────────────────────────────────

/// Count how many terminal rows `rendered` occupies, accounting for ANSI
/// escape sequences (stripped for measurement) and soft line-wrapping.
fn count_visual_lines(rendered: &str, term_width: u16) -> u16 {
    if term_width == 0 {
        return 0;
    }
    console::strip_ansi_codes(rendered)
        .lines()
        .map(|l| (l.chars().count() as u16).saturating_sub(1) / term_width + 1)
        .sum()
}

// ── Shell chrome ──────────────────────────────────────────────────────────────

pub(crate) fn print_banner(selected_model: &EnrichedModel, theme: &ConsoleTheme) {
    // mandatory content
    println!(
        "\n{} {} {}\n",
        "─── Conversation using".with(theme.chrome).bold(),
        selected_model.id.as_str().with(theme.model_name).bold(),
        "───".with(theme.chrome).bold(),
    );

    // optional content
    let desc = selected_model.info.description.trim();
    if !desc.is_empty() {
        println!("description: {}", desc.with(theme.meta).italic());
    }

    let family = selected_model.info.family.trim();
    if !family.is_empty() {
        println!("family: {}", family.with(theme.meta).italic());
    }

    if let Some(ref release) = selected_model.info.release {
        let release = release.trim();
        if !release.is_empty() {
            println!("release: {}", release.with(theme.model_name).bold());
        }
    }
}

pub(crate) fn build_user_prompt(
    model: &str,
    tokens: &TokenUsage,
    max: Option<u32>,
    theme: &ConsoleTheme,
) -> String {
    let time = Local::now().format("%H:%M:%S").to_string();

    let tok_color = match max {
        None => theme.token_medium,
        Some(m) => {
            let ratio = u32::from(tokens) as f64 / m as f64;
            if ratio < 0.50 {
                theme.token_low
            } else if ratio < 0.75 {
                theme.token_medium
            } else if ratio < 0.90 {
                theme.token_warn
            } else {
                theme.token_critical
            }
        }
    };

    format!(
        "{}{}{}",
        format!("[{time}]").with(theme.timestamp),
        format!("[{model}]").with(theme.tag),
        format!("[{tokens}]").with(tok_color),
    )
}

/// Returns a styled elapsed-time string, e.g. `[1.23s]`.
pub(crate) fn get_duration(start: Instant, theme: &ConsoleTheme) -> String {
    format!("[{:.2}s]", start.elapsed().as_secs_f64())
        .with(theme.duration)
        .to_string()
}
