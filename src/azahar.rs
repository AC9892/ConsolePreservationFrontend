//! External Azahar process adapter. The runtime receives a validated title;
//! firmware selection and version metadata remain in the firmware module.
use crate::cartridge::CartridgeImage;
use crate::firmware::{existing_absolute_path, ValidatedHomeMenu};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
#[cfg(windows)]
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    PostMessageW, WM_CLOSE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendError {
    ExecutableUnavailable,
    PortableUserDirectoryRequired,
    SystemConfigurationUnavailable,
    NetworkUpdateCheckEnabled,
    PerTitleConfigurationIncompatible,
    InputConfigurationUnavailable,
    CartridgeConfigurationUnavailable,
    CartridgeSourceUnavailable,
    RuntimeBusy,
    SdStorageUnavailable,
    SdTitleAlreadyInstalled,
    SdSourceChanged,
    SdInstallVerificationFailed,
    StorageOverlapsSource,
    StartFailed,
    ProcessFailed,
}

impl BackendError {
    pub fn label(self) -> &'static str {
        match self {
            Self::ExecutableUnavailable => "Azahar executable unavailable",
            Self::PortableUserDirectoryRequired => "Azahar portable user directory required",
            Self::SystemConfigurationUnavailable => "Azahar portable system configuration unavailable or ambiguous",
            Self::NetworkUpdateCheckEnabled => "Azahar automatic update check must be disabled in the portable configuration",
            Self::PerTitleConfigurationIncompatible => "Azahar per-title configuration may override the checked system settings",
            Self::InputConfigurationUnavailable => "Azahar active keyboard button profile is unavailable or unsupported by the live shell",
            Self::CartridgeConfigurationUnavailable => "Azahar startup cartridge configuration is unavailable or ambiguous",
            Self::CartridgeSourceUnavailable => "selected cartridge is unavailable or failed preflight; eject or select it again before starting",
            Self::RuntimeBusy => "this portable runtime is in use by another CPF operation",
            Self::SdStorageUnavailable => "virtual SD storage is disabled, redirected, or unavailable",
            Self::SdTitleAlreadyInstalled => "this title already has installed content or a ticket; existing data was preserved",
            Self::SdSourceChanged => "CIA source changed after validation",
            Self::SdInstallVerificationFailed => "installed SD metadata or content did not match the validated CIA",
            Self::StorageOverlapsSource => "Azahar writable storage overlaps preserved source material",
            Self::StartFailed => "Azahar process failed to start",
            Self::ProcessFailed => "Azahar process operation failed",
        }
    }
}

pub struct AzaharBackend {
    executable: PathBuf,
    portable_user_dir: PathBuf,
    system_settings: AzaharSystemSettings,
    button_keys: Option<AzaharButtonKeys>,
}

/// Windows virtual keys for the pictured A, B, X, Y, Up, Down, Left,
/// Right, Start, Select, and Home buttons, in that order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzaharButtonKeys(pub [u16; 11]);

impl AzaharButtonKeys {
    pub const DEFAULT: Self = Self([
        0x41, 0x53, 0x5a, 0x58, 0x54, 0x47, 0x46, 0x48, 0x4d, 0x4e, 0x42,
    ]);

