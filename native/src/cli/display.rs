use std::io::stdout;
use std::time::{Duration, Instant};

use crossterm::style::{self, Stylize as _}; // .with(color), .bold(), .italic()
use crossterm::{cursor, execute, terminal};
use portable::Theme;
use termimad::crossterm::style::Attribute::*; // Bold, Italic, CrossedOut, Underlined
use termimad::crossterm::style::{Attributes, Color};
use termimad::{CompoundStyle, MadSkin, StyledChar, gray};
use thiserror::Error;
use tracing::warn;
use unicode_width::UnicodeWidthStr;

use crate::cli::themes::ConsoleColors;
use crate::models::EnrichedModel;

// ── Decoration characters ─────────────────────────────────────────────────────

const BULLET_CHAR: char = '●';
const HRULE_CHAR: char = '─';
const QUOTE_CHAR: char = '▐';
const SCROLLBAR_THUMB: char = '▐';
const SCROLLBAR_TRACK: char = '│';

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum DisplayError {
    #[error("Model not found : {0}")]
    ModelNotFound(String),
}
// ── Live markdown ─────────────────────────────────────────────────────────────

const LIVE_UPDATE_HEIGHT_PERCENT: f32 = 0.6;

/// Streams partial markdown to the terminal with in-place re-rendering.
pub(crate) struct LiveMarkdown {
    skin: MadSkin,
    lines_on_screen: u16,
    term_width: u16,
    term_height: u16,
    last_render: Instant,
    disabled: bool,
    refresh_ms: u64,
}

impl LiveMarkdown {
    pub(crate) fn new(theme: &Theme, refresh_ms: u64) -> Self {
        let (w, h) = terminal::size().unwrap_or((120, 40));
        Self {
            skin: make_skin(theme),
            lines_on_screen: 0,
            term_width: w,
            term_height: h,
            last_render: Instant::now() - Duration::from_secs(1),
            disabled: false,
            refresh_ms,
        }
    }

