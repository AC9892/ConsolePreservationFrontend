//! Guided public setup writes only to the user's separate private runtime.
use crate::{
    azahar::AzaharBackend,
    firmware::{existing_absolute_path, FirmwareConfig},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const NEW_CONFIG: &str = "[System]\nis_new_3ds=false\nis_new_3ds\\default=false\nregion_value=1\nregion_value\\default=false\n[Miscellaneous]\ncheck_for_update_on_start=false\ncheck_for_update_on_start\\default=false\n[Data Storage]\nuse_custom_storage=false\nuse_custom_storage\\default=false\nuse_virtual_sd=true\nuse_virtual_sd\\default=false\n[Layout]\nlayout_option=4\nlayout_option\\default=false\n[UI]\nsingleWindowMode=false\nsingleWindowMode\\default=false\nenable_discord_presence=false\nenable_discord_presence\\default=false\n";

fn real_directory(path: &Path) -> Result<(), &'static str> {
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(
            "Cannot create private runtime storage. Extract Azahar to a writable private folder.",
        ),
    }
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "Private runtime directory is unavailable.")?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err("Linked runtime folders are unsupported.");
        }
    }
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Linked runtime folders are unsupported.");
    }
    Ok(())
}

fn initial_config(user: &Path) -> Result<(), &'static str> {
    real_directory(user)?;
    let config = user.join("config");
    real_directory(&config)?;
    let path = config.join("qt-config.ini");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => file
            .write_all(NEW_CONFIG.as_bytes())
            .map_err(|_| "Unable to write initial runtime settings."),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| "Existing runtime configuration is unavailable.")?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err("Linked runtime configuration is unsupported.");
            }
            Ok(()) // Existing settings and user data are never replaced.
        }
        Err(_) => Err("Unable to create initial runtime settings."),
    }
}

pub fn prepare(
    home: &Path,
    backup: &Path,
    executable: &Path,
) -> Result<(PathBuf, PathBuf), &'static str> {
    let config = FirmwareConfig::from_original_home_source(home)
        .map_err(|_| "The HOME Menu source is unavailable.")?;
    let validated=config.validate_home_menu().map_err(|_| "Choose a decrypted original USA HOME Menu CXI (0004003000008F02), not a NAND image, game, or newer firmware title.")?;
    let backup =
        existing_absolute_path(backup).map_err(|_| "The verified HOME backup is unavailable.")?;
    if validated.path() == backup {
        return Err("Select your separate verified backup copy for step 2, rather than the same source file.");
    }
    if crate::runtime_setup::hash_file(validated.path()).ok()
        != Some(
            crate::runtime_setup::hash_file(&backup)
                .map_err(|_| "Unable to read the HOME backup.")?,
        )
    {
        return Err("The HOME source and backup do not match. No runtime state was prepared.");
    }
    let executable = existing_absolute_path(executable)
        .map_err(|_| "The portable backend executable is unavailable.")?;
    if !executable.is_file() {
        return Err("Choose the portable Azahar executable.");
    }
    let parent = executable
        .parent()
        .ok_or("The portable backend folder is unavailable.")?;
    let user = parent.join("user");
    for source in [validated.path(), backup.as_path()] {
        if user.starts_with(source.parent().ok_or("Source directory is unavailable.")?)
            || source.starts_with(&user)
        {
            return Err("Keep HOME sources and backups outside the backend folder. Writable runtime storage must be separate from preserved files.");
        }
    }
    initial_config(&user)?;
    let backend=AzaharBackend::new(&executable).map_err(|_| "Existing portable settings are incompatible. Use a fresh extracted Azahar folder, or configure Old 3DS, USA, default portable storage, and disable update checks.")?;
    let system = backend.system_settings();
    if !config.accepts_backend_system(system.is_new_3ds, system.region_value) {
        return Err("This runtime is not set to Old 3DS / USA. Existing settings were kept.");
    }
    backend.prepare_home_return(&validated,&backup).map_err(|_| "Unable to prepare HOME return. Close other runtime sessions and check for conflicting installed HOME content; existing data was kept.")?;
    let profiles = user.join("cpf-setup");
    real_directory(&profiles)?;
    let profile = profiles.join("original-2ds.cpf-profile.toml");
    let text = config
        .private_profile_text()
        .map_err(|_| "Cannot represent the selected private HOME path.")?;
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&profile)
    {
        Ok(mut file) => file
            .write_all(text.as_bytes())
            .map_err(|_| "Unable to save the private setup profile.")?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&profile)
                .map_err(|_| "Existing setup profile is unavailable.")?;
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || fs::read_to_string(&profile).ok().as_deref() != Some(text.as_str())
            {
                return Err("This runtime already has a different setup profile. Choose a separate fresh runtime folder; its existing profile was kept.");
            }
        }
        Err(_) => return Err("Unable to save the private setup profile."),
    }
    Ok((profile, executable))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_settings_validate_and_existing_settings_are_preserved() {
        let root = std::env::temp_dir().join(format!("cpf-public-settings-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let exe = root.join("azahar.exe");
        fs::write(&exe, b"not executed").unwrap();
        let user = root.join("user");
        initial_config(&user).unwrap();
        let backend = AzaharBackend::new(&exe).unwrap();
        assert!(!backend.system_settings().is_new_3ds);
        assert_eq!(backend.system_settings().region_value, 1);
        assert!(backend.button_keys().is_ok());
        backend.prepare_separate_windows().unwrap();
        let path = user.join("config/qt-config.ini");
        fs::write(&path, b"existing private settings").unwrap();
        initial_config(&user).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"existing private settings");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_home_is_rejected_before_creating_runtime_state() {
        let root = std::env::temp_dir().join(format!("cpf-public-invalid-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("wrong.cxi");
        fs::write(&source, b"not HOME").unwrap();
        let exe = root.join("azahar.exe");
        fs::write(&exe, b"not executed").unwrap();
        assert!(prepare(&source, &source, &exe).is_err());
        assert!(!root.join("user").exists());
        fs::remove_dir_all(root).unwrap();
    }
}
