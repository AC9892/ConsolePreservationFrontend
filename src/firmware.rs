//! Private, runtime-supplied firmware configuration. No dump path is compiled into CPF.
use crate::{inspect_file, HeaderCheck, Kind, ProcessImage};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareError {
    InvalidProfile,
    UnsupportedProfile,
    SourceUnavailable,
    WrongHomeMenu,
    UnverifiedHomeMenu,
}

impl FirmwareError {
    pub fn label(self) -> &'static str {
        match self {
            Self::InvalidProfile => "invalid private firmware profile",
            Self::UnsupportedProfile => "firmware profile outside the current original-2DS scope",
            Self::SourceUnavailable => "HOME Menu source unavailable",
            Self::WrongHomeMenu => "source is not the USA HOME Menu NCCH candidate",
            Self::UnverifiedHomeMenu => "HOME Menu structure or executable hashes did not validate",
        }
    }
}

/// One protected firmware source. The declared version is descriptive metadata,
/// not proof of the installed version in a NAND image or title export.
pub struct FirmwareConfig {
    pub declared_firmware: String,
    home_menu_cxi: PathBuf,
}

impl FirmwareConfig {
    pub fn from_original_home_source(path: &Path) -> Result<Self, FirmwareError> {
        let path = existing_absolute_path(path).map_err(|_| FirmwareError::SourceUnavailable)?;
        Ok(Self {
            declared_firmware: "6.0.0-12U".into(),
            home_menu_cxi: path,
        })
    }

    pub fn private_profile_text(&self) -> Result<String, FirmwareError> {
        let source = self
            .home_menu_cxi
            .to_str()
            .ok_or(FirmwareError::InvalidProfile)?;
        let mut table = toml::Table::new();
        table.insert("schema".into(), toml::Value::Integer(1));
        for (key, value) in [
            ("hardware", "original-2ds"),
            ("region", "USA"),
            ("declared_firmware", self.declared_firmware.as_str()),
            ("home_menu_cxi", source),
        ] {
            table.insert(key.into(), toml::Value::String(value.into()));
        }
        Ok(table.to_string())
    }

    pub fn load(path: &Path) -> Result<Self, FirmwareError> {
        let size = fs::metadata(path)
            .map_err(|_| FirmwareError::InvalidProfile)?
            .len();
        if size > 16 * 1024 {
            return Err(FirmwareError::InvalidProfile);
        }
        let text = fs::read_to_string(path).map_err(|_| FirmwareError::InvalidProfile)?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, FirmwareError> {
        let table: toml::Table = text.parse().map_err(|_| FirmwareError::InvalidProfile)?;
        let field = |name| {
            table
                .get(name)
                .and_then(toml::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .ok_or(FirmwareError::InvalidProfile)
        };
        if table.get("schema").and_then(toml::Value::as_integer) != Some(1) {
            return Err(FirmwareError::InvalidProfile);
        }
        if field("hardware")? != "original-2ds"
            || field("region")? != "USA"
            || field("declared_firmware")? != "6.0.0-12U"
        {
            return Err(FirmwareError::UnsupportedProfile);
        }
        let home_menu_cxi = PathBuf::from(field("home_menu_cxi")?);
        if !home_menu_cxi.is_absolute() {
            return Err(FirmwareError::InvalidProfile);
        }
        Ok(Self {
            declared_firmware: "6.0.0-12U".into(),
            home_menu_cxi,
        })
    }

    pub fn validate_home_menu(&self) -> Result<ValidatedHomeMenu, FirmwareError> {
        let entry =
            inspect_file(&self.home_menu_cxi).map_err(|_| FirmwareError::SourceUnavailable)?;
        if entry.kind != Kind::Ncch || !entry.known_home_menu_candidate {
            return Err(FirmwareError::WrongHomeMenu);
        }
        if entry.header_check != HeaderCheck::BoundsConsistent
            || entry.ncch_encrypted
            || entry.ncch_hash_mismatched != 0
            || !entry.exefs_code_verified
        {
            return Err(FirmwareError::UnverifiedHomeMenu);
        }
        // Limit this configuration to the USA title without exposing the ID in logs.
        let mut file =
            File::open(&self.home_menu_cxi).map_err(|_| FirmwareError::SourceUnavailable)?;
        file.seek(SeekFrom::Start(0x118))
            .map_err(|_| FirmwareError::SourceUnavailable)?;
        let mut id = [0; 8];
        file.read_exact(&mut id)
            .map_err(|_| FirmwareError::SourceUnavailable)?;
        let title_id = u64::from_le_bytes(id);
        if title_id != 0x0004_0030_0000_8f02 {
            return Err(FirmwareError::WrongHomeMenu);
        }
        ProcessImage::load(&self.home_menu_cxi).map_err(|_| FirmwareError::UnverifiedHomeMenu)?;
        let path = existing_absolute_path(&self.home_menu_cxi)
            .map_err(|_| FirmwareError::SourceUnavailable)?;
        Ok(ValidatedHomeMenu { path, title_id })
    }

    /// Current target policy. The Azahar execution adapter only reports its
    /// settings and does not know which firmware or console CPF selected.
    pub fn accepts_backend_system(&self, is_new_3ds: bool, region_value: i32) -> bool {
        !is_new_3ds && region_value == 1
    }
}

/// Some mounted Windows drives can be opened but cannot be canonicalized by
/// Rust's filesystem API. Accept only an existing, already absolute path with
/// no relative components as the fallback; never construct a relative path.
pub(crate) fn existing_absolute_path(path: &Path) -> std::io::Result<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Ok(canonical);
    }
    let absolute = std::path::absolute(path)?;
    if absolute
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path is not clean and absolute",
        ));
    }
    std::fs::metadata(&absolute)?;
    Ok(absolute)
}

pub struct ValidatedHomeMenu {
    path: PathBuf,
    title_id: u64,
}

impl ValidatedHomeMenu {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn title_id(&self) -> u64 {
        self.title_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_profile_requires_explicit_absolute_home_source() {
        let profile = r#"
schema = 1
hardware = "original-2ds"
region = "USA"
declared_firmware = "6.0.0-12U"
home_menu_cxi = "C:/private/home.cxi"
"#;
        assert!(Path::new("C:/private/home.cxi").is_absolute());
        assert!(
            FirmwareConfig::parse(profile).is_ok(),
            "{:?}",
            FirmwareConfig::parse(profile).err()
        );
        assert_eq!(
            FirmwareConfig::parse(&profile.replace("C:/private/home.cxi", "home.cxi")).err(),
            Some(FirmwareError::InvalidProfile)
        );
        assert_eq!(
            FirmwareConfig::parse(&profile.replace("6.0.0-12U", "11.17.0-50U")).err(),
            Some(FirmwareError::UnsupportedProfile)
        );
        let config = FirmwareConfig::parse(profile).unwrap();
        assert!(config.accepts_backend_system(false, 1));
        assert!(!config.accepts_backend_system(true, 1));
        assert!(!config.accepts_backend_system(false, -1));
    }
}
