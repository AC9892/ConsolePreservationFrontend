//! Local, read-only Xbox 360 preservation catalog. Never copies firmware into the build tree.
use axum::{extract::{Path as RoutePath, State}, http::{header, HeaderMap, StatusCode}, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs::{self, File}, io::{Read, Seek, Write}, path::{Component, PathBuf}, sync::{Arc, Mutex}};

#[cfg(windows)]
fn reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn reparse(metadata: &fs::Metadata) -> bool { metadata.file_type().is_symlink() }

#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub kind: String,
    pub sha256: String,
    pub duplicate_of: Option<String>,
    pub details: Vec<String>,
    pub source: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub label: String,
    pub source_root: PathBuf,
    pub source: String,
    pub dashboard_version: Option<String>,
    pub version_evidence: String,
    pub motherboard: Option<String>,
    pub files: Vec<Entry>,
    pub issues: Vec<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Library { pub collections: Vec<Collection> }

#[derive(Serialize)]
pub struct ValidationSummary {
    pub files: usize,
    pub bytes: u64,
    pub kinds: std::collections::BTreeMap<String, usize>,
    pub duplicates: usize,
    pub warnings: usize,
    pub warning_types: std::collections::BTreeMap<String, usize>,
    pub nand_bootloader_headers: usize,
    pub xex_execution_versions: usize,
    pub stfs_known_content_types: usize,
}

/// Inspect a user-selected local copy without saving paths, hashes, or source bytes.
pub fn validate_source(path: &str) -> std::io::Result<ValidationSummary> {
    validate_source_with_mode(path, "automatic")
}

/// Manual mode requires an explicitly selected root; results remain aggregate and are not saved.
pub fn validate_source_with_mode(path: &str, mode: &str) -> std::io::Result<ValidationSummary> {
    if !matches!(mode, "automatic" | "manual") { return Err(std::io::Error::other("invalid validation mode")); }
    let collection = scan(&ImportRequest {
        path: path.to_owned(), mode: mode.into(), source: None,
        label: None, dashboard_version: None,
    })?;
    let mut kinds = std::collections::BTreeMap::new();
    for file in &collection.files { *kinds.entry(file.kind.clone()).or_insert(0) += 1; }
    let mut warning_types = std::collections::BTreeMap::new();
    for warning in &collection.issues {
        let category = warning.split_once(": ").map(|(category, _)| category).unwrap_or(warning);
        *warning_types.entry(category.to_owned()).or_insert(0) += 1;
    }
    Ok(ValidationSummary {
        files: collection.files.len(),
        bytes: collection.files.iter().map(|file| file.bytes).sum(),
        kinds,
        duplicates: collection.files.iter().filter(|file| file.duplicate_of.is_some()).count(),
        warnings: collection.issues.len(),
        warning_types,
        nand_bootloader_headers: collection.files.iter().flat_map(|file| &file.details)
            .filter(|detail| detail.starts_with("Bootloader header candidate:")).count(),
        xex_execution_versions: collection.files.iter().filter(|file| file.kind == "XEX" &&
            file.details.iter().any(|detail| detail.starts_with("Executable version:"))).count(),
        stfs_known_content_types: collection.files.iter().filter(|file| file.kind == "STFS package" &&
            file.details.iter().any(|detail| detail.starts_with("Content type:") && !detail.starts_with("Content type: Unknown"))).count(),
    })
}

pub struct Store { path: PathBuf, library: Mutex<Library> }

pub struct VerificationSummary {
    pub verified: usize,
    pub total: usize,
    pub changed: usize,
    pub missing_or_unreadable: usize,
}

pub fn router(store: Arc<Store>) -> Router {
    Router::new()
        .route("/library", get(list))
        .route("/import", post(import))
        .route("/collections/{id}/files/{index}/preview", get(preview))
        .route("/compatibility", get(compatibility))
        .route("/backend/detect", post(detect_backend))
        .route("/collections/{id}/verify", post(verify))
        .with_state(store)
}

#[derive(Deserialize)]
pub struct ImportRequest {
    path: String,
    mode: String,
    source: Option<String>,
    label: Option<String>,
    dashboard_version: Option<String>,
}

#[derive(Deserialize)]
pub struct BackendRequest { path: String }

#[derive(Serialize)]
pub struct PublicCollection<'a> {
    id: &'a str, label: &'a str, source: &'a str,
    dashboard_version: &'a Option<String>, version_evidence: &'a str,
    motherboard: &'a Option<String>, files: &'a [Entry], issues: &'a [String],
}