    pub fn encode(self) -> String {
        self.0
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn decode(value: &str) -> Option<Self> {
        let values = value
            .split(',')
            .map(str::parse::<u16>)
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        Some(Self(values.try_into().ok()?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AzaharSystemSettings {
    pub is_new_3ds: bool,
    pub region_value: i32,
}

impl AzaharBackend {
    pub fn new(executable: &Path) -> Result<Self, BackendError> {
        let executable =
            existing_absolute_path(executable).map_err(|_| BackendError::ExecutableUnavailable)?;
        if !executable.is_file() {
            return Err(BackendError::ExecutableUnavailable);
        }
        let directory = executable
            .parent()
            .ok_or(BackendError::ExecutableUnavailable)?;
        // Azahar uses the existing "user" directory beside its executable
        // for portable storage on Windows. Require it to avoid the default
        // roaming profile and keep this CPF session's writes isolated.
        let portable_user_dir = directory.join("user");
        if !portable_user_dir.is_dir() {
            return Err(BackendError::PortableUserDirectoryRequired);
        }
        let portable_user_dir = existing_absolute_path(&portable_user_dir)
            .map_err(|_| BackendError::PortableUserDirectoryRequired)?;
        let config_path = portable_user_dir.join("config").join("qt-config.ini");
        let metadata =
            fs::metadata(&config_path).map_err(|_| BackendError::SystemConfigurationUnavailable)?;
        if metadata.len() > 1024 * 1024 {
            return Err(BackendError::SystemConfigurationUnavailable);
        }
        let config = fs::read_to_string(&config_path)
            .map_err(|_| BackendError::SystemConfigurationUnavailable)?;
        let system_settings =
            parse_system_settings(&config).ok_or(BackendError::SystemConfigurationUnavailable)?;
        if !update_check_disabled(&config) {
            return Err(BackendError::NetworkUpdateCheckEnabled);
        }
        let button_keys = parse_button_keys(&config);
        Ok(Self {
            executable,
            portable_user_dir,
            system_settings,
            button_keys,
        })
    }

    pub fn system_settings(&self) -> AzaharSystemSettings {
        self.system_settings
    }

    fn acquire_runtime_lock(&self) -> Result<fs::File, BackendError> {
        let path = self.portable_user_dir.join("cpf-runtime.lock");
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| !metadata.is_file() || metadata.file_type().is_symlink())
        {
            return Err(BackendError::RuntimeBusy);
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|_| BackendError::RuntimeBusy)?;
        lock.try_lock().map_err(|_| BackendError::RuntimeBusy)?;
        Ok(lock)
    }

    pub fn start_sd_install(
        &self,
        package: &crate::sd_media::SdPackage,
    ) -> Result<AzaharInstallSession, BackendError> {
        let runtime_lock = self.acquire_runtime_lock()?;
        let config = fs::read_to_string(self.portable_user_dir.join("config/qt-config.ini"))
            .map_err(|_| BackendError::SdStorageUnavailable)?;
        if !default_storage_selected(&config)
            || effective_config_bool(&config, "Data Storage", "use_virtual_sd", true) != Some(true)
        {
            return Err(BackendError::SdStorageUnavailable);
        }
        if package.path().starts_with(&self.portable_user_dir) {
            return Err(BackendError::StorageOverlapsSource);
        }
        self.ensure_sd_title_absent(package.title_id())?;
        if !package.source_unchanged() {
            return Err(BackendError::SdSourceChanged);
        }
        let directory = self
            .executable
            .parent()
            .ok_or(BackendError::ExecutableUnavailable)?;
        let mut child = Command::new(&self.executable)
            .arg("-i")
            .arg(package.path())
            .current_dir(directory)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| BackendError::StartFailed)?;
        let job = crate::process_lifecycle::ProcessJob::attach(&mut child)
            .map_err(|_| BackendError::StartFailed)?;
        Ok(AzaharInstallSession {
            session: AzaharSession {
                _job: job,
                _cartridge: None,
                child,
                _runtime_lock: runtime_lock,
            },
        })
    }

    fn sd_title_directory(&self, title_id: u64) -> Result<PathBuf, BackendError> {
        unredirected_storage_path(&self.portable_user_dir, Path::new(&format!(
            "sdmc/Nintendo 3DS/00000000000000000000000000000000/00000000000000000000000000000000/title/{:08x}/{:08x}",
            title_id >> 32, title_id as u32
        )))
    }

    fn ensure_sd_title_absent(&self, title_id: u64) -> Result<(), BackendError> {
        if self.sd_title_directory(title_id)?.exists() {
            return Err(BackendError::SdTitleAlreadyInstalled);
        }
        let tickets =
            unredirected_storage_path(&self.portable_user_dir, Path::new("nand/dbs/ticket.db"))?;
        if tickets.is_dir() {
            let prefix = format!("{title_id:016X}.");
            for entry in fs::read_dir(tickets).map_err(|_| BackendError::SdStorageUnavailable)? {
                let entry = entry.map_err(|_| BackendError::SdStorageUnavailable)?;
                if entry
                    .file_name()
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with(&prefix)
                {
                    return Err(BackendError::SdTitleAlreadyInstalled);
                }
            }
        }
        Ok(())
    }

    /// Read-only title verification outside an active installation session.
    pub fn verify_sd_install_idle(
        &self,
        package: &crate::sd_media::SdPackage,
    ) -> Result<(), BackendError> {
        let _runtime_lock = self.acquire_runtime_lock()?;
        self.verify_sd_install(package)
    }

    pub fn verify_sd_install(
        &self,
        package: &crate::sd_media::SdPackage,
    ) -> Result<(), BackendError> {
        if !package.source_unchanged() {
            return Err(BackendError::SdSourceChanged);
        }
        let directory = self.sd_title_directory(package.title_id())?;
        unredirected_storage_path(
            &self.portable_user_dir,
            &directory
                .strip_prefix(&self.portable_user_dir)
                .map_err(|_| BackendError::SdStorageUnavailable)?
                .join("content"),
        )?;
        if !package.installed_metadata_matches(&directory)
            || !package.installed_contents_match(&directory)
        {
            return Err(BackendError::SdInstallVerificationFailed);
        }
        Ok(())
    }

    /// Select or eject the read-only game-card image for the next startup.
    /// This edits the portable runtime configuration only.
    pub fn set_startup_cartridge(
        &self,
        cartridge: Option<&CartridgeImage>,
    ) -> Result<(), BackendError> {
        let _runtime_lock = self.acquire_runtime_lock()?;
        let value = if let Some(cartridge) = cartridge {
            if cartridge.path().starts_with(&self.portable_user_dir) {
                return Err(BackendError::StorageOverlapsSource);
            }
            Some(
                cartridge
                    .path()
                    .to_str()
                    .ok_or(BackendError::CartridgeConfigurationUnavailable)?,
            )
        } else {
            None
        };
        let path = self.portable_user_dir.join("config").join("qt-config.ini");
        if fs::metadata(&path)
            .map_err(|_| BackendError::CartridgeConfigurationUnavailable)?
            .len()
            > 1024 * 1024
        {
            return Err(BackendError::CartridgeConfigurationUnavailable);
        }
        let text = fs::read_to_string(&path)
            .map_err(|_| BackendError::CartridgeConfigurationUnavailable)?;
        let updated = with_startup_cartridge(&text, value)
            .ok_or(BackendError::CartridgeConfigurationUnavailable)?;
        if updated != text {
            fs::write(path, updated)
                .map_err(|_| BackendError::CartridgeConfigurationUnavailable)?;
        }
        Ok(())
    }

    pub fn prepare_home_return(
        &self,
        home: &ValidatedHomeMenu,
        backup: &Path,
    ) -> Result<crate::runtime_setup::ReturnTitleState, crate::runtime_setup::SetupError> {
        use crate::runtime_setup::SetupError;
        let _runtime_lock = self
            .acquire_runtime_lock()
            .map_err(|_| SetupError::RuntimeBusy)?;
        let path = self.portable_user_dir.join("config").join("qt-config.ini");
        if fs::metadata(&path)
            .map_err(|_| SetupError::StorageUnavailable)?
            .len()
            > 1024 * 1024
        {
            return Err(SetupError::StorageUnavailable);
        }
        let config = fs::read_to_string(path).map_err(|_| SetupError::StorageUnavailable)?;
        if !default_storage_selected(&config) {
            return Err(SetupError::StorageUnavailable);
        }
        crate::runtime_setup::prepare_return_title(
            &self.portable_user_dir,
            home.path(),
            backup,
            home.title_id(),
        )
    }

    /// Select native screen windows for the device shell. This only edits the
    /// portable emulator configuration, never the preserved console source.
    pub fn prepare_separate_windows(&self) -> Result<(), BackendError> {
        let _runtime_lock = self.acquire_runtime_lock()?;
        let path = self.portable_user_dir.join("config").join("qt-config.ini");
        let text =
            fs::read_to_string(&path).map_err(|_| BackendError::SystemConfigurationUnavailable)?;
        let mut section = "";
        let mut layout = 0;
        let mut layout_default = 0;
        let mut single = 0;
        let mut single_default = 0;
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed;
            }
            let replacement = match (section, trimmed.split_once('=').map(|(key, _)| key)) {
                ("[Layout]", Some("layout_option")) => {
                    layout += 1;
                    Some("layout_option=4")
                }
                ("[Layout]", Some("layout_option\\default")) => {
                    layout_default += 1;
                    Some("layout_option\\default=false")
                }
                ("[UI]", Some("singleWindowMode")) => {
                    single += 1;
                    Some("singleWindowMode=false")
                }
                ("[UI]", Some("singleWindowMode\\default")) => {
                    single_default += 1;
                    Some("singleWindowMode\\default=false")
                }
                _ => None,
            };
            if let Some(value) = replacement {
                out.push_str(value);
                if line.ends_with("\r\n") {
                    out.push_str("\r\n");
                } else if line.ends_with('\n') {
                    out.push('\n');
                }
            } else {
                out.push_str(line);
            }
        }
        if (layout, layout_default, single, single_default) != (1, 1, 1, 1) {
            return Err(BackendError::SystemConfigurationUnavailable);
        }
        if out != text {
            fs::write(path, out).map_err(|_| BackendError::SystemConfigurationUnavailable)?;
        }
        Ok(())
    }

    pub fn button_keys(&self) -> Result<AzaharButtonKeys, BackendError> {
        self.button_keys
            .ok_or(BackendError::InputConfigurationUnavailable)
    }

    pub fn start(&self, home: &ValidatedHomeMenu) -> Result<AzaharSession, BackendError> {
        let runtime_lock = self.acquire_runtime_lock()?;
        let config_path = self.portable_user_dir.join("config/qt-config.ini");
        if fs::metadata(&config_path)
            .map_err(|_| BackendError::CartridgeConfigurationUnavailable)?
            .len()
            > 1024 * 1024
        {
            return Err(BackendError::CartridgeConfigurationUnavailable);
        }
        let config = fs::read_to_string(config_path)
            .map_err(|_| BackendError::CartridgeConfigurationUnavailable)?;
        let cartridge = startup_cartridge_path(&config)
            .ok_or(BackendError::CartridgeConfigurationUnavailable)?
            .map(|path| {
                CartridgeImage::open(&path).map_err(|_| BackendError::CartridgeSourceUnavailable)
            })
            .transpose()?;
        if cartridge
            .as_ref()
            .is_some_and(|image| image.path().starts_with(&self.portable_user_dir))
        {
            return Err(BackendError::StorageOverlapsSource);
        }
        let source_dir = home
            .path()
            .parent()
            .ok_or(BackendError::StorageOverlapsSource)?;
        if self.portable_user_dir.starts_with(source_dir)
            || home.path().starts_with(&self.portable_user_dir)
        {
            return Err(BackendError::StorageOverlapsSource);
        }
        let per_title = self
            .portable_user_dir
            .join("config")
            .join("custom")
            .join(format!("{:016X}.ini", home.title_id()));
        if per_title.exists() {
            let metadata = fs::metadata(&per_title)
                .map_err(|_| BackendError::PerTitleConfigurationIncompatible)?;
            if metadata.len() > 1024 * 1024 {
                return Err(BackendError::PerTitleConfigurationIncompatible);
            }
            let config = fs::read_to_string(&per_title)
                .map_err(|_| BackendError::PerTitleConfigurationIncompatible)?;
            if !per_title_uses_global_system(&config) {
                return Err(BackendError::PerTitleConfigurationIncompatible);
            }
        }
        let directory = self
            .executable
            .parent()
            .ok_or(BackendError::ExecutableUnavailable)?;
        let mut child = Command::new(&self.executable)
            // The top-level Azahar getopt parser can mistake --windowed for
            // compression options before Qt sees it. The short form reaches Qt.
            .arg("-w")
            .arg(home.path())
            .current_dir(directory)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| BackendError::StartFailed)?;
        let job = crate::process_lifecycle::ProcessJob::attach(&mut child)
            .map_err(|_| BackendError::StartFailed)?;
        Ok(AzaharSession {
            _job: job,
            _cartridge: cartridge,
            child,
            _runtime_lock: runtime_lock,
        })
    }
}

