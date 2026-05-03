use portable::Theme;
use termimad::crossterm::style::Color;
use termimad::crossterm::style::Color::*;
use termimad::gray;

// ── Theme ─────────────────────────────────────────────────────────────────────

/// Every colour decision lives here.  The rest of the module refers only to
/// semantic role names — never to raw colour literals.
pub(crate) struct ConsoleColors {
    // ── Markdown headings ─────────────────────────────────────────────────────
    /// H1 — most prominent heading
    pub(crate) heading_1: Color,
    /// H2
    pub(crate) heading_2: Color,
    /// H3
    pub(crate) heading_3: Color,
    /// H4 and below — progressively less prominent, same colour
    pub(crate) heading_n: Color,

    // ── Inline text styles ────────────────────────────────────────────────────
    /// **bold** spans
    pub(crate) strong: Color,
    /// *italic* / emphasis spans
    pub(crate) emphasis: Color,
    /// ~~strikethrough~~ / deleted text
    pub(crate) deleted: Color,

    // ── Code ─────────────────────────────────────────────────────────────────
    /// Foreground for both inline `code` and fenced code blocks
    pub(crate) code: Color,
    /// Background for inline code (slightly darker shade than `code_block_bg`)
    /// and code blocks.  `None` falls back to the termimad default grays.
    /// Use `Some(gray(n))` from termimad for ANSI grey ramp values (0–23).
    pub(crate) code_bg: Option<Color>,

    // ── Structural / decorative markdown elements ─────────────────────────────
    /// Bullet markers, horizontal rules, scrollbar thumb
    pub(crate) accent: Color,
    /// Blockquote bar, table borders
    pub(crate) border: Color,

    // ── Shell chrome ──────────────────────────────────────────────────────────
    /// Static banner decorators ("───", "description:", …)
    pub(crate) chrome: Color,
    /// Model identifier — shown in banner and prompt tag
    pub(crate) model_name: Color,
    /// Supplementary model info (description, family, release)
    pub(crate) meta: Color,
    /// [HH:MM:SS] timestamp in the user prompt
    pub(crate) timestamp: Color,
    /// Secondary bracketed tag, e.g. [model-id]
    pub(crate) tag: Color,
    /// Elapsed-time readout
    pub(crate) duration: Color,

    // ── Token-usage thresholds ────────────────────────────────────────────────
    /// < 50 % — unobtrusive / de-emphasised
    pub(crate) token_low: Color,
    /// 50 – 75 % — neutral
    pub(crate) token_medium: Color,
    /// 75 – 90 % — approaching limit
    pub(crate) token_warn: Color,
    /// ≥ 90 % — critical
    pub(crate) token_critical: Color,
}

impl ConsoleColors {
    /// Build a console theme from a them enum
    pub(crate) fn new(theme: &Theme) -> Self {
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
    pub(crate) fn dark() -> Self {
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
            token_low: Blue,     // barely there
            token_medium: Green, // neutral presence
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
    pub(crate) fn light() -> Self {
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
            token_low: Blue,         // barely there
            token_medium: DarkGreen, // neutral presence
            token_warn: DarkYellow,  // amber warning
            token_critical: DarkRed, // clear alarm
        }
    }
}
