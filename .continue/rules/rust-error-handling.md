---
name: Rust Error Handling
globs: ["src/**/*.rs"]
description: Error handling patterns using thiserror
---

# Rust Error Management

## Error Types

- **Library modules**: Use `thiserror` crate with module-specific error enums at the top
- **Binaries/WASM**: At the application boundary, convert library errors to user-facing types (Axum `IntoResponse` trait, HTTP responses codes, with UI messages body)
- Never silently drop errors; always propagate and explicitly log them

## For Axum handlers (server-side)

Implement `IntoResponse` on error enums for automatic HTTP conversion.

## Error Enum Pattern

Each library module has a single error enum with variants for its failure modes:

- Use `#[from]` to auto-implement `From` trait (for error chaining)
- Use `#[source]` when you can't/shouldn't auto-implement `From`
- Every variant must include context: include relevant data in the error

## Error Propagation

- Use `?` operator on every fallible statement; it auto-converts with `From` impl
- If a `?` returns the wrong error type, convert with `.map_err(|e| MyError::Variant(e))?`

## Example

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error in '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Validation failed: {0}")]
    Validation(String),
}

// Library module function
pub fn load_config(path: &str) -> Result<Config, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&content)
        .map_err(|e| ConfigError::Parse { path: path.to_string(), source: e })?;
    Ok(config)
}

// Native binary example (main.rs or bin)
fn main() -> anyhow::Result<()> {
    let config = load_config("config.toml")?;
    println!("Loaded config: {:?}", config);
    Ok(())
}

// WASM boundary (Leptos app)
#[wasm_bindgen]
pub fn load_config_wasm(path: String) -> Result<JsValue, JsValue> {
    load_config(&path)
        .map(|c| serde_wasm_bindgen::to_value(&c).unwrap())
        .map_err(|e| JsValue::from_str(&e.to_string()))
}
```

## Module Organization

- Group by concern: parsing in one module, API ops in another
- One error enum per module
