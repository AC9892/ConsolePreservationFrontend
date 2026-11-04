//! Read-only preflight for the initial, decrypted NCSD cartridge path.
use crate::{firmware::existing_absolute_path, validate_ncch, validate_ncsd, HeaderCheck};
use std::{
    fs::OpenOptions,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CartridgeError {
    SourceUnavailable,
    UnsupportedFormat,
    InvalidLayout,
    NotExecutableCartridge,
    EncryptedContent,
}

impl CartridgeError {
    pub fn label(self) -> &'static str {
        match self {
            Self::SourceUnavailable => "cartridge source unavailable",
            Self::UnsupportedFormat => {
                "initial cartridge support requires an uncompressed .cci or .3ds image"
            }
            Self::InvalidLayout => "cartridge container or executable partition bounds are invalid",
            Self::NotExecutableCartridge => {
                "source is not an NCSD cartridge with an executable first partition"
            }
            Self::EncryptedContent => "initial CPF cartridge mounting requires decrypted content",
        }
    }
}

pub struct CartridgeImage {
    path: PathBuf,
    _source: std::fs::File,
    pub title_id: u64,
    pub partition_count: usize,
    pub trimmed: bool,
}

impl CartridgeImage {
    pub fn open(path: &Path) -> Result<Self, CartridgeError> {
        let extension = path.extension().and_then(|extension| extension.to_str());
        if !extension.is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cci") || extension.eq_ignore_ascii_case("3ds")
        }) {
            return Err(CartridgeError::UnsupportedFormat);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Azahar may open another read handle, but the selected source must
            // not be written, replaced, or deleted while CPF owns this handle.
            options.share_mode(1); // FILE_SHARE_READ
        }
        let mut file = options
            .open(path)
            .map_err(|_| CartridgeError::SourceUnavailable)?;
        let size = file
            .metadata()
            .map_err(|_| CartridgeError::SourceUnavailable)?
            .len();
        let mut header = [0u8; 0x200];
        file.read_exact(&mut header)
            .map_err(|_| CartridgeError::InvalidLayout)?;
        if &header[0x100..0x104] != b"NCSD" {
            return Err(CartridgeError::NotExecutableCartridge);
        }
        let check = validate_ncsd(&header, size);
        if !matches!(
            check,
            HeaderCheck::BoundsConsistent | HeaderCheck::MediaLargerThanFile
        ) {
            return Err(CartridgeError::InvalidLayout);
        }
        let mut ranges = Vec::new();
        let mut main = None;
        for index in 0..8 {
            let at = 0x120 + index * 8;
            let start =
                u64::from(u32::from_le_bytes(header[at..at + 4].try_into().unwrap())) * 0x200;
            let length = u64::from(u32::from_le_bytes(
                header[at + 4..at + 8].try_into().unwrap(),
            )) * 0x200;
            if length == 0 {
                continue;
            }
            if start < 0x200 || length < 0x200 {
                return Err(CartridgeError::InvalidLayout);
            }
            ranges.push((start, start + length));
            if index == 0 {
                main = Some((start, length));
            }
        }
        ranges.sort_unstable();
        if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(CartridgeError::InvalidLayout);
        }
        let (start, length) = main.ok_or(CartridgeError::NotExecutableCartridge)?;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| CartridgeError::InvalidLayout)?;
        let mut ncch = [0u8; 0x200];
        file.read_exact(&mut ncch)
            .map_err(|_| CartridgeError::InvalidLayout)?;
        if &ncch[0x100..0x104] != b"NCCH" || ncch[0x18d] & 2 == 0 {
            return Err(CartridgeError::NotExecutableCartridge);
        }
        if validate_ncch(&ncch, length) != HeaderCheck::BoundsConsistent {
            return Err(CartridgeError::InvalidLayout);
        }
        if ncch[0x18f] & 4 == 0 {
            return Err(CartridgeError::EncryptedContent);
        }
        if u32::from_le_bytes(ncch[0x180..0x184].try_into().unwrap()) < 0x400
            || u32::from_le_bytes(ncch[0x1a4..0x1a8].try_into().unwrap()) == 0
        {
            return Err(CartridgeError::NotExecutableCartridge);
        }
        Ok(Self {
            path: existing_absolute_path(path).map_err(|_| CartridgeError::SourceUnavailable)?,
            _source: file,
            title_id: u64::from_le_bytes(ncch[0x118..0x120].try_into().unwrap()),
            partition_count: ranges.len(),
            trimmed: check == HeaderCheck::MediaLargerThanFile,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_bounded_trimmed_cartridge_and_rejects_overlap_encryption_and_nand() {
        let path = std::env::temp_dir().join(format!("cpf-cartridge-{}.cci", std::process::id()));
        let mut image = vec![0u8; 0xc00];
        image[0x100..0x104].copy_from_slice(b"NCSD");
        image[0x104..0x108].copy_from_slice(&16u32.to_le_bytes());
        image[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        image[0x124..0x128].copy_from_slice(&5u32.to_le_bytes());
        let ncch = &mut image[0x200..0x400];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&5u32.to_le_bytes());
        ncch[0x118..0x120].copy_from_slice(&0x0004_0000_1234_5600u64.to_le_bytes());
        ncch[0x180..0x184].copy_from_slice(&0x400u32.to_le_bytes());
        ncch[0x18d] = 2;
        ncch[0x18f] = 4;
        ncch[0x1a0..0x1a4].copy_from_slice(&3u32.to_le_bytes());
        ncch[0x1a4..0x1a8].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&path, &image).unwrap();
        let cartridge = CartridgeImage::open(&path).unwrap();
        assert!(cartridge.trimmed);
        assert_eq!(cartridge.partition_count, 1);
        assert_eq!(cartridge.title_id, 0x0004_0000_1234_5600);
        #[cfg(windows)]
        {
            assert!(std::fs::OpenOptions::new().write(true).open(&path).is_err());
            assert!(std::fs::remove_file(&path).is_err());
            assert!(std::fs::File::open(&path).is_ok());
        }
        drop(cartridge);
        image[0x200 + 0x18f] = 0;
        std::fs::write(&path, &image).unwrap();
        assert!(matches!(
            CartridgeImage::open(&path),
            Err(CartridgeError::EncryptedContent)
        ));
        image[0x200 + 0x18f] = 4;
        image[0x128..0x12c].copy_from_slice(&2u32.to_le_bytes());
        image[0x12c..0x130].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&path, &image).unwrap();
        assert!(matches!(
            CartridgeImage::open(&path),
            Err(CartridgeError::InvalidLayout)
        ));
        image[0x128..0x130].fill(0);
        image[0x300..0x304].fill(0);
        std::fs::write(&path, &image).unwrap();
        assert!(matches!(
            CartridgeImage::open(&path),
            Err(CartridgeError::NotExecutableCartridge)
        ));
        std::fs::remove_file(&path).unwrap();
    }
}