fn startup_cartridge_path(text: &str) -> Option<Option<PathBuf>> {
    let mut in_ui = false;
    let mut seen_ui = false;
    let mut value = None;
    let mut default = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_ui = line == "[UI]";
            if in_ui && std::mem::replace(&mut seen_ui, true) {
                return None;
            }
            continue;
        }
        if !in_ui {
            continue;
        }
        let Some((key, data)) = line.split_once('=') else {
            continue;
        };
        let data = data.trim();
        match key.trim().replace('\\', "/").as_str() {
            "Paths/inserted_cartridge" => {
                if value.replace(data).is_some() {
                    return None;
                }
            }
            "Paths/inserted_cartridge/default" => {
                let flag = match data {
                    "true" | "1" => true,
                    "false" | "0" => false,
                    _ => return None,
                };
                if default.replace(flag).is_some() {
                    return None;
                }
            }
            _ => {}
        }
    }
    if default == Some(true) {
        return Some(None);
    }
    let raw = value.unwrap_or("");
    let path = if raw.starts_with('"') {
        raw.strip_prefix('"')?.strip_suffix('"')?
    } else {
        // An unquoted comma denotes a Qt list, not one media path.
        if raw.contains(',') {
            return None;
        }
        raw
    };
    // CPF writes forward slashes. Reject unsupported Qt escapes/typed values
    // rather than guessing which path the backend would actually receive.
    if path.contains(['\\', '"']) || path.chars().any(char::is_control) || path.starts_with('@') {
        return None;
    }
    if path.is_empty() {
        return Some(None);
    }
    let path = PathBuf::from(path);
    path.is_absolute().then_some(Some(path))
}

