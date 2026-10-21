use cpf_core::{hash_file, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const MACHINE_ID: &str = "Nintendo DSi.cpfmachine";

#[derive(Deserialize)]
pub struct ImportFiles {
    pub bios9: String,
    pub bios7: String,
    pub bios9i: String,
    pub bios7i: String,
    pub firmware: String,
    pub nand: String,
}

#[derive(Serialize)]
pub struct SetupStatus {
    pub configured: bool,
    pub message: String,
}

pub fn workspace() -> Result<Workspace, String> {
    let appdata = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    let root = PathBuf::from(appdata).join("CPF").join("Nintendo DSi");
    fs::create_dir_all(root.join("machines")).map_err(|e| e.to_string())?;
    fs::create_dir_all(root.join("imports/libretro")).map_err(|e| e.to_string())?;
    let bundled = std::env::current_exe().map_err(|e| e.to_string())?
        .parent().ok_or("Application directory is unavailable")?.join("backend");
    for name in ["melondsds_system_compat.dll"] {
        let source = bundled.join(name);
        if !source.is_file() { return Err(format!("Required backend file is missing: {name}")); }
        let target = root.join("imports/libretro").join(name);
        if !target.is_file() || hash_file(&target)? != hash_file(&source)? {
            fs::copy(&source, &target).map_err(|e| format!("Could not install backend {name}: {e}"))?;
            if hash_file(&target)? != hash_file(&source)? { return Err(format!("Backend copy failed verification: {name}")); }
        }
    }
    Workspace::open(root)
}

pub fn status(workspace: &Workspace) -> SetupStatus {
    let id = MACHINE_ID;
    let machine = match workspace.machine_dir(id) {
        Ok(machine) => machine,
        Err(_) => return SetupStatus { configured: false, message: "Import your own DSi BIOS, firmware, and NAND to continue".into() },
    };
    let provenance = cpf_core::read_json(&machine.join("provenance.json")).ok();
    let sources_valid = provenance.as_ref().is_some_and(|record| {
        ["bios9.bin", "bios7.bin", "dsi_bios9.bin", "dsi_bios7.bin", "dsi_firmware.bin", "dsi_nand.bin"]
            .iter().all(|name| {
                let path = record.get("paths").and_then(|v| v.get(name)).and_then(|v| v.as_str())
                    .map(PathBuf::from).unwrap_or_else(|| workspace.root.join("system-files").join(name));
                fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
                    && hash_file(&path).ok().as_deref() == record.get("sha256").and_then(|v| v.get(name)).and_then(|v| v.as_str())
            })
    });
    if !sources_valid { return SetupStatus { configured: false, message: "Imported source files are missing or changed. Reimport to repair; the previous machine will be preserved in recovery.".into() }; }
    match workspace.readiness(id) {
        Ok(ready) if ready.ready_to_attempt_boot => SetupStatus { configured: true, message: "Ready to boot".into() },
        Ok(ready) => SetupStatus { configured: false, message: format!("Machine needs repair: {}. Reimport to preserve the previous machine and create a new one.", ready.missing.join(", ")) },
        Err(error) => SetupStatus { configured: false, message: format!("Machine needs repair: {error}. Reimport to preserve the previous machine and create a new one.") },
    }
}

pub fn scan_folder(folder: &Path) -> Result<serde_json::Value, String> {
    if !folder.is_absolute() || !folder.is_dir() { return Err("Select an existing folder".into()); }
    let patterns: [(&str, &[&str], &[u64]); 6] = [
        ("bios9", &["bios9.bin"], &[4096]),
        ("bios7", &["bios7.bin"], &[16384]),
        ("bios9i", &["dsi_bios9.bin", "bios9i.bin"], &[65536]),
        ("bios7i", &["dsi_bios7.bin", "bios7i.bin"], &[65536]),
        ("firmware", &["dsi_firmware.bin", "dsfirmware.bin", "firmware.bin"], &[131072, 262144, 524288]),
        ("nand", &["dsi_nand.bin", "nand.bin"], &[251658304, 268435456]),
    ];
    let mut found = serde_json::Map::new();
    for (key, names, sizes) in patterns {
        let path = names.iter().map(|name| folder.join(name)).find(|candidate| {
            fs::symlink_metadata(candidate).is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink() && sizes.contains(&meta.len()))
        });
        if let Some(path) = path { found.insert(key.into(), json!(path)); }
    }
    Ok(serde_json::Value::Object(found))
}

fn checked(path: &str, label: &str, sizes: &[u64]) -> Result<PathBuf, String> {
    let source = PathBuf::from(path.trim());
    if !source.is_absolute() { return Err(format!("{label}: select an absolute file path")); }
    let meta = fs::symlink_metadata(&source).map_err(|e| format!("{label}: {e}"))?;
    if !meta.is_file() || meta.file_type().is_symlink() || !sizes.contains(&meta.len()) {
        return Err(format!("{label}: expected a regular file of size {} bytes; found {} bytes", sizes.iter().map(u64::to_string).collect::<Vec<_>>().join(" or "), meta.len()));
    }
    source.canonicalize().map_err(|e| format!("{label}: {e}"))
}

fn copy_verified(source: &Path, target: &Path) -> Result<String, String> {
    let before = hash_file(source)?;
    fs::copy(source, target).map_err(|e| format!("Could not copy {}: {e}", source.display()))?;
    if hash_file(target)? != before { return Err("Copied file failed SHA-256 verification".into()); }
    Ok(before)
}

