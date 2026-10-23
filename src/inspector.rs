use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: Option<String>,
    pub format: String,
    pub category: String,
    pub privacy: String,
    pub protection: String,
    pub metadata: BTreeMap<String, String>,
    pub dependencies: Vec<String>,
    pub dependency_candidates: BTreeMap<String, Vec<String>>,
    pub notes: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub level: String,
    pub path: String,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub root: String,
    pub started_unix: u64,
    pub complete: bool,
    pub files: Vec<Entry>,
    pub total_bytes: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub mounts: BTreeMap<String, String>,
    pub backend: String,
}
fn diag(r: &mut Report, level: &str, path: &str, message: impl Into<String>) {
    r.diagnostics.push(Diagnostic {
        level: level.into(),
        path: path.into(),
        message: message.into(),
    });
}
fn u16le(b: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        b.get(p..p.checked_add(2)?)?.try_into().ok()?,
    ))
}
fn u32le(b: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        b.get(p..p.checked_add(4)?)?.try_into().ok()?,
    ))
}
fn u64le(b: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        b.get(p..p.checked_add(8)?)?.try_into().ok()?,
    ))
}
fn cstr(b: &[u8], p: usize) -> Option<String> {
    let tail = b.get(p..)?;
    let end = tail.iter().position(|x| *x == 0)?;
    if end > 512 {
        return None;
    }
    Some(String::from_utf8_lossy(&tail[..end]).into_owned())
}
fn sfo(b: &[u8], e: &mut Entry) {
    let Some(count) = u32le(b, 16) else {
        e.notes.push("Truncated SFO header".into());
        return;
    };
    let Some(keys) = u32le(b, 8) else { return };
    let Some(data) = u32le(b, 12) else { return };
    if count > 4096 {
        e.notes.push("Invalid SFO entry count".into());
        return;
    }
    for i in 0..count as usize {
        let p = 20 + i * 16;
        let (Some(k), Some(kind), Some(n), Some(o)) = (
            u16le(b, p),
            u16le(b, p + 2),
            u32le(b, p + 4),
            u32le(b, p + 12),
        ) else {
            e.notes.push("Truncated SFO table".into());
            break;
        };
        let Some(key) = cstr(b, keys as usize + k as usize) else {
            continue;
        };
        if ![
            "TITLE",
            "TITLE_ID",
            "CATEGORY",
            "APP_VER",
            "VERSION",
            "SYSTEM_VER",
            "SDK_VERSION",
        ]
        .contains(&key.as_str())
        {
            continue;
        }
        let start = data as usize + o as usize;
        let Some(value) = b.get(start..start.saturating_add(n as usize)) else {
            e.notes.push(format!("SFO value out of bounds: {key}"));
            continue;
        };
        let v = if kind == 0x404 {
            u32le(value, 0).map(|v| format!("0x{v:08x}"))
        } else {
            Some(
                String::from_utf8_lossy(value)
                    .trim_end_matches('\0')
                    .chars()
                    .take(512)
                    .collect(),
            )
        };
        if let Some(v) = v {
            e.metadata.insert(key, v);
        }
    }
    e.notes.push(
        "SFO SYSTEM_VER is an application firmware requirement, not proof of installed firmware"
            .into(),
    );
}
fn elf(b: &[u8], e: &mut Entry) {
    if b.get(4..6) != Some(&[2, 1]) {
        e.notes
            .push("Only little-endian ELF64 dependency inspection is supported".into());
        return;
    }
    let (Some(phoff), Some(size), Some(count)) = (u64le(b, 32), u16le(b, 54), u16le(b, 56)) else {
        e.notes.push("Truncated ELF header".into());
        return;
    };
    if size < 56 || count > 4096 {
        e.notes.push("Invalid ELF program header table".into());
        return;
    }
    let mut loads = Vec::new();
    let mut dynamic = None;
    let mut sony = false;
    let mut sony_data = None;
    let mut sony_strings = None;
    let mut import_modules = Vec::new();
    let mut import_libs = Vec::new();
    let mut export_libs = Vec::new();
    let mut export_modules = Vec::new();
    let mut original_filename = None;
    for i in 0..count as u64 {
        let Some(p) = phoff
            .checked_add(i * size as u64)
            .and_then(|v| usize::try_from(v).ok())
        else {
            continue;
        };
        let (Some(t), Some(off), Some(va), Some(len)) = (
            u32le(b, p),
            u64le(b, p + 8),
            u64le(b, p + 16),
            u64le(b, p + 32),
        ) else {
            e.notes
                .push("ELF program headers exceed inspection prefix".into());
            break;
        };
        if t == 1 {
            loads.push((va, off, len));
        }
        if t == 0x61000000 {
            sony_data = Some(off);
        }
        if t == 2 {
            dynamic = Some((off, len));
        }
    }
    let mut needed = Vec::new();
    let mut strings = None;
    if let Some((off, len)) = dynamic {
        for i in 0..(len / 16).min(65536) {
            let Some(p) = off
                .checked_add(i * 16)
                .and_then(|v| usize::try_from(v).ok())
            else {
                break;
            };
            let (Some(tag), Some(val)) = (u64le(b, p), u64le(b, p + 8)) else {
                e.notes
                    .push("Dynamic table exceeds inspection prefix".into());
                break;
            };
            if tag == 0 {
                break;
            }
            if tag == 1 {
                needed.push(val)
            }
            if tag == 0x61000035 {
                sony_strings = Some(val);
            }
            if tag == 0x61000009 {
                original_filename = Some(val);
            }
            if tag == 0x6100000f {
                import_modules.push(val & 0xffffffff);
            }
            if tag == 0x61000015 {
                import_libs.push(val & 0xffffffff);
            }
            if tag == 0x61000013 {
                export_libs.push(val & 0xffffffff);
            }
            if tag == 0x6100000d {
                export_modules.push(val & 0xffffffff);
            }
            if tag == 5 {
                strings = Some(val)
            }
            if tag >= 0x61000000 {
                sony = true;
            }
        }
    }
    let string_offset = if let (Some(base), Some(offset)) = (sony_data, sony_strings) {
        base.checked_add(offset)
    } else {
        strings.and_then(|va| {
            loads
                .iter()
                .find(|(base, _, len)| va >= *base && va - *base < *len)
                .and_then(|(base, off, _)| off.checked_add(va - base))
        })
    };
    if let Some(base) = string_offset {
        let read_name = |n: u64| {
            base.checked_add(n)
                .and_then(|v| usize::try_from(v).ok())
                .and_then(|p| cstr(b, p))
        };
        if let Some(name) = original_filename.and_then(read_name) {
            e.metadata.insert("SCE_ORIGINAL_FILENAME".into(), name);
        }
        for n in needed {
            if let Some(s) = read_name(n) {
                e.dependencies.push(s);
            } else {
                e.notes.push("Unresolved DT_NEEDED string offset".into());
            }
        }
        for (key, offsets) in [
            ("SCE_IMPORT_MODULES", import_modules),
            ("SCE_IMPORT_LIBRARIES", import_libs),
            ("SCE_EXPORT_MODULES", export_modules),
            ("SCE_EXPORT_LIBRARIES", export_libs),
        ] {
            let names: Vec<_> = offsets.into_iter().filter_map(read_name).collect();
            if !names.is_empty() {
                e.metadata.insert(key.into(), names.join(", "));
            }
        }
    }
    if sony {
        e.notes.push(
            "SCE module/library names inspected; symbol NIDs, ABI versions and service contracts are not resolved"
                .into(),
        );
    }
    e.notes
        .push("Static dependency names only; an empty list does not mean execution-ready".into());
}
fn inspect(path: &Path, relative: String, cancel: &AtomicBool) -> Result<Entry, String> {
    let mut file = File::open(path).map_err(|x| x.to_string())?;
    let before = file.metadata().map_err(|x| x.to_string())?;
    let mut hash = Sha256::new();
    let mut prefix = Vec::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut bytes = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Inspection cancelled".into());
        }
        let n = file.read(&mut buf).map_err(|x| x.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        bytes += n as u64;
        let limit = if prefix.starts_with(b"\x7fELF")
            || (prefix.is_empty() && buf[..n].starts_with(b"\x7fELF"))
        {
            64 * 1024 * 1024usize
        } else {
            4 * 1024 * 1024usize
        };
        let keep = n.min(limit.saturating_sub(prefix.len()));
        prefix.extend_from_slice(&buf[..keep]);
    }
    let after = file.metadata().map_err(|x| x.to_string())?;
    if before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
        || bytes != before.len()
    {
        return Err("File changed during hashing; no stable hash recorded".into());
    }
    let lower = relative.to_lowercase();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let privacy = if lower.split('/').any(|s| {
        [
            "user",
            "home",
            "system_data",
            "priv",
            "savedata",
            "trophy",
            "license",
        ]
        .contains(&s)
    }) || lower.contains("sflash")
    {
        "potentially-user-or-device-specific"
    } else {
        "firmware-candidate-unverified"
    };
    let category = if ["sprx", "prx"].contains(&ext.as_str()) {
        "system-library"
    } else if ["self", "elf"].contains(&ext.as_str()) || lower.ends_with("eboot.bin") {
        "executable"
    } else if ["db", "dat"].contains(&ext.as_str()) {
        "database-or-data"
    } else {
        "resource"
    };
    let mut e = Entry {
        path: relative,
        bytes,
        sha256: Some(format!("{:x}", hash.finalize())),
        format: "unknown".into(),
        category: category.into(),
        privacy: privacy.into(),
        protection: "unknown; no decryption attempted".into(),
        metadata: BTreeMap::new(),
        dependencies: vec![],
        dependency_candidates: BTreeMap::new(),
        notes: vec![],
    };
    let b = prefix.as_slice();
    if b.starts_with(b"\x7fELF") {
        e.format = "ELF".into();
        e.protection = "ELF header visible; segment protection not verified".into();
        elf(b, &mut e);
    } else if b.starts_with(&[0x4f, 0x15, 0x3d, 0x1d]) {
        e.format = "SCE SELF".into();
        e.protection = "Signed container; payload encryption/compression undetermined".into();
    } else if b.starts_with(b"\0PSF") {
        e.format = "SFO".into();
        e.protection = "metadata header visible".into();
        sfo(b, &mut e);
    } else if b.starts_with(b"\x7fCNT") {
        e.format = "PS4 PKG".into();
        e.protection = "Package container; possibly encrypted".into();
    } else if b.starts_with(b"SQLite format 3\0") {
        e.format = "SQLite".into();
    } else if b.starts_with(b"\x1f\x8b") {
        e.format = "gzip".into();
        e.protection = "compressed signature".into();
    } else if b.starts_with(b"PK\x03\x04") {
        e.format = "ZIP".into();
        e.protection = "archive; compression/encryption not inspected".into();
    } else if b.starts_with(b"\xfd7zXZ\0") {
        e.format = "XZ".into();
        e.protection = "compressed signature".into();
    } else if b.starts_with(b"\x28\xb5\x2f\xfd") {
        e.format = "Zstandard".into();
        e.protection = "compressed signature".into();
    } else if b.starts_with(b"\x89PNG") {
        e.format = "PNG".into();
    } else if b.starts_with(b"<?xml") {
        e.format = "XML".into();
    }
    if lower.ends_with("version.xml") && bytes <= 65536 {
        let t = String::from_utf8_lossy(b);
        for tag in ["release", "version", "build", "system_version"] {
            if let Some(start) = t.find(&format!("<{tag}>")) {
                let tail = &t[start + tag.len() + 2..];
                if let Some(end) = tail.find(&format!("</{tag}>")) {
                    e.metadata.insert(
                        format!("version.xml:{tag}"),
                        tail[..end].chars().take(128).collect(),
                    );
                }
            }
        }
    }
    if bytes == 0 {
        e.notes
            .push("Empty file; copy completeness requires verification".into());
    }
    Ok(e)
}
pub fn scan(
    root: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(usize, u64),
) -> Result<Report, String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if fs::symlink_metadata(root)
            .map_err(|x| x.to_string())?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err("Select a real directory, not a reparse point".into());
        }
    }
    if fs::symlink_metadata(root)
        .map_err(|x| x.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Select a real directory, not a symbolic link".into());
    }
    let root = fs::canonicalize(root).map_err(|x| x.to_string())?;
    if !root.is_dir() {
        return Err("Dump root must be a directory".into());
    }
    let mut r = Report {
        schema_version: 1,
        root: root.to_string_lossy().into_owned(),
        started_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        complete: true,
        files: vec![],
        total_bytes: 0,
        diagnostics: vec![],
        mounts: BTreeMap::new(),
        backend:
            "External shadPS4 research adapter available in PS4.exe; authentic dashboard execution remains unverified. Inspection executes no firmware."
                .into(),
    };
    let mut walker = WalkDir::new(&root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();
    while let Some(item) = walker.next() {
        if cancel.load(Ordering::Relaxed) {
            r.complete = false;
            diag(&mut r, "warning", "", "Cancelled; report is partial");
            break;
        }
        let item = match item {
            Ok(v) => v,
            Err(err) => {
                r.complete = false;
                diag(&mut r, "error", "", err.to_string());
                continue;
            }
        };
        let path = item.path();
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let meta = match fs::symlink_metadata(path) {
            Ok(v) => v,
            Err(err) => {
                r.complete = false;
                diag(&mut r, "error", &relative, err.to_string());
                continue;
            }
        };
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                if meta.is_dir() {
                    walker.skip_current_dir();
                }
                diag(&mut r, "warning", &relative, "Reparse point skipped");
                continue;
            }
        }
        if meta.file_type().is_symlink() {
            diag(&mut r, "warning", &relative, "Symbolic link skipped");
            continue;
        }
        if !meta.is_file() {
            continue;
        }
        match inspect(path, relative.clone(), cancel) {
            Ok(e) => {
                for note in &e.notes {
                    if [
                        "Truncated",
                        "Invalid",
                        "out of bounds",
                        "exceed",
                        "Unresolved",
                    ]
                    .iter()
                    .any(|text| note.contains(text))
                    {
                        diag(&mut r, "warning", &relative, note.clone());
                    }
                }
                r.total_bytes += e.bytes;
                r.files.push(e);
                progress(r.files.len(), r.total_bytes);
            }
            Err(err) => {
                r.complete = false;
                diag(&mut r, "error", &relative, err);
            }
        }
    }
    let dirs: BTreeSet<_> = r
        .files
        .iter()
        .flat_map(|e| e.path.split('/').map(str::to_owned))
        .collect();
    for mount in [
        "system",
        "system_ex",
        "preinst",
        "preinst2",
        "system_data",
        "dev",
    ] {
        let state = if dirs.contains(mount) {
            "observed-path-component; completeness unverified"
        } else {
            "not-observed; may be absent or selected root may be this mount"
        };
        r.mounts.insert(mount.into(), state.into());
    }
    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in &r.files {
        if let Some(name) = e.path.rsplit('/').next() {
            names
                .entry(module_key(name))
                .or_default()
                .push(e.path.clone());
        }
        if let Some(original) = e.metadata.get("SCE_ORIGINAL_FILENAME") {
            if let Some(name) = original.rsplit(['/', '\\']).next() {
                let paths = names.entry(module_key(name)).or_default();
                if !paths.contains(&e.path) {
                    paths.push(e.path.clone());
                }
            }
        }
    }
    let mut missing = BTreeSet::new();
    for e in &mut r.files {
        for d in &e.dependencies {
            if let Some(paths) = names.get(&module_key(d)) {
                e.dependency_candidates.insert(d.clone(), paths.clone());
            } else {
                missing.insert((e.path.clone(), d.clone()));
            }
        }
    }
    for (p, d) in missing {
        diag(&mut r,"warning",&p,format!("DT_NEEDED has no filename candidate in selected dump: {d}; .prx/.sprx and original-filename aliases considered; HLE availability not evaluated"));
    }
    diag(&mut r,"info","","No completeness baseline supplied. Path presence and hashes do not establish a complete or bootable firmware dump.");
    diag(&mut r,"warning","","Dependency candidates are filename hints, not ABI or symbol resolution. SCE symbol resolution, firmware boot, system service IPC, and dashboard execution are unsupported.");
    Ok(r)
}
fn module_key(name: &str) -> String {
    let lower = name.to_lowercase();
    if let Some(stem) = lower
        .strip_suffix(".sprx")
        .or_else(|| lower.strip_suffix(".prx"))
    {
        format!("{stem}.prx")
    } else {
        lower
    }
}
pub fn export(report: &Report, destination: &Path) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or("Report must have a parent directory")?;
    let parent = fs::canonicalize(parent).map_err(|x| x.to_string())?;
    let root = fs::canonicalize(&report.root).map_err(|x| x.to_string())?;
    if parent.starts_with(root) {
        return Err("Reports must be saved outside the selected dump directory".into());
    }
    let output = parent.join(destination.file_name().ok_or("Missing report filename")?);
    let bytes = serde_json::to_vec_pretty(report).map_err(|x| x.to_string())?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .map_err(|x| x.to_string())?;
    f.write_all(&bytes).map_err(|x| x.to_string())?;
    f.sync_all().map_err(|x| x.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "ps4-inspector-{}-{}-{sequence}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(p.join("dump/system")).unwrap();
        p
    }
    #[test]
    fn sony_dependencies_and_prx_alias_candidates() {
        let p = fixture();
        let mut bytes = vec![0u8; 512];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&2u16.to_le_bytes());
        bytes[64..68].copy_from_slice(&2u32.to_le_bytes());
        bytes[72..80].copy_from_slice(&192u64.to_le_bytes());
        bytes[96..104].copy_from_slice(&64u64.to_le_bytes());
        bytes[120..124].copy_from_slice(&0x61000000u32.to_le_bytes());
        bytes[128..136].copy_from_slice(&320u64.to_le_bytes());
        for (i, (tag, value)) in [(0x61000035u64, 0u64), (1, 0), (0x61000015, 16)]
            .iter()
            .enumerate()
        {
            let offset = 192 + i * 16;
            bytes[offset..offset + 8].copy_from_slice(&tag.to_le_bytes());
            bytes[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
        }
        bytes[320..332].copy_from_slice(b"libTest.prx\0");
        bytes[336..344].copy_from_slice(b"testLib\0");
        fs::write(p.join("dump/system/app.elf"), bytes).unwrap();
        fs::write(p.join("dump/system/libTest.sprx"), b"library").unwrap();
        let r = scan(&p.join("dump"), &AtomicBool::new(false), |_, _| {}).unwrap();
        let e = r
            .files
            .iter()
            .find(|e| e.path.ends_with("app.elf"))
            .unwrap();
        assert_eq!(e.dependencies, vec!["libTest.prx"]);
        assert_eq!(e.metadata.get("SCE_IMPORT_LIBRARIES").unwrap(), "testLib");
        assert_eq!(
            e.dependency_candidates["libTest.prx"],
            vec!["system/libTest.sprx"]
        );
        assert!(!r
            .diagnostics
            .iter()
            .any(|d| d.message.contains("no filename candidate")));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn hash_read_only_and_export_guards() {
        let p = fixture();
        let dump = p.join("dump");
        let file = dump.join("system/test.sprx");
        fs::write(&file, b"abc").unwrap();
        let r = scan(&dump, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert!(r.complete);
        assert_eq!(r.files.len(), 1);
        assert_eq!(
            r.files[0].sha256.as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert!(export(&r, &dump.join("forbidden.json")).is_err());
        assert!(!dump.join("forbidden.json").exists());
        export(&r, &p.join("report.json")).unwrap();
        assert!(export(&r, &p.join("report.json")).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"abc");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn malformed_sfo_is_bounded_and_cancellation_is_partial() {
        let p = fixture();
        fs::write(p.join("dump/truncated.sfo"), b"\0PSF").unwrap();
        let r = scan(&p.join("dump"), &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(r.files[0].format, "SFO");
        assert!(!r.files[0].notes.is_empty());
        let r = scan(&p.join("dump"), &AtomicBool::new(true), |_, _| {}).unwrap();
        assert!(!r.complete);
        assert!(r.files.is_empty());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn privacy_and_container_signatures() {
        let p = fixture();
        fs::create_dir_all(p.join("dump/priv")).unwrap();
        fs::write(p.join("dump/priv/a.self"), [0x4f, 0x15, 0x3d, 0x1d]).unwrap();
        let r = scan(&p.join("dump"), &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(r.files[0].format, "SCE SELF");
        assert_eq!(r.files[0].privacy, "potentially-user-or-device-specific");
        assert!(r.files[0].protection.contains("undetermined"));
        fs::remove_dir_all(p).unwrap();
    }
}
