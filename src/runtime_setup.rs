//! Explicit preparation of private writable runtime state from a verified backup.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnTitleState {
    Created,
    AlreadyVerified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    SourceUnavailable,
    BackupMismatch,
    StorageUnavailable,
    StorageOverlapsSource,
    ExistingTitleConflict,
    CopyFailed,
    RuntimeBusy,
}

impl SetupError {
    pub fn label(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "selected HOME source or backup is unavailable",
            Self::BackupMismatch => "backup does not match the selected HOME source",
            Self::StorageUnavailable => "portable runtime storage is unavailable or redirected",
            Self::StorageOverlapsSource => "runtime storage overlaps a preserved source",
            Self::ExistingTitleConflict => {
                "an existing installed title conflicts with HOME return preparation"
            }
            Self::CopyFailed => "HOME return working copy could not be verified",
            Self::RuntimeBusy => "close the CPF session before preparing its runtime",
        }
    }
}

pub(crate) fn prepare_return_title(
    user: &Path,
    selected_source: &Path,
    backup: &Path,
    title_id: u64,
) -> Result<ReturnTitleState, SetupError> {
    let user = crate::firmware::existing_absolute_path(user)
        .map_err(|_| SetupError::StorageUnavailable)?;
    for source in [selected_source, backup] {
        let source = crate::firmware::existing_absolute_path(source)
            .map_err(|_| SetupError::SourceUnavailable)?;
        let parent = source.parent().ok_or(SetupError::StorageOverlapsSource)?;
        if user.starts_with(parent) || source.starts_with(&user) {
            return Err(SetupError::StorageOverlapsSource);
        }
    }
    let expected = hash_file(selected_source).map_err(|_| SetupError::SourceUnavailable)?;
    if hash_file(backup).map_err(|_| SetupError::SourceUnavailable)? != expected {
        return Err(SetupError::BackupMismatch);
    }
    // This is Azahar's writable title layout. The title ID comes from the
    // validated firmware selection, rather than a compiled firmware dependency.
    let components = [
        "nand".to_owned(),
        "00000000000000000000000000000000".to_owned(),
        "title".to_owned(),
        format!("{:08x}", title_id >> 32),
        format!("{:08x}", title_id as u32),
        "content".to_owned(),
    ];
    let mut content = user;
    for component in components {
        content.push(component);
        match content.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err(SetupError::StorageUnavailable),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&content).map_err(|_| SetupError::StorageUnavailable)?;
            }
            Err(_) => return Err(SetupError::StorageUnavailable),
        }
    }
    let target = content.join("00000000.app");
    for entry in fs::read_dir(&content).map_err(|_| SetupError::StorageUnavailable)? {
        let entry = entry.map_err(|_| SetupError::StorageUnavailable)?;
        let path = entry.path();
        let extension = path.extension().and_then(|value| value.to_str());
        if extension.is_some_and(|value| value.eq_ignore_ascii_case("tmd"))
            || (extension.is_some_and(|value| value.eq_ignore_ascii_case("app")) && path != target)
        {
            return Err(SetupError::ExistingTitleConflict);
        }
    }
    if let Ok(metadata) = target.symlink_metadata() {
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(SetupError::ExistingTitleConflict);
        }
        return if hash_file(&target).map_err(|_| SetupError::CopyFailed)? == expected {
            Ok(ReturnTitleState::AlreadyVerified)
        } else {
            Err(SetupError::ExistingTitleConflict)
        };
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|_| SetupError::ExistingTitleConflict)?;
    let copied = (|| -> io::Result<()> {
        io::copy(&mut File::open(backup)?, &mut output)?;
        output.sync_all()?;
        if hash_file(&target)? != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "copy hash mismatch",
            ));
        }
        Ok(())
    })();
    drop(output);
    if copied.is_err() {
        // Only this invocation's newly created file can be removed here.
        let _ = fs::remove_file(&target);
        return Err(SetupError::CopyFailed);
    }
    Ok(ReturnTitleState::Created)
}

pub(crate) fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buffer[..size]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "cpf-return-copy-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            for directory in ["runtime", "archive", "backup"] {
                fs::create_dir_all(root.join(directory)).unwrap();
            }
            fs::write(root.join("archive/home.cxi"), b"selected title fixture").unwrap();
            fs::write(root.join("backup/home.cxi"), b"selected title fixture").unwrap();
            Self(root)
        }
        fn prepare(&self) -> Result<ReturnTitleState, SetupError> {
            prepare_return_title(
                &self.0.join("runtime"),
                &self.0.join("archive/home.cxi"),
                &self.0.join("backup/home.cxi"),
                0x0004_0030_1234_5678,
            )
        }
        fn target(&self) -> std::path::PathBuf {
            self.0.join("runtime/nand/00000000000000000000000000000000/title/00040030/12345678/content/00000000.app")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn repeat_preparation_verifies_and_never_overwrites_a_different_title() {
        let fixture = Fixture::new();
        assert_eq!(fixture.prepare(), Ok(ReturnTitleState::Created));
        assert_eq!(fixture.prepare(), Ok(ReturnTitleState::AlreadyVerified));
        fs::write(fixture.target(), b"different installed title").unwrap();
        assert_eq!(fixture.prepare(), Err(SetupError::ExistingTitleConflict));
        assert_eq!(
            fs::read(fixture.target()).unwrap(),
            b"different installed title"
        );
        assert_eq!(
            fs::read(fixture.0.join("archive/home.cxi")).unwrap(),
            b"selected title fixture"
        );
        assert_eq!(
            fs::read(fixture.0.join("backup/home.cxi")).unwrap(),
            b"selected title fixture"
        );
    }

    #[test]
    fn mismatched_backup_cannot_create_a_runtime_copy() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("backup/home.cxi"), b"another version").unwrap();
        assert_eq!(fixture.prepare(), Err(SetupError::BackupMismatch));
        assert!(!fixture.0.join("runtime/nand").exists());
    }

    #[test]
    fn installed_metadata_is_preserved() {
        let fixture = Fixture::new();
        let target = fixture.target();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            target.parent().unwrap().join("00000001.tmd"),
            b"existing metadata",
        )
        .unwrap();
        assert_eq!(fixture.prepare(), Err(SetupError::ExistingTitleConflict));
        assert!(!target.exists());
    }
}
