//! Validated, decrypted base application packages for private virtual SD storage.
use crate::{
    align_40, firmware::existing_absolute_path, read_section, runtime_setup::hash_file,
    signature_body_start, title_metadata, validate_ncch, HeaderCheck, TitleFormat,
};
use std::{
    fs::File,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdPackageError {
    UnsupportedFormat,
    InvalidPackage,
    NotBaseSdApplication,
    InvalidExecutable,
    SourceUnavailable,
}

impl SdPackageError {
    pub fn label(self) -> &'static str {
        match self {
            Self::UnsupportedFormat => "initial SD installation requires an uncompressed .cia package",
            Self::InvalidPackage => "CIA structure, metadata, or decrypted content hashes did not validate",
            Self::NotBaseSdApplication => "initial SD installation accepts base CTR applications only; system titles, updates, and DLC are excluded",
            Self::InvalidExecutable => "CIA main content is not a matching decrypted executable NCCH",
            Self::SourceUnavailable => "CIA source unavailable",
        }
    }
}

pub struct SdPackage {
    path: PathBuf,
    title_id: u64,
    title_version: u16,
    source_hash: [u8; 32],
    contents: Vec<InstalledContent>,
    content_subdirectory: bool,
    tmd_size: u64,
    tmd_hash: [u8; 32],
}

struct InstalledContent {
    id: u32,
    size: u64,
    hash: [u8; 32],
}

