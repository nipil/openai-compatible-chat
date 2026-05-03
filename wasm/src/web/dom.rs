use wasm_bindgen::JsCast;
use web_sys::Storage;

use crate::BrowserError;

pub(crate) fn get_window() -> Result<web_sys::Window, BrowserError> {
    web_sys::window().ok_or(BrowserError::NoWindow)
}

pub(crate) fn get_document(window: web_sys::Window) -> Result<web_sys::Document, BrowserError> {
    window.document().ok_or(BrowserError::NoDocument)
}

pub(crate) fn get_document_element(
    doc: web_sys::Document,
) -> Result<web_sys::Element, BrowserError> {
    doc.document_element()
        .ok_or(BrowserError::NoDocumentElement)
}

pub(crate) fn get_storage(window: web_sys::Window) -> Result<Option<Storage>, BrowserError> {
    let storage = window
        .session_storage()
        .map_err(|e| BrowserError::SessionStorage { source: e.into() })?;

    if storage.is_none() {
        web_sys::console::warn_1(&"No browser storage available".into());
    }

    Ok(storage)
}

pub(crate) fn get_html_doc(doc: web_sys::Document) -> Result<web_sys::HtmlDocument, BrowserError> {
    doc.dyn_into::<web_sys::HtmlDocument>()
        .or(Err(BrowserError::DocumentIsNotHtml))
}

pub(crate) fn set_element_attribute(
    element: web_sys::Element,
    name: &str,
    value: &str,
) -> Result<(), BrowserError> {
    element
        .set_attribute(name, value)
        .map_err(|e| BrowserError::ElementAccess { source: e.into() })
}
