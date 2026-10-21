//! Locally imported shell artwork for the public DSi window.
use base64::Engine;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 4 * 1024 * 1024;
const MIN_SIDE: u32 = 512;
const MAX_SIDE: u32 = 4096;

fn destination(root: &Path) -> PathBuf {
    root.join("ui").join("custom-shell.png")
}

fn validate(data: &[u8]) -> Result<(), String> {
    if data.len() < 24 || data.len() as u64 > MAX_BYTES || &data[..8] != b"\x89PNG\r\n\x1a\n" || &data[12..16] != b"IHDR" {
        return Err("Choose a PNG image no larger than 4 MiB".into());
    }
    let width = u32::from_be_bytes(data[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(data[20..24].try_into().unwrap());
    if !(MIN_SIDE..=MAX_SIDE).contains(&width) || !(MIN_SIDE..=MAX_SIDE).contains(&height) {
        return Err("Shell width and height must each be 512–4096 pixels".into());
    }
    let aspect = width as f64 / height as f64;
    if !(0.9..=1.1).contains(&aspect) {
        return Err("Use a roughly square shell image (width/height 0.9–1.1)".into());
    }
    Ok(())
}

pub fn load(root: &Path) -> Result<Option<String>, String> {
    let path = destination(root);
    if !path.exists() { return Ok(None); }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("Stored shell image is invalid".into());
    }
    let data = fs::read(&path).map_err(|e| e.to_string())?;
    validate(&data)?;
    Ok(Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(data))))
}

pub fn set(root: &Path, source: Option<&str>) -> Result<(), String> {
    let target = destination(root);
    let Some(source) = source else {
        if target.exists() { fs::remove_file(&target).map_err(|e| e.to_string())?; }
        return Ok(());
    };
    let source = Path::new(source);
    if !source.is_absolute() { return Err("Choose an absolute image path".into()); }
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
        return Err("Choose a regular PNG image no larger than 4 MiB".into());
    }
    let data = fs::read(source).map_err(|e| e.to_string())?;
    validate(&data)?;
    fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
    if let Ok(existing) = fs::symlink_metadata(&target) {
        if !existing.is_file() || existing.file_type().is_symlink() {
            return Err("Stored shell path is not a regular file".into());
        }
    }
    fs::write(target, data).map_err(|e| e.to_string())
}