impl SdPackage {
    pub fn open(path: &Path) -> Result<Self, SdPackageError> {
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cia"))
        {
            return Err(SdPackageError::UnsupportedFormat);
        }
        let source_hash = hash_file(path).map_err(|_| SdPackageError::SourceUnavailable)?;
        let title = title_metadata(path)
            .map_err(|_| SdPackageError::InvalidPackage)?
            .filter(|title| title.format == TitleFormat::Cia)
            .ok_or(SdPackageError::InvalidPackage)?;
        // Format/category policy, independent of firmware version. This also
        // prevents a game installation command from writing system NAND titles.
        if title.title_id >> 32 != 0x0004_0000 || title.title_id & 2 != 0 {
            return Err(SdPackageError::NotBaseSdApplication);
        }
        let mut file = File::open(path).map_err(|_| SdPackageError::SourceUnavailable)?;
        let layout = (|| -> std::io::Result<_> {
            let header = read_section(&mut file, 0, 0x2020)?;
            if header[0x20] & 0x80 == 0 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            let word = |offset: usize| {
                u64::from(u32::from_le_bytes(
                    header[offset..offset + 4].try_into().unwrap(),
                ))
            };
            let ticket = align_40(0x2040 + word(8)).unwrap();
            let tmd = align_40(ticket + word(0xc)).unwrap();
            let content = align_40(tmd + word(0x10)).unwrap();
            let signature =
                u32::from_be_bytes(read_section(&mut file, tmd, 4)?.try_into().unwrap());
            let body = signature_body_start(signature).ok_or(std::io::ErrorKind::InvalidData)?;
            let body_header = read_section(&mut file, tmd + body, 0xa0)?;
            let count = u16::from_be_bytes(body_header[0x9e..0xa0].try_into().unwrap());
            let chunks = read_section(&mut file, tmd + body + 0x9c4, usize::from(count) * 0x30)?;
            if count == 0 || chunks[4..6] != [0, 0] {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            let main_size = u64::from_be_bytes(chunks[8..16].try_into().unwrap());
            if main_size < 0x200 {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            let ncch: [u8; 0x200] = read_section(&mut file, content, 0x200)?.try_into().unwrap();
            let ncch_id = u64::from_le_bytes(ncch[0x118..0x120].try_into().unwrap());
            if &ncch[0x100..0x104] != b"NCCH"
                || validate_ncch(&ncch, main_size) != HeaderCheck::BoundsConsistent
                || ncch_id != title.title_id
                || ncch[0x18d] & 2 == 0
                || ncch[0x18f] & 4 == 0
                || u32::from_le_bytes(ncch[0x180..0x184].try_into().unwrap()) < 0x400
                || u32::from_le_bytes(ncch[0x1a4..0x1a8].try_into().unwrap()) == 0
            {
                return Err(std::io::ErrorKind::InvalidData.into());
            }
            let subdirectory = count > 1
                && u16::from_be_bytes(chunks[0x36..0x38].try_into().unwrap()) & 0x4000 != 0;
            let mut ids = std::collections::BTreeSet::new();
            let mut indices = std::collections::BTreeSet::new();
            let mut contents = Vec::new();
            for chunk in chunks.chunks_exact(0x30) {
                let id = u32::from_be_bytes(chunk[..4].try_into().unwrap());
                let index = u16::from_be_bytes(chunk[4..6].try_into().unwrap()) as usize;
                if !ids.insert(id) || !indices.insert(index) {
                    return Err(std::io::ErrorKind::InvalidData.into());
                }
                if header[0x20 + (index >> 3)] & (0x80 >> (index & 7)) != 0 {
                    contents.push(InstalledContent {
                        id,
                        size: u64::from_be_bytes(chunk[8..16].try_into().unwrap()),
                        hash: chunk[0x10..0x30].try_into().unwrap(),
                    });
                }
            }
            // Azahar serializes the signed header, content-info table, and
            // content records; packaging padding is not part of its saved TMD.
            let tmd_size = body + 0x9c4 + u64::from(count) * 0x30;
            use sha2::Digest;
            let tmd_hash: [u8; 32] =
                sha2::Sha256::digest(read_section(&mut file, tmd, tmd_size as usize)?).into();
            Ok((contents, subdirectory, tmd_size, tmd_hash))
        })()
        .map_err(|_| SdPackageError::InvalidExecutable)?;
        if hash_file(path).map_err(|_| SdPackageError::SourceUnavailable)? != source_hash {
            return Err(SdPackageError::InvalidPackage);
        }
        Ok(Self {
            path: existing_absolute_path(path).map_err(|_| SdPackageError::SourceUnavailable)?,
            title_id: title.title_id,
            title_version: title.title_version.ok_or(SdPackageError::InvalidPackage)?,
            source_hash,
            contents: layout.0,
            content_subdirectory: layout.1,
            tmd_size: layout.2,
            tmd_hash: layout.3,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn title_id(&self) -> u64 {
        self.title_id
    }

    pub fn title_version(&self) -> u16 {
        self.title_version
    }

    pub fn source_unchanged(&self) -> bool {
        hash_file(&self.path).is_ok_and(|hash| hash == self.source_hash)
    }

    pub(crate) fn installed_contents_match(&self, title_directory: &Path) -> bool {
        let mut content = title_directory.join("content");
        if !content
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            return false;
        }
        if self.content_subdirectory {
            content.push("00000000");
            if !content
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
            {
                return false;
            }
        }
        self.contents.iter().all(|expected| {
            let path = content.join(format!("{:08x}.app", expected.id));
            path.symlink_metadata().is_ok_and(|metadata| {
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() == expected.size
            }) && hash_file(&path).is_ok_and(|hash| hash == expected.hash)
        })
    }

    pub(crate) fn installed_metadata_matches(&self, title_directory: &Path) -> bool {
        let content = title_directory.join("content");
        if !content
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            return false;
        }
        // The initial installer refuses existing titles. The pinned backend
        // creates this base TMD for a new title, outside optional-content dirs.
        let path = content.join("00000000.tmd");
        path.symlink_metadata().is_ok_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == self.tmd_size
        }) && hash_file(&path).is_ok_and(|hash| hash == self.tmd_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture(title_id: u64, ncch_id: u64) -> Vec<u8> {
        let mut cia = vec![0u8; 0x3840];
        cia[..4].copy_from_slice(&0x2020u32.to_le_bytes());
        cia[0xc..0x10].copy_from_slice(&0x2b8u32.to_le_bytes());
        cia[0x10..0x14].copy_from_slice(&0xb34u32.to_le_bytes());
        cia[0x18..0x20].copy_from_slice(&0xa00u64.to_le_bytes());
        cia[0x20] = 0x80;
        cia[0x2040..0x2044].copy_from_slice(&0x10004u32.to_be_bytes());
        cia[0x2300..0x2304].copy_from_slice(&0x10004u32.to_be_bytes());
        cia[0x22e8..0x22ec].copy_from_slice(&0x14u32.to_be_bytes());
        cia[0x221c..0x2224].copy_from_slice(&title_id.to_be_bytes());
        cia[0x248c..0x2494].copy_from_slice(&title_id.to_be_bytes());
        cia[0x24dc..0x24de].copy_from_slice(&7u16.to_be_bytes());
        cia[0x24de..0x24e0].copy_from_slice(&1u16.to_be_bytes());
        cia[0x2e0c..0x2e14].copy_from_slice(&0xa00u64.to_be_bytes());
        let ncch = &mut cia[0x2e40..];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&5u32.to_le_bytes());
        ncch[0x118..0x120].copy_from_slice(&ncch_id.to_le_bytes());
        ncch[0x180..0x184].copy_from_slice(&0x400u32.to_le_bytes());
        ncch[0x18d] = 2;
        ncch[0x18f] = 4;
        ncch[0x1a0..0x1a4].copy_from_slice(&3u32.to_le_bytes());
        ncch[0x1a4..0x1a8].copy_from_slice(&2u32.to_le_bytes());
        let hash = Sha256::digest(&cia[0x2e40..]);
        cia[0x2e14..0x2e34].copy_from_slice(&hash);
        cia[0x2506..0x2508].copy_from_slice(&1u16.to_be_bytes());
        let hash = Sha256::digest(&cia[0x2e04..0x2e34]);
        cia[0x2508..0x2528].copy_from_slice(&hash);
        let hash = Sha256::digest(&cia[0x2504..0x2e04]);
        cia[0x24e4..0x2504].copy_from_slice(&hash);
        cia
    }

    fn rehash_two_content_tmd(cia: &mut [u8]) {
        let hash = Sha256::digest(&cia[0x2e04..0x2e64]);
        cia[0x2508..0x2528].copy_from_slice(&hash);
        let hash = Sha256::digest(&cia[0x2504..0x2e04]);
        cia[0x24e4..0x2504].copy_from_slice(&hash);
    }

    #[test]
    fn installed_metadata_must_match_the_packaged_base_tmd() {
        let root = std::env::temp_dir().join(format!("cpf-sd-tmd-{}", std::process::id()));
        let content = root.join("installed/content");
        std::fs::create_dir_all(&content).unwrap();
        let source = root.join("game.cia");
        let cia = two_content_fixture(true, true, 1);
        std::fs::write(&source, &cia).unwrap();
        let package = SdPackage::open(&source).unwrap();
        let title = root.join("installed");
        assert!(!package.installed_metadata_matches(&title));
        let path = content.join("00000000.tmd");
        let mut tmd = cia[0x2300..0x2e64].to_vec();
        std::fs::write(&path, &tmd).unwrap();
        assert!(package.installed_metadata_matches(&title));
        // A changed content link is detected even at the original file size.
        tmd[0xb04] ^= 1;
        std::fs::write(&path, &tmd).unwrap();
        assert!(!package.installed_metadata_matches(&title));
        tmd[0xb04] ^= 1;
        tmd.push(0);
        std::fs::write(&path, &tmd).unwrap();
        assert!(!package.installed_metadata_matches(&title));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(!package.installed_metadata_matches(&title));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn two_content_fixture(optional: bool, present: bool, index: u16) -> Vec<u8> {
        let id = 0x0004_0000_1234_5600;
        let original = fixture(id, id);
        let payload = [0x5au8; 64];
        let content_size = 0xa00 + if present { payload.len() } else { 0 };
        let mut cia = vec![0u8; 0x2e80 + content_size];
        cia[..0x2e34].copy_from_slice(&original[..0x2e34]);
        cia[0x10..0x14].copy_from_slice(&0xb64u32.to_le_bytes());
        cia[0x18..0x20].copy_from_slice(&(content_size as u64).to_le_bytes());
        cia[0x24de..0x24e0].copy_from_slice(&2u16.to_be_bytes());
        cia[0x2506..0x2508].copy_from_slice(&2u16.to_be_bytes());
        cia[0x2e34..0x2e38].copy_from_slice(&1u32.to_be_bytes());
        cia[0x2e38..0x2e3a].copy_from_slice(&index.to_be_bytes());
        cia[0x2e3a..0x2e3c].copy_from_slice(&(if optional { 0x4000u16 } else { 0 }).to_be_bytes());
        cia[0x2e3c..0x2e44].copy_from_slice(&64u64.to_be_bytes());
        cia[0x2e44..0x2e64].copy_from_slice(&Sha256::digest(payload));
        cia[0x2e80..0x3880].copy_from_slice(&original[0x2e40..0x3840]);
        if present {
            cia[0x3880..].copy_from_slice(&payload);
            cia[0x20 + usize::from(index >> 3)] |= 0x80 >> (index & 7);
        }
        rehash_two_content_tmd(&mut cia);
        cia
    }

    #[test]
    fn verifies_secondary_contents_and_optional_directory_layout() {
        let root = std::env::temp_dir().join(format!("cpf-sd-multiple-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("multiple.cia");
        for (optional, present, index) in [
            (false, true, 1),
            (true, true, 1),
            (true, false, 1),
            (false, true, u16::MAX),
        ] {
            let cia = two_content_fixture(optional, present, index);
            std::fs::write(&source, &cia).unwrap();
            let package = SdPackage::open(&source).unwrap();
            let title = root.join(format!("installed-{optional}-{present}-{index}"));
            let mut content = title.join("content");
            if optional {
                content.push("00000000");
            }
            std::fs::create_dir_all(&content).unwrap();
            std::fs::write(content.join("00000000.app"), &cia[0x2e80..0x3880]).unwrap();
            if present {
                assert!(!package.installed_contents_match(&title));
                std::fs::write(content.join("00000001.app"), &cia[0x3880..]).unwrap();
            }
            assert!(package.installed_contents_match(&title));
            if present {
                // Corruption outside the main executable must fail verification.
                std::fs::write(content.join("00000001.app"), [0x5bu8; 64]).unwrap();
                assert!(!package.installed_contents_match(&title));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_content_records_which_alias_an_installed_file_or_index() {
        let source =
            std::env::temp_dir().join(format!("cpf-sd-duplicate-{}.cia", std::process::id()));
        for offset in [0x2e34, 0x2e38] {
            let mut cia = two_content_fixture(false, true, 1);
            if offset == 0x2e34 {
                cia[offset..offset + 4].fill(0);
            } else {
                cia[offset..offset + 2].fill(0);
            }
            rehash_two_content_tmd(&mut cia);
            std::fs::write(&source, cia).unwrap();
            assert!(SdPackage::open(&source).is_err());
        }
        std::fs::remove_file(source).unwrap();
    }

    #[test]
    fn accepts_verified_base_package_and_detects_source_changes() {
        let root = std::env::temp_dir().join(format!("cpf-sd-valid-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("game.cia");
        let id = 0x0004_0000_1234_5600;
        let mut cia = fixture(id, id);
        std::fs::write(&path, &cia).unwrap();
        let package = SdPackage::open(&path).unwrap();
        assert_eq!(package.title_id, id);
        assert_eq!(package.title_version, 7);
        assert!(package.source_unchanged());
        let title = root.join("installed");
        std::fs::create_dir_all(title.join("content")).unwrap();
        std::fs::write(title.join("content/00000000.app"), &cia[0x2e40..]).unwrap();
        assert!(package.installed_contents_match(&title));
        std::fs::write(
            title.join("content/00000000.app"),
            b"wrong installed content",
        )
        .unwrap();
        assert!(!package.installed_contents_match(&title));
        cia[0x2e40] ^= 1;
        std::fs::write(&path, cia).unwrap();
        assert!(!package.source_unchanged());
        assert!(matches!(
            SdPackage::open(&path),
            Err(SdPackageError::InvalidPackage)
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_system_update_dlc_and_wrong_executable_id() {
        let path = std::env::temp_dir().join(format!("cpf-sd-reject-{}.cia", std::process::id()));
        for id in [
            0x0004_0030_0000_9702,
            0x0004_0010_0002_1700,
            0x0004_000e_1234_5600,
            0x0004_008c_1234_5600,
        ] {
            std::fs::write(&path, fixture(id, id)).unwrap();
            assert!(matches!(
                SdPackage::open(&path),
                Err(SdPackageError::NotBaseSdApplication)
            ));
        }
        std::fs::write(&path, fixture(0x0004_0000_1234_5600, 0x0004_0000_1234_5700)).unwrap();
        assert!(matches!(
            SdPackage::open(&path),
            Err(SdPackageError::InvalidExecutable)
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_missing_or_out_of_bounds_ticket_index_before_installation() {
        let path =
            std::env::temp_dir().join(format!("cpf-sd-ticket-index-{}.cia", std::process::id()));
        let id = 0x0004_0000_1234_5600;
        let mut cia = fixture(id, id);
        cia[0xc..0x10].copy_from_slice(&0x2a4u32.to_le_bytes());
        std::fs::write(&path, &cia).unwrap();
        assert!(matches!(
            SdPackage::open(&path),
            Err(SdPackageError::InvalidPackage)
        ));
        cia[0xc..0x10].copy_from_slice(&0x2b8u32.to_le_bytes());
        cia[0x22e8..0x22ec].copy_from_slice(&0x1000u32.to_be_bytes());
        std::fs::write(&path, &cia).unwrap();
        assert!(matches!(
            SdPackage::open(&path),
            Err(SdPackageError::InvalidPackage)
        ));
        std::fs::remove_file(path).unwrap();
    }
}