    /// Call after every streamed chunk — throttled to `refresh_ms`.
    pub(crate) fn update(&mut self, text: &str) {
        if self.disabled {
            return;
        }
        if self.last_render.elapsed() < Duration::from_millis(self.refresh_ms) {
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
    /// - Disables live updates once content exceeds `LIVE_UPDATE_HEIGHT_PERCENT`
    ///   of the terminal height to avoid thrashing on very long responses.
    /// - only clear/updates the last line if possible, to avoid flicker
    fn paint(&mut self, text: &str) -> std::io::Result<()> {
        let rendered = format!("{}", self.skin.term_text(text));
        let lines = count_visual_lines(&rendered, self.term_width);

        // Guard: nothing is on-screen yet and there is still nothing to render.
        // Without this, the patch branch below would call MoveUp(1) from the
        // cursor position immediately after any pre-existing terminal output
        // (e.g. the separator drawn before streaming starts), erasing it.
        // This triggers when an early token produces empty/whitespace output
        // that termimad collapses to "": count_visual_lines returns 0, so
        // `lines > lines_on_screen` is false and we'd fall into the patch
        // branch where visual_rows_for_line("") == 1 unconditionally.
        if lines == 0 && self.lines_on_screen == 0 {
            return Ok(());
        }

        // Disable live updates once the block becomes tall.
        let threshold = (self.term_height as f32 * LIVE_UPDATE_HEIGHT_PERCENT) as u16;
        if !self.disabled && lines > threshold {
            self.disabled = true;
            // Flush whatever we have left as plain text so nothing is lost.
            if self.lines_on_screen == 0 {
                // Batch into one write to avoid tearing
                execute!(stdout(), style::Print(&rendered))?;
            }
            return Ok(());
        }

        let mut out = stdout().lock();

        // After every paint() call, regardless of which path was taken, the
        // cursor is at the start of the line immediately below the rendered
        // content.
        if lines > self.lines_on_screen {
            // ── A new line was added: full clear + redraw ──────────────────
            // Infrequent (only on newlines), so flicker here is imperceptible.
            if self.lines_on_screen > 0 {
                execute!(
                    out,
                    cursor::MoveUp(self.lines_on_screen),
                    terminal::Clear(terminal::ClearType::FromCursorDown),
                    style::Print(&rendered),
                )?;
            } else {
                execute!(out, style::Print(&rendered))?;
            }
            // check for ending newline (see above)
            if !rendered.ends_with('\n') {
                execute!(out, style::Print("\n"))?;
            }
            self.lines_on_screen = lines;
        } else {
            // ── Same line count: patch only the last visual line ───────────
            // This is the hot path — runs on every token, no flicker.
            let last = last_rendered_line(&rendered);
            let last_rows = visual_rows_for_line(last, self.term_width);
            execute!(
                out,
                cursor::MoveUp(last_rows),
                terminal::Clear(terminal::ClearType::FromCursorDown),
                style::Print(last),
            )?;
            // check for ending newline (see above)
            if !last.ends_with('\n') {
                execute!(out, style::Print("\n"))?;
            }
        }

        Ok(())
    }
}

// ── Live markdown helpers ─────────────────────────────────────────────────────

/// Count how many terminal rows `rendered` occupies, accounting for ANSI
/// escape sequences (stripped for measurement) and soft line-wrapping.
fn count_visual_lines(rendered: &str, term_width: u16) -> u16 {
    if term_width == 0 {
        return 0;
    }
    let plain = console::strip_ansi_codes(rendered);

    // Count actual newlines rather than using .lines(), so we handle the
    // trailing-newline case explicitly and don't silently drop it.
    let segments: Vec<&str> = plain.split('\n').collect();

    // If the text ends with '\n' the last segment is "", which represents
    // the cursor sitting on the next blank line — don't add a row for it.
    let meaningful = if segments.last().map_or(false, |s| s.is_empty()) {
        &segments[..segments.len() - 1]
    } else {
        &segments[..]
    };

    meaningful
        .iter()
        .map(|l| visual_rows_for_line(l, term_width))
        .sum()
}

/// How many terminal rows a single (already-plain) line occupies after wrapping.
fn visual_rows_for_line(line: &str, term_width: u16) -> u16 {
    if term_width == 0 {
        return 1;
    }
    // Strip ANSI in case the caller passes a raw rendered line
    let plain = console::strip_ansi_codes(line);
    let w = UnicodeWidthStr::width(plain.as_ref()) as u16;
    if w == 0 { 1 } else { w.div_ceil(term_width) }
}

/// Extract the last logical line from a rendered string (ANSI codes intact),
/// trimming any trailing newline first so we don't get an empty slice.
fn last_rendered_line(rendered: &str) -> &str {
    let trimmed = rendered.trim_end_matches('\n');
    trimmed.rsplit_once('\n').map_or(trimmed, |(_, last)| last)
}

// ── Markdown skin ─────────────────────────────────────────────────────────────

/// Build a `MadSkin` driven entirely by the provided `Theme`.
fn make_skin(theme: &Theme) -> MadSkin {
    // build a default skin
    let mut skin = match theme {
        Theme::Dark => MadSkin::default_dark(),
        Theme::Light => MadSkin::default_light(),
    };
    // customize colors
    let theme = ConsoleColors::new(&theme);

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
    // When code_bg is set, inline code uses that shade and block code uses the
    // next step along the ramp (slightly more contrasted) — same two-tone trick
    // as before, now driven by the theme.
    let (inline_bg, block_bg) = match theme.code_bg {
        Some(Color::AnsiValue(n)) => (
            Some(Color::AnsiValue(n)),
            Some(Color::AnsiValue(n.saturating_add(1))),
        ),
        other => (other, other),
    };
    skin.inline_code = CompoundStyle::new(Some(theme.code), inline_bg, Bold.into());
    skin.code_block.compound_style =
        CompoundStyle::new(Some(theme.code), block_bg, Attributes::default());

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

// ── Shell chrome ──────────────────────────────────────────────────────────────

pub(crate) fn print_banner(selected_model: &EnrichedModel, theme: &Theme) {
    let theme = ConsoleColors::new(&theme);

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

/// Returns a styled elapsed-time string, e.g. `[1.23s]`.
pub(crate) fn get_duration(start: Instant, theme: &Theme) -> String {
    let theme = ConsoleColors::new(&theme);
    format!("[{:.2}s]", start.elapsed().as_secs_f64())
        .with(theme.duration)
        .to_string()
}
