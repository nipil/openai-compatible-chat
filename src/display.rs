use std::{
    io::{Write, stdout},
    time::{Duration, Instant},
};

use crossterm::{cursor, execute, terminal};
use owo_colors::OwoColorize;
use termimad::MadSkin;

// ── Structured log lines (mimics rich's RichHandler colour scheme) ────────────

pub fn log_info(msg: &str) {
    eprintln!("{}", msg.white());
}
pub fn log_warning(msg: &str) {
    eprintln!("{} {}", "warn :".yellow().bold(), msg.yellow());
}
pub fn log_error(msg: &str) {
    eprintln!("{} {}", "error:".red().bold(), msg.red());
}
pub fn log_critical(msg: &str) {
    eprintln!("{} {}", "crit :".magenta().bold(), msg.magenta());
}

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

fn make_skin() -> MadSkin {
    // `default_dark()` targets dark-background terminals.
    // Switch to `MadSkin::default()` for automatic detection.
    MadSkin::default_dark()
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
