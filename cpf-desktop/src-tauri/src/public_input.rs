//! Validated, user-editable keyboard bindings for the public DSi app.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

const BUTTONS: [&str; 12] = ["UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "L", "R", "START", "SELECT"];
const RESERVED: [&str; 5] = ["Escape", "F5", "F6", "F8", "F9"];
const MAX_CONFIG_BYTES: u64 = 4096;

#[derive(Deserialize, Serialize)]
pub struct InputConfig {
    version: u8,
    buttons: BTreeMap<String, String>,
}

fn defaults() -> InputConfig {
    let pairs = [
        ("UP", "ArrowUp"), ("DOWN", "ArrowDown"), ("LEFT", "ArrowLeft"), ("RIGHT", "ArrowRight"),
        ("A", "KeyX"), ("B", "KeyZ"), ("X", "KeyS"), ("Y", "KeyA"),
        ("L", "KeyQ"), ("R", "KeyW"), ("START", "Enter"), ("SELECT", "ShiftRight"),
    ];
    InputConfig { version: 1, buttons: pairs.into_iter().map(|(button, code)| (button.into(), code.into())).collect() }
}

fn validate(config: &InputConfig) -> Result<(), String> {
    if config.version != 1 { return Err("input.json: unsupported version; use version 1".into()); }
    if config.buttons.len() != BUTTONS.len() || BUTTONS.iter().any(|button| !config.buttons.contains_key(*button)) {
        return Err("input.json: provide exactly the 12 DSi buttons listed in the guide".into());
    }
    let mut used = HashSet::new();
    for (button, code) in &config.buttons {
        if code.len() < 2 || code.len() > 32 || !code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(format!("input.json: {button} needs a KeyboardEvent.code such as KeyX or ArrowUp"));
        }
        if RESERVED.contains(&code.as_str()) {
            return Err(format!("input.json: {code} is reserved for an app control"));
        }
        if !used.insert(code) { return Err(format!("input.json: {code} is assigned more than once")); }
    }
    Ok(())
}

pub fn load(root: &Path) -> Result<InputConfig, String> {
    let path = root.join("input.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = defaults();
            let serialized = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
            fs::write(&path, serialized).map_err(|e| format!("Could not create input.json: {e}"))?;
            return Ok(config);
        }
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_CONFIG_BYTES {
        return Err("input.json must be a regular JSON file no larger than 4 KiB".into());
    }
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let config: InputConfig = serde_json::from_slice(&data).map_err(|e| format!("input.json: {e}"))?;
    validate(&config)?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_conflicting_and_reserved_bindings() {
        let mut config = defaults();
        assert!(validate(&config).is_ok());
        config.buttons.insert("A".into(), "F8".into());
        assert!(validate(&config).is_err());
        config.buttons.insert("A".into(), "KeyZ".into());
        assert!(validate(&config).is_err());
    }

    #[test]
    fn creates_and_reloads_user_config() {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("cpf-input-{}-{unique}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let path = root.join("input.json");
        let mut config = load(&root).unwrap();
        assert!(path.is_file());
        assert_eq!(config.buttons["A"], "KeyX");
        config.buttons.insert("A".into(), "KeyC".into());
        fs::write(&path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        assert_eq!(load(&root).unwrap().buttons["A"], "KeyC");
        fs::write(&path, b"not json").unwrap();
        assert!(load(&root).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