fn with_startup_cartridge(text: &str, path: Option<&str>) -> Option<String> {
    let path = path.unwrap_or("");
    if path.chars().any(char::is_control) {
        return None;
    }
    let value = format!(
        "Paths\\inserted_cartridge=\"{}\"",
        path.replace('\\', "/").replace('"', "\\\"")
    );
    let default = "Paths\\inserted_cartridge\\default=false";
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut in_ui = false;
    let mut ui_start = None;
    let mut ui_end = lines.len();
    let mut value_at = None;
    let mut default_at = None;
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.starts_with('[') && line.ends_with(']') {
            if in_ui {
                ui_end = index;
            }
            in_ui = line == "[UI]";
            if in_ui && ui_start.replace(index).is_some() {
                return None;
            }
            continue;
        }
        if !in_ui {
            continue;
        }
        let Some((key, _)) = line.split_once('=') else {
            continue;
        };
        let slot = match key.trim().replace('\\', "/").as_str() {
            "Paths/inserted_cartridge" => &mut value_at,
            "Paths/inserted_cartridge/default" => &mut default_at,
            _ => continue,
        };
        if slot.replace(index).is_some() {
            return None;
        }
    }
    let mut missing = Vec::new();
    if let Some(index) = value_at {
        lines[index] = value;
    } else {
        missing.push(value);
    }
    if let Some(index) = default_at {
        lines[index] = default.into();
    } else {
        missing.push(default.into());
    }
    if ui_start.is_none() {
        lines.push("[UI]".into());
        lines.extend(missing);
    } else {
        lines.splice(ui_end..ui_end, missing);
    }
    Some(lines.join(newline) + newline)
}

fn unredirected_storage_path(user: &Path, relative: &Path) -> Result<PathBuf, BackendError> {
    let mut path = user.to_path_buf();
    for component in relative.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err(BackendError::SdStorageUnavailable);
        }
        path.push(component);
        match path.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(BackendError::SdStorageUnavailable),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(BackendError::SdStorageUnavailable),
        }
    }
    Ok(path)
}

fn default_storage_selected(text: &str) -> bool {
    effective_config_bool(text, "Data Storage", "use_custom_storage", false) == Some(false)
}

