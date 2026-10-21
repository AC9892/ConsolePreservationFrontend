use crate::machine::{atomic_json, hash_file, read_json, Result, Workspace};
use crate::preservation::MachineOperationLock;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct CartridgeInfo {
    pub title_from_header: Option<String>,
    pub game_code: String,
    pub maker_code: String,
    pub header_identifier_valid: bool,
    pub unit_code: u8,
    pub icon_offset_bytes: Option<u64>,
    pub banner_title_english: Option<String>,
    pub banner_icon_available: bool,
    pub size_bytes: u64,
    pub sha256: String,
    pub header_parsed: bool,
    pub full_rom_validated: bool,
    pub save_validated: bool,
}

fn regular_nds(source: &Path) -> Result<u64> {
    if !source.is_absolute() || !source.extension().is_some_and(|v| v.eq_ignore_ascii_case("nds")) {
        return Err("Select an absolute .nds file path".into());
    }
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("Cartridge image must be a regular file".into());
    }
    if metadata.len() < 0x200 { return Err("Image is too small for a DS cartridge header".into()); }
    Ok(metadata.len())
}

fn ascii_field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_owned()
}

pub fn inspect_nds(source: &Path) -> Result<CartridgeInfo> {
    let size = regular_nds(source)?;
    let mut file = File::open(source).map_err(|e| e.to_string())?;
    let mut header = [0u8; 0x200];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    let game_code = ascii_field(&header[12..16]);
    let maker_code = ascii_field(&header[16..18]);
    let icon_offset = u32::from_le_bytes(header[0x68..0x6c].try_into().unwrap()) as u64;
    let banner_available = icon_offset >= 0x200 && icon_offset.checked_add(0x840).is_some_and(|end| end <= size);
    let banner_title = if banner_available {
        file.seek(SeekFrom::Start(icon_offset + 0x340)).map_err(|e| e.to_string())?;
        let mut title = [0u8; 0x100];
        file.read_exact(&mut title).map_err(|e| e.to_string())?;
        let units: Vec<u16> = title.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|unit| *unit != 0).collect();
        let value = String::from_utf16_lossy(&units).trim().to_owned();
        (!value.is_empty()).then_some(value)
    } else { None };
    let title = ascii_field(&header[0..12]);
    Ok(CartridgeInfo {
        title_from_header: (!title.is_empty()).then_some(title),
        game_code: game_code.clone(), maker_code: maker_code.clone(),
        header_identifier_valid: game_code.len() == 4 && maker_code.len() == 2
            && game_code.bytes().chain(maker_code.bytes()).all(|b| b.is_ascii_alphanumeric()),
        unit_code: header[0x12], icon_offset_bytes: (icon_offset > 0 && icon_offset < size).then_some(icon_offset),
        banner_title_english: banner_title, banner_icon_available: banner_available,
        size_bytes: size, sha256: hash_file(source)?, header_parsed: true,
        full_rom_validated: false, save_validated: false,
    })
}

fn event(machine: &Path, value: &Value) -> Result<()> {
    let path = machine.join("logs/events.jsonl");
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
    writeln!(file, "{value}").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}

fn now() -> Result<String> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(|e| e.to_string())
}

impl Workspace {
    pub fn cartridge_slot(&self, id: &str) -> Result<Value> {
        let machine = self.machine_dir(id)?;
        let slot = machine.join("runtime/cartridge.json");
        if slot.exists() { read_json(&slot) } else { Ok(json!({"inserted": false, "image": null})) }
    }

