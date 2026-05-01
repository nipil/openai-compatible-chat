use portable::TokenUsage;

pub fn token_color_style(count: &TokenUsage, max: Option<u32>) -> String {
    let (bg, color) = match max {
        None => ("rgba(128,128,128,0.4)", "var(--text-banner)"),
        Some(m) => {
            let pct = u32::from(count) as f64 / m as f64;
            if pct < 0.50 {
                ("transparent", "var(--text-banner)")
            } else if pct < 0.75 {
                ("#ffd700", "#333")
            } else if pct < 0.90 {
                ("#ff8c00", "white")
            } else {
                ("#cc0000", "white")
            }
        }
    };
    format!("background:{bg}; color:{color};")
}
