use portable::Theme;

use crate::BrowserError;
use crate::web::dom::{get_document, get_document_element, get_window, set_element_attribute};

pub fn apply_theme(theme: &Theme) -> Result<(), BrowserError> {
    let doc_el = get_document_element(get_document(get_window()?)?)?;
    set_element_attribute(doc_el, "data-theme", theme.as_ref())
}