impl Store {
    pub fn open() -> std::io::Result<Self> {
        let base = std::env::var_os("CPF_PRIVATE_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("CPF")))
            .ok_or_else(|| std::io::Error::other("set CPF_PRIVATE_LIBRARY or LOCALAPPDATA"))?;
        fs::create_dir_all(&base)?;
        let path = base.join("xbox360-library.json");
        let library = if path.exists() { serde_json::from_slice(&fs::read(&path)?).map_err(std::io::Error::other)? } else { Library::default() };
        Ok(Self { path, library: Mutex::new(library) })
    }

    fn save(&self, library: &Library) -> std::io::Result<()> {
        let temp = self.path.with_extension("json.tmp");
        let mut file = File::create(&temp)?;
        serde_json::to_writer(&mut file, library)?;
        file.flush()?;
        file.sync_all()?;
        if let Err(error) = fs::rename(&temp, &self.path) {
            let _ = fs::remove_file(temp);
            return Err(error);
        }
        Ok(())
    }

    fn replace_collection(&self, collection: Collection) -> std::io::Result<String> {
        let id = collection.id.clone();
        let mut current = self.library.lock().unwrap();
        let mut next = current.clone();
        next.collections.retain(|item| item.id != id);
        next.collections.push(collection);
        self.save(&next)?;
        *current = next;
        Ok(id)
    }

    pub fn import_local(&self, path: String, mode: String, source: Option<String>, label: Option<String>, dashboard_version: Option<String>) -> std::io::Result<()> {
        let collection = scan(&ImportRequest { path, mode, source, label, dashboard_version })?;
        self.replace_collection(collection)?;
        Ok(())
    }

    pub fn collections_snapshot(&self) -> Vec<Collection> {
        self.library.lock().unwrap().collections.clone()
    }

    pub fn verify_local(&self, id: &str) -> std::io::Result<VerificationSummary> {
        let collection = self.library.lock().unwrap().collections.iter().find(|item| item.id == id)
            .cloned().ok_or_else(|| std::io::Error::other("collection not found"))?;
        let mut result = VerificationSummary { verified: 0, total: collection.files.len(), changed: 0, missing_or_unreadable: 0 };
        for entry in &collection.files {
            match verify_entry(&collection.source_root, entry) {
                Ok(true) => result.verified += 1,
                Ok(false) => result.changed += 1,
                Err(_) => result.missing_or_unreadable += 1,
            }
        }
        Ok(result)
    }

    pub fn preview_local(&self, id: &str, index: usize) -> std::io::Result<String> {
        let (root, entry) = {
            let library = self.library.lock().unwrap();
            let collection = library.collections.iter().find(|item| item.id == id)
                .ok_or_else(|| std::io::Error::other("collection not found"))?;
            (collection.source_root.clone(), collection.files.get(index).cloned()
                .ok_or_else(|| std::io::Error::other("file not found"))?)
        };
        if !matches!(entry.kind.as_str(), "XEX" | "Original Xbox XBE" | "Console font") {
            return Err(std::io::Error::other("preview not allowed for this file type"));
        }
        let value = preview_file(&root, &entry)?;
        Ok(format!("{}\r\n\r\n{}\r\n\r\nASCII: {}",
            value["path"].as_str().unwrap_or(""), value["hex"].as_str().unwrap_or(""),
            value["ascii"].as_str().unwrap_or("")))
    }
}

pub fn inspect_backend_local(path: &str) -> std::io::Result<serde_json::Value> {
    inspect_backend(&PathBuf::from(path))
}

fn local_host(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    (host.starts_with("127.0.0.1:") || host.starts_with("localhost:"))
        && host.rsplit(':').next().is_some_and(|port| port.parse::<u16>().is_ok())
}

fn local_request(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let origin_ok = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
        .is_some_and(|origin| origin == format!("http://{host}"));
    let json_ok = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    local_host(headers) && origin_ok && json_ok
}

pub async fn list(State(store): State<Arc<Store>>, headers: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
    if !local_host(&headers) { return Err(StatusCode::FORBIDDEN); }
    let library = store.library.lock().unwrap();
    Ok(Json(serde_json::json!({"collections": library.collections.iter().map(|c| PublicCollection {
        id: &c.id, label: &c.label, source: &c.source,
        dashboard_version: &c.dashboard_version, version_evidence: &c.version_evidence,
        motherboard: &c.motherboard, files: &c.files, issues: &c.issues,
    }).collect::<Vec<_>>() })))
}

pub async fn import(State(store): State<Arc<Store>>, headers: HeaderMap, Json(request): Json<ImportRequest>) -> Result<Json<serde_json::Value>, (StatusCode, &'static str)> {
    if !local_request(&headers) { return Err((StatusCode::FORBIDDEN, "Local dashboard request required")); }
    let result = tokio::task::spawn_blocking(move || scan(&request)).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Scan failed"))?
        .map_err(|_| (StatusCode::BAD_REQUEST, "Cannot read selected path or import limit reached"))?;
    let id = store.replace_collection(result).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Could not save private catalog"))?;
    Ok(Json(serde_json::json!({"id": id})))
}

pub async fn preview(RoutePath((id, index)): RoutePath<(String, usize)>, State(store): State<Arc<Store>>, headers: HeaderMap)
    -> Result<Json<serde_json::Value>, StatusCode> {
    if !local_host(&headers) { return Err(StatusCode::FORBIDDEN); }
    let (root, entry) = {
        let library = store.library.lock().unwrap();
        let collection = library.collections.iter().find(|c| c.id == id).ok_or(StatusCode::NOT_FOUND)?;
        (collection.source_root.clone(), collection.files.get(index).cloned().ok_or(StatusCode::NOT_FOUND)?)
    };
    if !matches!(entry.kind.as_str(), "XEX" | "Original Xbox XBE" | "Console font") {
        return Err(StatusCode::FORBIDDEN);
    }
    let result = tokio::task::spawn_blocking(move || preview_file(&root, &entry)).await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(result))
}

pub async fn compatibility(headers: HeaderMap) -> Result<Json<serde_json::Value>, StatusCode> {
    if !local_host(&headers) { return Err(StatusCode::FORBIDDEN); }
    let records = serde_json::from_str(include_str!("compatibility.json"))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(records))
}

pub async fn detect_backend(State(_store): State<Arc<Store>>, headers: HeaderMap, Json(request): Json<BackendRequest>)
    -> Result<Json<serde_json::Value>, (StatusCode, &'static str)> {
    if !local_request(&headers) { return Err((StatusCode::FORBIDDEN, "Local dashboard request required")); }
    let result = tokio::task::spawn_blocking(move || inspect_backend(&PathBuf::from(request.path))).await
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "Backend inspection failed"))?
        .map_err(|_| (StatusCode::BAD_REQUEST, "No valid Xenia Canary executable at that path"))?;
    Ok(Json(result))
}

