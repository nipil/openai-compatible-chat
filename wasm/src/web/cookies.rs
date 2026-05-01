use std::str::FromStr;

use portable::Theme;

use crate::BrowserError;
use crate::web::dom::{get_document, get_html_doc, get_window};

pub const COOKIE_MODEL: &str = "model";
pub const COOKIE_THEME: &str = "theme";
pub const COOKIE_THEME_DEFAULT: Theme = Theme::Dark;
pub const COOKIE_MAX_AGE: u32 = 31_536_000; // 1 year

fn get_cookies(html_doc: web_sys::HtmlDocument) -> Result<String, BrowserError> {
    html_doc
        .cookie()
        .map_err(|e| BrowserError::CookieAccess { source: e.into() })
}

fn set_cookies(
    html_doc: web_sys::HtmlDocument,
    name: &str,
    value: &str,
) -> Result<(), BrowserError> {
    html_doc
        .set_cookie(&format!(
            "{name}={value}; max-age={COOKIE_MAX_AGE}; SameSite=Strict; path=/"
        ))
        .map_err(|e| BrowserError::CookieAccess { source: e.into() })
}

pub fn get_cookie(name: &str) -> Result<Option<String>, BrowserError> {
    let cookies = get_cookies(get_html_doc(get_document(get_window()?)?)?)?;
    let found = cookies.split(';').find_map(|pair| {
        pair.trim()
            .strip_prefix(&format!("{name}="))
            .map(str::to_string)
    });
    Ok(found)
}

pub fn set_cookie(name: &str, value: &str) -> Result<(), BrowserError> {
    let html_doc = get_html_doc(get_document(get_window()?)?)?;
    set_cookies(html_doc, name, value)
}

pub fn get_cookie_theme() -> Theme {
    match get_cookie(COOKIE_THEME).map(|x| x.map(|y| Theme::from_str(&y))) {
        Ok(Some(Ok(theme))) => theme,
        _ => COOKIE_THEME_DEFAULT,
    }
}
