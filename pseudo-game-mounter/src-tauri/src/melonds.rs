use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

#[cfg(windows)]
fn running_clone(executable: &Path) -> Result<bool, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let expected = executable.canonicalize().map_err(|error| error.to_string())?;
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err("Could not check for an existing melonDS session".into());
        }
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more {
            let end = entry.szExeFile.iter().position(|ch| *ch == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if name.eq_ignore_ascii_case("melonDS.exe") {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !process.is_null() {
                    let mut buffer = vec![0u16; 32768];
                    let mut length = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length) != 0 {
                        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
                        if let Ok(actual) = path.canonicalize() {
                            if actual.to_string_lossy().eq_ignore_ascii_case(&expected.to_string_lossy()) {
                                found = true;
                            }
                        }
                    }
                    CloseHandle(process);
                }
            }
            if found { break; }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        Ok(found)
    }
}

#[cfg(not(windows))]
fn running_clone(_executable: &Path) -> Result<bool, String> { Ok(false) }

#[derive(Clone, Serialize)]
pub struct PreparedEmulator {
    pub executable: PathBuf,
    pub source_config: PathBuf,
    pub working_nand: PathBuf,
}

#[derive(Serialize, Deserialize)]
struct Origin {
    source_nand: String,
    source_firmware: String,
}

pub fn data_root() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unavailable")?;
    Ok(PathBuf::from(local).join("PseudoGameMounter"))
}

#[cfg(windows)]
pub fn close_managed(child: &mut Child) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };

    struct Target { pid: u32, posted: bool }
    unsafe extern "system" fn close_window(hwnd: HWND, context: LPARAM) -> i32 {
        let target = &mut *(context as *mut Target);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == target.pid && IsWindowVisible(hwnd) != 0 {
            target.posted = PostMessageW(hwnd, WM_CLOSE, 0, 0) != 0;
            return 0;
        }
        1
    }

    let mut target = Target { pid: child.id(), posted: false };
    unsafe { EnumWindows(Some(close_window), &mut target as *mut Target as LPARAM); }
    if !target.posted {
        return Err("Could not ask the managed melonDS window to close. Close it manually, then retry.".into());
    }
    for _ in 0..100 {
        if child.try_wait().map_err(|error| error.to_string())?.is_some() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("melonDS did not close within 10 seconds. Close its window manually; the replacement was not started.".into())
}

#[cfg(not(windows))]
pub fn close_managed(_child: &mut Child) -> Result<(), String> {
    Err("Managed emulator restart is supported on Windows only".into())
}

pub fn packaged_backend() -> Option<PathBuf> {
    let app_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let executable = app_dir.join("runtime").join("melonDS").join("melonDS.exe");
    let metadata = fs::symlink_metadata(&executable).ok()?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        Some(executable)
    } else {
        None
    }
}

pub fn ensure_packaged_config_dir(executable: &Path) -> Result<(), String> {
    if packaged_backend().as_deref() == Some(executable) {
        let portable = executable.parent().ok_or("Invalid packaged backend path")?.join("portable");
        fs::create_dir_all(&portable)
            .map_err(|error| format!("Could not create the packaged melonDS portable settings folder: {error}"))?;
    }
    Ok(())
}

fn create_working_copy(source: &Path, destination: &Path) -> Result<(), String> {
    if destination.exists() { return Ok(()); }
    let temporary = destination.with_extension(format!("copying-{}", std::process::id()));
    let copy = (|| {
        fs::copy(source, &temporary).map_err(|error| error.to_string())?;
        let source_len = fs::metadata(source).map_err(|error| error.to_string())?.len();
        let copy_len = fs::metadata(&temporary).map_err(|error| error.to_string())?.len();
        if source_len != copy_len { return Err("Working copy size does not match source".into()); }
        fs::rename(&temporary, destination).map_err(|error| error.to_string())
    })();
    if copy.is_err() { let _ = fs::remove_file(&temporary); }
    copy
}

fn source_config(executable: &Path) -> Result<PathBuf, String> {
    let parent = executable.parent().ok_or("Invalid melonDS executable path")?;
    let portable = parent.join("portable/melonDS.toml");
    if portable.is_file() { return Ok(portable); }
    if packaged_backend().as_deref() == Some(executable) {
        return Err("Configure the packaged melonDS backend first. Open its settings, add your own DSi BIOS, firmware, and NAND, then close it.".into());
    }
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let regular = PathBuf::from(roaming).join("melonDS/melonDS.toml");
        if regular.is_file() { return Ok(regular); }
    }
    Err("No melonDS.toml found. Configure melonDS for DSi mode first, then select its executable again.".into())
}

fn table<'a>(root: &'a toml::Value, name: &str) -> Result<&'a toml::value::Table, String> {
    root.get(name).and_then(toml::Value::as_table)
        .ok_or_else(|| format!("melonDS configuration has no [{name}] section"))
}

fn required_path(root: &toml::Value, section: &str, key: &str, config: &Path) -> Result<PathBuf, String> {
    let raw = table(root, section)?.get(key).and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Set {section}.{key} in melonDS before mounting"))?;
    let path = PathBuf::from(raw);
    let path = if path.is_absolute() { path } else {
        config.parent().unwrap().join(path)
    };
    if !path.is_file() { return Err(format!("Configured {section}.{key} is missing: {}", path.display())); }
    Ok(path)
}