fn inspect_backend(path: &PathBuf) -> std::io::Result<serde_json::Value> {
    if !path.is_absolute() || path.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return Err(std::io::Error::other("absolute path required"));
    }
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("").to_ascii_lowercase();
    if !matches!(name.as_str(), "xenia_canary.exe" | "xenia-canary.exe") {
        return Err(std::io::Error::other("unexpected executable name"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || reparse(&metadata) || metadata.len() < 0x100 || metadata.len() > 1_000_000_000 {
        return Err(std::io::Error::other("invalid executable"));
    }
    let mut file = File::open(path)?;
    let mut header = [0u8; 4096];
    let count = file.read(&mut header)?;
    if count < 0x40 || !header.starts_with(b"MZ") { return Err(std::io::Error::other("not a PE executable")); }
    let pe_offset = u32::from_le_bytes(header[0x3c..0x40].try_into().unwrap()) as usize;
    if pe_offset.checked_add(6).is_none_or(|end| end > count) || &header[pe_offset..pe_offset + 4] != b"PE\0\0" ||
        header[pe_offset + 4..pe_offset + 6] != [0x64, 0x86] {
        return Err(std::io::Error::other("not an x64 PE executable"));
    }
    file.rewind()?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop { let count = file.read(&mut buffer)?; if count == 0 { break; } digest.update(&buffer[..count]); }
    if file.metadata()?.len() != metadata.len() { return Err(std::io::Error::other("file changed")); }
    Ok(serde_json::json!({
        "backend":"Xenia Canary candidate", "executable_verified":"x64 PE header only",
        "version":null, "version_status":"Unknown; no trusted version metadata extracted",
        "sha256":format!("{:x}",digest.finalize()), "bytes":metadata.len(),
        "dashboard_execution":"Not Tested", "launch_available":false
    }))
}

pub async fn verify(RoutePath(id): RoutePath<String>, State(store): State<Arc<Store>>, headers: HeaderMap)
    -> Result<Json<serde_json::Value>, StatusCode> {
    if !local_request(&headers) { return Err(StatusCode::FORBIDDEN); }
    let collection = {
        let library = store.library.lock().unwrap();
        library.collections.iter().find(|c| c.id == id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let result = tokio::task::spawn_blocking(move || {
        let mut verified = 0usize;
        let mut failures = Vec::new();
        for entry in &collection.files {
            match verify_entry(&collection.source_root, entry) {
                Ok(true) => verified += 1,
                Ok(false) => failures.push(serde_json::json!({"path":entry.path,"status":"changed"})),
                Err(_) => failures.push(serde_json::json!({"path":entry.path,"status":"missing or unreadable"})),
            }
        }
        serde_json::json!({"verified":verified,"total":collection.files.len(),"failures":failures})
    }).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(result))
}

fn source_path(root: &PathBuf, entry: &Entry) -> std::io::Result<PathBuf> {
    let relative = PathBuf::from(&entry.path);
    if relative.is_absolute() || relative.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(std::io::Error::other("invalid catalog path"));
    }
    let root_metadata = fs::symlink_metadata(root)?;
    if reparse(&root_metadata) { return Err(std::io::Error::other("source changed")); }
    if root_metadata.is_file() {
        if relative.file_name() != root.file_name() { return Err(std::io::Error::other("source changed")); }
        return Ok(root.clone());
    }
    let mut candidate = root.clone();
    for part in relative.components() {
        candidate.push(part);
        if reparse(&fs::symlink_metadata(&candidate)?) { return Err(std::io::Error::other("reparse point")); }
    }
    Ok(candidate)
}

fn verify_entry(root: &PathBuf, entry: &Entry) -> std::io::Result<bool> {
    let mut file = File::open(source_path(root, entry)?)?;
    let before = file.metadata()?;
    if before.len() != entry.bytes { return Ok(false); }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop { let count = file.read(&mut buffer)?; if count == 0 { break; } digest.update(&buffer[..count]); }
    let after = file.metadata()?;
    Ok(before.len() == after.len() && before.modified().ok() == after.modified().ok() &&
        format!("{:x}", digest.finalize()) == entry.sha256)
}

fn preview_file(root: &PathBuf, entry: &Entry) -> std::io::Result<serde_json::Value> {
    let mut file = File::open(source_path(root, entry)?)?;
    let before = file.metadata()?;
    if before.len() != entry.bytes { return Err(std::io::Error::other("source changed")); }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut first = [0u8; 64];
    let mut filled = 0usize;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        digest.update(&buffer[..count]);
        let kept = (first.len() - filled).min(count);
        first[filled..filled + kept].copy_from_slice(&buffer[..kept]);
        filled += kept;
    }
    let after = file.metadata()?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() ||
        format!("{:x}", digest.finalize()) != entry.sha256 { return Err(std::io::Error::other("checksum mismatch")); }
    let hex = first[..filled].chunks(16).enumerate().map(|(row, chunk)| {
        format!("{:08X}  {}", row * 16, chunk.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" "))
    }).collect::<Vec<_>>().join("\n");
    let ascii = first[..filled].iter().map(|byte| if (0x20..=0x7e).contains(byte) { *byte as char } else { '.' }).collect::<String>();
    Ok(serde_json::json!({"path":entry.path,"hex":hex,"ascii":ascii,"bytes":filled}))
}