fn effective_config_bool(text: &str, section: &str, setting: &str, fallback: bool) -> Option<bool> {
    let mut in_storage = false;
    let mut enabled = None;
    let mut default_used = None;
    let section = format!("[{section}]");
    let default_key = format!("{setting}/default");
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_storage = line == section;
            continue;
        }
        if !in_storage {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key.trim().replace('\\', "/").as_str() {
            name if name == setting => &mut enabled,
            name if name == default_key => &mut default_used,
            _ => continue,
        };
        if slot.is_some() {
            return None;
        }
        *slot = match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => return None,
        };
    }
    Some(if default_used == Some(true) {
        fallback
    } else {
        enabled.unwrap_or(fallback)
    })
}

fn parse_button_keys(text: &str) -> Option<AzaharButtonKeys> {
    const NAMES: [&str; 11] = [
        "button_a",
        "button_b",
        "button_x",
        "button_y",
        "button_up",
        "button_down",
        "button_left",
        "button_right",
        "button_start",
        "button_select",
        "button_home",
    ];
    let mut in_controls = false;
    let mut saw_controls = false;
    let mut profile = None;
    let mut bindings = Vec::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_controls = &line[1..line.len() - 1] == "Controls";
            saw_controls |= in_controls;
            continue;
        }
        if !in_controls {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('\\', "/");
        if key == "profile" {
            if profile.is_some() {
                return None;
            }
            profile = Some(value.trim().parse::<usize>().ok()?);
        } else if !key.ends_with("/default") {
            bindings.push((key, value.trim()));
        }
    }
    if !saw_controls {
        return Some(AzaharButtonKeys::DEFAULT);
    }
    let profile_number = profile.unwrap_or(0).checked_add(1)?;
    let prefix = format!("profiles/{profile_number}/");
    let mut keys = [0u16; 11];
    for (index, name) in NAMES.iter().enumerate() {
        let path = format!("{prefix}{name}");
        let mut values = bindings
            .iter()
            .filter(|(key, _)| key == &path)
            .map(|(_, value)| *value);
        let value = values.next()?;
        if values.next().is_some() {
            return None;
        }
        keys[index] = parse_keyboard_binding(value)?;
    }
    Some(AzaharButtonKeys(keys))
}

fn parse_keyboard_binding(value: &str) -> Option<u16> {
    let value = value.trim_matches('"');
    let mut code = None;
    let mut keyboard = false;
    for part in value.split(',') {
        let (key, value) = part.trim().split_once(':')?;
        match key {
            "code" => {
                if code.is_some() {
                    return None;
                }
                code = value.parse::<u32>().ok();
            }
            "engine" => {
                if keyboard {
                    return None;
                }
                keyboard = value == "keyboard";
            }
            _ => return None,
        }
    }
    if !keyboard {
        return None;
    }
    let code = code?;
    match code {
        0x30..=0x39 | 0x41..=0x5a | 0x20 => Some(code as u16),
        0x01000000 => Some(0x1b),              // Escape
        0x01000001 => Some(0x09),              // Tab
        0x01000003 => Some(0x08),              // Backspace
        0x01000004 | 0x01000005 => Some(0x0d), // Return / Enter
        0x01000012 => Some(0x25),              // Left
        0x01000013 => Some(0x26),              // Up
        0x01000014 => Some(0x27),              // Right
        0x01000015 => Some(0x28),              // Down
        0x01000030..=0x0100003b => Some((code - 0x01000030 + 0x70) as u16), // F1-F12
        _ => None,
    }
}

fn parse_system_settings(text: &str) -> Option<AzaharSystemSettings> {
    let mut in_system = false;
    let mut model = None;
    let mut region = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_system = &line[1..line.len() - 1] == "System";
            continue;
        }
        if !in_system {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('\\', "/");
        let value = value.trim();
        match key.as_str() {
            "is_new_3ds" => {
                if model.is_some() {
                    return None;
                }
                model = Some(match value.to_ascii_lowercase().as_str() {
                    "true" | "1" => true,
                    "false" | "0" => false,
                    _ => return None,
                });
            }
            "region_value" => {
                if region.is_some() {
                    return None;
                }
                region = value.parse::<i32>().ok();
                region?;
            }
            "is_new_3ds/default" | "region_value/default"
                if value.eq_ignore_ascii_case("true") || value == "1" =>
            {
                return None
            }
            "is_new_3ds/use_global" | "region_value/use_global"
                if value.eq_ignore_ascii_case("false") || value == "0" =>
            {
                return None
            }
            _ => {}
        }
    }
    Some(AzaharSystemSettings {
        is_new_3ds: model?,
        region_value: region?,
    })
}

fn per_title_uses_global_system(text: &str) -> bool {
    let mut in_system = false;
    let mut model_global = None;
    let mut region_global = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_system = &line[1..line.len() - 1] == "System";
            continue;
        }
        if !in_system {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('\\', "/");
        let is_global = value.trim().eq_ignore_ascii_case("true") || value.trim() == "1";
        match key.as_str() {
            "is_new_3ds/use_global" => {
                if model_global.is_some() {
                    return false;
                }
                model_global = Some(is_global);
            }
            "region_value/use_global" => {
                if region_global.is_some() {
                    return false;
                }
                region_global = Some(is_global);
            }
            _ => {}
        }
    }
    model_global == Some(true) && region_global == Some(true)
}

