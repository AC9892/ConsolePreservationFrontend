use crate::media::{self, MediaInfo};
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const MARKER: &str = "pseudo-game-library.json";
const SCHEMA: &str = "pseudo.nds-library/1";

#[derive(Serialize)]
pub struct LibraryListing {
    pub folder: Option<String>,
    pub games: Vec<MediaInfo>,
    pub skipped: usize,
}

fn verify(folder: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(folder).map_err(|error| error.to_string())?;
    if !folder.is_absolute() || !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Choose a real, absolute game library folder".into());
    }
    let marker = fs::read(folder.join(MARKER)).map_err(|_| "This folder is missing its Pseudo game library identifier".to_string())?;
    let value: Value = serde_json::from_slice(&marker).map_err(|_| "The game library identifier is invalid".to_string())?;
    if value.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        return Err("This folder has an unsupported game library identifier".into());
    }
    Ok(())
}

pub fn create_or_open(folder: &Path) -> Result<PathBuf, String> {
    if !folder.is_absolute() { return Err("Choose an absolute folder".into()); }
    let meta = fs::symlink_metadata(folder).map_err(|error| error.to_string())?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("Choose a real folder, not a folder link".into());
    }
    let folder = folder.canonicalize().map_err(|error| error.to_string())?;
    let marker = folder.join(MARKER);
    if marker.exists() { verify(&folder)?; }
    else {
        let payload = serde_json::json!({"schema":SCHEMA,"kind":"Nintendo DS game library"});
        let bytes = serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        options.open(&marker).and_then(|mut file| file.write_all(&bytes)).or_else(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                verify(&folder).map_err(std::io::Error::other)
            }
            else { Err(error) }
        }).map_err(|error| error.to_string())?;
    }
    Ok(folder)
}

pub fn create_on_virtual_sd(sd_host: &Path) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(sd_host).map_err(|error| error.to_string())?;
    if !sd_host.is_absolute() || !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("The virtual SD host folder must be a real directory".into());
    }
    let folder = sd_host.join(".nds");
    if !folder.exists() {
        fs::create_dir(&folder).map_err(|error| error.to_string())?;
    }
    create_or_open(&folder)
}

pub fn list(folder: Option<&Path>) -> Result<LibraryListing, String> {
    let Some(folder) = folder else {
        return Ok(LibraryListing { folder: None, games: Vec::new(), skipped: 0 });
    };
    verify(folder)?;
    let mut games = Vec::new();
    let mut skipped = 0;
    for entry in fs::read_dir(folder).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("nds")) { continue; }
        match media::inspect(&path) {
            Ok(info) => games.push(info),
            Err(_) => skipped += 1,
        }
    }
    games.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()).then(a.path.cmp(&b.path)));
    Ok(LibraryListing { folder: Some(folder.display().to_string()), games, skipped })
}

pub fn game(folder: &Path, file_name: &str) -> Result<(PathBuf, MediaInfo), String> {
    verify(folder)?;
    let name = Path::new(file_name);
    if name.components().count() != 1 || name.file_name().is_none_or(|part| part != file_name) {
        return Err("Select a game from this library".into());
    }
    let path = folder.join(name);
    let info = media::inspect(&path)?;
    Ok((path, info))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_folder_lists_drop_in_nds_and_rejects_escape() {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("pseudo-library-test-{}-{unique}", std::process::id()));
        fs::create_dir(&folder).unwrap();
        let game_path = folder.join("Example.nds");
        let mut header = [0u8; 0x200];
        header[..7].copy_from_slice(b"EXAMPLE");
        header[0x0c..0x10].copy_from_slice(b"ABCD");
        fs::write(&game_path, header).unwrap();
        assert!(list(Some(&folder)).is_err());
        create_or_open(&folder).unwrap();
        assert_eq!(list(Some(&folder)).unwrap().games.len(), 1);
        assert_eq!(game(&folder, "Example.nds").unwrap().1.game_code, "ABCD");
        assert!(game(&folder, "..\\Example.nds").is_err());
        fs::remove_file(game_path).unwrap();
        fs::remove_file(folder.join(MARKER)).unwrap();
        fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn creates_identified_library_in_generated_sd_folder() {
        let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .unwrap().as_nanos();
        let host = std::env::temp_dir().join(format!("pseudo-sd-test-{}-{unique}", std::process::id()));
        fs::create_dir(&host).unwrap();
        let folder = create_on_virtual_sd(&host).unwrap();
        assert_eq!(folder, host.join(".nds").canonicalize().unwrap());
        assert_eq!(list(Some(&folder)).unwrap().games.len(), 0);
        fs::remove_file(folder.join(MARKER)).unwrap();
        fs::remove_dir(folder).unwrap();
        fs::remove_dir(host).unwrap();
    }
}