fn scan(request: &ImportRequest) -> std::io::Result<Collection> {
    if !matches!(request.mode.as_str(), "automatic" | "manual" | "advanced") {
        return Err(std::io::Error::other("invalid mode"));
    }
    let source = if request.mode == "advanced" {
        request.source.as_deref().filter(|s| matches!(*s, "NAND" | "Flash" | "HddX" | "other"))
            .ok_or_else(|| std::io::Error::other("invalid source"))?
    } else { "unspecified" };
    let root = PathBuf::from(&request.path);
    if !root.is_absolute() || root.components().any(|part| matches!(part, Component::ParentDir | Component::CurDir)) {
        return Err(std::io::Error::other("absolute path required"));
    }
    let metadata = fs::symlink_metadata(&root)?;
    if reparse(&metadata) { return Err(std::io::Error::other("reparse point")); }
    let restricted_name = |path: &PathBuf| path.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("Very important DONT SHARE"));
    if metadata.is_dir() && restricted_name(&root) && request.mode == "automatic" {
        return Err(std::io::Error::other("restricted folder requires direct Manual or Advanced selection"));
    }
    let mut paths = Vec::new();
    let mut skipped_restricted = false;
    if metadata.is_file() { paths.push(root.clone()); }
    else if metadata.is_dir() {
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            for child in fs::read_dir(directory)? {
                let child = child?;
                let metadata = fs::symlink_metadata(child.path())?;
                if reparse(&metadata) { continue; }
                if metadata.is_dir() {
                    if restricted_name(&child.path()) { skipped_restricted = true; continue; }
                    pending.push(child.path());
                }
                else if metadata.is_file() { paths.push(child.path()); }
                if paths.len() + pending.len() > 20_000 { return Err(std::io::Error::other("scan limit")); }
            }
        }
    } else { return Err(std::io::Error::other("unsupported source")); }
    paths.sort();
    let mut hashes: HashMap<String, String> = HashMap::new();
    let mut files = Vec::with_capacity(paths.len());
    let mut issues = Vec::new();
    if skipped_restricted { issues.push("Restricted folder skipped; select it directly in Manual or Advanced mode to inspect.".into()); }
    for path in paths {
        let relative = if root.is_file() { root.file_name().unwrap().to_string_lossy().into_owned() }
            else { path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/") };
        let mut file = File::open(&path)?;
        let before = file.metadata()?;
        let mut head = [0u8; 32];
        let count = file.read(&mut head)?;
        let (kind, mut details) = identify(&relative, &head[..count], before.len());
        let mut digest = Sha256::new();
        digest.update(&head[..count]);
        let mut buffer = [0u8; 65536];
        loop { let count = file.read(&mut buffer)?; if count == 0 { break; } digest.update(&buffer[..count]); }
        if kind == "XEX" || kind == "STFS package" || kind == "NAND image" {
            file.rewind()?;
            let cap = if kind == "XEX" { 65536 } else if kind == "NAND image" { 128 } else { 1024 };
            let mut prefix = vec![0u8; cap.min(before.len() as usize)];
            file.read_exact(&mut prefix)?;
            let (more, problem) = if kind == "XEX" { inspect_xex(&prefix, before.len()) }
                else if kind == "NAND image" { inspect_nand(&prefix, before.len()) }
                else { inspect_stfs(&prefix, before.len()) };
            details.extend(more);
            if let Some(problem) = problem { issues.push(format!("{problem}: {relative}")); }
            if kind == "NAND image" && matches!(before.len(), 16_777_216 | 17_301_504) &&
                prefix.len() >= 2 && matches!((prefix[0], prefix[1]), (0xFF | 0x0F, 0x3F | 0x4F)) {
                details.extend(inspect_bootloader_headers(&mut file, &prefix, before.len(), before.len() == 17_301_504)?);
                if before.len() == 17_301_504 {
                    let (marker_details, ordinary_anomalies) = inspect_spare_markers(&mut file)?;
                    details.extend(marker_details);
                    if ordinary_anomalies > 0 { issues.push(format!("Spare-marker anomalies require review: {relative}")); }
                }
            }
        }
        let after = file.metadata()?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            issues.push(format!("Changed while hashing: {relative}"));
            continue;
        }
        let hash = format!("{:x}", digest.finalize());
        let duplicate_of = hashes.get(&hash).cloned();
        hashes.entry(hash.clone()).or_insert_with(|| relative.clone());
        files.push(Entry { path: relative, bytes: before.len(), kind, sha256: hash, duplicate_of, details, source: source.to_owned() });
    }
    if !files.iter().any(|f| f.kind == "NAND image") { issues.push("No NAND image in this collection; extracted files can still be preserved.".into()); }
    if !files.iter().any(|f| f.path.eq_ignore_ascii_case("dash.xex") || f.path.to_ascii_lowercase().ends_with("/dash.xex")) {
        issues.push("No dash.xex found; dashboard execution cannot be attempted from this collection.".into());
    }
    let mut id_hash = Sha256::new(); id_hash.update(root.to_string_lossy().as_bytes());
    let id = format!("{:x}", id_hash.finalize())[..16].to_owned();
    let label = request.label.as_deref().filter(|v| !v.trim().is_empty() && v.len() <= 100)
        .map(str::to_owned).unwrap_or_else(|| root.file_name().unwrap_or_default().to_string_lossy().into_owned());
    let dashboard_version = request.dashboard_version.as_deref().filter(|v| !v.trim().is_empty() && v.len() <= 40).map(str::to_owned);
    let version_evidence = if dashboard_version.is_some() { "user supplied; unverified" } else { "unknown" }.to_owned();
    Ok(Collection { id, label, source_root: root, source: source.to_owned(), dashboard_version, version_evidence, motherboard: None, files, issues })
}

fn identify(name: &str, bytes: &[u8], size: u64) -> (String, Vec<String>) {
    let lower = name.to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if base.starts_with("kv_") || base == "cpukey.txt" || base == "key.bin" {
        return ("Sensitive console data".into(), vec!["Contents are not parsed or displayed".into()]);
    }
    if starts(b"XEX2") && size >= 24 { return ("XEX".into(), vec!["XEX2 header verified".into()]); }
    if starts(b"XEX2") { return ("Damaged XEX".into(), vec!["Truncated XEX2 header".into()]); }
    if starts(b"CON ") || starts(b"LIVE") || starts(b"PIRS") { return ("STFS package".into(), vec!["Container signature verified".into()]); }
    if starts(b"XBEH") { return ("Original Xbox XBE".into(), vec!["XBE header verified; separate execution backend required".into()]); }
    if base == "flashdmp.bin" || base == "nanddump.bin" || base == "nand.bin" {
        return ("NAND image".into(), vec!["NAND candidate by filename; full integrity not verified".into()]);
    }
    if matches!(size, 16_777_216 | 17_301_504) && bytes.len() >= 2 &&
        matches!((bytes[0], bytes[1]), (0xFF | 0x0F, 0x3F | 0x4F)) {
        return ("NAND image".into(), vec!["NAND candidate by flash marker and 16 MiB size class; full integrity not verified".into()]);
    }
    if base.starts_with("smc_") || base.starts_with("fcrt_") { return ("Firmware component".into(), vec!["Identified by filename only".into()]); }
    if lower.ends_with(".xex") { return ("Unverified XEX".into(), vec!["XEX2 signature missing".into()]); }
    if lower.ends_with(".xtt") || lower.ends_with(".xttp1") || lower.ends_with(".xtf") { return ("Console font".into(), vec!["Preserved without conversion".into()]); }
    if lower.ends_with(".xzp") { return ("System resource candidate".into(), vec!["Format not inspected".into()]); }
    ("Other file".into(), Vec::new())
}

fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}

