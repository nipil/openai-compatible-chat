use crate::models::{EnrichedModel, EnrichedModels};
use anyhow::{Result, anyhow};
use crossterm::{cursor, execute, terminal};
use dialoguer::FuzzySelect;
use std::{
    io::{Write, stdout},
    time::{Duration, Instant},
};
use termimad::{
    CompoundStyle, MadSkin, StyledChar,
    crossterm::style::{Attribute::*, Attributes, Color::*},
    gray,
};
use tracing::info;

const BULLET_CHAR: char = '●';
const HRULE_CHAR: char = '─';
const QUOTE_CHAR: char = '▐';
const SCROLLBAR_THUMB: char = '▐';
const SCROLLBAR_TRACK: char = '│';

// ── Live markdown display ─────────────────────────────────────────────────────

/// Streams partial markdown to the terminal with in-place re-rendering,
/// throttled to ≤10 fps. Falls back to plain passthrough when the
/// content exceeds 60 % of terminal height (same policy as the Python original).
pub struct LiveMarkdown {
    skin: MadSkin,
    lines_on_screen: u16,
    term_width: u16,
    term_height: u16,
    last_render: Instant,
    disabled: bool,
}

impl LiveMarkdown {
    pub fn new() -> Self {
        let (w, h) = terminal::size().unwrap_or((120, 40));
        Self {
            skin: make_skin(),
            lines_on_screen: 0,
            term_width: w,
            term_height: h,
            last_render: Instant::now() - Duration::from_secs(1),
            disabled: false,
        }
    }

    /// Call after every streamed chunk — internally throttled.
    pub fn update(&mut self, text: &str) {
        if self.disabled {
            return;
        }
        if self.last_render.elapsed() < Duration::from_millis(100) {
            return;
        }
        let _ = self.paint(text);
        self.last_render = Instant::now();
    }

    /// Force a final, unthrottled render and move the cursor past it.
    pub fn finish(&mut self, text: &str) {
        if !text.is_empty() {
            let _ = self.paint(text);
        }
        println!();
    }

