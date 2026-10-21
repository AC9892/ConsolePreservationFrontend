use crate::machine::{
    assert_runtime_stopped, atomic_json, hash_file, read_json, Result, Workspace,
};
use crate::preservation::MachineOperationLock;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Serialize)]
pub struct StorageComparison {
    pub historical_source: String,
    pub current_source: String,
    pub current_contents_indexed: bool,
    pub content_equality_verified: bool,
    pub historical_entry_count: usize,
    pub current_entry_count: Option<usize>,
    pub same_path_type_size: Option<usize>,
    pub missing_paths: Vec<String>,
    pub extra_paths: Vec<String>,
    pub different_type_or_size: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SdSnapshot {
    pub snapshot_id: String,
    pub created_at: String,
    pub image_sha256: String,
    pub index_sha256: Option<String>,
    pub host_entry_count: Option<usize>,
    pub manifest_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct SdSnapshotSummary {
    pub snapshot_id: String,
    pub created_at: String,
    pub schema: String,
}

#[derive(Debug, Serialize)]
pub struct SdRestoreResult {
    pub restored_snapshot: String,
    pub previous_snapshot: Option<String>,
    pub image_sha256: String,
}

#[derive(Debug, Serialize)]
pub struct SdImportResult {
    pub staged_entries: usize,
    pub staged_bytes: u64,
    pub prior_snapshot: String,
    pub applies_at_next_boot: bool,
}

#[derive(Clone)]
struct Entry {
    kind: String,
    size: Option<u64>,
}

fn walk(root: &Path) -> Result<BTreeMap<String, Entry>> {
    if fs::symlink_metadata(root)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Linked SD root is not supported".into());
    }
    let mut entries = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        for item in fs::read_dir(&directory).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let path = item.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Linked SD path is not supported: {}",
                    path.display()
                ));
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if metadata.is_dir() {
                entries.insert(
                    relative,
                    Entry {
                        kind: "directory".into(),
                        size: None,
                    },
                );
                stack.push(path);
            } else if metadata.is_file() {
                entries.insert(
                    relative,
                    Entry {
                        kind: "file".into(),
                        size: Some(metadata.len()),
                    },
                );
            } else {
                return Err(format!("Unsupported SD entry: {}", path.display()));
            }
        }
    }
    Ok(entries)
}

fn copied_tree(source: &Path, target: &Path) -> Result<BTreeMap<String, Value>> {
    let entries = walk(source)?;
    fs::create_dir(target).map_err(|e| e.to_string())?;
    let mut recorded = BTreeMap::new();
    for (relative, item) in entries {
        let from = source.join(&relative);
        let to = target.join(&relative);
        if item.kind == "directory" {
            fs::create_dir_all(&to).map_err(|e| e.to_string())?;
            recorded.insert(relative, json!({"type": "directory"}));
        } else {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
            let before = hash_file(&from)?;
            let after = hash_file(&to)?;
            if before != after {
                return Err(format!("SD file changed while copying: {relative}"));
            }
            recorded.insert(
                relative,
                json!({"type": "file", "size_bytes": item.size, "sha256": after}),
            );
        }
    }
    Ok(recorded)
}

fn indexed_tree(root: &Path) -> Result<BTreeMap<String, Value>> {
    let mut recorded = BTreeMap::new();
    for (relative, item) in walk(root)? {
        let value = if item.kind == "directory" {
            json!({"type":"directory"})
        } else {
            json!({"type":"file", "size_bytes":item.size,
                "sha256":hash_file(&root.join(&relative))?})
        };
        recorded.insert(relative, value);
    }
    Ok(recorded)
}

fn overlaps(a: &Path, b: &Path) -> bool {
    fn key(path: &Path) -> String {
        path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase()
    }
    let a = key(a);
    let b = key(b);
    a == b || a.starts_with(&(b.clone() + "\\")) || b.starts_with(&(a + "\\"))
}

fn now() -> Result<String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|e| e.to_string())
}

fn manifest_hash(value: &Value) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn snapshot_path(machine: &Path, snapshot_id: &str) -> Result<PathBuf> {
    if snapshot_id.is_empty()
        || snapshot_id == "."
        || snapshot_id == ".."
        || !snapshot_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("Invalid SD snapshot ID".into());
    }
    let root = machine.join("sd/snapshots").join(snapshot_id);
    let metadata = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("SD snapshot must be a real directory".into());
    }
    Ok(root)
}