fn xex_version(value: u32) -> String {
    format!("{}.{}.{}.{}", value >> 28, (value >> 24) & 0xF, (value >> 8) & 0xFFFF, value & 0xFF)
}

fn inspect_xex(bytes: &[u8], file_len: u64) -> (Vec<String>, Option<&'static str>) {
    if bytes.len() < 24 || !bytes.starts_with(b"XEX2") { return (Vec::new(), Some("Invalid XEX header")); }
    let flags = be32(bytes, 4).unwrap();
    let header_size = be32(bytes, 8).unwrap() as u64;
    let security_offset = be32(bytes, 16).unwrap() as u64;
    let count = be32(bytes, 20).unwrap() as usize;
    let mut details = vec![format!("Module flags: 0x{flags:08X}"), format!("Header size: {header_size} bytes"),
        format!("Optional header entries: {count}")];
    if flags & 0x08 != 0 { details.push("Module type: DLL".into()); }
    else if flags & 0x01 != 0 { details.push("Module type: title".into()); }
    if count > 4096 || 24usize.checked_add(count.saturating_mul(8)).is_none_or(|end| end > bytes.len() || end as u64 > header_size) ||
        header_size > file_len || security_offset >= file_len {
        return (details, Some("Implausible XEX header"));
    }
    for index in 0..count {
        let offset = 24 + index * 8;
        let key = be32(bytes, offset).unwrap();
        let value = be32(bytes, offset + 4).unwrap();
        match key {
            0x00010100 => details.push(format!("Entry point: 0x{value:08X}")),
            0x00010201 => details.push(format!("Image base: 0x{value:08X}")),
            0x00030000 => details.push(format!("System flags: 0x{value:08X}")),
            0x00030100 => details.push(format!("System flags 2: 0x{value:08X}")),
            0x000003FF => {
                let at = value as usize;
                if at.checked_add(8).is_some_and(|end| end <= bytes.len() && end as u64 <= header_size) {
                    let encryption = u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]);
                    let compression = u16::from_be_bytes([bytes[at + 6], bytes[at + 7]]);
                    details.push(format!("Base-file encryption: {}", match encryption { 0 => "none", 1 => "encrypted", _ => "unknown" }));
                    details.push(format!("Base-file compression: {}", match compression { 1 => "raw", 2 => "compressed", 3 => "delta compressed", _ => "unknown" }));
                } else if at.checked_add(8).is_some_and(|end| end as u64 <= header_size && end as u64 <= file_len) {
                    details.push("Base-file format lies beyond the 64 KiB inspection window".into());
                } else { return (details, Some("XEX base-file format offset out of bounds")); }
            }
            0x000103FF => details.push("Import library directory present; names not parsed".into()),
            0x000002FF => details.push("Resource directory present; resources not extracted".into()),
            0x00040006 => {
                let at = value as usize;
                if at.checked_add(24).is_some_and(|end| end <= bytes.len() && end as u64 <= header_size) {
                    let version = be32(bytes, at + 4).unwrap();
                    let base_version = be32(bytes, at + 8).unwrap();
                    details.push(format!("Executable version: {} (not a verified dashboard/kernel version)", xex_version(version)));
                    details.push(format!("Base executable version: {}", xex_version(base_version)));
                    details.push(format!("Execution platform code: {}", bytes[at + 16]));
                    details.push(format!("Execution type code: {}", bytes[at + 17]));
                } else if at.checked_add(24).is_some_and(|end| end as u64 <= header_size && end as u64 <= file_len) {
                    details.push("Execution record lies beyond the 64 KiB inspection window".into());
                } else { return (details, Some("XEX execution record offset out of bounds")); }
            }
            _ => {}
        }
    }
    (details, None)
}

fn inspect_stfs(bytes: &[u8], file_len: u64) -> (Vec<String>, Option<&'static str>) {
    if file_len < 0x380 || bytes.len() < 0x380 { return (Vec::new(), Some("Truncated STFS header")); }
    let header_size = be32(bytes, 0x340).unwrap() as u64;
    let mut details = vec![format!("STFS header size: {header_size} bytes")];
    let table_blocks = u16::from_le_bytes([bytes[0x37C], bytes[0x37D]]);
    details.push(format!("File-table blocks declared: {table_blocks}"));
    if !(0x1000..=file_len).contains(&header_size) {
        return (details, Some("Implausible STFS header"));
    }
    let content_type = be32(bytes, 0x344).unwrap();
    let content_name = match content_type {
        0x00000001 => "Saved game", 0x00001000 => "Xbox 360 title", 0x00004000 => "Installed game",
        0x00005000 => "Original Xbox game", 0x00007000 => "Game on demand",
        0x00008000 => "Avatar asset pack", 0x00009000 => "Avatar item",
        0x00010000 => "Profile", 0x00020000 => "Gamer picture",
        0x00030000 => "Theme", 0x00040000 => "Cache file",
        0x00050000 => "Storage download", 0x00070000 => "Xbox download",
        0x00080000 => "Game demo", 0x000B0000 => "Installer",
        0x000D0000 => "Arcade game", _ => "Unknown",
    };
    details.push(format!("Content type: {content_name} (0x{content_type:08X})"));
    let content_size = u64::from_be_bytes(bytes[0x34C..0x354].try_into().unwrap());
    details.push(format!("Declared content size: {content_size} bytes (unverified)"));
    details.push(format!("Package version: {}", xex_version(be32(bytes, 0x358).unwrap())));
    details.push(format!("Base package version: {}", xex_version(be32(bytes, 0x35C).unwrap())));
    // Console ID and profile ID begin at 0x36C; never expose or persist them as details.
    if table_blocks == 0 {
        details.push("No file-table blocks declared; empty or alternate layout, contents unverified".into());
    }
    (details, None)
}

