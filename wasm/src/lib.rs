pub(crate) mod api;
pub mod components;
pub(crate) mod utils;
pub mod web;

use std::fmt;

use eventsource_stream::EventStreamError;
use leptos::prelude::{RwSignal, Update};
use portable::ChatEventError;
use thiserror::Error;
use wasm_bindgen::JsValue;

#[derive(Debug, Error)]
/// A JsValue to Error wrapper for early-return
pub struct JsError(pub(crate) JsValue);

impl fmt::Display for JsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl From<JsValue> for JsError {
    fn from(value: JsValue) -> Self {
        Self(value)
    }
}

/// Separate error type to avoid EventStreamError cycle
#[derive(Error, Debug)]
pub(crate) enum FutureStreamError {
    #[error("Could not get chunk out of stream {0}")]
    Chunk(#[from] JsError),
}

/// Browser errors (DOM, storage, cookies…)
#[derive(Error, Debug)]
pub enum BrowserError {
    #[error("Could not create abort controller : {source}")]
    AbortController { source: JsError },

    #[error("Could not display an alert : {source}")]
    AlertFailed { source: JsError },

    #[error("Could not access cookies : {source}")]
    CookieAccess { source: JsError },

    #[error("Could not get current address")]
    CurrentUrl { source: JsError },

    #[error("Document is not an HTML document")]
    DocumentIsNotHtml,

    #[error("Could not access element : {source}")]
    ElementAccess { source: JsError },

    #[error("Window has no document")]
    NoDocument,

    #[error("Document has no element")]
    NoDocumentElement,

    #[error("Could not get window")]
    NoWindow,

    #[error("Could not open new window")]
    OpenWindow { source: JsError },

    #[error("Could not reload window")]
    ReloadFailed { source: JsError },

    #[error("Could not get session storage : {source}")]
    SessionStorage { source: JsError },
}

/// HTTP request and SSE streaming errors
#[derive(Error, Debug)]
pub(crate) enum RequestError {
    #[error("Connection error during request : {source}")]
    ConnectionError { source: gloo_net::Error },

    #[error("Could not convert fetch result to a response : {source}")]
    ConvertResponse { source: JsError },

    #[error("Could not create request headers : {source}")]
    CreateHeaders { source: JsError },

    #[error("Could not create request : {source}")]
    CreateRequest { source: JsError },

    #[error("Could not convert a server-side event to a chat event : {0}")]
    EventConversion(#[from] ChatEventError),

    #[error("Error while streaming chunk : {0}")]
    EventStream(#[from] EventStreamError<FutureStreamError>),

    #[error("Could not fetch request : {source}")]
    FetchRequest { source: JsError },

    #[error("HTTP error during request : {status} {message}")]
    HttpError { status: u16, message: String },

    #[error("Could not get response body")]
    NoBody,

    #[error("Could not set request header : {source}")]
    SetHeader { source: JsError },
}

/// Application logic errors
#[derive(Error, Debug)]
pub(crate) enum LogicError {
    #[error("Could not serialize/deserialize data : {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("No model selected")]
    NoModelSelected,
}

/// Aggregate error for storage
#[derive(Error, Debug)]
pub(crate) enum StorageError {
    #[error("Could not access storage : {0}")]
    Browser(#[from] BrowserError),

    #[error("Could not convert data : {0}")]
    JsonError(#[from] serde_json::Error),
}

/// Aggregated error for the application
#[derive(Error, Debug)]
pub(crate) enum AppError {
    #[error("Browser error : {0}")]
    Browser(#[from] BrowserError),

    #[error("Logic error : {0}")]
    Logic(#[from] LogicError),

    #[error("Request error : {0}")]
    Request(#[from] RequestError),

    #[error("Storage error : {0}")]
    Storage(#[from] StorageError),
}

pub(crate) fn show_err_get_default<T: Default>(errors: RwSignal<Vec<String>>, e: AppError) -> T {
    let msg = e.to_string();
    web_sys::console::error_1(&msg.clone().into());
    errors.update(|v| v.push(msg));
    T::default()
}

pub(crate) fn handle_err<T: Default>(errors: RwSignal<Vec<String>>, res: Result<T, AppError>) -> T {
    match res {
        Ok(t) => t,
        Err(e) => show_err_get_default::<T>(errors, e),
    }
}

pub(crate) fn handle_err_clos_1<F, A, T>(errors: RwSignal<Vec<String>>, f: F) -> impl Fn(A) -> T
where
    F: Fn(A) -> Result<T, AppError>,
    T: Default,
{
    move |a| match f(a) {
        Ok(t) => t,
        Err(e) => show_err_get_default::<T>(errors, e),
    }
}

pub(crate) fn handle_err_fut_0<F, T>(
    errors: RwSignal<Vec<String>>,
    fut: F,
) -> impl std::future::Future<Output = T>
where
    F: std::future::Future<Output = Result<T, AppError>>,
    T: Default,
{
    async move {
        match fut.await {
            Ok(v) => v,
            Err(e) => show_err_get_default::<T>(errors, e),
        }
    }
}
