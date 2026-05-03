use serde::de::DeserializeOwned;

use crate::RequestError;

pub(crate) async fn get_url_path<T: DeserializeOwned>(path: &str) -> Result<T, RequestError> {
    let resp = gloo_net::http::Request::get(path)
        .send()
        .await
        .map_err(|e| RequestError::ConnectionError { source: e })?;

    if !resp.ok() {
        return Err(RequestError::HttpError {
            status: resp.status(),
            message: resp.status_text(),
        });
    }
    resp.json::<T>()
        .await
        .map_err(|e| RequestError::ConnectionError { source: e })
}