fn file_matches(path: &Path, expected: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "Snapshot file is not a regular file: {}",
            path.display()
        ));
    }
    if hash_file(path)? != expected {
        return Err(format!("Snapshot hash mismatch: {}", path.display()));
    }
    Ok(())
}

fn verify_image_snapshot(machine: &Path, snapshot_id: &str) -> Result<Value> {
    let root = snapshot_path(machine, snapshot_id)?;
    let mut manifest = read_json(&root.join("snapshot.json"))?;
    let expected = manifest
        .get("manifest_sha256")
        .and_then(Value::as_str)
        .ok_or("Snapshot manifest hash missing")?
        .to_owned();
    manifest
        .as_object_mut()
        .ok_or("Snapshot manifest is invalid")?
        .remove("manifest_sha256");
    if manifest_hash(&manifest)? != expected {
        return Err("Snapshot manifest hash mismatch".into());
    }
    if manifest.get("schema").and_then(Value::as_str) != Some("cpf.dsi.sd-image-snapshot/1") {
        return Err("This restore control supports runtime SD image snapshots only".into());
    }
    if manifest.get("snapshot_id").and_then(Value::as_str) != Some(snapshot_id) {
        return Err("Snapshot ID does not match its directory".into());
    }
    let image_hash = manifest
        .get("image_sha256")
        .and_then(Value::as_str)
        .ok_or("Snapshot image hash missing")?;
    file_matches(&root.join("dsi_sd_card.bin"), image_hash)?;
    if let Some(index_hash) = manifest.get("index_sha256").and_then(Value::as_str) {
        file_matches(&root.join("dsi_sd_card.bin.idx"), index_hash)?;
    }
    if let Some(host_files) = manifest.get("host_files").and_then(Value::as_object) {
        verify_host_tree(&root.join("host_files"), host_files)?;
    }
    Ok(manifest)
}

fn verify_host_tree(root: &Path, expected_files: &serde_json::Map<String, Value>) -> Result<()> {
    let actual = walk(root)?;
    if actual.len() != expected_files.len() {
        return Err("Snapshot host files changed".into());
    }
    for (path, item) in actual {
        let expected = expected_files
            .get(&path)
            .ok_or("Snapshot host file list changed")?;
        if expected.get("type").and_then(Value::as_str) != Some(item.kind.as_str())
            || (item.kind == "file"
                && (expected.get("size_bytes").and_then(Value::as_u64) != item.size
                    || expected.get("sha256").and_then(Value::as_str)
                        != Some(hash_file(&root.join(&path))?.as_str())))
        {
            return Err(format!("Snapshot host file changed: {path}"));
        }
    }
    Ok(())
}

fn copy_verified(source: &Path, target: &Path, expected: &str) -> Result<()> {
    fs::copy(source, target).map_err(|e| e.to_string())?;
    file_matches(target, expected)
}

fn checked_generated_path(machine: &Path, path: &Path) -> Result<()> {
    let base = machine.canonicalize().map_err(|e| e.to_string())?;
    let resolved = if path.exists() {
        path.canonicalize().map_err(|e| e.to_string())?
    } else {
        path.parent()
            .ok_or("Invalid generated path")?
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(path.file_name().ok_or("Invalid generated path")?)
    };
    if resolved == base || !resolved.starts_with(&base) {
        return Err(format!(
            "Generated target is outside the machine: {}",
            path.display()
        ));
    }
    Ok(())
}

fn historical_entries(inventory: &Value) -> Result<BTreeMap<String, Entry>> {
    let mut found = BTreeMap::new();
    let items = inventory
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("SD inventory has no entries")?;
    for item in items {
        let raw = item
            .get("path")
            .and_then(Value::as_str)
            .ok_or("SD inventory path is missing")?;
        let path = raw
            .strip_prefix("sd:/")
            .ok_or("Invalid historical SD path")?;
        let kind = item
            .get("type")
            .and_then(Value::as_str)
            .ok_or("SD inventory type is missing")?;
        let size = item.get("size_bytes").and_then(Value::as_u64);
        found.insert(
            path.to_owned(),
            Entry {
                kind: kind.into(),
                size,
            },
        );
    }
    Ok(found)
}