fn inspect_nand(bytes: &[u8], file_len: u64) -> (Vec<String>, Option<&'static str>) {
    if bytes.len() < 2 { return (Vec::new(), Some("Truncated NAND candidate")); }
    let marker = matches!((bytes[0], bytes[1]), (0xFF | 0x0F, 0x3F | 0x4F));
    if !marker { return (vec!["NAND filename only; flash header marker absent".into()], Some("Unverified NAND candidate")); }
    let mut details = vec!["Xbox 360 flash header marker present".into()];
    if matches!(file_len, 16_777_216 | 17_301_504) {
        if file_len == 16_777_216 {
            details.push("16 MiB image size without spare bytes; ECC and bad blocks cannot be checked here".into());
        } else {
            details.push("16 MiB data image with 16-byte page spare areas; ECC and bad blocks not verified".into());
        }
        if bytes.len() >= 0x80 {
            let size = be32(bytes, 0x78).unwrap() as u64;
            let offset = be32(bytes, 0x7C).unwrap() as u64;
            if size > 0 && offset.checked_add(size).is_some_and(|end| end <= 16_777_216) {
                details.push(format!("Declared SMC range: offset 0x{offset:X}, {size} bytes (not decrypted)"));
            } else { return (details, Some("Implausible SMC range in NAND header")); }
        }
    } else { details.push("NAND image size and layout not recognized".into()); }
    (details, None)
}

