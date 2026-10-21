use crate::machine::{assert_runtime_stopped, atomic_json, hash_file, read_json, replace_file, Result, Workspace};
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Serialize)]
pub struct RestoredFile {
    pub status: String,
    pub sha256: Option<String>,
    pub reversal_snapshot: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CleanNandResult {
    pub virtual_nand: RestoredFile,
    pub runtime_nand: RestoredFile,
    pub runtime_firmware: RestoredFile,
}

#[derive(Debug, Serialize)]
pub struct CloneResult {
    pub machine_id: String,
    pub source_machine_id: String,
    pub copied_files: usize,
}

#[derive(Debug, Serialize)]
pub struct SaveImportResult {
    pub category: String,
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub previous_snapshot: Option<String>,
    pub format_validation: String,
}

struct PendingRestore {
    target: PathBuf,
    staged: PathBuf,
    before: String,
    clean: String,
    snapshot: PathBuf,
    manifest: PathBuf,
    reason: &'static str,
    id: String,
    committed: bool,
}
impl Drop for PendingRestore {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.staged);
            let _ = fs::remove_file(&self.snapshot);
        }
    }
}

pub(crate) struct MachineOperationLock(PathBuf);
impl MachineOperationLock {
    pub(crate) fn acquire(machine: &Path, operation: &str) -> Result<Self> {
        assert_runtime_stopped(machine)?;
        let path = machine.join("runtime/libretro/running.json");
        fs::create_dir_all(path.parent().ok_or("Runtime directory missing")?).map_err(|e| e.to_string())?;
        let value = json!({"pid":std::process::id(),"state":operation,"started":now()?});
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path)
            .map_err(|e| format!("Machine is busy: {e}"))?;
        if let Err(error) = writeln!(file, "{value}").and_then(|_| file.sync_all()) {
            let _ = fs::remove_file(&path);
            return Err(error.to_string());
        }
        Ok(Self(path))
    }
}
impl Drop for MachineOperationLock {
    fn drop(&mut self) { let _ = fs::remove_file(&self.0); }
}

