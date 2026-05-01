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
    /// Background for inline code (slightly darker shade than `code_block_bg`)
    /// and code blocks.  `None` falls back to the termimad default grays.
    /// Use `Some(gray(n))` from termimad for ANSI grey ramp values (0–23).
    pub code_bg: Option<Color>,

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
            Theme::Light => Self::light(),
        }
    }
    /// Dark-terminal theme — designed for black or near-black backgrounds.
    ///
    /// Colour strategy:
    ///   - Headings use fully-saturated bright hues: they need to pierce the
    ///     dark background without extra weight.
    ///   - Inline styles lean on White and Yellow — the two colours that feel
    ///     "light" without being neon, keeping body text comfortable at length.
    ///   - Code uses Green on a near-black panel — the classic terminal look,
    ///     with just enough background lift to visually box the snippet.
    ///   - Chrome / secondary info uses White → DarkGrey as a two-level
    ///     hierarchy: primary labels are bright, ambient noise fades back.
    ///   - Token thresholds follow the traffic-light convention with full-
    ///     brightness variants that stand out against the dark surface.
    pub fn dark() -> Self {
        Self {
            // ── Headings — vivid hues that cut through dark backgrounds ───────
            heading_1: Cyan,    // bright teal   — commanding, cool
            heading_2: Magenta, // bright violet — clearly secondary
            heading_3: Yellow,  // bright amber  — warm third level
            heading_n: White,   // plain bright  — lowest heading weight

            // ── Inline styles ─────────────────────────────────────────────────
            strong: White,    // bright white bold — pure contrast pop
            emphasis: Yellow, // warm amber italic — distinct without clashing
            deleted: Red,     // bright red strikethrough — unmistakably "wrong"

            // ── Code ─────────────────────────────────────────────────────────
            code: Green, // classic terminal green — sharp on dark BG
            // gray(2) = near-black — barely-visible panel behind green text
            code_bg: Some(gray(2)),

            // ── Structural / decorative ───────────────────────────────────────
            accent: Cyan, // bullets, hrules, scrollbar — echoes heading_1
            border: Blue, // table borders, blockquote bar — quieter than Cyan

            // ── Shell chrome ──────────────────────────────────────────────────
            chrome: White,      // "───" decorators — full brightness
            model_name: Cyan,   // prominent ID — mirrors heading_1
            meta: White,        // description / family — same weight as chrome
            timestamp: White,   // [HH:MM:SS] — visible but not dominant
            tag: DarkGrey,      // [model-id] secondary tag — recedes
            duration: DarkGrey, // elapsed time — background noise

            // ── Token thresholds — traffic-light on dark BG ───────────────────
            token_low: DarkGrey, // barely there
            token_medium: White, // neutral presence
            token_warn: Yellow,  // amber warning — mirrors emphasis
            token_critical: Red, // clear alarm
        }
    }

    /// Light-terminal theme — designed for white or near-white backgrounds.
    ///
    /// Colour strategy:
    ///   - Headings use the *Dark* variants of the primary hues so they pop
    ///     against white without bleeding into each other.
    ///   - Inline styles stay in the dark-ink range so bold/italic feel
    ///     intentional, not washed out.
    ///   - Code uses DarkGreen — the classic "terminal green" remains very
    ///     legible on light surfaces.
    ///   - Chrome / secondary info uses Black → DarkGrey → Grey as a clear
    ///     three-level hierarchy of visual weight.
    ///   - Token thresholds mirror the dark theme's traffic-light intent but
    ///     with darker/more-saturated variants that show up on light BG.
    pub fn light() -> Self {
        Self {
            // ── Headings — each a distinct hue, darker than the BG ───────────
            heading_1: DarkCyan,    // deep teal   — prominent, calm
            heading_2: DarkMagenta, // deep violet — clearly secondary
            heading_3: DarkYellow,  // olive/amber  — warm third level
            heading_n: Black,       // plain ink    — lowest heading weight

            // ── Inline styles ─────────────────────────────────────────────────
            strong: Black,         // crisp bold black — maximum contrast
            emphasis: DarkMagenta, // italic violet — warm without clashing
            deleted: DarkRed,      // dark red strikethrough — clearly "wrong"

            // ── Code ─────────────────────────────────────────────────────────
            code: DarkGreen, // deep green — universally readable on white
            // gray(20) = light silver — subtle off-white panel, just enough
            // separation from the page background without jarring contrast
            code_bg: Some(gray(20)),

            // ── Structural / decorative ───────────────────────────────────────
            accent: DarkCyan, // bullets, hrules, scrollbar
            border: DarkBlue, // table borders, blockquote bar

            // ── Shell chrome ──────────────────────────────────────────────────
            chrome: Black,        // "───" decorators — strong ink
            model_name: DarkBlue, // prominent ID, distinct from headings
            meta: DarkGrey,       // description / family — quiet secondary
            timestamp: DarkGrey,  // [HH:MM:SS] — present but unobtrusive
            tag: Grey,            // [model-id] secondary tag — lightest chrome
            duration: Grey,       // elapsed time — background noise

            // ── Token thresholds — traffic-light on light BG ──────────────────
            token_low: Grey,         // barely there
            token_medium: DarkGrey,  // neutral presence
            token_warn: DarkYellow,  // amber warning
            token_critical: DarkRed, // clear alarm
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
    pub(crate) fn new(theme: &Theme) -> Self {
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
fn make_skin(theme: &Theme) -> MadSkin {
    // build a default skin
    let mut skin = match theme {
        Theme::Dark => MadSkin::default_dark(),
        Theme::Light => MadSkin::default_light(),
    };
    // customize colors
    let theme = ConsoleTheme::new(&theme);

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

pub(crate) fn print_banner(selected_model: &EnrichedModel, theme: &Theme) {
    let theme = ConsoleTheme::new(&theme);

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
    theme: &Theme,
) -> String {
    let time = Local::now().format("%H:%M:%S").to_string();
    let theme = ConsoleTheme::new(&theme);

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
pub(crate) fn get_duration(start: Instant, theme: &Theme) -> String {
    let theme = ConsoleTheme::new(&theme);
    format!("[{:.2}s]", start.elapsed().as_secs_f64())
        .with(theme.duration)
        .to_string()
}