fn write(path: &Path, value: serde_json::Value) -> Result<(), String> {
    fs::write(path, serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn import(workspace: &Workspace, files: ImportFiles) -> Result<SetupStatus, String> {
    let existing_machine = workspace.root.join("machines").join(MACHINE_ID);
    if status(workspace).configured {
        return Err("This DSi is already configured. Keep using the existing machine.".into());
    }
    if existing_machine.exists() {
        if fs::symlink_metadata(&existing_machine).map_err(|e| e.to_string())?.file_type().is_symlink() {
            return Err("Existing machine is a link; setup cannot safely repair it".into());
        }
        if workspace.runtime_status(MACHINE_ID).is_ok_and(|v| v["running"].as_bool() == Some(true)) {
            return Err("Close the existing DSi session before repairing setup".into());
        }
    }
    let entries = [
        ("bios9.bin", "DS ARM9 BIOS", files.bios9, vec![4096]),
        ("bios7.bin", "DS ARM7 BIOS", files.bios7, vec![16384]),
        ("dsi_bios9.bin", "DSi ARM9i BIOS", files.bios9i, vec![65536]),
        ("dsi_bios7.bin", "DSi ARM7i BIOS", files.bios7i, vec![65536]),
        ("dsi_firmware.bin", "DSi firmware", files.firmware, vec![131072, 262144, 524288]),
        ("dsi_nand.bin", "DSi NAND", files.nand, vec![251658304, 268435456]),
    ];
    let checked_files = entries.iter().map(|(name, label, path, sizes)|
        checked(path, label, sizes).map(|p| ((*name).to_owned(), p)))
        .collect::<Result<Vec<_>, _>>()?;
    let nand_path = &checked_files[5].1;
    let mut nand = File::open(nand_path).map_err(|e| e.to_string())?;
    nand.seek(SeekFrom::Start(510)).map_err(|e| e.to_string())?;
    let mut signature = [0u8; 2];
    nand.read_exact(&mut signature).map_err(|e| e.to_string())?;
    nand.seek(SeekFrom::End(-64)).map_err(|e| e.to_string())?;
    let mut footer = [0u8; 16];
    nand.read_exact(&mut footer).map_err(|e| e.to_string())?;
    if signature != [0x55, 0xAA] && &footer != b"DSi eMMC CID/CPU" {
        return Err("DSi NAND: neither a plain MBR nor a recognized no$gba DSi footer was found".into());
    }

    let staging = workspace.root.join(format!(".setup-{}", uuid::Uuid::new_v4()));
    let machine = staging.join("machine");
    let result: Result<(), String> = (|| {
        for dir in ["nand", "runtime", "logs", "sd", "snapshots", "saves"] {
            fs::create_dir_all(machine.join(dir)).map_err(|e| e.to_string())?;
        }
        let mut hashes = serde_json::Map::new();
        let mut paths = serde_json::Map::new();
        for (name, source) in &checked_files {
            hashes.insert(name.clone(), json!(hash_file(source)?));
            paths.insert(name.clone(), json!(source));
        }
        let nand_hash = hashes["dsi_nand.bin"].as_str().ok_or("NAND hash missing")?;
        let virtual_nand = machine.join("nand/virtual_nand.bin");
        copy_verified(nand_path, &virtual_nand)?;
        let mut permissions = fs::metadata(&virtual_nand).map_err(|e| e.to_string())?.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&virtual_nand, permissions).map_err(|e| e.to_string())?;
        write(&machine.join("machine.json"), json!({
            "schema":"cpf.dsi.machine/1", "machine_id":uuid::Uuid::new_v4().to_string(),
            "name":"Nintendo DSi", "platform":"Nintendo DSi", "mode":"DSi",
            "nand_association":"user_imported_unverified", "virtual_nand_sha256":nand_hash,
            "virtual_sd_mode":"runtime_image", "runtime_backend":"melonDS DS libretro"
        }))?;
        write(&machine.join("provenance.json"), json!({"source":"User-supplied system dumps", "association":"unverified", "sha256":hashes, "paths":paths}))?;
        write(&machine.join("observations.json"), json!({}))?;
        write(&machine.join("nand/source.json"), json!({
            "path":nand_path,
            "calculated_sha256":nand_hash, "size_bytes":nand_path.metadata().map_err(|e| e.to_string())?.len(),
            "read_only_source":true, "label":"User-supplied NAND reference; CPF never writes to this file; association unverified"
        }))?;
        write(&machine.join("nand/titles.json"), json!({"titles":[],"note":"NAND title analysis has not run"}))?;
        write(&machine.join("sd/virtual.json"), json!({"source":"Generated by CPF","mode":"runtime_image"}))?;
        write(&machine.join("runtime/config.json"), json!({
            "virtual_sd_enabled":true,"sound_compatibility":true,
            "bios9":checked_files[0].1,
            "bios7":checked_files[1].1,
            "bios9i":checked_files[2].1,
            "bios7i":checked_files[3].1,
            "firmware":checked_files[4].1
        }))?;
        let recovery = workspace.root.join("recovery").join(format!("setup-{}", uuid::Uuid::new_v4()));
        if existing_machine.exists() {
            fs::create_dir_all(&recovery).map_err(|e| e.to_string())?;
            fs::rename(&existing_machine, recovery.join("machine"))
                .map_err(|error| format!("Could not preserve existing machine before repair: {error}"))?;
        }
        fs::rename(&machine, workspace.root.join("machines").join(MACHINE_ID)).map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_dir_all(&staging); }
    result?;
    let _ = fs::remove_dir(&staging);
    let result = status(workspace);
    if !result.configured { return Err("Import completed, but required runtime files are still missing".into()); }
    Ok(result)
}
