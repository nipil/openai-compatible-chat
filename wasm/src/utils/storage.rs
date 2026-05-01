use portable::Message;

use crate::web::dom::{get_storage, get_window};
use crate::{BrowserError, StorageError};

pub const STORAGE_KEY_OPENAI: &str = "openai";

pub fn save_chat(messages: &[Message]) -> Result<(), StorageError> {
    let Some(storage) = get_storage(get_window()?)? else {
        return Ok(());
    };

    let json = serde_json::to_string(messages)?;

    storage
        .set_item(STORAGE_KEY_OPENAI, &json)
        .map_err(|e| BrowserError::SessionStorage { source: e.into() })?;

    Ok(())
}

pub fn load_chat() -> Result<Vec<Message>, StorageError> {
    let Some(storage) = get_storage(get_window()?)? else {
        return Ok(vec![]);
    };
    let Some(text) = storage
        .get_item(STORAGE_KEY_OPENAI)
        .map_err(|e| BrowserError::SessionStorage { source: e.into() })?
    else {
        return Ok(vec![]);
    };
    serde_json::from_str(&text).map_err(Into::into)
}