fn update_check_disabled(text: &str) -> bool {
    let mut in_misc = false;
    let mut disabled = None;
    let mut default_used = false;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_misc = &line[1..line.len() - 1] == "Miscellaneous";
            continue;
        }
        if !in_misc {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().replace('\\', "/");
        let value = value.trim();
        match key.as_str() {
            "check_for_update_on_start" => {
                if disabled.is_some() {
                    return false;
                }
                disabled = Some(value.eq_ignore_ascii_case("false") || value == "0");
            }
            "check_for_update_on_start/default" => {
                if default_used {
                    return false;
                }
                default_used = value.eq_ignore_ascii_case("true") || value == "1";
            }
            _ => {}
        }
    }
    disabled == Some(true) && !default_used
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sd_install_refuses_existing_title_or_ticket_without_changing_either() {
        let root = std::env::temp_dir().join(format!("cpf-sd-existing-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let backend = AzaharBackend {
            executable: root.join("backend.exe"),
            portable_user_dir: root.clone(),
            system_settings: AzaharSystemSettings {
                is_new_3ds: false,
                region_value: 1,
            },
            button_keys: None,
        };
        let id = 0x0004_0000_1234_5600;
        assert_eq!(backend.ensure_sd_title_absent(id), Ok(()));
        let title = backend.sd_title_directory(id).unwrap();
        fs::create_dir_all(&title).unwrap();
        fs::write(title.join("saved-fixture"), b"preserve").unwrap();
        assert_eq!(
            backend.ensure_sd_title_absent(id),
            Err(BackendError::SdTitleAlreadyInstalled)
        );
        assert_eq!(fs::read(title.join("saved-fixture")).unwrap(), b"preserve");
        let other = id + 0x100;
        let tickets = root.join("nand/dbs/ticket.db");
        fs::create_dir_all(&tickets).unwrap();
        let ticket = tickets.join(format!("{other:016x}.0000000000000000.tik"));
        fs::write(&ticket, b"preserved ticket fixture").unwrap();
        assert_eq!(
            backend.ensure_sd_title_absent(other),
            Err(BackendError::SdTitleAlreadyInstalled)
        );
        assert_eq!(fs::read(ticket).unwrap(), b"preserved ticket fixture");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_lock_blocks_parallel_operations_and_releases_on_drop() {
        let root = std::env::temp_dir().join(format!("cpf-runtime-lock-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let backend = AzaharBackend {
            executable: root.join("backend.exe"),
            portable_user_dir: root.clone(),
            system_settings: AzaharSystemSettings {
                is_new_3ds: false,
                region_value: 1,
            },
            button_keys: None,
        };
        let lock = backend.acquire_runtime_lock().unwrap();
        assert!(matches!(
            backend.acquire_runtime_lock(),
            Err(BackendError::RuntimeBusy)
        ));
        drop(lock);
        drop(backend.acquire_runtime_lock().unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn virtual_sd_uses_effective_defaults_and_rejects_disabled_storage() {
        assert_eq!(
            effective_config_bool(
                "[Data Storage]\nuse_virtual_sd=false\n",
                "Data Storage",
                "use_virtual_sd",
                true
            ),
            Some(false)
        );
        assert_eq!(
            effective_config_bool(
                "[Data Storage]\nuse_virtual_sd=false\nuse_virtual_sd\\default=true\n",
                "Data Storage",
                "use_virtual_sd",
                true
            ),
            Some(true)
        );
        assert_eq!(
            effective_config_bool(
                "[Data Storage]\nuse_virtual_sd=false\nuse_virtual_sd=true\n",
                "Data Storage",
                "use_virtual_sd",
                true
            ),
            None
        );
    }

    #[test]
    fn startup_cartridge_reader_round_trips_selected_paths_and_ejection() {
        let path = std::env::temp_dir().join("CPF test games/title, one.cci");
        let path = path.to_str().unwrap().replace('\\', "/");
        let config = with_startup_cartridge("[UI]\n", Some(&path)).unwrap();
        assert_eq!(
            startup_cartridge_path(&config),
            Some(Some(PathBuf::from(&path)))
        );
        let ejected = with_startup_cartridge(&config, None).unwrap();
        assert_eq!(startup_cartridge_path(&ejected), Some(None));
        assert_eq!(
            startup_cartridge_path("[UI]\nPaths\\inserted_cartridge=\n"),
            Some(None)
        );
        assert_eq!(
            startup_cartridge_path(
                "[UI]\nPaths\\inserted_cartridge=old\nPaths\\inserted_cartridge\\default=true\n"
            ),
            Some(None)
        );
    }

    #[test]
    fn startup_cartridge_reader_refuses_ambiguous_qt_values() {
        for config in [
            "[UI]\nPaths\\inserted_cartridge=one,two",
            "[UI]\nPaths\\inserted_cartridge=\"unterminated",
            "[UI]\nPaths\\inserted_cartridge=relative.cci",
            "[UI]\nPaths\\inserted_cartridge=@Invalid()",
            "[UI]\nPaths\\inserted_cartridge=\"R:\\escaped.cci\"",
            "[UI]\nPaths\\inserted_cartridge=\nPaths\\inserted_cartridge=",
            "[UI]\nPaths\\inserted_cartridge\\default=false\nPaths\\inserted_cartridge/default=false",
            "[UI]\nPaths\\inserted_cartridge\\default=unknown",
            "[UI]\n[UI]\n",
        ] {
            assert!(startup_cartridge_path(config).is_none(), "{config}");
        }
    }

    #[test]
    fn cartridge_setting_preserves_other_configuration_and_supports_ejection() {
        let source = "[UI]\r\nsingleWindowMode=false\r\nPaths\\inserted_cartridge\\default=true\r\nPaths\\inserted_cartridge=old\r\n[System]\r\nis_new_3ds=false\r\nregion_value=1\r\n";
        let selected = with_startup_cartridge(source, Some("R:\\test games\\a,b.cci")).unwrap();
        assert!(selected.contains("Paths\\inserted_cartridge=\"R:/test games/a,b.cci\"\r\n"));
        assert!(selected.contains("Paths\\inserted_cartridge\\default=false\r\n"));
        assert!(selected.ends_with("[System]\r\nis_new_3ds=false\r\nregion_value=1\r\n"));
        assert!(selected.contains("singleWindowMode=false\r\n"));
        assert_eq!(
            with_startup_cartridge(&selected, Some("R:\\test games\\a,b.cci")).unwrap(),
            selected
        );
        let ejected = with_startup_cartridge(&selected, None).unwrap();
        assert!(ejected.contains("Paths\\inserted_cartridge=\"\"\r\n"));
        assert!(!ejected.contains("a,b.cci"));
    }

    #[test]
    fn cartridge_setting_rejects_injected_lines_and_ambiguous_keys() {
        assert!(
            with_startup_cartridge("[UI]\n", Some("R:/game.cci\n[System]\nregion_value=0"))
                .is_none()
        );
        assert!(with_startup_cartridge(
            "[UI]\nPaths\\inserted_cartridge=a\nPaths\\inserted_cartridge=b\n",
            None
        )
        .is_none());
        assert!(with_startup_cartridge("[UI]\n[UI]\n", None).is_none());
        let added = with_startup_cartridge("[System]\nregion_value=1\n", None).unwrap();
        assert!(added.ends_with(
            "[UI]\nPaths\\inserted_cartridge=\"\"\nPaths\\inserted_cartridge\\default=false\n"
        ));
    }

    #[test]
    fn runtime_preparation_rejects_custom_or_ambiguous_storage() {
        assert!(default_storage_selected("[System]\nis_new_3ds=false\n"));
        assert!(default_storage_selected(
            "[Data Storage]\nuse_custom_storage=false\n"
        ));
        assert!(!default_storage_selected(
            "[Data Storage]\nuse_custom_storage=true\n"
        ));
        assert!(default_storage_selected(
            "[Data Storage]\nuse_custom_storage=true\nuse_custom_storage\\default=true\n"
        ));
        assert!(!default_storage_selected(
            "[Data Storage]\nuse_custom_storage=false\nuse_custom_storage=true\n"
        ));
        assert!(!default_storage_selected(
            "[Data Storage]\nuse_custom_storage=unknown\n"
        ));
    }

    #[test]
    fn portable_settings_require_explicit_unambiguous_system_values() {
        let valid = "[System]\nis_new_3ds=false\nregion_value=1\n";
        assert_eq!(
            parse_system_settings(valid),
            Some(AzaharSystemSettings {
                is_new_3ds: false,
                region_value: 1
            })
        );
        assert_eq!(
            parse_system_settings("[System]\nis_new_3ds=true\nregion_value=1\n")
                .unwrap()
                .is_new_3ds,
            true
        );
        assert!(parse_system_settings("[System]\nregion_value=1\n").is_none());
        assert!(parse_system_settings(
            "[System]\nis_new_3ds=false\nregion_value=1\nregion_value=2\n"
        )
        .is_none());
        assert!(parse_system_settings(
            "[System]\nis_new_3ds=false\nregion_value=1\nis_new_3ds/default=true\n"
        )
        .is_none());
        assert!(parse_system_settings(
            "[System]\nis_new_3ds=false\nregion_value=1\nis_new_3ds\\default=true\n"
        )
        .is_none());
        assert!(per_title_uses_global_system(
            "[System]\nis_new_3ds\\use_global=true\nregion_value\\use_global=true\n"
        ));
        assert!(!per_title_uses_global_system(
            "[System]\nis_new_3ds\\use_global=false\nregion_value\\use_global=true\n"
        ));
        assert!(update_check_disabled("[Miscellaneous]\ncheck_for_update_on_start=false\ncheck_for_update_on_start\\default=false\n"));
        assert!(!update_check_disabled(
            "[Miscellaneous]\ncheck_for_update_on_start=true\n"
        ));
        assert!(!update_check_disabled("[Miscellaneous]\ncheck_for_update_on_start=false\ncheck_for_update_on_start\\default=true\n"));
    }

    #[test]
    fn active_keyboard_profile_supplies_case_buttons() {
        assert_eq!(
            parse_button_keys("[System]\nregion_value=1\n"),
            Some(AzaharButtonKeys::DEFAULT)
        );
        let names = [
            "button_a",
            "button_b",
            "button_x",
            "button_y",
            "button_up",
            "button_down",
            "button_left",
            "button_right",
            "button_start",
            "button_select",
            "button_home",
        ];
        let mut config = String::from("[Controls]\nprofile=1\n");
        for (index, name) in names.iter().enumerate() {
            config.push_str(&format!(
                "profiles\\2\\{name}=\"code:{},engine:keyboard\"\n",
                65 + index
            ));
        }
        assert_eq!(
            parse_button_keys(&config),
            Some(AzaharButtonKeys([
                65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75
            ]))
        );
        assert_eq!(
            AzaharButtonKeys::decode(&parse_button_keys(&config).unwrap().encode()),
            parse_button_keys(&config)
        );
        assert!(parse_button_keys(
            "[Controls]\nprofile=1\nprofiles\\1\\button_a=\"code:65,engine:keyboard\"\n"
        )
        .is_none());
        assert_eq!(
            parse_keyboard_binding("\"code:16777235,engine:keyboard\""),
            Some(0x26)
        );
        assert_eq!(parse_keyboard_binding("\"code:65,engine:sdl\""), None);
        assert_eq!(
            parse_keyboard_binding("\"code:999999,engine:keyboard\""),
            None
        );
    }
}

/// The pinned GUI backend presents a terminal success/failure dialog for -i.
/// Keep this behavior confined to a process spawned for installation only.
pub struct AzaharInstallSession {
    session: AzaharSession,
}

impl AzaharInstallSession {
    pub fn wait(&mut self) -> Result<ExitStatus, BackendError> {
        #[cfg(windows)]
        {
            loop {
                if let Some(status) = self.session.try_wait()? {
                    return Ok(status);
                }
                unsafe {
                    let mut result = InstallerResultWindow {
                        pid: self.session.pid(),
                        window: std::ptr::null_mut(),
                    };
                    EnumWindows(
                        Some(find_installer_result),
                        (&mut result as *mut InstallerResultWindow) as LPARAM,
                    );
                    if !result.window.is_null() {
                        // These are the backend's terminal status notifications,
                        // not prompts requesting permission to install content.
                        PostMessageW(result.window, WM_CLOSE, 0, 0);
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        #[cfg(not(windows))]
        self.session.wait()
    }
}

#[cfg(windows)]
struct InstallerResultWindow {
    pid: u32,
    window: HWND,
}

#[cfg(windows)]
unsafe extern "system" fn find_installer_result(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let result = &mut *(parameter as *mut InstallerResultWindow);
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if pid == result.pid && IsWindowVisible(hwnd) != 0 {
        let mut title = [0u16; 128];
        let size = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
        if size > 0
            && matches!(
                String::from_utf16_lossy(&title[..size as usize]).as_str(),
                "Success" | "Failure"
            )
        {
            result.window = hwnd;
            return 0;
        }
    }
    1
}

pub struct AzaharSession {
    _job: crate::process_lifecycle::ProcessJob,
    _cartridge: Option<CartridgeImage>,
    child: Child,
    _runtime_lock: fs::File,
}

impl Drop for AzaharSession {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl AzaharSession {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }
    /// Find the first visible top-level window owned by this exact child
    /// process. This is runtime window discovery, independent of title data.
    #[cfg(windows)]
    pub fn wait_for_window(&mut self, timeout: Duration) -> Result<Option<HWND>, BackendError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(hwnd) = visible_window_for_pid(self.child.id()) {
                return Ok(Some(hwnd));
            }
            if self.try_wait()?.is_some() || Instant::now() >= deadline {
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, BackendError> {
        self.child
            .try_wait()
            .map_err(|_| BackendError::ProcessFailed)
    }

    pub fn wait(&mut self) -> Result<ExitStatus, BackendError> {
        self.child.wait().map_err(|_| BackendError::ProcessFailed)
    }

    pub fn stop(&mut self) -> Result<ExitStatus, BackendError> {
        if let Some(status) = self.try_wait()? {
            return Ok(status);
        }
        self.child.kill().map_err(|_| BackendError::ProcessFailed)?;
        self.wait()
    }
}

#[cfg(windows)]
pub fn visible_window_for_pid(pid: u32) -> Option<HWND> {
    let mut search = WindowSearch {
        pid,
        found: std::ptr::null_mut(),
        largest_area: 0,
    };
    unsafe {
        EnumWindows(
            Some(find_window),
            (&mut search as *mut WindowSearch) as LPARAM,
        );
    }
    if search.found.is_null() {
        None
    } else {
        Some(search.found)
    }
}

#[cfg(windows)]
struct WindowSearch {
    pid: u32,
    found: HWND,
    largest_area: i64,
}

#[cfg(windows)]
unsafe extern "system" fn find_window(hwnd: HWND, parameter: LPARAM) -> BOOL {
    let search = &mut *(parameter as *mut WindowSearch);
    let mut owner = 0u32;
    GetWindowThreadProcessId(hwnd, &mut owner);
    if owner == search.pid && IsWindowVisible(hwnd) != 0 {
        let mut bounds: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut bounds) != 0 {
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            if width > 800 && height > 400 {
                let area = i64::from(width) * i64::from(height);
                if area > search.largest_area {
                    search.found = hwnd;
                    search.largest_area = area;
                }
            }
        }
    }
    1
}