    pub fn insert_cartridge(&self, id: &str, source: &Path) -> Result<Value> {
        let machine = self.machine_dir(id)?;
        let _lock = MachineOperationLock::acquire(&machine, "cartridge_import")?;
        if self.cartridge_slot(id)?["inserted"].as_bool() == Some(true) {
            return Err("Eject the current cartridge first".into());
        }
        let info = inspect_nds(source)?;
        let directory = machine.join("runtime/cartridges");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let destination = directory.join(format!("{}.nds", Uuid::new_v4()));
        let copied: Result<()> = (|| {
            fs::copy(source, &destination).map_err(|e| e.to_string())?;
            if hash_file(&destination)? != info.sha256 { return Err("Cartridge import hash mismatch".into()); }
            Ok(())
        })();
        if let Err(error) = copied {
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        let record = json!({
            "inserted": true,
            "image": format!("runtime/cartridges/{}", destination.file_name().unwrap().to_string_lossy()),
            "source_path": source.to_string_lossy(),
            "source": "User-owned cartridge image",
            "metadata": info,
            "runtime_inserted": false
        });
        if let Err(error) = atomic_json(&machine.join("runtime/cartridge.json"), &record) {
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        event(&machine, &json!({"kind":"CARTRIDGE_INSERTED", "timestamp":now()?,
            "source":"User-owned cartridge image", "runtime_inserted":false,
            "game_code":info.game_code, "sha256":info.sha256}))?;
        Ok(record)
    }

    pub fn eject_cartridge(&self, id: &str) -> Result<Value> {
        let machine = self.machine_dir(id)?;
        let _lock = MachineOperationLock::acquire(&machine, "cartridge_eject")?;
        let prior = self.cartridge_slot(id)?;
        if prior["inserted"].as_bool() != Some(true) {
            return Ok(json!({"inserted":false,"status":"already_empty"}));
        }
        atomic_json(&machine.join("runtime/cartridge.json"), &json!({"inserted":false,"image":null}))?;
        event(&machine, &json!({"kind":"CARTRIDGE_EJECTED", "timestamp":now()?,
            "source":"Generated by CPF", "runtime_inserted":false,
            "prior_sha256":prior["metadata"]["sha256"]}))?;
        Ok(json!({"inserted":false,"ejected":prior["metadata"]}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cartridge_roundtrip_preserves_user_source_and_requires_stopped_runtime() {
        let root = std::env::temp_dir().join(format!("cpf-cartridge-{}", Uuid::new_v4()));
        let machine = root.join("machines/Test DSi.cpfmachine");
        fs::create_dir_all(machine.join("runtime/libretro")).unwrap();
        let source = root.join("game.nds");
        let mut bytes = vec![0u8; 0xa40];
        bytes[..9].copy_from_slice(b"Test Game");
        bytes[12..16].copy_from_slice(b"TEST");
        bytes[16..18].copy_from_slice(b"01");
        bytes[0x68..0x6c].copy_from_slice(&0x200u32.to_le_bytes());
        for (i, unit) in "English Title".encode_utf16().enumerate() {
            bytes[0x540 + 2*i..0x542 + 2*i].copy_from_slice(&unit.to_le_bytes());
        }
        fs::write(&source, &bytes).unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        let inspected = inspect_nds(&source).unwrap();
        assert_eq!(inspected.banner_title_english.as_deref(), Some("English Title"));
        assert!(!inspected.full_rom_validated);
        atomic_json(&machine.join("runtime/libretro/running.json"),
            &json!({"pid": std::process::id(), "state":"running"})).unwrap();
        assert!(workspace.insert_cartridge("Test DSi.cpfmachine", &source).is_err());
        fs::remove_file(machine.join("runtime/libretro/running.json")).unwrap();
        let slot = workspace.insert_cartridge("Test DSi.cpfmachine", &source).unwrap();
        assert_eq!(slot["metadata"]["sha256"], inspected.sha256);
        assert!(workspace.insert_cartridge("Test DSi.cpfmachine", &source).is_err());
        assert!(workspace.cartridge_slot("Test DSi.cpfmachine").unwrap()["inserted"] == true);
        let ejected = workspace.eject_cartridge("Test DSi.cpfmachine").unwrap();
        assert_eq!(ejected["inserted"], false);
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert!(!machine.join("runtime/libretro/running.json").exists());
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
