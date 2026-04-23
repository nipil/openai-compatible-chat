use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use serde_json::{Map, Value};

#[derive(Parser)]
#[command(
    name = "jsonfmt",
    about = "Pretty-print JSON files in place by sorting keys recursively"
)]

struct Args {
    /// File or directory to process (defaults to current directory)
    path: Option<PathBuf>,
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut new_map = Map::new();
            let mut keys: Vec<_> = map.keys().cloned().collect();
            keys.sort();

            for key in keys {
                let v = map.get(&key).unwrap().clone();
                new_map.insert(key, sort_json(v));
            }

            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(sort_json).collect()),
        other => other,
    }
}

fn process_file(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let input = fs::read_to_string(path)?;
    let json: Value = serde_json::from_str(&input)?;
    let sorted = sort_json(json);

    let pretty = serde_json::to_string_pretty(&sorted)?;
    fs::write(path, pretty)?;

    println!("Formatted {}", path.display());
    Ok(())
}

fn process_dir(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            process_file(&path)?;
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    match args.path {
        Some(path) if path.is_file() => process_file(&path)?,
        Some(path) if path.is_dir() => process_dir(&path)?,
        Some(path) => eprintln!("Invalid path: {}", path.display()),
        None => process_dir(Path::new("."))?,
    }

    Ok(())
}