fn set_path(root: &mut toml::Value, section: &str, key: &str, path: &Path) -> Result<(), String> {
    let target = root.get_mut(section).and_then(toml::Value::as_table_mut)
        .ok_or_else(|| format!("Missing [{section}] configuration"))?;
    target.insert(key.into(), toml::Value::String(path.display().to_string()));
    Ok(())
}

pub fn prepare(executable: &Path) -> Result<PreparedEmulator, String> {
    let metadata = fs::symlink_metadata(executable).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink()
        || !executable.file_name().is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("melonDS.exe")) {
        return Err("Select a regular melonDS.exe file".into());
    }
    let config_path = source_config(executable)?;
    let config_text = fs::read_to_string(&config_path).map_err(|error| error.to_string())?;
    let mut config: toml::Value = toml::from_str(&config_text)
        .map_err(|error| format!("Cannot read melonDS.toml: {error}"))?;
    let emu = table(&config, "Emu")?;
    if emu.get("ConsoleType").and_then(toml::Value::as_integer) != Some(1) {
        return Err("Set melonDS Console Type to Nintendo DSi before mounting".into());
    }
    if emu.get("DirectBoot").and_then(toml::Value::as_bool) != Some(false) {
        return Err("Turn off Direct Boot in melonDS so the original DSi Menu loads first".into());
    }
    let bios9 = required_path(&config, "DSi", "BIOS9Path", &config_path)?;
    let bios7 = required_path(&config, "DSi", "BIOS7Path", &config_path)?;
    let firmware = required_path(&config, "DSi", "FirmwarePath", &config_path)?;
    let nand = required_path(&config, "DSi", "NANDPath", &config_path)?;
    let working = data_root()?.join("melonDS");
    let cloned_executable = working.join("melonDS.exe");
    if cloned_executable.exists() && running_clone(&cloned_executable)? {
        return Err("This mounter's melonDS session is already running. Close it before mounting another cartridge.".into());
    }
    let portable = working.join("portable");
    fs::create_dir_all(&portable).map_err(|error| error.to_string())?;
    let saves = working.join("saves");
    let states = working.join("savestates");
    let cheats = working.join("cheats");
    for path in [&saves, &states, &cheats] {
        fs::create_dir_all(path).map_err(|error| error.to_string())?;
    }
    let origin_path = working.join("source.json");
    let source_nand = nand.canonicalize().map_err(|error| error.to_string())?.display().to_string();
    let source_firmware = firmware.canonicalize().map_err(|error| error.to_string())?.display().to_string();
    if origin_path.exists() {
        let origin: Origin = serde_json::from_slice(&fs::read(&origin_path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        if origin.source_nand != source_nand || origin.source_firmware != source_firmware {
            return Err("This mounter already has a working DSi from different source files. Move its LocalAppData/PseudoGameMounter/melonDS folder aside before starting a separate machine.".into());
        }
    }
    let working_nand = working.join("working_nand.bin");
    let working_firmware = working.join("working_firmware.bin");
    if !origin_path.exists() {
        let origin = Origin { source_nand, source_firmware };
        fs::write(&origin_path, serde_json::to_vec_pretty(&origin).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    }
    create_working_copy(&nand, &working_nand)
        .map_err(|error| format!("NAND working copy failed: {error}"))?;
    create_working_copy(&firmware, &working_firmware)
        .map_err(|error| format!("Firmware working copy failed: {error}"))?;
    set_path(&mut config, "DSi", "BIOS9Path", &bios9)?;
    set_path(&mut config, "DSi", "BIOS7Path", &bios7)?;
    set_path(&mut config, "DSi", "FirmwarePath", &working_firmware)?;
    set_path(&mut config, "DSi", "NANDPath", &working_nand)?;
    let root = config.as_table_mut().ok_or("Invalid melonDS configuration root")?;
    let instance = root.entry("Instance0").or_insert_with(|| toml::Value::Table(toml::value::Table::new()))
        .as_table_mut().ok_or("Invalid melonDS [Instance0] configuration")?;
    for (key, path) in [
        ("SaveFilePath", &saves),
        ("SavestatePath", &states),
        ("CheatFilePath", &cheats),
    ] {
        instance.insert(key.into(), toml::Value::String(path.display().to_string()));
    }
    if config.get("DS").and_then(toml::Value::as_table).is_some() {
        for key in ["BIOS9Path", "BIOS7Path", "FirmwarePath"] {
            if let Ok(path) = required_path(&config, "DS", key, &config_path) {
                set_path(&mut config, "DS", key, &path)?;
            }
        }
    }
    let clone_config = toml::to_string_pretty(&config).map_err(|error| error.to_string())?;
    fs::write(portable.join("melonDS.toml"), clone_config)
        .map_err(|error| error.to_string())?;
    fs::copy(executable, &cloned_executable)
        .map_err(|error| format!("Copying melonDS executable failed: {error}"))?;
    Ok(PreparedEmulator { executable: cloned_executable, source_config: config_path, working_nand })
}

pub fn launch(prepared: &PreparedEmulator, cartridge: &Path) -> Result<Child, String> {
    Command::new(&prepared.executable)
        .arg("--boot=always")
        .arg(cartridge)
        .current_dir(prepared.executable.parent().unwrap())
        .spawn()
        .map_err(|error| format!("Could not start melonDS: {error}"))
}
