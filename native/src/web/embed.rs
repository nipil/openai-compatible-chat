use std::convert::Infallible;

use axum::body::Body;
use axum::http::StatusCode;
use axum::response::Response;
use http::header;
use rust_embed::RustEmbed;
use tracing::debug;

// ── Embedding files into binary ───────────────────────────────────────────────

/// The path resolution works as follows:
/// - In debug and when debug-embed feature is not enabled, the folder path is
///   resolved relative to where the binary is run from.
/// - In release or when debug-embed feature is enabled, the folder path is
///   resolved relative to where Cargo.toml is.
#[derive(RustEmbed)]
#[folder = "../wasm/dist"]
pub(crate) struct Assets;

#[cfg(feature = "embed")]
pub(crate) fn serve_asset(path: &str) -> Result<Response, Infallible> {
    let mut path = path.trim_start_matches('/');
    if path.is_empty() {
        path = "index.html";
    }

    let response = match Assets::get(path) {
        Some(content) => {
            debug!(file = path, "Asset found");

            let mime = mime_guess::from_path(path).first_or_octet_stream();

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
        }

        None => {
            debug!(file = path, "Asset not found");

            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::from("Not Found"))
        }
    };

    Ok(response.expect("Hardcoded response should not fail"))
}
