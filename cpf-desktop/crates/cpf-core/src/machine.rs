use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct MachineSummary {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub mode: String,
    pub backend: String,
    pub runtime_running: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Readiness {
    pub ready_to_attempt_boot: bool,
    pub missing: Vec<String>,
    pub menu_boot_verified: bool,
    pub nand_association: String,
    pub backend: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MachineDetail {
    pub summary: MachineSummary,
    pub profile: Value,
    pub provenance: Value,
    pub observations: Value,
    pub source: Value,
    pub titles: Value,
    pub sd: Value,
    pub runtime_status: Value,
    pub readiness: Readiness,
    pub events: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineSettings {
    pub virtual_sd_enabled: bool,
    #[serde(default = "enabled_by_default")]
    pub sound_compatibility: bool,
    pub bios9i: Option<String>,
    pub bios7i: Option<String>,
    pub firmware: Option<String>,
}

fn enabled_by_default() -> bool { true }

pub fn read_json(path: &Path) -> Result<Value> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_reader(file).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn optional_json(path: &Path) -> Value {
    if path.is_file() {
        read_json(path).unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

fn tail(path: &Path, limit: u64) -> Result<String> {
    if !path.is_file() {
        return Ok(String::new());
    }
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    file.seek(SeekFrom::Start(size.saturating_sub(limit)))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn atomic_json(path: &Path, value: &Value) -> Result<()> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temp = path.with_extension(format!("json.tmp-{}-{stamp}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        serde_json::to_writer_pretty(&mut file, value).map_err(|e| e.to_string())?;
        file.write_all(b"\n").map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        replace_file(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(crate) fn assert_runtime_stopped(machine: &Path) -> Result<()> {
    let lock = machine.join("runtime/libretro/running.json");
    if !lock.is_file() {
        return Ok(());
    }
    let record = read_json(&lock)?;
    let pid = record.get("pid").and_then(Value::as_u64).unwrap_or(0);
    #[cfg(windows)]
    let active = {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ACCESS_DENIED};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        if pid == 0 || pid > u32::MAX as u64 {
            false
        } else {
            let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
            if handle.is_null() {
                (unsafe { GetLastError() }) == ERROR_ACCESS_DENIED
            } else {
                let mut code = 0u32;
                let valid = unsafe { GetExitCodeProcess(handle, &mut code) } != 0;
                unsafe {
                    CloseHandle(handle);
                }
                valid && code == 259 // STILL_ACTIVE
            }
        }
    };
    #[cfg(not(windows))]
    let active = pid != 0;
    if active {
        return Err("Close the console session before changing generated machine files".into());
    }
    fs::remove_file(lock).map_err(|e| format!("Could not clear stale runtime lock: {e}"))
}

#[cfg(windows)]
pub(crate) fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let success = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) fn replace_file(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

impl Workspace {
    pub fn discover() -> Result<Self> {
        if let Ok(path) = env::var("CPF_WORKSPACE") {
            return Self::open(PathBuf::from(path));
        }
        let mut candidates = Vec::new();
        if let Ok(path) = env::current_dir() {
            candidates.extend(path.ancestors().map(Path::to_path_buf));
        }
        if let Ok(path) = env::current_exe() {
            candidates.extend(path.ancestors().map(Path::to_path_buf));
        }
        for path in candidates {
            if path.join("machines").is_dir() && path.join("cpf_dsi_window.pyw").is_file() {
                return Self::open(path);
            }
        }
        Err("CPF workspace not found. Set CPF_WORKSPACE to the Full Console folder.".into())
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        // Some user-owned Windows volumes disallow GetFinalPathNameByHandle
        // (canonicalize/realpath) while normal reads and writes still work.
        let root = if path.is_absolute() {
            path
        } else {
            env::current_dir().map_err(|e| e.to_string())?.join(path)
        };
        if !root.join("machines").is_dir() {
            return Err("No machines directory in CPF workspace".into());
        }
        Ok(Self { root })
    }

    pub fn machine_dir(&self, id: &str) -> Result<PathBuf> {
        let stem = id.strip_suffix(".cpfmachine").ok_or("Invalid machine ID")?;
        if stem.is_empty()
            || stem.len() > 64
            || !stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-')
        {
            return Err("Invalid machine ID".into());
        }
        let base = self.root.join("machines");
        let candidate = base.join(id);
        let metadata = fs::symlink_metadata(&candidate).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Machine is outside the CPF workspace".into());
        }
        Ok(candidate)
    }

    pub fn machines(&self) -> Result<Vec<MachineSummary>> {
        let mut found = Vec::new();
        for entry in fs::read_dir(self.root.join("machines")).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !id.ends_with(".cpfmachine") {
                continue;
            }
            let machine = self.machine_dir(&id)?;
            let profile = read_json(&machine.join("machine.json"))?;
            let runtime_running = assert_runtime_stopped(&machine).is_err();
            found.push(MachineSummary {
                id,
                name: field(&profile, "name").to_owned(),
                platform: field(&profile, "platform").to_owned(),
                mode: field(&profile, "mode").to_owned(),
                backend: field(&profile, "runtime_backend").to_owned(),
                runtime_running,
            });
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    pub fn detail(&self, id: &str) -> Result<MachineDetail> {
        let machine = self.machine_dir(id)?;
        let profile = read_json(&machine.join("machine.json"))?;
        let summary = self
            .machines()?
            .into_iter()
            .find(|m| m.id == id)
            .ok_or("Machine not found")?;
        Ok(MachineDetail {
            summary,
            provenance: read_json(&machine.join("provenance.json"))?,
            observations: read_json(&machine.join("observations.json"))?,
            source: read_json(&machine.join("nand/source.json"))?,
            titles: read_json(&machine.join("nand/titles.json"))?,
            sd: read_json(&machine.join("sd/virtual.json"))?,
            runtime_status: self.runtime_status(id)?,
            readiness: self.readiness(id)?,
            events: tail(&machine.join("logs/events.jsonl"), 30_000)?,
            profile,
        })
    }

    pub fn runtime_status(&self, id: &str) -> Result<Value> {
        let machine = self.machine_dir(id)?;
        let mut status = optional_json(&machine.join("runtime/libretro/status.json"));
        if !status.is_object() {
            status = json!({});
        }
        status
            .as_object_mut()
            .ok_or("Runtime status is invalid")?
            .insert(
                "running".into(),
                json!(assert_runtime_stopped(&machine).is_err()),
            );
        Ok(status)
    }

    pub fn settings(&self, id: &str) -> Result<MachineSettings> {
        let machine = self.machine_dir(id)?;
        let config = read_json(&machine.join("runtime/config.json"))?;
        Ok(MachineSettings {
            virtual_sd_enabled: config
                .get("virtual_sd_enabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            sound_compatibility: config
                .get("sound_compatibility")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            bios9i: config
                .get("bios9i")
                .and_then(Value::as_str)
                .map(str::to_owned),
            bios7i: config
                .get("bios7i")
                .and_then(Value::as_str)
                .map(str::to_owned),
            firmware: config
                .get("firmware")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    pub fn save_settings(&self, id: &str, settings: MachineSettings) -> Result<MachineSettings> {
        let machine = self.machine_dir(id)?;
        assert_runtime_stopped(&machine)?;
        fn validated(path: Option<String>, sizes: &[u64], label: &str) -> Result<Value> {
            match path.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
                None => Ok(Value::Null),
                Some(value) => {
                    let path = PathBuf::from(&value);
                    let metadata =
                        fs::symlink_metadata(&path).map_err(|e| format!("{label}: {e}"))?;
                    if metadata.file_type().is_symlink()
                        || !metadata.is_file()
                        || !sizes.contains(&metadata.len())
                    {
                        return Err(format!(
                            "{label} must be a regular user-owned file of the expected size"
                        ));
                    }
                    if !path.is_absolute() {
                        return Err(format!("{label} path must be absolute"));
                    }
                    Ok(Value::String(path.to_string_lossy().into_owned()))
                }
            }
        }
        let path = machine.join("runtime/config.json");
        let mut config = read_json(&path)?;
        let object = config
            .as_object_mut()
            .ok_or("Runtime config is not a JSON object")?;
        object.insert(
            "virtual_sd_enabled".into(),
            json!(settings.virtual_sd_enabled),
        );
        object.insert("sound_compatibility".into(), json!(settings.sound_compatibility));
        object.insert(
            "bios9i".into(),
            validated(settings.bios9i, &[65536], "ARM9i BIOS")?,
        );
        object.insert(
            "bios7i".into(),
            validated(settings.bios7i, &[65536], "ARM7i BIOS")?,
        );
        object.insert(
            "firmware".into(),
            validated(settings.firmware, &[131072, 262144, 524288], "DSi firmware")?,
        );
        atomic_json(&path, &config)?;
        self.settings(id)
    }

    pub fn boot_files(&self, id: &str) -> Result<Vec<(String, Option<PathBuf>)>> {
        let machine = self.machine_dir(id)?;
        let config = read_json(&machine.join("runtime/config.json"))?;
        let folders = [
            self.root.join("system-files"),
            self.root.join("BIOS"),
            self.root.join("BIOS/dsidump"),
            self.root.join("BIOS & extra"),
            self.root.join("BIOS & extra/dsidump"),
            self.root
                .parent()
                .ok_or("No parent workspace")?
                .join("BIOS BACKUP/dsidump"),
        ];
        let specifications: [(&str, &[&str], Option<&str>, &[u64]); 5] = [
            (
                "dsi_bios9.bin",
                &["dsi_bios9.bin", "bios9i.bin"],
                Some("bios9i"),
                &[65536],
            ),
            (
                "dsi_bios7.bin",
                &["dsi_bios7.bin", "bios7i.bin"],
                Some("bios7i"),
                &[65536],
            ),
            (
                "dsi_firmware.bin",
                &["dsi_firmware.bin", "dsfirmware.bin", "firmware.bin"],
                Some("firmware"),
                &[131072, 262144, 524288],
            ),
            ("bios9.bin", &["bios9.bin"], Some("bios9"), &[4096]),
            ("bios7.bin", &["bios7.bin"], Some("bios7"), &[16384]),
        ];
        Ok(specifications
            .iter()
            .map(|(name, aliases, key, sizes)| {
                let configured = key
                    .and_then(|k| config.get(k))
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                let path = configured
                    .into_iter()
                    .chain(
                        folders
                            .iter()
                            .flat_map(|folder| aliases.iter().map(move |alias| folder.join(alias))),
                    )
                    .find(|p| {
                        p.metadata()
                            .map(|m| m.is_file() && sizes.contains(&m.len()))
                            .unwrap_or(false)
                    });
                ((*name).to_owned(), path)
            })
            .collect())
    }

    pub fn readiness(&self, id: &str) -> Result<Readiness> {
        let machine = self.machine_dir(id)?;
        let profile = read_json(&machine.join("machine.json"))?;
        let mut missing = Vec::new();
        let files = self.boot_files(id)?;
        for (name, path) in &files {
            if path.is_none() {
                missing.push(name.clone());
            }
        }
        if !["melondsds_libretro.dll", "melondsds_system_compat.dll"]
            .iter()
            .any(|name| self.root.join("imports/libretro").join(name).is_file())
        {
            missing.push("melonDS DS execution core".into());
        }
        let virtual_nand = machine.join("nand/virtual_nand.bin");
        if !virtual_nand.is_file()
            || hash_file(&virtual_nand)? != field(&profile, "virtual_nand_sha256")
        {
            missing.push("verified virtual NAND".into());
        }
        let record = optional_json(&machine.join("runtime/menu_boot_verification.json"));
        let screenshot = record
            .get("screenshot")
            .and_then(Value::as_str)
            .map(|p| machine.join(p));
        let menu_boot_verified = record.get("source_nand_sha256").and_then(Value::as_str)
            == Some(field(&profile, "virtual_nand_sha256"))
            && screenshot.as_ref().is_some_and(|p| {
                p.is_file()
                    && hash_file(p).ok().as_deref()
                        == record.get("screenshot_sha256").and_then(Value::as_str)
            })
            && files.iter().all(|(name, path)| {
                match record.get("boot_file_sha256").and_then(|v| v.get(name)) {
                    Some(expected) => path
                        .as_ref()
                        .is_some_and(|p| hash_file(p).ok().as_deref() == expected.as_str()),
                    None => true,
                }
            });
        Ok(Readiness {
            ready_to_attempt_boot: missing.is_empty(),
            missing,
            menu_boot_verified,
            nand_association: field(&profile, "nand_association").to_owned(),
            backend: "CPF console shell + melonDS DS libretro adapter".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_ids_cannot_escape_workspace() {
        let workspace = Workspace {
            root: PathBuf::from("C:/not-a-workspace"),
        };
        assert!(workspace.machine_dir("../bad.cpfmachine").is_err());
        assert!(workspace.machine_dir("\\bad.cpfmachine").is_err());
        assert!(workspace.machine_dir("bad").is_err());
    }

    #[test]
    fn settings_preserve_existing_runtime_fields() {
        let unique = format!(
            "cpf-settings-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let root = env::temp_dir().join(unique);
        let machine = root.join("machines/Test DSi.cpfmachine");
        fs::create_dir_all(machine.join("runtime")).unwrap();
        let config = machine.join("runtime/config.json");
        fs::write(
            &config,
            b"{\"backend\":\"melonDS process\",\"virtual_sd_enabled\":false}",
        )
        .unwrap();
        let workspace = Workspace::open(root.clone()).unwrap();
        let saved = workspace
            .save_settings(
                "Test DSi.cpfmachine",
                MachineSettings {
                    virtual_sd_enabled: true,
                    sound_compatibility: true,
                    bios9i: None,
                    bios7i: None,
                    firmware: None,
                },
            )
            .unwrap();
        assert!(saved.virtual_sd_enabled);
        let content = read_json(&config).unwrap();
        assert_eq!(content["backend"], "melonDS process");
        assert_eq!(content["virtual_sd_enabled"], true);
        fs::remove_file(config).unwrap();
        fs::remove_dir(machine.join("runtime")).unwrap();
        fs::remove_dir(machine).unwrap();
        fs::remove_dir(root.join("machines")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