    fn paint(&mut self, text: &str) -> std::io::Result<()> {
        let rendered = format!("{}", self.skin.term_text(text));
        let lines = count_visual_lines(&rendered, self.term_width);

        // Disable live updates once the block becomes tall.
        if !self.disabled && lines > (self.term_height as f32 * 0.6) as u16 {
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

        print!("{rendered}");
        stdout().flush()?;
        self.lines_on_screen = lines;
        Ok(())
    }
}

// gray(n) takes values 0–23 (0 = near-black, 23 = near-white).
// Adjust gray(2)/gray(3) for the code block background
// if it blends too much with your terminal's default.

/// Build a Rich-inspired MadSkin for dark terminals.
///
/// Rich colour mapping:
///   H1  → bold cyan          (Rich: "Markdown H1" — bright cyan)
///   H2  → bold magenta       (Rich: "Markdown H2" — magenta)
///   H3  → bold yellow        (Rich: "Markdown H3" — yellow)
///   H4+ → bold white
///   bold       → bright white bold
///   italic     → yellow italic     (Rich uses yellow for emphasis)
///   strikeout  → red crossed-out
///   inline code → green on dark    (Rich: green text, dark panel)
///   code block  → green on near-black
///   bullet     → cyan  BULLET_CHAR
///   quote mark → blue  QUOTE_CHAR  italic
///   horiz rule → cyan  HRULE_CHAR
///   table      → blue borders
///   scrollbar  → cyan / dark
pub fn make_skin() -> MadSkin {
    // imports are at the top of display.rs — nothing needed here

    let mut skin = MadSkin::default_dark();

    // ── Headers ───────────────────────────────────────────────────────────────
    // h1: bold cyan, underlined — mimics Rich's prominent header style
    skin.headers[0].compound_style =
        CompoundStyle::new(Some(Cyan), None, Attributes::from(Bold) | Underlined);
    // h2: bold magenta
    skin.headers[1].compound_style = CompoundStyle::new(Some(Magenta), None, Bold.into());
    // h3: bold yellow
    skin.headers[2].compound_style = CompoundStyle::new(Some(Yellow), None, Bold.into());
    // h4–h8: bold white (progressively less prominent, same colour)
    for h in &mut skin.headers[3..] {
        h.compound_style = CompoundStyle::new(Some(White), None, Bold.into());
    }

    // ── Inline styles ─────────────────────────────────────────────────────────
    // Bold: bright white — Rich renders **bold** as white on dark backgrounds
    skin.bold = CompoundStyle::new(Some(White), None, Bold.into());

    // Italic: yellow — Rich uses yellow/gold for *emphasis*
    skin.italic = CompoundStyle::new(Some(Yellow), None, Italic.into());

    // Strikeout: red crossed-out — Rich renders ~~struck~~ in red
    skin.strikeout = CompoundStyle::new(Some(Red), None, CrossedOut.into());

    // ── Code ─────────────────────────────────────────────────────────────────
    // Inline code: bold green on near-black — Rich's default code style
    skin.inline_code = CompoundStyle::new(Some(Green), Some(gray(2)), Bold.into());
    // Code block: same fg, slightly lighter dark background for contrast
    skin.code_block.compound_style =
        CompoundStyle::new(Some(Green), Some(gray(3)), Attributes::default());

    // ── Structural elements ───────────────────────────────────────────────────
    // Bullet: cyan filled circle — Rich uses cyan BULLET_CHAR markers
    skin.bullet = StyledChar::new(
        CompoundStyle::new(Some(Cyan), None, Bold.into()),
        BULLET_CHAR,
    );

    // Blockquote mark: blue italic bar — Rich renders quotes in blue/dim
    skin.quote_mark = StyledChar::new(
        CompoundStyle::new(Some(Blue), None, Italic.into()),
        QUOTE_CHAR,
    );

    // Horizontal rule: cyan dashes
    skin.horizontal_rule = StyledChar::new(
        CompoundStyle::new(Some(Cyan), None, Attributes::default()),
        HRULE_CHAR,
    );

    // Table borders: blue — Rich tables use blue/dim borders
    skin.table.compound_style = CompoundStyle::new(Some(Blue), None, Attributes::default());

    // Scrollbar: cyan thumb on dark track
    skin.scrollbar.thumb = StyledChar::new(
        CompoundStyle::new(Some(Cyan), None, Attributes::default()),
        SCROLLBAR_THUMB,
    );
    skin.scrollbar.track = StyledChar::new(
        CompoundStyle::new(Some(gray(6)), None, Attributes::default()),
        SCROLLBAR_TRACK,
    );

    skin
}

/// Count how many terminal rows `rendered` occupies, accounting for ANSI
/// escape sequences (stripped for measurement) and soft line-wrapping.
fn count_visual_lines(rendered: &str, term_width: u16) -> u16 {
    if term_width == 0 {
        return 0;
    }
    let stripped = console::strip_ansi_codes(rendered);
    stripped
        .lines()
        .map(|l| (l.chars().count() as u16).saturating_sub(1) / term_width + 1)
        .sum()
}

// ── Display / selection ───────────────────────────────────────────────────────

/// Opens an interactive fuzzy-search and returns the selected model ID.
pub fn select_model(models: &EnrichedModels) -> Result<Option<EnrichedModel<'_>>> {
    // Cancel immediately if nothing is available
    if models.is_empty() {
        return Ok(None);
    }

    // Autoselect if there is only one
    if models.len() == 1 {
        let Some((model_id, model_info)) = models.iter().next() else {
            return Err(anyhow!("No models available even though we had one."));
        };
        info!(model = model_id, "Auto-selected model");
        return Ok(Some(EnrichedModel::new(model_id, model_info)));
    }

    // Build sorted list of models
    let mut choices: Vec<&str> = models.keys().map(|k| k.as_str()).collect();
    choices.sort();

    // Choose (maybe) one of them
    let Some(index) = FuzzySelect::new()
        .with_prompt("Select model")
        .items(&choices)
        .default(0)
        .interact_opt()
        .map_err(|e| anyhow!("Selection failed: {e}"))?
    else {
        // no choice was done
        return Ok(None);
    };

    // Look up the key from the index
    let Some(model_id) = choices.get(index) else {
        return Err(anyhow!(
            "No models id for choice number, even though we had one."
        ));
    };

    // Look up the info from the id
    let Some(model_info) = models.get(*model_id) else {
        return Err(anyhow!(
            "No models id for choice number, even though we had one."
        ));
    };

    return Ok(Some(EnrichedModel::new(model_id, model_info)));
}