fn read_nand_header(file: &mut (impl Read + Seek), offset: u64, with_spare: bool) -> std::io::Result<[u8; 16]> {
    let mut bytes = [0u8; 16];
    if !with_spare {
        file.seek(std::io::SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
    } else {
        let mut done = 0;
        while done < bytes.len() {
            let logical = offset + done as u64;
            let in_page = logical % 512;
            let physical = (logical / 512) * 528 + in_page;
            let take = (512 - in_page).min((bytes.len() - done) as u64) as usize;
            file.seek(std::io::SeekFrom::Start(physical))?;
            file.read_exact(&mut bytes[done..done + take])?;
            done += take;
        }
    }
    Ok(bytes)
}

/// Read only 16-byte bootloader headers. Do not read the bootloader body or keyvault.
fn inspect_bootloader_headers(file: &mut (impl Read + Seek), header: &[u8], file_len: u64, with_spare: bool) -> std::io::Result<Vec<String>> {
    let Some(mut offset) = be32(header, 8).map(u64::from) else { return Ok(Vec::new()); };
    let mut details = Vec::new();
    let logical_len = if with_spare { file_len / 528 * 512 } else { file_len };
    if offset < 0x80 || offset >= logical_len {
        details.push("Bootloader offset unverified".into());
        return Ok(details);
    }
    let mut cb_count = 0;
    for _ in 0..5 {
        if offset.checked_add(16).is_none_or(|end| end > logical_len) { break; }
        // The encrypted keyvault occupies this range in a 16 MiB image.
        if offset < 0x8000 && offset + 16 > 0x4000 { break; }
        let bytes = read_nand_header(file, offset, with_spare)?;
        let id = &bytes[..2];
        let name = match id {
            b"CB" => { cb_count += 1; if cb_count == 1 { "CB_A" } else { "CB_B" } },
            b"CD" => "CD",
            b"CE" => "CE",
            b"SB" => "SB candidate",
            b"SC" => "SC candidate",
            b"SD" => "SD candidate",
            b"SE" => "SE candidate",
            _ => break,
        };
        let build = u16::from_be_bytes([bytes[2], bytes[3]]);
        let size = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as u64;
        let Some(end) = offset.checked_add(size) else { break; };
        if size < 16 || end > logical_len || build == 0 { break; }
        details.push(format!("Bootloader header candidate: {name}, build {build}, offset 0x{offset:X}, length {size} bytes; payload unverified"));
        offset = (end + 15) & !15;
        if id == b"CE" || id == b"SE" { break; }
    }
    if details.is_empty() { details.push("Bootloader headers not identified; offset or layout unverified".into()); }
    Ok(details)
}

/// Marker-only screening based on J-Runner's small-block layout. No ECC or remap claim.
fn inspect_spare_markers(file: &mut (impl Read + Seek)) -> std::io::Result<(Vec<String>, usize)> {
    let mut spare = [0u8; 16];
    let mut ordinary = 0usize;
    let mut reserved = 0usize;
    for index in 0..1024 {
        let mut suspect = false;
        for page in 0..32 {
            let offset = ((index * 32 + page) * 528 + 512) as u64;
            file.seek(std::io::SeekFrom::Start(offset))?;
            file.read_exact(&mut spare)?;
            if spare.iter().all(|byte| *byte == 0) || spare[5] != 0xFF {
                suspect = true;
                break;
            }
        }
        if suspect {
            if index >= 0x3E0 { reserved += 1; } else { ordinary += 1; }
        }
    }
    Ok((vec![
        format!("Spare-marker anomaly candidates outside reserve area: {ordinary} blocks (unverified; ECC/remaps not checked)"),
        format!("Spare-marker anomaly candidates in reserve area: {reserved} blocks (unverified)"),
    ], ordinary))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn identifies_signatures_without_trusting_extensions() {
        assert_eq!(identify("dash.xex", b"XEX2\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", 24).0, "XEX");
        assert_eq!(identify("dash.xex", b"bad", 3).0, "Unverified XEX");
        assert_eq!(identify("save.bin", b"CON ", 4).0, "STFS package");
        assert_eq!(identify("KV_dec.bin", b"private", 7).0, "Sensitive console data");
        assert_eq!(identify("KV_dec.bin", b"XEX2", 100).0, "Sensitive console data");
        assert_eq!(identify("renamed-copy.dat", &[0xFF, 0x4F], 17_301_504).0, "NAND image");
    }
    #[test]
    fn parses_bounded_xex_optional_headers() {
        let mut bytes = vec![0u8; 96];
        bytes[..4].copy_from_slice(b"XEX2");
        bytes[8..12].copy_from_slice(&80u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&80u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&2u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&0x00010100u32.to_be_bytes());
        bytes[28..32].copy_from_slice(&0x82000000u32.to_be_bytes());
        bytes[32..36].copy_from_slice(&0x000003FFu32.to_be_bytes());
        bytes[36..40].copy_from_slice(&64u32.to_be_bytes());
        bytes[64..68].copy_from_slice(&8u32.to_be_bytes());
        bytes[68..70].copy_from_slice(&1u16.to_be_bytes());
        bytes[70..72].copy_from_slice(&2u16.to_be_bytes());
        let (details, issue) = inspect_xex(&bytes, bytes.len() as u64);
        assert!(issue.is_none());
        assert!(details.contains(&"Entry point: 0x82000000".to_owned()));
        assert!(details.contains(&"Base-file encryption: encrypted".to_owned()));
        assert!(details.contains(&"Base-file compression: compressed".to_owned()));
        bytes[20..24].copy_from_slice(&5000u32.to_be_bytes());
        assert_eq!(inspect_xex(&bytes, bytes.len() as u64).1, Some("Implausible XEX header"));
    }
    #[test]
    fn reads_xex_execution_version_without_identifiers() {
        let mut bytes = vec![0u8; 128];
        bytes[..4].copy_from_slice(b"XEX2");
        bytes[8..12].copy_from_slice(&128u32.to_be_bytes());
        bytes[16..20].copy_from_slice(&96u32.to_be_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        bytes[24..28].copy_from_slice(&0x00040006u32.to_be_bytes());
        bytes[28..32].copy_from_slice(&64u32.to_be_bytes());
        bytes[64..68].copy_from_slice(&0xDEADBEEFu32.to_be_bytes());
        bytes[68..72].copy_from_slice(&0x21234567u32.to_be_bytes());
        bytes[72..76].copy_from_slice(&0x10000102u32.to_be_bytes());
        bytes[80] = 2; bytes[81] = 1;
        bytes[84..88].copy_from_slice(&0xAABBCCDDu32.to_be_bytes());
        let (details, issue) = inspect_xex(&bytes, bytes.len() as u64);
        assert!(issue.is_none());
        assert!(details.iter().any(|line| line.contains("Executable version: 2.1.9029.103")));
        assert!(!details.join(" ").contains("DEADBEEF"));
        assert!(!details.join(" ").contains("AABBCCDD"));
        bytes[28..32].copy_from_slice(&120u32.to_be_bytes());
        assert_eq!(inspect_xex(&bytes, bytes.len() as u64).1, Some("XEX execution record offset out of bounds"));
    }
    #[test]
    fn checks_stfs_header_bounds() {
        let mut bytes = vec![0u8; 0x380];
        bytes[..4].copy_from_slice(b"CON ");
        bytes[0x340..0x344].copy_from_slice(&0x1000u32.to_be_bytes());
        bytes[0x37C..0x37E].copy_from_slice(&1u16.to_le_bytes());
        assert!(inspect_stfs(&bytes, 0x2000).1.is_none());
        assert_eq!(inspect_stfs(&bytes, 0x380).1, Some("Implausible STFS header"));
        bytes[0x37C..0x37E].copy_from_slice(&0u16.to_le_bytes());
        let (details, issue) = inspect_stfs(&bytes, 0x2000);
        assert!(issue.is_none());
        assert!(details.iter().any(|line| line.contains("No file-table blocks")));
        bytes[0x344..0x348].copy_from_slice(&0x10000u32.to_be_bytes());
        bytes[0x34C..0x354].copy_from_slice(&1234u64.to_be_bytes());
        bytes[0x358..0x35C].copy_from_slice(&0x10000102u32.to_be_bytes());
        bytes[0x36C..0x371].copy_from_slice(&[1, 2, 3, 4, 5]);
        let (details, issue) = inspect_stfs(&bytes, 0x2000);
        assert!(issue.is_none());
        assert!(details.iter().any(|line| line.contains("Content type: Profile")));
        assert!(details.iter().any(|line| line.contains("Declared content size: 1234")));
        assert!(!details.join(" ").contains("01 02 03 04 05"));
    }
    #[test]
    fn checks_nand_marker_and_declared_smc_range_without_decryption() {
        let mut bytes = vec![0u8; 128];
        bytes[0] = 0xFF; bytes[1] = 0x4F;
        bytes[0x78..0x7C].copy_from_slice(&0x300u32.to_be_bytes());
        bytes[0x7C..0x80].copy_from_slice(&0x1000u32.to_be_bytes());
        let (details, issue) = inspect_nand(&bytes, 16 * 1024 * 1024);
        assert!(issue.is_none());
        assert!(details.iter().any(|line| line.contains("Declared SMC range")));
        let (details, issue) = inspect_nand(&bytes, 17_301_504);
        assert!(issue.is_none());
        assert!(details.iter().any(|line| line.contains("Declared SMC range")));
        bytes[0] = 0;
        assert_eq!(inspect_nand(&bytes, 16 * 1024 * 1024).1, Some("Unverified NAND candidate"));
    }
    #[test]
    fn reads_only_bounded_nand_bootloader_headers() {
        let mut image = vec![0u8; 0x200];
        image[8..12].copy_from_slice(&0x100u32.to_be_bytes());
        image[0x100..0x102].copy_from_slice(b"CB");
        image[0x102..0x104].copy_from_slice(&1234u16.to_be_bytes());
        image[0x10C..0x110].copy_from_slice(&0x20u32.to_be_bytes());
        image[0x120..0x122].copy_from_slice(b"CD");
        image[0x122..0x124].copy_from_slice(&5678u16.to_be_bytes());
        image[0x12C..0x130].copy_from_slice(&0x20u32.to_be_bytes());
        let mut cursor = std::io::Cursor::new(&image);
        let details = inspect_bootloader_headers(&mut cursor, &image[..128], image.len() as u64, false).unwrap();
        assert!(details.iter().any(|line| line.contains("CB_A, build 1234")));
        assert!(details.iter().any(|line| line.contains("CD, build 5678")));
        assert!(details.iter().all(|line| line.contains("payload unverified")));
        image[0x10C..0x110].copy_from_slice(&0xFFFFu32.to_be_bytes());
        let mut cursor = std::io::Cursor::new(&image);
        assert_eq!(inspect_bootloader_headers(&mut cursor, &image[..128], image.len() as u64, false).unwrap(),
            vec!["Bootloader headers not identified; offset or layout unverified".to_owned()]);
        let mut private_range = vec![0u8; 0x8100];
        private_range[8..12].copy_from_slice(&0x4000u32.to_be_bytes());
        private_range[0x4000..0x4002].copy_from_slice(b"CB");
        let mut cursor = std::io::Cursor::new(&private_range);
        let details = inspect_bootloader_headers(&mut cursor, &private_range[..128], private_range.len() as u64, false).unwrap();
        assert_eq!(details, vec!["Bootloader headers not identified; offset or layout unverified".to_owned()]);
        assert_eq!(cursor.position(), 0);
    }
    #[test]
    fn maps_spare_nand_logical_offsets_without_reading_spare_bytes() {
        let mut image = vec![0u8; 528 * 4];
        image[8..12].copy_from_slice(&0x200u32.to_be_bytes());
        image[512..528].fill(0xEE);
        image[528..530].copy_from_slice(b"CB");
        image[530..532].copy_from_slice(&4321u16.to_be_bytes());
        image[540..544].copy_from_slice(&0x20u32.to_be_bytes());
        let mut cursor = std::io::Cursor::new(&image);
        let details = inspect_bootloader_headers(&mut cursor, &image[..128], image.len() as u64, true).unwrap();
        assert!(details.iter().any(|line| line.contains("CB_A, build 4321")));
        assert_eq!(read_nand_header(&mut cursor, 0x1F8, true).unwrap()[8..10], *b"CB");
    }
    #[test]
    fn screens_spare_markers_without_claiming_bad_blocks() {
        let mut image = vec![0u8; 17_301_504];
        for page in image.chunks_exact_mut(528) { page[512..528].fill(0xFF); }
        image[0x4200 + 0x205] = 0;
        image[0x3E0 * 0x4200 + 0x205] = 0;
        let mut cursor = std::io::Cursor::new(&image);
        let (details, ordinary) = inspect_spare_markers(&mut cursor).unwrap();
        assert_eq!(ordinary, 1);
        assert!(details[0].contains("outside reserve area: 1 blocks"));
        assert!(details[1].contains("in reserve area: 1 blocks"));
    }
    #[test]
    fn backend_detection_is_read_only_and_does_not_claim_compatibility() {
        let root = std::env::current_dir().unwrap().join("target").join(format!("cpf-backend-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("xenia_canary.exe");
        let mut bytes = vec![0u8; 0x100];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let result = inspect_backend(&path).unwrap();
        assert_eq!(result["launch_available"], false);
        assert_eq!(result["version"], serde_json::Value::Null);
        bytes[0x80] = 0;
        fs::write(&path, bytes).unwrap();
        assert!(inspect_backend(&path).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn scans_synthetic_files_read_only() {
        let root = std::env::current_dir().unwrap().join("target").join(format!("cpf-xbox-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("dash.xex"), b"XEX2\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        fs::write(root.join("copy.xex"), b"XEX2\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").unwrap();
        let collection = scan(&ImportRequest { path: root.to_string_lossy().into(), mode: "automatic".into(), source: None, label: None, dashboard_version: None }).unwrap();
        assert_eq!(collection.files.len(), 2);
        assert_eq!(collection.files.iter().filter(|f| f.duplicate_of.is_some()).count(), 1);
        assert!(verify_entry(&root, &collection.files[0]).unwrap());
        let preview = preview_file(&root, &collection.files[0]).unwrap();
        assert!(preview["hex"].as_str().unwrap().contains("58 45 58 32"));
        fs::write(root.join(&collection.files[0].path), b"changed").unwrap();
        assert!(!verify_entry(&root, &collection.files[0]).unwrap());
        assert!(preview_file(&root, &collection.files[0]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parent_scan_skips_restricted_folder_until_direct_manual_selection() {
        let root = std::env::current_dir().unwrap().join("target").join(format!("cpf-restricted-test-{}", std::process::id()));
        let restricted = root.join("Very important DONT SHARE");
        fs::create_dir_all(&restricted).unwrap();
        fs::write(root.join("safe.txt"), b"safe").unwrap();
        fs::write(restricted.join("private.bin"), b"private").unwrap();
        let request = |path: &PathBuf, mode: &str| ImportRequest {
            path: path.to_string_lossy().into_owned(), mode: mode.into(), source: None,
            label: None, dashboard_version: None,
        };
        let parent = scan(&request(&root, "automatic")).unwrap();
        assert_eq!(parent.files.len(), 1);
        assert_eq!(parent.files[0].path, "safe.txt");
        assert!(parent.issues.iter().any(|issue| issue.contains("Restricted folder skipped")));
        assert!(scan(&request(&restricted, "automatic")).is_err());
        assert_eq!(scan(&request(&restricted, "manual")).unwrap().files.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_catalog_save_preserves_in_memory_library() {
        let root = std::env::current_dir().unwrap().join("target").join(format!("cpf-save-test-{}", std::process::id()));
        let path = root.join("catalog.json");
        fs::create_dir_all(&path).unwrap();
        let store = Store { path: path.clone(), library: Mutex::new(Library::default()) };
        let collection = Collection {
            id: "synthetic".into(), label: "synthetic".into(), source_root: root.clone(),
            source: "unspecified".into(), dashboard_version: None, version_evidence: "unknown".into(),
            motherboard: None, files: Vec::new(), issues: Vec::new(),
        };
        assert!(store.replace_collection(collection).is_err());
        assert!(store.library.lock().unwrap().collections.is_empty());
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn catalog_replacement_persists_one_collection_per_source() {
        let root = std::env::current_dir().unwrap().join("target").join(format!("cpf-replace-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let store = Store { path: root.join("catalog.json"), library: Mutex::new(Library::default()) };
        let mut collection = Collection {
            id: "synthetic".into(), label: "first".into(), source_root: root.clone(),
            source: "unspecified".into(), dashboard_version: None, version_evidence: "unknown".into(),
            motherboard: None, files: Vec::new(), issues: Vec::new(),
        };
        store.replace_collection(collection.clone()).unwrap();
        collection.label = "second".into();
        store.replace_collection(collection).unwrap();
        let saved: Library = serde_json::from_slice(&fs::read(&store.path).unwrap()).unwrap();
        assert_eq!(saved.collections.len(), 1);
        assert_eq!(saved.collections[0].label, "second");
        fs::remove_dir_all(root).unwrap();
    }
}
