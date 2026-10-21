//! User-adjustable hit areas for the DSi shell artwork.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BUTTONS: [&str; 13] = ["UP", "DOWN", "LEFT", "RIGHT", "A", "B", "X", "Y", "L", "R", "START", "SELECT", "POWER"];
const MAX_BYTES: u64 = 4096;

#[derive(Clone, Deserialize, Serialize)]
pub struct Position { pub left: f64, pub top: f64 }

#[derive(Deserialize, Serialize)]
pub struct ButtonLayout { pub version: u8, pub positions: BTreeMap<String, Position> }

fn path(root: &Path, public: bool) -> PathBuf {
    root.join("ui").join(if public { "button-layout.json" } else { "development-button-layout.json" })
}

fn validate(layout: &ButtonLayout) -> Result<(), String> {
    if layout.version != 1 || layout.positions.len() != BUTTONS.len()
        || BUTTONS.iter().any(|button| !layout.positions.contains_key(*button)) {
        return Err("Button layout needs version 1 and all 13 controls".into());
    }
    for (name, point) in &layout.positions {
        if !point.left.is_finite() || !point.top.is_finite()
            || !(0.0..=100.0).contains(&point.left) || !(0.0..=100.0).contains(&point.top) {
            return Err(format!("Invalid position for {name}"));
        }
    }
    Ok(())
}

pub fn load(root: &Path, public: bool) -> Result<Option<ButtonLayout>, String> {
    let path = path(root, public);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("Button layout must be a regular JSON file no larger than 4 KiB".into());
    }
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    let layout: ButtonLayout = serde_json::from_slice(&data).map_err(|e| format!("Button layout: {e}"))?;
    validate(&layout)?;
    Ok(Some(layout))
}

pub fn save(root: &Path, public: bool, layout: ButtonLayout) -> Result<(), String> {
    validate(&layout)?;
    let path = path(root, public);
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Ok(existing) = fs::symlink_metadata(&path) {
        if !existing.is_file() || existing.file_type().is_symlink() {
            return Err("Button layout path is not a regular file".into());
        }
    }
    let data = serde_json::to_vec_pretty(&layout).map_err(|e| e.to_string())?;
    fs::write(path, data).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_requires_every_control_and_bounded_positions() {
        let mut layout = ButtonLayout { version: 1, positions: BUTTONS.iter().map(|name| ((*name).into(), Position { left: 50.0, top: 50.0 })).collect() };
        assert!(validate(&layout).is_ok());
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("cpf-layout-{}-{unique}", std::process::id()));
        fs::create_dir(&root).unwrap();
        save(&root, false, ButtonLayout { version: 1, positions: layout.positions.clone() }).unwrap();
        assert_eq!(load(&root, false).unwrap().unwrap().positions["POWER"].left, 50.0);
        assert!(load(&root, true).unwrap().is_none());
        layout.positions.remove("POWER");
        assert!(validate(&layout).is_err());
        layout.positions.insert("POWER".into(), Position { left: 101.0, top: 50.0 });
        assert!(validate(&layout).is_err());
        fs::remove_file(path(&root, false)).unwrap();
        fs::remove_dir(root.join("ui")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