fn now() -> Result<String> {
    OffsetDateTime::now_utc().format(&Rfc3339).map_err(|e| e.to_string())
}
fn token() -> String {
    format!("{}-{}", std::process::id(), OffsetDateTime::now_utc().unix_timestamp_nanos())
}
fn regular(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(format!("Expected a regular file: {}", path.display()));
    }
    Ok(())
}
fn copy_verified(source: &Path, destination: &Path, expected: &str) -> Result<()> {
    regular(source)?;
    let mut from = File::open(source).map_err(|e| e.to_string())?;
    let mut to = OpenOptions::new().write(true).create_new(true).open(destination)
        .map_err(|e| format!("{}: {e}", destination.display()))?;
    io::copy(&mut from, &mut to).map_err(|e| e.to_string())?;
    to.sync_all().map_err(|e| e.to_string())?;
    if hash_file(destination)? != expected {
        return Err(format!("Copy verification failed: {}", destination.display()));
    }
    Ok(())
}
fn append(path: &Path, value: &Value) -> Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    writeln!(file, "{value}").map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())
}
fn relative(machine: &Path, path: &Path) -> Result<String> {
    Ok(path.strip_prefix(machine).map_err(|e| e.to_string())?
        .to_string_lossy().replace('\\', "/"))
}
fn relocate(value: &mut Value, old_prefix: &str, new_prefix: &str) {
    match value {
        Value::String(text) => {
            if let Some(prefix) = text.get(..old_prefix.len()).filter(|prefix| prefix.replace('/', "\\").eq_ignore_ascii_case(&old_prefix.replace('/', "\\"))) {
                *text = format!("{}{}", new_prefix, &text[prefix.len()..]);
            }
        }
        Value::Array(items) => for item in items { relocate(item, old_prefix, new_prefix); },
        Value::Object(fields) => for item in fields.values_mut() { relocate(item, old_prefix, new_prefix); },
        _ => {}
    }
}
fn prepare(
    machine: &Path, target: &Path, source: &Path, clean: &str,
    category: &str, manifest: &Path, reason: &'static str,
) -> Result<(RestoredFile, Option<PendingRestore>)> {
    if !target.exists() {
        return Ok((RestoredFile { status: "not_created".into(), sha256: None, reversal_snapshot: None }, None));
    }
    regular(target)?;
    let before = hash_file(target)?;
    if before == clean {
        return Ok((RestoredFile { status: "already_clean".into(), sha256: Some(before), reversal_snapshot: None }, None));
    }
    let id = token();
    let snapshot_dir = machine.join("snapshots").join(category);
    fs::create_dir_all(&snapshot_dir).map_err(|e| e.to_string())?;
    let snapshot = snapshot_dir.join(format!("before-restore-{id}.bin"));
    let staged = target.with_file_name(format!(".restore-{id}.bin"));
    let result: Result<()> = (|| {
        copy_verified(target, &snapshot, &before)?;
        copy_verified(source, &staged, clean)?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&staged); let _ = fs::remove_file(&snapshot); }
    result?;
    let item = RestoredFile { status: "restored".into(), sha256: Some(clean.to_owned()), reversal_snapshot: Some(relative(machine, &snapshot)?) };
    Ok((item, Some(PendingRestore { target: target.to_path_buf(), staged, before, clean: clean.to_owned(), snapshot, manifest: manifest.to_path_buf(), reason, id, committed: false })))
}
fn commit(machine: &Path, mut plan: PendingRestore) -> Result<()> {
    let transaction = json!({
        "modification_id": plan.id,
        "target": relative(machine, &plan.target)?,
        "original_hash": plan.before,
        "modified_hash": plan.clean,
        "reason": plan.reason,
        "tool_version": "CPF Rust 0.1",
        "timestamp": now()?,
        "reversible": true,
        "reversal_snapshot": relative(machine, &plan.snapshot)?,
        "source": "Generated by CPF"
    });
    let mut prepared = transaction.clone();
    prepared["state"] = json!("prepared");
    append(&plan.manifest, &prepared)?;
    replace_file(&plan.staged, &plan.target).map_err(|e| e.to_string())?;
    plan.committed = true;
    let mut committed = transaction.clone();
    committed["state"] = json!("committed");
    append(&plan.manifest, &committed)?;
    let mut event = transaction;
    event["kind"] = json!(if plan.reason == "Restore Clean Working Firmware" { "FIRMWARE_MODIFIED" } else { "NAND_MODIFIED" });
    append(&machine.join("logs/events.jsonl"), &event)
}