impl Workspace {
    pub fn import_runtime_sd_backup(&self, id: &str, source: &Path) -> Result<SdImportResult> {
        let machine = self.machine_dir(id)?;
        if !source.is_absolute() || source.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err("SD backup path must be absolute without parent traversal".into());
        }
        let source_meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if source_meta.file_type().is_symlink() || !source_meta.is_dir() {
            return Err("SD backup must be a real directory".into());
        }
        if overlaps(source, &machine) {
            return Err("SD backup cannot be inside or contain the machine".into());
        }
        let profile = read_json(&machine.join("machine.json"))?;
        if profile["virtual_sd_mode"].as_str() != Some("runtime_image") {
            return Err("This import requires a generated runtime SD image".into());
        }
        assert_runtime_stopped(&machine)?;
        let base = machine.join("runtime/libretro/save/melonDS DS");
        let image = base.join("dsi_sd_card.bin");
        let image_meta = fs::symlink_metadata(&image).map_err(|_| "Start the DSi once to create its virtual SD image before importing files".to_string())?;
        if image_meta.file_type().is_symlink() || !image_meta.is_file() {
            return Err("Generated SD image must be a regular file".into());
        }
        let expected = indexed_tree(source)?;
        let total_bytes = expected.values().try_fold(0u64, |sum, item| {
            sum.checked_add(item["size_bytes"].as_u64().unwrap_or(0))
                .ok_or("SD backup size overflow".to_string())
        })?;
        let index = base.join("dsi_sd_card.bin.idx");
        let capacity = fs::read_to_string(&index).ok().and_then(|value| {
            value.lines().next().and_then(|line| line.strip_prefix("SIZE ")).and_then(|size| size.parse::<u64>().ok())
        });
        if capacity.is_some_and(|size| total_bytes > size.saturating_mul(9) / 10) {
            return Err("SD backup is too large for this virtual card".into());
        }
        let before = self.snapshot_sd_image(id)?;
        let _lock = MachineOperationLock::acquire(&machine, "sd_import")?;
        if hash_file(&image)? != before.image_sha256 {
            return Err("Runtime SD image changed after the preservation snapshot".into());
        }
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|e| e.to_string())?.as_nanos();
        let staged = machine.join("sd").join(format!(".import-runtime-{stamp}"));
        let history = machine.join("sd/import-history");
        fs::create_dir_all(&history).map_err(|e| e.to_string())?;
        let prior_host = history.join(format!("before-{stamp}"));
        let host = base.join("dsi_sd_card");
        for path in [&staged, &history, &prior_host, &host] { checked_generated_path(&machine, path)?; }
        let copied = copied_tree(source, &staged);
        if let Err(error) = copied {
            if staged.is_dir() { let _ = fs::remove_dir_all(&staged); }
            return Err(error);
        }
        let staged_map = indexed_tree(&staged)?;
        if staged_map != expected {
            let _ = fs::remove_dir_all(&staged);
            return Err("Imported SD copy failed file hash verification".into());
        }
        let config_path = machine.join("sd/virtual.json");
        let mut config = read_json(&config_path)?;
        let had_host = host.exists();
        if had_host {
            let metadata = fs::symlink_metadata(&host).map_err(|e| e.to_string())?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                let _ = fs::remove_dir_all(&staged);
                return Err("Existing SD host path is not a real directory".into());
            }
            fs::rename(&host, &prior_host).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(&staged, &host) {
            if had_host { let _ = fs::rename(&prior_host, &host); }
            let _ = fs::remove_dir_all(&staged);
            return Err(error.to_string());
        }
        config["state"] = json!("pending_runtime_import");
        config["contents_source"] = json!("Imported user-owned SD backup");
        config["host_sync_active"] = json!(false);
        config["host_sync_path"] = json!(host);
        config["pending_import_files"] = json!(expected);
        config["prior_snapshot"] = json!(before.snapshot_id);
        if let Err(error) = atomic_json(&config_path, &config) {
            let _ = fs::rename(&host, &staged);
            if had_host { let _ = fs::rename(&prior_host, &host); }
            return Err(error);
        }
        let result = SdImportResult { staged_entries: expected.len(), staged_bytes: total_bytes,
            prior_snapshot: before.snapshot_id, applies_at_next_boot: true };
        let mut log = OpenOptions::new().create(true).append(true)
            .open(machine.join("logs/events.jsonl")).map_err(|e| e.to_string())?;
        let event = json!({"kind":"SD_IMPORT_STAGED", "timestamp":now()?,
            "source":"Imported user-owned SD backup", "entry_count":result.staged_entries,
            "bytes":result.staged_bytes, "prior_snapshot":result.prior_snapshot,
            "applies_at_next_boot":true});
        writeln!(log, "{event}").map_err(|e| e.to_string())?;
        log.sync_all().map_err(|e| e.to_string())?;
        Ok(result)
    }

    pub fn verify_sd_image_snapshot(&self, id: &str, snapshot_id: &str) -> Result<()> {
        let machine = self.machine_dir(id)?;
        verify_image_snapshot(&machine, snapshot_id).map(|_| ())
    }

    pub fn sd_snapshots(&self, id: &str) -> Result<Vec<SdSnapshotSummary>> {
        let machine = self.machine_dir(id)?;
        let root = machine.join("sd/snapshots");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        for item in fs::read_dir(root).map_err(|e| e.to_string())? {
            let item = item.map_err(|e| e.to_string())?;
            let metadata = item.file_type().map_err(|e| e.to_string())?;
            if !metadata.is_dir() || metadata.is_symlink() {
                continue;
            }
            let snapshot_id = item.file_name().to_string_lossy().into_owned();
            let Ok(path) = snapshot_path(&machine, &snapshot_id) else {
                continue;
            };
            let Ok(manifest) = read_json(&path.join("snapshot.json")) else {
                continue;
            };
            found.push(SdSnapshotSummary {
                snapshot_id,
                created_at: manifest
                    .get("created_at")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .into(),
                schema: manifest
                    .get("schema")
                    .and_then(Value::as_str)
                    .unwrap_or("Unknown")
                    .into(),
            });
        }
        found.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(found)
    }

    pub fn restore_sd_image(&self, id: &str, snapshot_id: &str) -> Result<SdRestoreResult> {
        let machine = self.machine_dir(id)?;
        assert_runtime_stopped(&machine)?;
        let manifest = verify_image_snapshot(&machine, snapshot_id)?;
        let source = snapshot_path(&machine, snapshot_id)?;
        let base = machine.join("runtime/libretro/save/melonDS DS");
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let image = base.join("dsi_sd_card.bin");
        let index = base.join("dsi_sd_card.bin.idx");
        let host = base.join("dsi_sd_card");
        let has_host_files = manifest.get("host_files").is_some_and(Value::is_object);
        if !has_host_files {
            let indexed = fs::read_to_string(source.join("dsi_sd_card.bin.idx"))
                .unwrap_or_default()
                .lines()
                .any(|line| line.starts_with("FILE ") || line.starts_with("DIR "));
            if indexed || (host.is_dir() && !walk(&host)?.is_empty()) {
                return Err(
                    "Older snapshot lacks host-sync files; restoring it could lose SD contents"
                        .into(),
                );
            }
        }
        let previous = if image.is_file() {
            Some(self.snapshot_sd_image(id)?.snapshot_id)
        } else {
            None
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let staged_image = base.join(format!(".restore-{stamp}.bin"));
        let staged_index = base.join(format!(".restore-{stamp}.idx"));
        let staged_host = base.join(format!(".restore-host-{stamp}"));
        let prior_image = base.join(format!(".before-restore-{stamp}.bin"));
        let prior_index = base.join(format!(".before-restore-{stamp}.idx"));
        let prior_host = base.join(format!(".before-host-restore-{stamp}"));
        let image_hash = manifest
            .get("image_sha256")
            .and_then(Value::as_str)
            .ok_or("Image hash missing")?;
        copy_verified(&source.join("dsi_sd_card.bin"), &staged_image, image_hash)?;
        let index_hash = manifest.get("index_sha256").and_then(Value::as_str);
        if let Some(expected) = index_hash {
            copy_verified(&source.join("dsi_sd_card.bin.idx"), &staged_index, expected)?;
        }
        if has_host_files {
            copied_tree(&source.join("host_files"), &staged_host)?;
        }
        for path in [
            &image,
            &index,
            &host,
            &staged_image,
            &staged_index,
            &staged_host,
            &prior_image,
            &prior_index,
            &prior_host,
        ] {
            checked_generated_path(&machine, path)?;
        }
        let had_image = image.exists();
        let had_index = index.exists();
        let had_host = host.exists();
        let mut moved_image = false;
        let mut moved_index = false;
        let mut moved_host = false;
        let commit = (|| -> Result<()> {
            if had_image {
                fs::rename(&image, &prior_image).map_err(|e| e.to_string())?;
                moved_image = true;
            }
            fs::rename(&staged_image, &image).map_err(|e| e.to_string())?;
            if had_index {
                fs::rename(&index, &prior_index).map_err(|e| e.to_string())?;
                moved_index = true;
            }
            if index_hash.is_some() {
                fs::rename(&staged_index, &index).map_err(|e| e.to_string())?;
            }
            if has_host_files {
                if had_host {
                    fs::rename(&host, &prior_host).map_err(|e| e.to_string())?;
                    moved_host = true;
                }
                fs::rename(&staged_host, &host).map_err(|e| e.to_string())?;
            }
            file_matches(&image, image_hash)?;
            if let Some(expected) = index_hash {
                file_matches(&index, expected)?;
            }
            if has_host_files {
                verify_host_tree(
                    &host,
                    manifest["host_files"]
                        .as_object()
                        .ok_or("Host manifest invalid")?,
                )?;
            }
            let path = machine.join("sd/virtual.json");
            let mut config = read_json(&path)?;
            let object = config
                .as_object_mut()
                .ok_or("Virtual SD record is invalid")?;
            object.insert("state".into(), json!("runtime_image"));
            object.insert("backend_image".into(), json!(image.to_string_lossy()));
            object.insert("image_sha256".into(), json!(image_hash));
            object.insert(
                "logical_size_bytes".into(),
                manifest["logical_size_bytes"].clone(),
            );
            object.insert("actual_file_contents_indexed".into(), json!(false));
            object.insert("host_sync_active".into(), json!(has_host_files));
            object.insert(
                "contents_source".into(),
                json!("Generated by CPF from SD snapshot"),
            );
            atomic_json(&path, &config)?;
            Ok(())
        })();
        if let Err(error) = commit {
            // Keep any failed replacement as a generated recovery artifact; restore the prior paths.
            if image.exists() {
                let _ = fs::rename(&image, base.join(format!(".failed-restore-{stamp}.bin")));
            }
            if moved_image {
                let _ = fs::rename(&prior_image, &image);
            }
            if index.exists() {
                let _ = fs::rename(&index, base.join(format!(".failed-restore-{stamp}.idx")));
            }
            if moved_index {
                let _ = fs::rename(&prior_index, &index);
            }
            if has_host_files && host.exists() {
                let _ = fs::rename(&host, base.join(format!(".failed-host-restore-{stamp}")));
            }
            if moved_host {
                let _ = fs::rename(&prior_host, &host);
            }
            return Err(format!(
                "SD restore failed; previous generated files were retained: {error}"
            ));
        }
        // The verified pre-restore snapshot is the durable reversal path.
        if prior_image.exists() {
            let _ = fs::remove_file(&prior_image);
        }
        if prior_index.exists() {
            let _ = fs::remove_file(&prior_index);
        }
        if prior_host.exists() {
            checked_generated_path(&machine, &prior_host)?;
            let _ = fs::remove_dir_all(&prior_host);
        }
        let result = SdRestoreResult {
            restored_snapshot: snapshot_id.into(),
            previous_snapshot: previous,
            image_sha256: image_hash.into(),
        };
        let event = json!({"kind": "SD_RESTORED", "timestamp": now()?,
            "source": "Generated by CPF", "snapshot_id": snapshot_id,
            "previous_snapshot": result.previous_snapshot, "image_sha256": result.image_sha256});
        let mut log = OpenOptions::new()
            .append(true)
            .create(true)
            .open(machine.join("logs/events.jsonl"))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut log, &event).map_err(|e| e.to_string())?;
        log.write_all(b"\n").map_err(|e| e.to_string())?;
        log.sync_all().map_err(|e| e.to_string())?;
        Ok(result)
    }
    pub fn compare_storage(&self, id: &str) -> Result<StorageComparison> {
        let machine = self.machine_dir(id)?;
        let profile = read_json(&machine.join("machine.json"))?;
        let capture_id = profile
            .get("capture_id")
            .and_then(Value::as_str)
            .ok_or("Missing capture ID")?;
        if capture_id.is_empty()
            || !capture_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err("Invalid capture ID".into());
        }
        let inventory = read_json(
            &machine
                .join("captures")
                .join(capture_id)
                .join("sources/sd-inventory.json"),
        )?;
        let historical = historical_entries(&inventory)?;
        let config = read_json(&machine.join("sd/virtual.json"))?;
        let mode = profile
            .get("virtual_sd_mode")
            .and_then(Value::as_str)
            .unwrap_or("");
        let (source, root): (&str, Option<PathBuf>) = match mode {
            "runtime_image" => {
                let path = machine.join("runtime/libretro/save/melonDS DS/dsi_sd_card");
                if config.get("host_sync_active").and_then(Value::as_bool) == Some(true)
                    && path.is_dir()
                {
                    ("Synced virtual SD files", Some(path))
                } else {
                    ("Generated virtual SD image; contents not indexed", None)
                }
            }
            "host_directory" => (
                "Mounted host directory",
                Some(PathBuf::from(
                    config
                        .get("host_path")
                        .and_then(Value::as_str)
                        .ok_or("Host SD path is missing")?,
                )),
            ),
            _ => ("Virtual SD directory", Some(machine.join("sd/virtual"))),
        };
        let actual = root.as_deref().map(walk).transpose()?;
        let mut missing = Vec::new();
        let mut extra = Vec::new();
        let mut different = Vec::new();
        let mut same = 0;
        if let Some(ref actual) = actual {
            for (path, old) in &historical {
                match actual.get(path) {
                    None => missing.push(path.clone()),
                    Some(new)
                        if old.kind != new.kind || (old.kind == "file" && old.size != new.size) =>
                    {
                        different.push(path.clone())
                    }
                    Some(_) => same += 1,
                }
            }
            extra.extend(
                actual
                    .keys()
                    .filter(|path| !historical.contains_key(*path))
                    .cloned(),
            );
        }
        Ok(StorageComparison {
            historical_source: "SD Metadata Capture".into(),
            current_source: source.into(),
            current_contents_indexed: actual.is_some(),
            content_equality_verified: false,
            historical_entry_count: historical.len(),
            current_entry_count: actual.as_ref().map(BTreeMap::len),
            same_path_type_size: actual.as_ref().map(|_| same),
            missing_paths: missing,
            extra_paths: extra,
            different_type_or_size: different,
        })
    }

    pub fn snapshot_sd_image(&self, id: &str) -> Result<SdSnapshot> {
        let machine = self.machine_dir(id)?;
        let profile = read_json(&machine.join("machine.json"))?;
        if profile.get("virtual_sd_mode").and_then(Value::as_str) != Some("runtime_image") {
            return Err(
                "This Rust snapshot operation currently supports runtime SD images only".into(),
            );
        }
        assert_runtime_stopped(&machine)?;
        let base = machine.join("runtime/libretro/save/melonDS DS");
        let image = base.join("dsi_sd_card.bin");
        let index = base.join("dsi_sd_card.bin.idx");
        let host = base.join("dsi_sd_card");
        let metadata = fs::symlink_metadata(&image).map_err(|e| format!("SD image: {e}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("The generated SD image must be a regular file".into());
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let snapshot_id = format!("{}-{:x}-{:x}", stamp, std::process::id(), stamp & 0xffff);
        let snapshots = machine.join("sd/snapshots");
        fs::create_dir_all(&snapshots).map_err(|e| e.to_string())?;
        let target = snapshots.join(&snapshot_id);
        fs::create_dir(&target).map_err(|e| e.to_string())?;

        // An incomplete directory has no manifest and is never presented as a snapshot.
        let original_hash = hash_file(&image)?;
        let copied_image = target.join("dsi_sd_card.bin");
        fs::copy(&image, &copied_image).map_err(|e| e.to_string())?;
        if hash_file(&copied_image)? != original_hash {
            return Err("SD image snapshot failed hash verification".into());
        }
        let index_hash = if index.is_file() {
            if fs::symlink_metadata(&index)
                .map_err(|e| e.to_string())?
                .file_type()
                .is_symlink()
            {
                return Err("Linked SD index is not supported".into());
            }
            let expected = hash_file(&index)?;
            fs::copy(&index, target.join("dsi_sd_card.bin.idx")).map_err(|e| e.to_string())?;
            if hash_file(&target.join("dsi_sd_card.bin.idx"))? != expected {
                return Err("SD index snapshot failed hash verification".into());
            }
            Some(expected)
        } else {
            None
        };
        let host_files = if host.is_dir() {
            Some(copied_tree(&host, &target.join("host_files"))?)
        } else {
            None
        };
        let logical_size = if index.is_file() {
            fs::read_to_string(&index).ok().and_then(|text| {
                text.lines()
                    .next()
                    .and_then(|line| line.strip_prefix("SIZE "))
                    .and_then(|size| size.parse::<u64>().ok())
            })
        } else {
            None
        };
        let created_at = now()?;
        let mut manifest = json!({
            "schema": "cpf.dsi.sd-image-snapshot/1", "snapshot_id": snapshot_id,
            "created_at": created_at, "source": "Generated by CPF",
            "image_sha256": original_hash, "index_sha256": index_hash,
            "logical_size_bytes": logical_size, "host_files": host_files,
            "actual_file_contents_indexed": host_files.is_some(),
        });
        let digest = manifest_hash(&manifest)?;
        manifest
            .as_object_mut()
            .ok_or("Invalid snapshot manifest")?
            .insert("manifest_sha256".into(), Value::String(digest.clone()));
        let manifest_path = target.join("snapshot.json");
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_path)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut output, &manifest).map_err(|e| e.to_string())?;
        output.write_all(b"\n").map_err(|e| e.to_string())?;
        output.sync_all().map_err(|e| e.to_string())?;
        let result = SdSnapshot {
            snapshot_id,
            created_at,
            image_sha256: original_hash,
            index_sha256: index_hash,
            host_entry_count: host_files.as_ref().map(BTreeMap::len),
            manifest_sha256: digest,
        };
        let event = json!({"kind": "SD_SNAPSHOT", "timestamp": result.created_at,
            "source": "Generated by CPF", "snapshot_id": result.snapshot_id,
            "image_sha256": result.image_sha256});
        let mut log = OpenOptions::new()
            .append(true)
            .create(true)
            .open(machine.join("logs/events.jsonl"))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(&mut log, &event).map_err(|e| e.to_string())?;
        log.write_all(b"\n").map_err(|e| e.to_string())?;
        log.sync_all().map_err(|e| e.to_string())?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn historical_sd_inventory_keeps_only_metadata() {
        let entries = historical_entries(&json!({"entries": [
            {"path":"sd:/homebrew/app.nds","type":"file","size_bytes":42}
        ]}))
        .unwrap();
        assert_eq!(entries["homebrew/app.nds"].size, Some(42));
        assert_eq!(entries["homebrew/app.nds"].kind, "file");
    }

    #[test]
    fn import_sd_backup_preserves_source_and_previous_runtime_contents() {
        let root = std::env::temp_dir().join(format!("cpf-sd-import-{}", std::process::id()));
        let machine = root.join("machines/Fixture.cpfmachine");
        let base = machine.join("runtime/libretro/save/melonDS DS");
        fs::create_dir_all(base.join("dsi_sd_card")).unwrap();
        fs::create_dir_all(machine.join("sd")).unwrap();
        fs::create_dir_all(machine.join("logs")).unwrap();
        fs::write(base.join("dsi_sd_card.bin"), b"generated SD image").unwrap();
        fs::write(base.join("dsi_sd_card.bin.idx"), b"SIZE 1000000\n").unwrap();
        fs::write(base.join("dsi_sd_card/old.txt"), b"old").unwrap();
        atomic_json(&machine.join("machine.json"), &json!({"virtual_sd_mode":"runtime_image"})).unwrap();
        atomic_json(&machine.join("sd/virtual.json"), &json!({"state":"runtime_image"})).unwrap();
        let source = root.join("user-sd-backup");
        fs::create_dir_all(source.join("apps")).unwrap();
        fs::write(source.join("apps/new.nds"), b"user file").unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        assert!(workspace.import_runtime_sd_backup("Fixture.cpfmachine", &machine).is_err());
        let result = workspace.import_runtime_sd_backup("Fixture.cpfmachine", &source).unwrap();
        assert_eq!(result.staged_entries, 2);
        assert_eq!(fs::read(source.join("apps/new.nds")).unwrap(), b"user file");
        assert_eq!(fs::read(base.join("dsi_sd_card/apps/new.nds")).unwrap(), b"user file");
        let config = read_json(&machine.join("sd/virtual.json")).unwrap();
        assert_eq!(config["state"], "pending_runtime_import");
        assert_eq!(config["pending_import_files"]["apps/new.nds"]["sha256"], hash_file(&source.join("apps/new.nds")).unwrap());
        let snapshot = machine.join("sd/snapshots").join(&result.prior_snapshot);
        assert_eq!(fs::read(snapshot.join("host_files/old.txt")).unwrap(), b"old");
        assert!(!machine.join("runtime/libretro/running.json").exists());
        let resolved_root = root.canonicalize().unwrap();
        assert!(snapshot.canonicalize().unwrap().starts_with(&resolved_root));
        fs::remove_dir_all(resolved_root).unwrap();
    }

    #[test]
    fn snapshot_copies_generated_sd_and_records_compatible_manifest() {
        let unique = format!(
            "cpf-snapshot-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        let machine = root.join("machines/Fixture.cpfmachine");
        let base = machine.join("runtime/libretro/save/melonDS DS");
        fs::create_dir_all(base.join("dsi_sd_card")).unwrap();
        fs::create_dir_all(machine.join("sd")).unwrap();
        fs::create_dir_all(machine.join("logs")).unwrap();
        fs::write(
            machine.join("machine.json"),
            b"{\"virtual_sd_mode\":\"runtime_image\"}",
        )
        .unwrap();
        fs::write(
            machine.join("sd/virtual.json"),
            b"{\"state\":\"runtime_image\"}",
        )
        .unwrap();
        fs::write(base.join("dsi_sd_card.bin"), b"generated SD image").unwrap();
        fs::write(base.join("dsi_sd_card.bin.idx"), b"SIZE 4096\n").unwrap();
        fs::write(base.join("dsi_sd_card/hello.txt"), b"hello").unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        let result = workspace.snapshot_sd_image("Fixture.cpfmachine").unwrap();
        let snapshot = machine.join("sd/snapshots").join(&result.snapshot_id);
        let manifest = read_json(&snapshot.join("snapshot.json")).unwrap();
        assert_eq!(manifest["schema"], "cpf.dsi.sd-image-snapshot/1");
        assert_eq!(manifest["logical_size_bytes"], 4096);
        assert_eq!(
            manifest["host_files"]["hello.txt"]["sha256"],
            hash_file(&base.join("dsi_sd_card/hello.txt")).unwrap()
        );
        assert_eq!(
            hash_file(&snapshot.join("dsi_sd_card.bin")).unwrap(),
            result.image_sha256
        );
        let mut unsigned = manifest.clone();
        unsigned.as_object_mut().unwrap().remove("manifest_sha256");
        assert_eq!(manifest_hash(&unsigned).unwrap(), result.manifest_sha256);
        fs::write(base.join("dsi_sd_card.bin"), b"changed SD image").unwrap();
        fs::write(base.join("dsi_sd_card/hello.txt"), b"changed").unwrap();
        let restored = workspace
            .restore_sd_image("Fixture.cpfmachine", &result.snapshot_id)
            .unwrap();
        assert!(restored.previous_snapshot.is_some());
        assert_eq!(
            fs::read(base.join("dsi_sd_card.bin")).unwrap(),
            b"generated SD image"
        );
        assert_eq!(
            fs::read(base.join("dsi_sd_card/hello.txt")).unwrap(),
            b"hello"
        );
        // The fixture is a fresh local temporary directory with a known generated subtree.
        let resolved_root = root.canonicalize().unwrap();
        let resolved_snapshot = snapshot.canonicalize().unwrap();
        assert!(resolved_snapshot.starts_with(&resolved_root));
        fs::remove_dir_all(resolved_root).unwrap();
    }
}