impl Workspace {
    pub fn import_save(&self, id: &str, category: &str, source: &Path, name: Option<&str>) -> Result<SaveImportResult> {
        let machine = self.machine_dir(id)?;
        if !["cartridge", "dsiware", "system", "sd"].contains(&category) {
            return Err("Save category must be cartridge, dsiware, system, or sd".into());
        }
        if !source.is_absolute() { return Err("Save source path must be absolute".into()); }
        regular(source)?;
        let size = fs::metadata(source).map_err(|e| e.to_string())?.len();
        if size == 0 { return Err("Save source must not be empty".into()); }
        let filename = name.filter(|name| !name.trim().is_empty()).map(str::to_owned)
            .or_else(|| source.file_name().map(|name| name.to_string_lossy().into_owned()))
            .ok_or("Save filename missing")?;
        let filename = filename.trim();
        if filename.is_empty() || filename == "." || filename == ".." || filename.len() > 255
            || filename.ends_with(' ') || filename.ends_with('.')
            || filename.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
            return Err("Save filename must be one plain filename".into());
        }
        let stem = filename.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit() && stem.as_bytes()[3] != b'0') {
            return Err("Save filename is reserved by Windows".into());
        }
        let directory = machine.join("saves").join(category);
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let target = directory.join(filename);
        let source_hash = hash_file(source)?;
        let mut previous_snapshot = None;
        if fs::symlink_metadata(&target).is_ok() {
            regular(&target)?;
            let previous_hash = hash_file(&target)?;
            let snapshots = machine.join("saves/snapshots");
            fs::create_dir_all(&snapshots).map_err(|e| e.to_string())?;
            let snapshot = snapshots.join(format!("{}-{filename}", uuid::Uuid::new_v4()));
            copy_verified(&target, &snapshot, &previous_hash)?;
            previous_snapshot = Some(relative(&machine, &snapshot)?);
        }
        let staged = directory.join(format!(".import-{}", uuid::Uuid::new_v4()));
        if let Err(error) = copy_verified(source, &staged, &source_hash) {
            let _ = fs::remove_file(&staged);
            return Err(error);
        }
        if let Err(error) = replace_file(&staged, &target) {
            let _ = fs::remove_file(&staged);
            return Err(error.to_string());
        }
        let result = SaveImportResult {
            category: category.into(), path: relative(&machine, &target)?,
            size_bytes: size, sha256: source_hash,
            previous_snapshot, format_validation: "not_available".into(),
        };
        append(&machine.join("logs/events.jsonl"), &json!({
            "kind":"SAVE_CHANGED", "timestamp":now()?, "category":result.category,
            "source":"Imported user-owned save", "path":result.path,
            "size_bytes":result.size_bytes, "sha256":result.sha256,
            "previous_snapshot":result.previous_snapshot,
            "format_validation":"not_available", "filename_alone_does_not_validate_save":true
        }))?;
        Ok(result)
    }

    pub fn clone_machine(&self, id: &str, new_name: &str) -> Result<CloneResult> {
        let source = self.machine_dir(id)?;
        let _operation_lock = MachineOperationLock::acquire(&source, "cloning")?;
        let name = new_name.trim();
        if name.is_empty() || name.len() > 64 || !name.starts_with(|c: char| c.is_ascii_alphanumeric())
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-')) {
            return Err("Machine name must be 1–64 letters, numbers, spaces, underscores, or hyphens".into());
        }
        let machine_id = format!("{name}.cpfmachine");
        let base = self.root.join("machines");
        let destination = base.join(&machine_id);
        if destination.exists() { return Err("A machine with that name already exists".into()); }
        let staging = base.join(format!(".cloning-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let result: Result<usize> = (|| {
            let mut pending = vec![source.clone()];
            let mut count = 0;
            while let Some(directory) = pending.pop() {
                for entry in fs::read_dir(&directory).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    if path == source.join("runtime/libretro/running.json") { continue; }
                    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                    if metadata.file_type().is_symlink() { return Err(format!("Linked machine entry: {}", path.display())); }
                    let relative = path.strip_prefix(&source).map_err(|e| e.to_string())?;
                    let target = staging.join(relative);
                    if metadata.is_dir() {
                        fs::create_dir(&target).map_err(|e| e.to_string())?;
                        pending.push(path);
                    } else if metadata.is_file() {
                        let digest = hash_file(&path)?;
                        copy_verified(&path, &target, &digest)?;
                        count += 1;
                    } else { return Err(format!("Unsupported machine entry: {}", path.display())); }
                }
            }
            let mut profile = read_json(&staging.join("machine.json"))?;
            let prior_id = profile.get("machine_id").and_then(Value::as_str).ok_or("Source machine ID missing")?.to_owned();
            profile["machine_id"] = json!(uuid::Uuid::new_v4().to_string());
            profile["name"] = json!(name);
            profile["created_at"] = json!(now()?);
            profile["cloned_from_machine_id"] = json!(prior_id.clone());
            atomic_json(&staging.join("machine.json"), &profile)?;
            let old_prefix = format!("{}{}", source.display(), std::path::MAIN_SEPARATOR);
            let new_prefix = format!("{}{}", destination.display(), std::path::MAIN_SEPARATOR);
            for relative in ["runtime/config.json", "runtime/user_boot_files.json"] {
                let path = staging.join(relative);
                if path.is_file() {
                    let mut value = read_json(&path)?;
                    relocate(&mut value, &old_prefix, &new_prefix);
                    atomic_json(&path, &value)?;
                }
            }
            let virtual_nand = staging.join("nand/virtual_nand.bin");
            let expected = profile.get("virtual_nand_sha256").and_then(Value::as_str).ok_or("Virtual NAND hash missing")?;
            if hash_file(&virtual_nand)? != expected { return Err("Cloned virtual NAND hash mismatch".into()); }
            append(&staging.join("logs/events.jsonl"), &json!({
                "timestamp":now()?, "kind":"MACHINE_CLONED", "source":"Generated by CPF",
                "from_machine_id":prior_id
            }))?;
            fs::rename(&staging, &destination).map_err(|e| e.to_string())?;
            Ok(count)
        })();
        if result.is_err() && staging.starts_with(&base)
            && staging.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".cloning-")) {
            let _ = fs::remove_dir_all(&staging);
        }
        Ok(CloneResult { machine_id, source_machine_id: id.to_owned(), copied_files: result? })
    }

    pub fn restore_clean_nand(&self, id: &str) -> Result<CleanNandResult> {
        let machine = self.machine_dir(id)?;
        let _operation_lock = MachineOperationLock::acquire(&machine, "restoring")?;
        let mut profile = read_json(&machine.join("machine.json"))?;
        let source_info = read_json(&machine.join("nand/source.json"))?;
        let source_path = source_info.get("path").and_then(Value::as_str).ok_or("NAND source path missing")?;
        let source = PathBuf::from(source_path);
        if !source.is_absolute() { return Err("NAND source path must be absolute".into()); }
        regular(&source)?;
        let clean = source_info.get("calculated_sha256").and_then(Value::as_str).ok_or("NAND source hash missing")?;
        if hash_file(&source)? != clean { return Err("Immutable NAND source hash changed; restore aborted".into()); }
        let virtual_nand = machine.join("nand/virtual_nand.bin");
        regular(&virtual_nand)?;
        let current = hash_file(&virtual_nand)?;
        if profile.get("virtual_nand_sha256").and_then(Value::as_str) != Some(&current) {
            return Err("Virtual NAND hash changed outside CPF; restore aborted".into());
        }
        let (virtual_result, virtual_plan) = prepare(&machine, &virtual_nand, &source, clean,
            "nand", &machine.join("nand/modifications.jsonl"), "Restore Clean NAND")?;
        let runtime_nand = machine.join("runtime/libretro/system/melonDS DS/dsi_nand.bin");
        let (runtime_result, runtime_plan) = prepare(&machine, &runtime_nand, &source, clean,
            "nand", &machine.join("nand/modifications.jsonl"), "Restore Clean NAND")?;
        let firmware = self.boot_files(id)?.into_iter().find(|(name, _)| name == "dsi_firmware.bin").and_then(|(_, path)| path);
        let runtime_firmware = machine.join("runtime/libretro/system/melonDS DS/dsi_firmware.bin");
        let (firmware_result, firmware_plan, firmware_hash) = if let Some(source) = firmware {
            regular(&source)?;
            let digest = hash_file(&source)?;
            let (result, plan) = prepare(&machine, &runtime_firmware, &source, &digest,
                "firmware", &machine.join("runtime/firmware_modifications.jsonl"), "Restore Clean Working Firmware")?;
            (result, plan, Some(digest))
        } else {
            (RestoredFile { status: "source_missing".into(), sha256: None, reversal_snapshot: None }, None, None)
        };
        if let Some(plan) = virtual_plan {
            commit(&machine, plan)?;
            profile["virtual_nand_sha256"] = json!(clean);
            atomic_json(&machine.join("machine.json"), &profile)?;
        }
        if let Some(plan) = runtime_plan { commit(&machine, plan)?; }
        if let Some(plan) = firmware_plan {
            commit(&machine, plan)?;
            atomic_json(&machine.join("runtime/libretro/firmware_base.json"),
                &json!({"base_sha256": firmware_hash, "source": "User-owned boot file"}))?;
        }
        if hash_file(&source)? != clean { return Err("NAND source changed during restore".into()); }
        Ok(CleanNandResult { virtual_nand: virtual_result, runtime_nand: runtime_result, runtime_firmware: firmware_result })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_uses_verified_sources_and_snapshots_only_generated_files() {
        let root = std::env::temp_dir().join(format!("cpf-restore-test-{}", token()));
        let machine = root.join("machines/Test DSi.cpfmachine");
        let nand_dir = machine.join("nand");
        let runtime_dir = machine.join("runtime/libretro/system/melonDS DS");
        fs::create_dir_all(&nand_dir).unwrap();
        fs::create_dir_all(&runtime_dir).unwrap();
        fs::create_dir_all(machine.join("logs")).unwrap();
        let source = root.join("source-nand.bin");
        fs::write(&source, b"clean NAND source").unwrap();
        let source_hash = hash_file(&source).unwrap();
        fs::write(nand_dir.join("virtual_nand.bin"), b"modified virtual NAND").unwrap();
        let virtual_hash = hash_file(&nand_dir.join("virtual_nand.bin")).unwrap();
        fs::write(runtime_dir.join("dsi_nand.bin"), b"modified runtime NAND").unwrap();
        let firmware = root.join("firmware.bin");
        fs::write(&firmware, vec![3u8; 131072]).unwrap();
        fs::write(runtime_dir.join("dsi_firmware.bin"), vec![4u8; 131072]).unwrap();
        atomic_json(&nand_dir.join("source.json"), &json!({"path":source,"calculated_sha256":source_hash})).unwrap();
        atomic_json(&machine.join("machine.json"), &json!({"virtual_nand_sha256":virtual_hash})).unwrap();
        atomic_json(&machine.join("runtime/config.json"), &json!({"firmware":firmware})).unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        let first = workspace.restore_clean_nand("Test DSi.cpfmachine").unwrap();
        assert_eq!(first.virtual_nand.status, "restored");
        assert_eq!(first.runtime_nand.status, "restored");
        assert_eq!(first.runtime_firmware.status, "restored");
        assert_eq!(hash_file(&source).unwrap(), source_hash);
        assert_eq!(hash_file(&nand_dir.join("virtual_nand.bin")).unwrap(), source_hash);
        assert_eq!(hash_file(&runtime_dir.join("dsi_nand.bin")).unwrap(), source_hash);
        assert_eq!(hash_file(&runtime_dir.join("dsi_firmware.bin")).unwrap(), hash_file(&firmware).unwrap());
        assert_eq!(read_json(&machine.join("machine.json")).unwrap()["virtual_nand_sha256"], source_hash);
        assert!(machine.join(first.virtual_nand.reversal_snapshot.unwrap()).is_file());
        assert!(machine.join(first.runtime_nand.reversal_snapshot.unwrap()).is_file());
        assert!(machine.join(first.runtime_firmware.reversal_snapshot.unwrap()).is_file());
        let second = workspace.restore_clean_nand("Test DSi.cpfmachine").unwrap();
        assert_eq!(second.virtual_nand.status, "already_clean");
        assert_eq!(second.runtime_nand.status, "already_clean");
        assert_eq!(second.runtime_firmware.status, "already_clean");
        fs::write(&source, b"source changed").unwrap();
        assert!(workspace.restore_clean_nand("Test DSi.cpfmachine").unwrap_err().contains("source hash changed"));
        assert_eq!(hash_file(&nand_dir.join("virtual_nand.bin")).unwrap(), source_hash);
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clone_copies_generated_machine_and_keeps_external_source_reference() {
        let root = std::env::temp_dir().join(format!("cpf-clone-test-{}", token()));
        let source = root.join("machines/Source DSi.cpfmachine");
        fs::create_dir_all(source.join("nand")).unwrap();
        fs::create_dir_all(source.join("runtime")).unwrap();
        fs::create_dir_all(source.join("logs")).unwrap();
        fs::write(source.join("nand/virtual_nand.bin"), b"generated NAND").unwrap();
        let hash = hash_file(&source.join("nand/virtual_nand.bin")).unwrap();
        atomic_json(&source.join("machine.json"), &json!({
            "machine_id":"original-id", "name":"Source DSi", "virtual_nand_sha256":hash
        })).unwrap();
        let external = root.join("user-source.bin");
        fs::write(&external, b"immutable user source").unwrap();
        atomic_json(&source.join("nand/source.json"), &json!({"path":external})).unwrap();
        atomic_json(&source.join("runtime/config.json"), &json!({
            "generated_path":source.join("nand/virtual_nand.bin"), "source_path":external
        })).unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        assert!(workspace.clone_machine("Source DSi.cpfmachine", "../escape").is_err());
        let result = workspace.clone_machine("Source DSi.cpfmachine", "Experiment").unwrap();
        assert_eq!(result.machine_id, "Experiment.cpfmachine");
        assert!(result.copied_files >= 4);
        let clone = root.join("machines/Experiment.cpfmachine");
        assert!(!clone.join("runtime/libretro/running.json").exists());
        assert!(!source.join("runtime/libretro/running.json").exists());
        let profile = read_json(&clone.join("machine.json")).unwrap();
        assert_eq!(profile["cloned_from_machine_id"], "original-id");
        assert_ne!(profile["machine_id"], "original-id");
        assert_eq!(hash_file(&clone.join("nand/virtual_nand.bin")).unwrap(), hash);
        assert_eq!(read_json(&clone.join("nand/source.json")).unwrap()["path"], external.to_string_lossy().as_ref());
        let config = read_json(&clone.join("runtime/config.json")).unwrap();
        assert_eq!(config["generated_path"].as_str().unwrap().replace('/', "\\"), clone.join("nand/virtual_nand.bin").to_string_lossy().replace('/', "\\"));
        assert_eq!(config["source_path"], external.to_string_lossy().as_ref());
        assert!(workspace.clone_machine("Source DSi.cpfmachine", "Experiment").is_err());
        assert_eq!(fs::read(&external).unwrap(), b"immutable user source");
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_replacement_preserves_prior_bytes_and_import_source() {
        let root = std::env::temp_dir().join(format!("cpf-save-test-{}", token()));
        let machine = root.join("machines/Test DSi.cpfmachine");
        fs::create_dir_all(machine.join("saves/cartridge")).unwrap();
        fs::create_dir_all(machine.join("logs")).unwrap();
        let source = root.join("new.sav");
        fs::write(&source, b"new user save").unwrap();
        fs::write(machine.join("saves/cartridge/game.sav"), b"old staged save").unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        assert!(workspace.import_save("Test DSi.cpfmachine", "bad", &source, None).is_err());
        assert!(workspace.import_save("Test DSi.cpfmachine", "cartridge", &source, Some("../escape")).is_err());
        let result = workspace.import_save("Test DSi.cpfmachine", "cartridge", &source, Some("game.sav")).unwrap();
        assert_eq!(result.format_validation, "not_available");
        assert_eq!(fs::read(machine.join("saves/cartridge/game.sav")).unwrap(), b"new user save");
        assert_eq!(fs::read(machine.join(result.previous_snapshot.unwrap())).unwrap(), b"old staged save");
        assert_eq!(fs::read(&source).unwrap(), b"new user save");
        assert!(fs::read_to_string(machine.join("logs/events.jsonl")).unwrap().contains("SAVE_CHANGED"));
        assert!(root.starts_with(std::env::temp_dir()));
        fs::remove_dir_all(root).unwrap();
    }
}
