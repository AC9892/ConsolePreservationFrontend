use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub mod arm;
pub mod azahar;
pub mod cartridge;
pub mod catalog;
pub mod firmware;
pub mod media_request;
pub mod operation_window;
pub mod preview;
pub mod process_lifecycle;
pub mod public_setup;
pub mod runtime_diagnostics;
pub mod runtime_setup;
pub mod sd_media;
#[cfg(all(windows, feature = "public-release"))]
pub mod setup_window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    ThreeDs,
    ThreeDsXl,
    TwoDs,
    NewThreeDs,
    NewThreeDsXl,
    NewTwoDsXl,
}

impl Model {
    pub fn name(self) -> &'static str {
        match self {
            Self::ThreeDs => "Nintendo 3DS",
            Self::ThreeDsXl => "Nintendo 3DS XL",
            Self::TwoDs => "Nintendo 2DS",
            Self::NewThreeDs => "New Nintendo 3DS",
            Self::NewThreeDsXl => "New Nintendo 3DS XL",
            Self::NewTwoDsXl => "New Nintendo 2DS XL",
        }
    }

    pub fn new_series(self) -> bool {
        matches!(
            self,
            Self::NewThreeDs | Self::NewThreeDsXl | Self::NewTwoDsXl
        )
    }

    pub fn stereoscopic_display(self) -> bool {
        !matches!(self, Self::TwoDs | Self::NewTwoDsXl)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ncsd,
    Ncch,
    ThreeDsx,
    DsiTitle,
    CiaCandidate,
    TicketCandidate,
    TmdCandidate,
    RawNandCandidate,
    BootRomCandidate,
    FirmwareCandidate,
    KeyOrIdentityCandidate,
    CalibrationCandidate,
    MemoryCandidate,
    ChecksumSidecar,
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ncsd => "NCSD header verified",
            Self::Ncch => "NCCH header verified",
            Self::ThreeDsx => "3DSX header verified",
            Self::DsiTitle => "DS/DSi title extension candidate; header not validated",
            Self::CiaCandidate => "CIA extension candidate",
            Self::TicketCandidate => {
                "ticket candidate; structure checked, signature not authenticated"
            }
            Self::TmdCandidate => "TMD candidate; signature not validated",
            Self::RawNandCandidate => "raw NAND candidate; not decrypted",
            Self::BootRomCandidate => "boot ROM candidate; not authenticated",
            Self::FirmwareCandidate => "firmware candidate; not authenticated",
            Self::KeyOrIdentityCandidate => "sensitive key or identity candidate",
            Self::CalibrationCandidate => "calibration candidate",
            Self::MemoryCandidate => "memory dump candidate",
            Self::ChecksumSidecar => "checksum sidecar; not trusted",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    pub size: u64,
    pub header_check: HeaderCheck,
    pub system_application_candidate: bool,
    pub known_home_menu_candidate: bool,
    pub ncsd_partitions: u8,
    pub ncch_encrypted: bool,
    pub cia_metadata: CiaMetadataCheck,
    pub cia_tmd_hashes: CiaMetadataCheck,
    pub cia_content_verified: u16,
    pub cia_content_mismatched: u16,
    pub cia_content_skipped_encrypted: u16,
    pub ncch_hash_verified: u8,
    pub ncch_hash_mismatched: u8,
    pub exefs_sections_verified: u8,
    pub exefs_sections_mismatched: u8,
    pub exefs_layout_invalid: bool,
    pub exefs_code_verified: bool,
    pub executable_layout: ExecutableLayoutCheck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleFormat {
    Ncch,
    Cia,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TitleMetadata {
    pub title_id: u64,
    /// TMD title version, available only when a CIA has validated metadata.
    pub title_version: Option<u16>,
    /// NCCH header version is a content field, not a firmware version.
    pub ncch_version: Option<u16>,
    pub format: TitleFormat,
}

/// Read public title fields only after the existing bounded and internal-hash
/// checks pass. This does not authenticate Nintendo signatures or firmware.
pub fn title_metadata(path: &Path) -> io::Result<Option<TitleMetadata>> {
    let entry = inspect_file(path)?;
    match entry.kind {
        Kind::Ncch => {
            if entry.header_check != HeaderCheck::BoundsConsistent
                || entry.ncch_encrypted
                || entry.ncch_hash_mismatched != 0
                || entry.exefs_layout_invalid
                || entry.exefs_sections_mismatched != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "NCCH validation failed",
                ));
            }
            let mut file = File::open(path)?;
            let header = read_section(&mut file, 0, 0x200)?;
            Ok(Some(TitleMetadata {
                title_id: u64::from_le_bytes(header[0x118..0x120].try_into().unwrap()),
                title_version: None,
                ncch_version: Some(u16::from_le_bytes(header[0x112..0x114].try_into().unwrap())),
                format: TitleFormat::Ncch,
            }))
        }
        Kind::CiaCandidate => {
            if entry.header_check != HeaderCheck::BoundsConsistent
                || entry.cia_metadata != CiaMetadataCheck::Consistent
                || entry.cia_tmd_hashes != CiaMetadataCheck::Consistent
                || entry.cia_content_mismatched != 0
                || entry.cia_content_skipped_encrypted != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "CIA validation failed",
                ));
            }
            let mut file = File::open(path)?;
            let header: [u8; 0x200] = read_section(&mut file, 0, 0x200)?.try_into().unwrap();
            let cert_size = u64::from(u32_at(&header, 8));
            let ticket_size = u64::from(u32_at(&header, 0x0c));
            let tmd_size = u64::from(u32_at(&header, 0x10));
            let ticket_offset = align_40(0x2040 + cert_size).unwrap();
            let tmd_offset = align_40(ticket_offset + ticket_size).unwrap();
            let signature_type =
                u32::from_be_bytes(read_section(&mut file, tmd_offset, 4)?.try_into().unwrap());
            let body = signature_body_start(signature_type)
                .filter(|start| start + 0xa0 <= tmd_size)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TMD header invalid"))?;
            let fields = read_section(&mut file, tmd_offset + body, 0xa0)?;
            Ok(Some(TitleMetadata {
                title_id: u64::from_be_bytes(fields[0x4c..0x54].try_into().unwrap()),
                title_version: Some(u16::from_be_bytes(fields[0x9c..0x9e].try_into().unwrap())),
                ncch_version: None,
                format: TitleFormat::Cia,
            }))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableLayoutCheck {
    NotChecked,
    UncompressedConsistent,
    CompressedConsistent,
    CompressedNotChecked,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiaMetadataCheck {
    NotChecked,
    Consistent,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderCheck {
    NotApplicable,
    BoundsConsistent,
    MediaLargerThanFile,
    Truncated,
    InvalidLayout,
}

impl HeaderCheck {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotApplicable => "not checked",
            Self::BoundsConsistent => "declared ranges fit file",
            Self::MediaLargerThanFile => "partitions fit file; declared media is larger",
            Self::Truncated => "header truncated",
            Self::InvalidLayout => "declared ranges inconsistent",
        }
    }
}

fn u32_at(header: &[u8; 0x200], offset: usize) -> u32 {
    u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap())
}

fn u64_at(header: &[u8; 0x200], offset: usize) -> u64 {
    u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap())
}

fn known_home_menu_title(program_id: u64) -> bool {
    // Known Japan, North America, and Europe program IDs. This is a candidate
    // check only: the header is not authenticated and other regions are omitted.
    matches!(
        program_id,
        0x0004_0030_0000_8202 | 0x0004_0030_0000_8f02 | 0x0004_0030_0000_9802
    )
}

fn range_fits(
    offset_units: u32,
    size_units: u32,
    unit: u64,
    limit: u64,
    allow_zero_start: bool,
) -> bool {
    if size_units == 0 {
        return offset_units == 0;
    }
    if offset_units == 0 && !allow_zero_start {
        return false;
    }
    let Some(start) = u64::from(offset_units).checked_mul(unit) else {
        return false;
    };
    let Some(length) = u64::from(size_units).checked_mul(unit) else {
        return false;
    };
    start.checked_add(length).is_some_and(|end| end <= limit)
}

fn validate_ncsd(header: &[u8; 0x200], file_size: u64) -> HeaderCheck {
    let media_size = u64::from(u32_at(header, 0x104)) * 0x200;
    if media_size < 0x200 {
        return HeaderCheck::InvalidLayout;
    }
    for index in 0..8 {
        let at = 0x120 + index * 8;
        let offset = u32_at(header, at);
        let size = u32_at(header, at + 4);
        if !range_fits(offset, size, 0x200, media_size, true)
            || !range_fits(offset, size, 0x200, file_size, true)
        {
            return HeaderCheck::InvalidLayout;
        }
    }
    if media_size > file_size {
        HeaderCheck::MediaLargerThanFile
    } else {
        HeaderCheck::BoundsConsistent
    }
}

fn validate_ncch(header: &[u8; 0x200], file_size: u64) -> HeaderCheck {
    let shift = u32::from(header[0x18e]);
    if shift > 54 {
        return HeaderCheck::InvalidLayout;
    }
    let Some(unit) = 0x200u64.checked_shl(shift) else {
        return HeaderCheck::InvalidLayout;
    };
    let Some(content_size) = u64::from(u32_at(header, 0x104)).checked_mul(unit) else {
        return HeaderCheck::InvalidLayout;
    };
    if content_size < 0x200 || content_size > file_size {
        return HeaderCheck::InvalidLayout;
    }
    let exheader_size = u64::from(u32_at(header, 0x180));
    if exheader_size > 0
        && 0x200u64
            .checked_add(exheader_size)
            .is_none_or(|end| end > content_size)
    {
        return HeaderCheck::InvalidLayout;
    }
    for at in [0x190, 0x198, 0x1a0, 0x1b0] {
        if !range_fits(
            u32_at(header, at),
            u32_at(header, at + 4),
            unit,
            content_size,
            false,
        ) {
            return HeaderCheck::InvalidLayout;
        }
    }
    if u32_at(header, 0x1a8) > u32_at(header, 0x1a4)
        || u32_at(header, 0x1b8) > u32_at(header, 0x1b4)
    {
        return HeaderCheck::InvalidLayout;
    }
    HeaderCheck::BoundsConsistent
}

fn hash_file_range(file: &mut File, offset: u64, size: u64, expected: &[u8]) -> io::Result<bool> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = size;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let length = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
        file.read_exact(&mut buffer[..length])?;
        hasher.update(&buffer[..length]);
        remaining -= length as u64;
    }
    Ok(hasher.finalize().as_slice() == expected)
}

fn check_ncch_hashes(file: &mut File, header: &[u8; 0x200]) -> io::Result<(u8, u8)> {
    let mut verified = 0;
    let mut mismatched = 0;
    let shift = u32::from(header[0x18e]);
    let unit = 0x200u64 << shift;
    let exheader_size = u64::from(u32_at(header, 0x180));
    let mut regions = Vec::new();
    if exheader_size > 0 {
        regions.push((0x200, exheader_size, &header[0x160..0x180]));
    }
    for (start_at, size_at, hash_at) in [(0x1a0, 0x1a8, 0x1c0), (0x1b0, 0x1b8, 0x1e0)] {
        let size = u64::from(u32_at(header, size_at)) * unit;
        if size > 0 {
            let offset = u64::from(u32_at(header, start_at)) * unit;
            regions.push((offset, size, &header[hash_at..hash_at + 0x20]));
        }
    }
    for (offset, size, expected) in regions {
        if hash_file_range(file, offset, size, expected)? {
            verified += 1;
        } else {
            mismatched += 1;
        }
    }
    Ok((verified, mismatched))
}

fn check_exefs_sections(file: &mut File, header: &[u8; 0x200]) -> io::Result<(u8, u8, bool, bool)> {
    let unit = 0x200u64 << u32::from(header[0x18e]);
    let exefs_size = u64::from(u32_at(header, 0x1a4)) * unit;
    if exefs_size == 0 {
        return Ok((0, 0, false, false));
    }
    if exefs_size < 0x200 {
        return Ok((0, 0, true, false));
    }
    let exefs_start = u64::from(u32_at(header, 0x1a0)) * unit;
    let section_header = read_section(file, exefs_start, 0x200)?;
    let mut verified = 0;
    let mut mismatched = 0;
    let mut code_verified = false;
    for (index, entry) in section_header[..0xa0].chunks_exact(0x10).enumerate() {
        let offset = u64::from(u32::from_le_bytes(entry[8..12].try_into().unwrap()));
        let size = u64::from(u32::from_le_bytes(entry[12..16].try_into().unwrap()));
        if size == 0 {
            continue;
        }
        let Some(relative_end) = 0x200u64
            .checked_add(offset)
            .and_then(|start| start.checked_add(size))
        else {
            return Ok((verified, mismatched, true, code_verified));
        };
        if relative_end > exefs_size {
            return Ok((verified, mismatched, true, code_verified));
        }
        let hash_start = 0x200 - (index + 1) * 0x20;
        let matches = hash_file_range(
            file,
            exefs_start + 0x200 + offset,
            size,
            &section_header[hash_start..hash_start + 0x20],
        )?;
        if matches {
            verified += 1;
            if entry[..8] == *b".code\0\0\0" {
                code_verified = true;
            }
        } else {
            mismatched += 1;
        }
    }
    Ok((verified, mismatched, false, code_verified))
}

fn check_executable_layout(
    file: &mut File,
    header: &[u8; 0x200],
) -> io::Result<ExecutableLayoutCheck> {
    if u32_at(header, 0x180) < 0x40 {
        return Ok(ExecutableLayoutCheck::Invalid);
    }
    let exheader = read_section(file, 0x200, 0x40)?;
    let mut total_pages = 0u64;
    let mut ranges = Vec::new();
    for at in [0x10, 0x20, 0x30] {
        let address = u64::from(u32::from_le_bytes(exheader[at..at + 4].try_into().unwrap()));
        let pages = u64::from(u32::from_le_bytes(
            exheader[at + 4..at + 8].try_into().unwrap(),
        ));
        let size = u64::from(u32::from_le_bytes(
            exheader[at + 8..at + 12].try_into().unwrap(),
        ));
        let Some(allocated) = pages.checked_mul(0x1000) else {
            return Ok(ExecutableLayoutCheck::Invalid);
        };
        if size > allocated || (pages == 0 && size != 0) || (pages > 0 && address % 0x1000 != 0) {
            return Ok(ExecutableLayoutCheck::Invalid);
        }
        let Some(end) = address.checked_add(allocated) else {
            return Ok(ExecutableLayoutCheck::Invalid);
        };
        if end > 0x1_0000_0000 {
            return Ok(ExecutableLayoutCheck::Invalid);
        }
        if pages > 0 {
            ranges.push((address, end));
        }
        let Some(sum) = total_pages.checked_add(allocated) else {
            return Ok(ExecutableLayoutCheck::Invalid);
        };
        total_pages = sum;
    }
    if ranges.iter().enumerate().any(|(i, (start, end))| {
        ranges
            .iter()
            .skip(i + 1)
            .any(|(other_start, other_end)| start < other_end && other_start < end)
    }) {
        return Ok(ExecutableLayoutCheck::Invalid);
    }
    let exefs_start = u64::from(u32_at(header, 0x1a0)) * (0x200u64 << u32::from(header[0x18e]));
    let sections = read_section(file, exefs_start, 0xa0)?;
    let Some(code) = sections
        .chunks_exact(0x10)
        .find(|entry| entry[..8] == *b".code\0\0\0")
    else {
        return Ok(ExecutableLayoutCheck::Invalid);
    };
    let stored_code_size = u64::from(u32::from_le_bytes(code[12..16].try_into().unwrap()));
    if exheader[0x0d] & 1 != 0 {
        const MAX_CODE_BYTES: u64 = 128 * 1024 * 1024;
        if stored_code_size > MAX_CODE_BYTES || total_pages > MAX_CODE_BYTES {
            return Ok(ExecutableLayoutCheck::CompressedNotChecked);
        }
        let relative = u64::from(u32::from_le_bytes(code[8..12].try_into().unwrap()));
        let compressed = read_section(
            file,
            exefs_start + 0x200 + relative,
            stored_code_size as usize,
        )?;
        return Ok(
            if decompress_exefs_code(&compressed, total_pages as usize).is_some() {
                ExecutableLayoutCheck::CompressedConsistent
            } else {
                ExecutableLayoutCheck::Invalid
            },
        );
    }
    Ok(if stored_code_size == total_pages {
        ExecutableLayoutCheck::UncompressedConsistent
    } else {
        ExecutableLayoutCheck::Invalid
    })
}

fn decompress_exefs_code(compressed: &[u8], expected_len: usize) -> Option<Vec<u8>> {
    if compressed.len() < 8 || expected_len < compressed.len() {
        return None;
    }
    let footer = compressed.len() - 8;
    let layout = u32::from_le_bytes(compressed[footer..footer + 4].try_into().ok()?);
    let added = u32::from_le_bytes(compressed[footer + 4..footer + 8].try_into().ok()?) as usize;
    if compressed.len().checked_add(added)? != expected_len {
        return None;
    }
    let top = (layout >> 24) as usize;
    let bottom = (layout & 0x00ff_ffff) as usize;
    if top < 8 || top > compressed.len() || bottom > compressed.len() {
        return None;
    }
    let mut input = compressed.len() - top;
    let stop = compressed.len() - bottom;
    if input < stop {
        return None;
    }
    let mut output = vec![0u8; expected_len];
    output[..compressed.len()].copy_from_slice(compressed);
    let mut write = expected_len;
    while input > stop {
        input -= 1;
        let mut control = compressed[input];
        for _ in 0..8 {
            if input <= stop {
                break;
            }
            if control & 0x80 == 0 {
                if input <= stop || write == 0 {
                    return None;
                }
                input -= 1;
                write -= 1;
                output[write] = compressed[input];
            } else {
                if input < stop + 2 {
                    return None;
                }
                input -= 2;
                let token = u16::from_le_bytes(compressed[input..input + 2].try_into().ok()?);
                let length = usize::from(token >> 12) + 3;
                let distance = usize::from(token & 0x0fff) + 2;
                if write < length {
                    return None;
                }
                for _ in 0..length {
                    let source = write.checked_add(distance)?;
                    if source >= output.len() {
                        return None;
                    }
                    let byte = output[source];
                    write -= 1;
                    output[write] = byte;
                }
            }
            control <<= 1;
        }
    }
    if write != stop {
        return None;
    }
    Some(output)
}

fn align_40(value: u64) -> Option<u64> {
    value.checked_add(0x3f).map(|v| v & !0x3f)
}

fn validate_cia(header: &[u8; 0x200], file_size: u64) -> HeaderCheck {
    if file_size < 0x20 {
        return HeaderCheck::Truncated;
    }
    let header_size = u64::from(u32_at(header, 0));
    if header_size != 0x2020 || header_size > file_size {
        return HeaderCheck::InvalidLayout;
    }
    let cert_size = u64::from(u32_at(header, 8));
    let ticket_size = u64::from(u32_at(header, 0x0c));
    let tmd_size = u64::from(u32_at(header, 0x10));
    let meta_size = u64::from(u32_at(header, 0x14));
    let content_size = u64_at(header, 0x18);
    let mut offset = match align_40(header_size) {
        Some(v) => v,
        None => return HeaderCheck::InvalidLayout,
    };
    for size in [cert_size, ticket_size, tmd_size] {
        let Some(end) = offset.checked_add(size) else {
            return HeaderCheck::InvalidLayout;
        };
        if end > file_size {
            return HeaderCheck::InvalidLayout;
        }
        offset = match align_40(end) {
            Some(v) => v,
            None => return HeaderCheck::InvalidLayout,
        };
    }
    let Some(content_end) = offset.checked_add(content_size) else {
        return HeaderCheck::InvalidLayout;
    };
    if content_end > file_size {
        return HeaderCheck::InvalidLayout;
    }
    if meta_size > 0 {
        let Some(meta_start) = align_40(content_end) else {
            return HeaderCheck::InvalidLayout;
        };
        if meta_start
            .checked_add(meta_size)
            .is_none_or(|end| end > file_size)
        {
            return HeaderCheck::InvalidLayout;
        }
    }
    HeaderCheck::BoundsConsistent
}

pub(crate) fn signature_body_start(signature_type: u32) -> Option<u64> {
    let signature_size = match signature_type {
        0x10000 | 0x10003 => 0x200,
        0x10001 | 0x10004 => 0x100,
        0x10002 | 0x10005 => 0x3c,
        _ => return None,
    };
    align_40(signature_size + 4)
}

fn validate_ticket(
    file: &mut File,
    header: &[u8; 0x200],
    file_size: u64,
) -> io::Result<HeaderCheck> {
    if file_size < 4 {
        return Ok(HeaderCheck::Truncated);
    }
    let signature_type = u32::from_be_bytes(header[..4].try_into().unwrap());
    let Some(body) = signature_body_start(signature_type) else {
        return Ok(HeaderCheck::InvalidLayout);
    };
    let Some(index_start) = body.checked_add(0x164) else {
        return Ok(HeaderCheck::InvalidLayout);
    };
    if index_start + 8 > file_size {
        return Ok(HeaderCheck::Truncated);
    }
    let index_header = read_section(file, index_start, 8)?;
    let index_size = u64::from(u32::from_be_bytes(index_header[4..8].try_into().unwrap()));
    if index_size < 8
        || index_start
            .checked_add(index_size)
            .is_none_or(|end| end > file_size)
    {
        return Ok(HeaderCheck::InvalidLayout);
    }
    Ok(HeaderCheck::BoundsConsistent)
}

fn read_section(file: &mut File, offset: u64, size: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; size];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn validate_cia_metadata(file: &mut File, header: &[u8; 0x200]) -> io::Result<CiaMetadataCheck> {
    let cert_size = u64::from(u32_at(header, 8));
    let ticket_size = u64::from(u32_at(header, 0x0c));
    let tmd_size = u64::from(u32_at(header, 0x10));
    let declared_content_size = u64_at(header, 0x18);
    let ticket_offset = align_40(0x2040 + cert_size).unwrap();
    let tmd_offset = align_40(ticket_offset + ticket_size).unwrap();
    if ticket_size < 4 || tmd_size < 4 {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let ticket_type = u32::from_be_bytes(read_section(file, ticket_offset, 4)?.try_into().unwrap());
    let tmd_type = u32::from_be_bytes(read_section(file, tmd_offset, 4)?.try_into().unwrap());
    let (Some(ticket_body), Some(tmd_body)) = (
        signature_body_start(ticket_type),
        signature_body_start(tmd_type),
    ) else {
        return Ok(CiaMetadataCheck::Invalid);
    };
    if ticket_body + 0x164 + 8 > ticket_size || tmd_body + 0x9c4 > tmd_size {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let ticket_index = read_section(file, ticket_offset + ticket_body + 0x164, 8)?;
    let index_size = u64::from(u32::from_be_bytes(ticket_index[4..8].try_into().unwrap()));
    if index_size < 8 || ticket_body + 0x164 + index_size > ticket_size {
        return Ok(CiaMetadataCheck::Invalid);
    }
    // Read only the public ID fields from the ticket; never load its title key.
    let ticket_title = read_section(file, ticket_offset + ticket_body + 0x9c, 8)?;
    let tmd = read_section(file, tmd_offset + tmd_body, 0x9c4)?;
    if ticket_title != tmd[0x4c..0x54] {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let content_count = u16::from_be_bytes(tmd[0x9e..0xa0].try_into().unwrap()) as u64;
    let Some(chunks_bytes) = content_count.checked_mul(0x30) else {
        return Ok(CiaMetadataCheck::Invalid);
    };
    if tmd_body + 0x9c4 + chunks_bytes > tmd_size {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let chunks = read_section(file, tmd_offset + tmd_body + 0x9c4, chunks_bytes as usize)?;
    let content_index = read_section(file, 0x20, 0x2000)?;
    let mut total = 0u64;
    for chunk in chunks.chunks_exact(0x30) {
        let index = u16::from_be_bytes(chunk[4..6].try_into().unwrap()) as usize;
        if content_index[index >> 3] & (0x80 >> (index & 7)) != 0 {
            let size = u64::from_be_bytes(chunk[8..16].try_into().unwrap());
            let Some(sum) = total.checked_add(size) else {
                return Ok(CiaMetadataCheck::Invalid);
            };
            total = sum;
        }
    }
    Ok(if total == declared_content_size {
        CiaMetadataCheck::Consistent
    } else {
        CiaMetadataCheck::Invalid
    })
}

fn validate_cia_tmd_hashes(file: &mut File, header: &[u8; 0x200]) -> io::Result<CiaMetadataCheck> {
    let cert_size = u64::from(u32_at(header, 8));
    let ticket_size = u64::from(u32_at(header, 0x0c));
    let tmd_size = u64::from(u32_at(header, 0x10));
    let ticket_offset = align_40(0x2040 + cert_size).unwrap();
    let tmd_offset = align_40(ticket_offset + ticket_size).unwrap();
    if tmd_size < 4 {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let signature_type = u32::from_be_bytes(read_section(file, tmd_offset, 4)?.try_into().unwrap());
    let Some(body_start) = signature_body_start(signature_type) else {
        return Ok(CiaMetadataCheck::Invalid);
    };
    if body_start + 0x9c4 > tmd_size {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let body = read_section(file, tmd_offset + body_start, 0x9c4)?;
    let count = u16::from_be_bytes(body[0x9e..0xa0].try_into().unwrap()) as usize;
    let chunks_len = count * 0x30;
    if body_start + 0x9c4 + chunks_len as u64 > tmd_size {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let chunks = read_section(file, tmd_offset + body_start + 0x9c4, chunks_len)?;
    let info = &body[0xc4..0x9c4];
    if Sha256::digest(info).as_slice() != &body[0xa4..0xc4] {
        return Ok(CiaMetadataCheck::Invalid);
    }
    let mut covered = vec![false; count];
    for record in info.chunks_exact(0x24) {
        let start = u16::from_be_bytes(record[..2].try_into().unwrap()) as usize;
        let length = u16::from_be_bytes(record[2..4].try_into().unwrap()) as usize;
        if length == 0 {
            continue;
        }
        let Some(end) = start.checked_add(length) else {
            return Ok(CiaMetadataCheck::Invalid);
        };
        if end > count || covered[start..end].iter().any(|present| *present) {
            return Ok(CiaMetadataCheck::Invalid);
        }
        if Sha256::digest(&chunks[start * 0x30..end * 0x30]).as_slice() != &record[4..0x24] {
            return Ok(CiaMetadataCheck::Invalid);
        }
        covered[start..end].fill(true);
    }
    Ok(if covered.iter().all(|present| *present) {
        CiaMetadataCheck::Consistent
    } else {
        CiaMetadataCheck::Invalid
    })
}

fn check_cia_content_hashes(file: &mut File, header: &[u8; 0x200]) -> io::Result<(u16, u16, u16)> {
    let ticket_offset = align_40(0x2040 + u64::from(u32_at(header, 8))).unwrap();
    let tmd_offset = align_40(ticket_offset + u64::from(u32_at(header, 0x0c))).unwrap();
    let tmd_size = u64::from(u32_at(header, 0x10));
    let content_start = align_40(tmd_offset + tmd_size).unwrap();
    let signature_type = u32::from_be_bytes(read_section(file, tmd_offset, 4)?.try_into().unwrap());
    let body_start = signature_body_start(signature_type).unwrap();
    let body = read_section(file, tmd_offset + body_start, 0xa0)?;
    let count = u16::from_be_bytes(body[0x9e..0xa0].try_into().unwrap()) as usize;
    let chunks = read_section(file, tmd_offset + body_start + 0x9c4, count * 0x30)?;
    let content_index = read_section(file, 0x20, 0x2000)?;
    let mut offset = content_start;
    let mut verified = 0u16;
    let mut mismatched = 0u16;
    let mut skipped = 0u16;
    for chunk in chunks.chunks_exact(0x30) {
        let index = u16::from_be_bytes(chunk[4..6].try_into().unwrap()) as usize;
        if content_index[index >> 3] & (0x80 >> (index & 7)) == 0 {
            continue;
        }
        let size = u64::from_be_bytes(chunk[8..16].try_into().unwrap());
        let encrypted = u16::from_be_bytes(chunk[6..8].try_into().unwrap()) & 1 != 0;
        if encrypted {
            skipped += 1;
        } else {
            file.seek(SeekFrom::Start(offset))?;
            let mut remaining = size;
            let mut hasher = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            while remaining > 0 {
                let length = usize::try_from(remaining.min(buffer.len() as u64)).unwrap();
                file.read_exact(&mut buffer[..length])?;
                hasher.update(&buffer[..length]);
                remaining -= length as u64;
            }
            if hasher.finalize().as_slice() == &chunk[0x10..0x30] {
                verified += 1;
            } else {
                mismatched += 1;
            }
        }
        offset += size;
    }
    Ok((verified, mismatched, skipped))
}

pub fn inspect_file(path: &Path) -> io::Result<Entry> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut header = [0u8; 0x200];
    let needed = usize::try_from(size.min(0x200)).unwrap();
    file.read_exact(&mut header[..needed])?;
    let read = needed;
    // NCSD/NCCH magic is at byte 0x100; checking the extension alone is insufficient.
    let kind = if read >= 0x104 && &header[0x100..0x104] == b"NCSD" {
        Kind::Ncsd
    } else if read >= 0x104 && &header[0x100..0x104] == b"NCCH" {
        Kind::Ncch
    } else if read >= 4 && &header[..4] == b"3DSX" {
        Kind::ThreeDsx
    } else {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if name.contains("nand") && ext == "bin" {
            Kind::RawNandCandidate
        } else if name == "boot9.bin" || name == "boot11.bin" {
            Kind::BootRomCandidate
        } else if name == "firm0.bin" || name == "firm1.bin" {
            Kind::FirmwareCandidate
        } else if name.starts_with("hwcal") {
            Kind::CalibrationCandidate
        } else if name.contains("otp")
            || name.contains("cid")
            || name.contains("seed")
            || name.contains("secureinfo")
            || name.contains("movable")
            || name.contains("titlekey")
        {
            Kind::KeyOrIdentityCandidate
        } else if ext == "cia" {
            Kind::CiaCandidate
        } else if ext == "tik" {
            Kind::TicketCandidate
        } else if ext == "tmd" {
            Kind::TmdCandidate
        } else if ext == "mem" {
            Kind::MemoryCandidate
        } else if ext == "sha" {
            Kind::ChecksumSidecar
        } else if ext == "nds" || ext == "srl" {
            Kind::DsiTitle
        } else {
            Kind::Unknown
        }
    };
    let header_check = match kind {
        Kind::Ncsd if read < 0x200 => HeaderCheck::Truncated,
        Kind::Ncsd => validate_ncsd(&header, size),
        Kind::Ncch if read < 0x200 => HeaderCheck::Truncated,
        Kind::Ncch => validate_ncch(&header, size),
        Kind::CiaCandidate => validate_cia(&header, size),
        Kind::TicketCandidate => validate_ticket(&mut file, &header, size)?,
        _ => HeaderCheck::NotApplicable,
    };
    let cia_metadata =
        if kind == Kind::CiaCandidate && header_check == HeaderCheck::BoundsConsistent {
            validate_cia_metadata(&mut file, &header)?
        } else {
            CiaMetadataCheck::NotChecked
        };
    let cia_tmd_hashes = if cia_metadata == CiaMetadataCheck::Consistent {
        validate_cia_tmd_hashes(&mut file, &header)?
    } else {
        CiaMetadataCheck::NotChecked
    };
    let (cia_content_verified, cia_content_mismatched, cia_content_skipped_encrypted) =
        if cia_tmd_hashes == CiaMetadataCheck::Consistent {
            check_cia_content_hashes(&mut file, &header)?
        } else {
            (0, 0, 0)
        };
    let program_id = if kind == Kind::Ncch && header_check == HeaderCheck::BoundsConsistent {
        Some(u64_at(&header, 0x118))
    } else {
        None
    };
    let system_application_candidate = program_id.is_some_and(|id| id >> 32 == 0x0004_0030);
    let known_home_menu_candidate = program_id.is_some_and(known_home_menu_title);
    let ncch_encrypted = program_id.is_some() && header[0x18f] & 0x04 == 0;
    let (ncch_hash_verified, ncch_hash_mismatched) =
        if kind == Kind::Ncch && header_check == HeaderCheck::BoundsConsistent && !ncch_encrypted {
            check_ncch_hashes(&mut file, &header)?
        } else {
            (0, 0)
        };
    let (
        exefs_sections_verified,
        exefs_sections_mismatched,
        exefs_layout_invalid,
        exefs_code_verified,
    ) = if kind == Kind::Ncch && header_check == HeaderCheck::BoundsConsistent && !ncch_encrypted {
        check_exefs_sections(&mut file, &header)?
    } else {
        (0, 0, false, false)
    };
    let executable_layout =
        if exefs_code_verified && ncch_hash_mismatched == 0 && !exefs_layout_invalid {
            check_executable_layout(&mut file, &header)?
        } else {
            ExecutableLayoutCheck::NotChecked
        };
    let ncsd_partitions = if kind == Kind::Ncsd
        && matches!(
            header_check,
            HeaderCheck::BoundsConsistent | HeaderCheck::MediaLargerThanFile
        ) {
        (0..8)
            .filter(|index| u32_at(&header, 0x124 + index * 8) > 0)
            .count() as u8
    } else {
        0
    };
    Ok(Entry {
        kind,
        size,
        header_check,
        system_application_candidate,
        known_home_menu_candidate,
        ncsd_partitions,
        ncch_encrypted,
        cia_metadata,
        cia_tmd_hashes,
        cia_content_verified,
        cia_content_mismatched,
        cia_content_skipped_encrypted,
        ncch_hash_verified,
        ncch_hash_mismatched,
        exefs_sections_verified,
        exefs_sections_mismatched,
        exefs_layout_invalid,
        exefs_code_verified,
        executable_layout,
    })
}

/// Read-only access to the partition spans declared by an NCSD image.
pub struct NcsdImage {
    file: File,
    partitions: [Option<(u64, u64)>; 8],
}

/// Bounded, read-only access to a hash-checked, unencrypted NCCH ExeFS .code file.
pub struct NcchCode {
    file: File,
    start: u64,
    len: u64,
    exheader_size: u32,
}

fn parse_svc_mask(descriptors: &[u8]) -> [bool; 192] {
    let mut allowed = [false; 192];
    for chunk in descriptors.chunks_exact(4) {
        let word = u32::from_le_bytes(chunk.try_into().unwrap());
        if word & 0xf800_0000 != 0xf000_0000 {
            continue;
        }
        let group = ((word >> 24) & 7) as usize;
        for bit in 0..24 {
            if word & (1 << bit) != 0 {
                allowed[group * 24 + bit] = true;
            }
        }
    }
    allowed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeSegment {
    pub address: u32,
    pub pages: u32,
    pub byte_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NcchExecutableLayout {
    pub text: CodeSegment,
    pub read_only: CodeSegment,
    pub data: CodeSegment,
    pub bss_size: u32,
    pub stack_size: u32,
    pub compressed: bool,
}

impl NcchExecutableLayout {
    pub fn code_bytes(self) -> Option<u64> {
        [self.text, self.read_only, self.data]
            .into_iter()
            .try_fold(0u64, |sum, segment| {
                sum.checked_add(u64::from(segment.pages) * 0x1000)
            })
    }
}

impl NcchCode {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = open_source(path)?;
        let file_size = file.metadata()?.len();
        let mut header = [0u8; 0x200];
        file.read_exact(&mut header)?;
        if &header[0x100..0x104] != b"NCCH"
            || validate_ncch(&header, file_size) != HeaderCheck::BoundsConsistent
            || header[0x18f] & 4 == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NCCH is invalid or encrypted",
            ));
        }
        let (_, mismatched) = check_ncch_hashes(&mut file, &header)?;
        let (_, section_mismatched, layout_invalid, code_verified) =
            check_exefs_sections(&mut file, &header)?;
        if mismatched > 0 || section_mismatched > 0 || layout_invalid || !code_verified {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NCCH hash or ExeFS code check failed",
            ));
        }
        let unit = 0x200u64 << u32::from(header[0x18e]);
        let exefs_start = u64::from(u32_at(&header, 0x1a0)) * unit;
        let sections = read_section(&mut file, exefs_start, 0xa0)?;
        for entry in sections.chunks_exact(0x10) {
            if entry[..8] == *b".code\0\0\0" {
                let relative = u64::from(u32::from_le_bytes(entry[8..12].try_into().unwrap()));
                let len = u64::from(u32::from_le_bytes(entry[12..16].try_into().unwrap()));
                return Ok(Self {
                    file,
                    start: exefs_start + 0x200 + relative,
                    len,
                    exheader_size: u32_at(&header, 0x180),
                });
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ExeFS code absent",
        ))
    }

    pub fn len(&self) -> u64 {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Read the process's SVC access mask from its hash-checked exheader ACI.
    /// Signature authenticity is not established here.
    pub fn svc_mask(&mut self) -> io::Result<Option<[bool; 192]>> {
        if self.exheader_size < 0x3e0 {
            return Ok(None);
        }
        let descriptors = read_section(&mut self.file, 0x200 + 0x370, 0x70)?;
        Ok(Some(parse_svc_mask(&descriptors)))
    }

    pub fn layout(&mut self) -> io::Result<NcchExecutableLayout> {
        if self.exheader_size < 0x40 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended header absent",
            ));
        }
        let bytes = read_section(&mut self.file, 0x200, 0x40)?;
        let segment = |at: usize| CodeSegment {
            address: u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
            pages: u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()),
            byte_size: u32::from_le_bytes(bytes[at + 8..at + 12].try_into().unwrap()),
        };
        let layout = NcchExecutableLayout {
            text: segment(0x10),
            read_only: segment(0x20),
            data: segment(0x30),
            bss_size: u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()),
            stack_size: u32::from_le_bytes(bytes[0x1c..0x20].try_into().unwrap()),
            compressed: bytes[0x0d] & 1 != 0,
        };
        let segments = [layout.text, layout.read_only, layout.data];
        for segment in segments {
            let allocated = u64::from(segment.pages) * 0x1000;
            let end = u64::from(segment.address) + allocated;
            if u64::from(segment.byte_size) > allocated
                || end > 0x1_0000_0000
                || (segment.pages > 0 && segment.address % 0x1000 != 0)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "code segment bounds invalid",
                ));
            }
        }
        for (index, first) in segments.iter().enumerate() {
            if first.pages == 0 {
                continue;
            }
            let first_end = u64::from(first.address) + u64::from(first.pages) * 0x1000;
            for second in segments.iter().skip(index + 1) {
                if second.pages == 0 {
                    continue;
                }
                let second_end = u64::from(second.address) + u64::from(second.pages) * 0x1000;
                if u64::from(first.address) < second_end && u64::from(second.address) < first_end {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "code segments overlap",
                    ));
                }
            }
        }
        if layout.code_bytes().is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "code size overflow",
            ));
        }
        Ok(layout)
    }

    pub fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
        let Some(end) = offset.checked_add(buffer.len() as u64) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "code offset overflow",
            ));
        };
        if end > self.len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read crosses code boundary",
            ));
        }
        self.file.seek(SeekFrom::Start(self.start + offset))?;
        self.file.read_exact(buffer)
    }

    /// Decode verified code into memory. The caller owns the returned bytes;
    /// no decoded firmware is written to disk.
    pub fn read_decoded(&mut self) -> io::Result<Vec<u8>> {
        const MAX_CODE_BYTES: u64 = 128 * 1024 * 1024;
        let layout = self.layout()?;
        let expected = layout.code_bytes().unwrap();
        if self.len > MAX_CODE_BYTES || expected > MAX_CODE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "code exceeds memory limit",
            ));
        }
        let mut stored = vec![0u8; self.len as usize];
        self.read_at(0, &mut stored)?;
        if layout.compressed {
            decompress_exefs_code(&stored, expected as usize).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "compressed code layout invalid")
            })
        } else if self.len == expected {
            Ok(stored)
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "code length differs from page layout",
            ))
        }
    }
}

/// A local process memory image; no instruction execution or system services.
pub struct ProcessImage {
    layout: NcchExecutableLayout,
    bytes: Vec<u8>,
    text_len: usize,
    read_only_len: usize,
    data_len: usize,
    stack_len: usize,
    tls_offset: usize,
    svc_mask: Option<[bool; 192]>,
}

impl ProcessImage {
    const MAIN_THREAD_TLS_ADDRESS: u32 = 0x1ff8_2000;
    const MAIN_THREAD_TLS_SIZE: usize = 0x200;

    pub fn load(path: &Path) -> io::Result<Self> {
        let mut code = NcchCode::open(path)?;
        let layout = code.layout()?;
        let decoded = code.read_decoded()?;
        let svc_mask = code.svc_mask()?;
        let mut process = Self::from_parts(layout, decoded)?;
        process.svc_mask = svc_mask;
        Ok(process)
    }

    pub(crate) fn from_parts(
        layout: NcchExecutableLayout,
        mut decoded: Vec<u8>,
    ) -> io::Result<Self> {
        const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;
        let text_len = usize::try_from(u64::from(layout.text.pages) * 0x1000).unwrap();
        let read_only_len = usize::try_from(u64::from(layout.read_only.pages) * 0x1000).unwrap();
        let code_len = layout
            .code_bytes()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "code length overflow"))?;
        let bss_len = u64::from(layout.bss_size)
            .checked_add(0xfff)
            .map(|value| value & !0xfff)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "BSS size overflow"))?;
        let stack_len = u64::from(layout.stack_size)
            .checked_add(0xfff)
            .map(|value| value & !0xfff)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stack size overflow"))?;
        let stack_top = 0x1000_0000u64;
        let stack_start = stack_top.checked_sub(stack_len).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "stack exceeds virtual address range",
            )
        })?;
        let total = code_len
            .checked_add(bss_len)
            .and_then(|value| value.checked_add(stack_len))
            .and_then(|value| value.checked_add(Self::MAIN_THREAD_TLS_SIZE as u64))
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "process image size overflow")
            })?;
        if total > MAX_IMAGE_BYTES || decoded.len() as u64 != code_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "process image size invalid",
            ));
        }
        let data_start = u64::from(layout.data.address);
        let data_end = data_start
            .checked_add(u64::from(layout.data.pages) * 0x1000 + bss_len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "data segment overflow"))?;
        if data_end > 0x1_0000_0000
            || [layout.text, layout.read_only].into_iter().any(|segment| {
                if segment.pages == 0 {
                    return false;
                }
                let start = u64::from(segment.address);
                let end = start + u64::from(segment.pages) * 0x1000;
                data_start < end && start < data_end
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "BSS overlaps another segment",
            ));
        }
        if stack_len > 0
            && [layout.text, layout.read_only, layout.data]
                .into_iter()
                .enumerate()
                .any(|(index, segment)| {
                    if (index != 2 && segment.pages == 0)
                        || (index == 2 && segment.pages == 0 && bss_len == 0)
                    {
                        return false;
                    }
                    let start = u64::from(segment.address);
                    let end = if index == 2 {
                        data_end
                    } else {
                        start + u64::from(segment.pages) * 0x1000
                    };
                    start < stack_top && stack_start < end
                })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "stack overlaps another segment",
            ));
        }
        let tls_start = u64::from(Self::MAIN_THREAD_TLS_ADDRESS);
        let tls_end = tls_start + Self::MAIN_THREAD_TLS_SIZE as u64;
        if [layout.text, layout.read_only, layout.data]
            .into_iter()
            .enumerate()
            .any(|(index, segment)| {
                let start = u64::from(segment.address);
                let end = if index == 2 {
                    data_end
                } else {
                    start + u64::from(segment.pages) * 0x1000
                };
                (segment.pages > 0 || index == 2 && bss_len > 0)
                    && start < tls_end
                    && tls_start < end
            })
            || stack_len > 0 && stack_start < tls_end && tls_start < stack_top
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TLS overlaps another segment",
            ));
        }
        let data_len = (u64::from(layout.data.pages) * 0x1000 + bss_len) as usize;
        let tls_offset = text_len + read_only_len + data_len + stack_len as usize;
        decoded.resize(total as usize, 0);
        Ok(Self {
            layout,
            bytes: decoded,
            text_len,
            read_only_len,
            data_len,
            stack_len: stack_len as usize,
            tls_offset,
            svc_mask: None,
        })
    }

    pub fn layout(&self) -> NcchExecutableLayout {
        self.layout
    }
    pub fn svc_allowed(&self, number: u32) -> Option<bool> {
        self.svc_mask
            .as_ref()
            .and_then(|mask| mask.get(number as usize).copied())
    }
    pub fn stack_range(&self) -> Option<(u32, u32)> {
        (self.stack_len > 0).then_some((0x1000_0000 - self.stack_len as u32, 0x1000_0000))
    }
    pub fn main_thread_tls_address(&self) -> u32 {
        Self::MAIN_THREAD_TLS_ADDRESS
    }

    /// Fetch an aligned Thumb halfword from executable text only.
    pub fn fetch_u16(&self, address: u32) -> io::Result<u16> {
        if address & 1 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unaligned instruction address",
            ));
        }
        let relative = u64::from(address)
            .checked_sub(u64::from(self.layout.text.address))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "instruction address outside text",
                )
            })?;
        if relative
            .checked_add(2)
            .is_none_or(|end| end > self.text_len as u64)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "instruction address outside text",
            ));
        }
        let offset = relative as usize;
        Ok(u16::from_le_bytes(
            self.bytes[offset..offset + 2].try_into().unwrap(),
        ))
    }

    /// Fetch an aligned ARM word from executable text only.
    pub fn fetch_u32(&self, address: u32) -> io::Result<u32> {
        if address & 3 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unaligned instruction address",
            ));
        }
        let relative = u64::from(address)
            .checked_sub(u64::from(self.layout.text.address))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "instruction address outside text",
                )
            })?;
        if relative
            .checked_add(4)
            .is_none_or(|end| end > self.text_len as u64)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "instruction address outside text",
            ));
        }
        let offset = relative as usize;
        Ok(u32::from_le_bytes(
            self.bytes[offset..offset + 4].try_into().unwrap(),
        ))
    }

    fn region(&self, address: u32, length: usize, writable: bool) -> io::Result<usize> {
        let address = u64::from(address);
        let length = length as u64;
        let descriptors = [
            (self.layout.text, 0usize, self.text_len, false),
            (
                self.layout.read_only,
                self.text_len,
                self.read_only_len,
                false,
            ),
            (
                self.layout.data,
                self.text_len + self.read_only_len,
                self.data_len,
                true,
            ),
        ];
        for (segment, offset, size, can_write) in descriptors {
            let start = u64::from(segment.address);
            if address < start || writable && !can_write {
                continue;
            }
            let relative = address - start;
            if relative
                .checked_add(length)
                .is_some_and(|end| end <= size as u64)
            {
                return Ok(offset + relative as usize);
            }
        }
        if let Some((start, end)) = self.stack_range() {
            let start = u64::from(start);
            if address >= start
                && address
                    .checked_add(length)
                    .is_some_and(|last| last <= u64::from(end))
            {
                return Ok(self.text_len
                    + self.read_only_len
                    + self.data_len
                    + (address - start) as usize);
            }
        }
        let tls_start = u64::from(Self::MAIN_THREAD_TLS_ADDRESS);
        if address >= tls_start
            && address
                .checked_add(length)
                .is_some_and(|last| last <= tls_start + Self::MAIN_THREAD_TLS_SIZE as u64)
        {
            return Ok(self.tls_offset + (address - tls_start) as usize);
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "virtual memory access outside mapped permissions",
        ))
    }

    pub fn read(&self, address: u32, output: &mut [u8]) -> io::Result<()> {
        let offset = self.region(address, output.len(), false)?;
        output.copy_from_slice(&self.bytes[offset..offset + output.len()]);
        Ok(())
    }

    pub fn write(&mut self, address: u32, input: &[u8]) -> io::Result<()> {
        let offset = self.region(address, input.len(), true)?;
        self.bytes[offset..offset + input.len()].copy_from_slice(input);
        Ok(())
    }
}

impl NcsdImage {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = open_source(path)?;
        let file_size = file.metadata()?.len();
        let mut header = [0u8; 0x200];
        file.read_exact(&mut header)?;
        if &header[0x100..0x104] != b"NCSD"
            || !matches!(
                validate_ncsd(&header, file_size),
                HeaderCheck::BoundsConsistent | HeaderCheck::MediaLargerThanFile
            )
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "NCSD header or partition bounds invalid",
            ));
        }
        let mut partitions = [None; 8];
        for (index, partition) in partitions.iter_mut().enumerate() {
            let at = 0x120 + index * 8;
            let size_units = u32_at(&header, at + 4);
            if size_units > 0 {
                *partition = Some((
                    u64::from(u32_at(&header, at)) * 0x200,
                    u64::from(size_units) * 0x200,
                ));
            }
        }
        Ok(Self { file, partitions })
    }

    pub fn partition_len(&self, index: usize) -> Option<u64> {
        self.partitions
            .get(index)
            .copied()
            .flatten()
            .map(|(_, len)| len)
    }

    pub fn read_partition(
        &mut self,
        index: usize,
        partition_offset: u64,
        buffer: &mut [u8],
    ) -> io::Result<()> {
        let Some((start, len)) = self.partitions.get(index).copied().flatten() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "partition absent",
            ));
        };
        let Some(end) = partition_offset.checked_add(buffer.len() as u64) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "partition offset overflow",
            ));
        };
        if end > len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read crosses partition boundary",
            ));
        }
        self.file.seek(SeekFrom::Start(start + partition_offset))?;
        self.file.read_exact(buffer)
    }
}

pub fn collect(root: &Path) -> io::Result<Vec<Entry>> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source is not a directory",
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(dir) = pending.pop() {
        for child in fs::read_dir(dir)? {
            let child = child?;
            let ty = child.file_type()?;
            // Do not follow directory links outside the archive.
            if ty.is_symlink() {
                continue;
            }
            if ty.is_dir() {
                pending.push(child.path());
            } else if ty.is_file() {
                entries.push(inspect_file(&child.path())?);
            }
        }
    }
    Ok(entries)
}

/// Opens source material read-only. The returned handle cannot write to the archive.
pub fn open_source(path: &Path) -> io::Result<File> {
    File::open(path)
}

/// Compare a source file with a 32-byte raw SHA-256 sidecar without logging either value.
pub fn verify_raw_sha256_sidecar(source: &Path, sidecar: &Path) -> io::Result<bool> {
    let mut expected_file = open_source(sidecar)?;
    if expected_file.metadata()?.len() != 32 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SHA-256 sidecar must be 32 raw bytes",
        ));
    }
    let mut expected = [0u8; 32];
    expected_file.read_exact(&mut expected)?;
    let mut input = open_source(source)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().as_slice() == expected)
}

fn reject_symlink_components(path: &Path) -> io::Result<()> {
    let mut cursor = PathBuf::new();
    for component in path.components() {
        cursor.push(component);
        if cursor.exists() && fs::symlink_metadata(&cursor)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "path contains a symlink",
            ));
        }
    }
    Ok(())
}

/// Validate a writable overlay location without creating anything in the archive.
pub fn validate_overlay(source: &Path, overlay: &Path) -> io::Result<PathBuf> {
    if source
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
        || overlay
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "parent traversal is not allowed",
        ));
    }
    let source = std::path::absolute(source)?;
    let overlay = std::path::absolute(overlay)?;
    if overlay.starts_with(&source) || source.starts_with(&overlay) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "overlay overlaps source archive",
        ));
    }
    // Existing symlinks could redirect the overlay into the archive.
    reject_symlink_components(&overlay)?;
    Ok(overlay)
}

pub const BLOCK_SIZE: usize = 4096;

/// File-backed copy-on-write blocks. Source data is never opened for writing.
pub struct CowStorage {
    source: File,
    source_len: u64,
    overlay: PathBuf,
}

impl CowStorage {
    pub fn open(source_archive: &Path, source_file: &Path, overlay: &Path) -> io::Result<Self> {
        let archive = std::path::absolute(source_archive)?;
        let file_path = std::path::absolute(source_file)?;
        if !file_path.starts_with(&archive) || !source_file.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source file must be in archive",
            ));
        }
        reject_symlink_components(&file_path)?;
        let overlay = validate_overlay(source_archive, overlay)?;
        let source = open_source(source_file)?;
        let source_len = source.metadata()?.len();
        fs::create_dir_all(&overlay)?;
        Ok(Self {
            source,
            source_len,
            overlay,
        })
    }

    pub fn read_block(&mut self, block: u64) -> io::Result<[u8; BLOCK_SIZE]> {
        let offset = block
            .checked_mul(BLOCK_SIZE as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "block index overflow"))?;
        if offset >= self.source_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "block outside source",
            ));
        }
        let path = self.overlay.join(format!("{block:016x}.block"));
        reject_symlink_components(&path)?;
        let mut data = [0u8; BLOCK_SIZE];
        if path.exists() {
            File::open(path)?.read_exact(&mut data)?;
        } else {
            self.source.seek(SeekFrom::Start(offset))?;
            let mut filled = 0;
            while filled < BLOCK_SIZE {
                let n = self.source.read(&mut data[filled..])?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
        }
        Ok(data)
    }

    pub fn write_block(&self, block: u64, data: &[u8; BLOCK_SIZE]) -> io::Result<()> {
        let offset = block
            .checked_mul(BLOCK_SIZE as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "block index overflow"))?;
        if offset >= self.source_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "block outside source",
            ));
        }
        let path = self.overlay.join(format!("{block:016x}.block"));
        reject_symlink_components(&path)?;
        let mut file = File::create(&path)
            .map_err(|e| io::Error::new(e.kind(), "overlay block create failed"))?;
        file.write_all(data)
            .map_err(|e| io::Error::new(e.kind(), "overlay write failed"))?;
        file.sync_all()
            .map_err(|e| io::Error::new(e.kind(), "overlay sync failed"))?;
        Ok(())
    }
}

pub fn has_ncsd_partition_table(path: &Path) -> io::Result<bool> {
    let mut file = open_source(path)?;
    file.seek(SeekFrom::Start(0x100))?;
    let mut magic = [0u8; 4];
    Ok(file.read_exact(&mut magic).is_ok() && &magic == b"NCSD")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_sha256_sidecar_detects_match_mismatch_and_wrong_format() {
        let base = std::env::temp_dir().join(format!("cpf-{}-raw-sha-test", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let source = base.join("image.bin");
        let sidecar = base.join("image.bin.sha");
        fs::write(&source, b"test image").unwrap();
        fs::write(&sidecar, Sha256::digest(b"test image")).unwrap();
        assert!(verify_raw_sha256_sidecar(&source, &sidecar).unwrap());
        fs::write(&source, b"altered image").unwrap();
        assert!(!verify_raw_sha256_sidecar(&source, &sidecar).unwrap());
        fs::write(&sidecar, b"not a raw digest").unwrap();
        assert_eq!(
            verify_raw_sha256_sidecar(&source, &sidecar)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn model_capabilities() {
        assert!(!Model::TwoDs.stereoscopic_display());
        assert!(Model::NewTwoDsXl.new_series());
        assert!(!Model::ThreeDs.new_series());
    }
    #[test]
    fn magic_over_extension() {
        let path = std::env::temp_dir().join(format!("cpf-{}-test.cxi", std::process::id()));
        let mut data = [0u8; 0x200];
        data[0x100..0x104].copy_from_slice(b"NCCH");
        data[0x104..0x108].copy_from_slice(&1u32.to_le_bytes());
        data[0x118..0x120].copy_from_slice(&0x0004_0030_0000_8f02u64.to_le_bytes());
        fs::write(&path, data).unwrap();
        assert_eq!(inspect_file(&path).unwrap().kind, Kind::Ncch);
        assert_eq!(
            inspect_file(&path).unwrap().header_check,
            HeaderCheck::BoundsConsistent
        );
        assert!(inspect_file(&path).unwrap().known_home_menu_candidate);
        assert!(inspect_file(&path).unwrap().ncch_encrypted);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_out_of_bounds_partition_and_section() {
        let mut ncsd = [0u8; 0x200];
        ncsd[0x100..0x104].copy_from_slice(b"NCSD");
        ncsd[0x104..0x108].copy_from_slice(&2u32.to_le_bytes());
        ncsd[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        ncsd[0x124..0x128].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(validate_ncsd(&ncsd, 0x400), HeaderCheck::InvalidLayout);

        let mut ncch = [0u8; 0x200];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&2u32.to_le_bytes());
        ncch[0x1a0..0x1a4].copy_from_slice(&1u32.to_le_bytes());
        ncch[0x1a4..0x1a8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(validate_ncch(&ncch, 0x400), HeaderCheck::InvalidLayout);
    }
    #[test]
    fn ncsd_allows_larger_declared_media_when_partitions_fit() {
        let mut ncsd = [0u8; 0x200];
        ncsd[0x100..0x104].copy_from_slice(b"NCSD");
        ncsd[0x104..0x108].copy_from_slice(&4u32.to_le_bytes());
        ncsd[0x124..0x128].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            validate_ncsd(&ncsd, 0x600),
            HeaderCheck::MediaLargerThanFile
        );
    }
    #[test]
    fn ncch_exheader_hash_detects_corruption() {
        let path = std::env::temp_dir().join(format!("cpf-{}-exheader.cxi", std::process::id()));
        let mut ncch = vec![0u8; 0x600];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&3u32.to_le_bytes());
        ncch[0x180..0x184].copy_from_slice(&0x400u32.to_le_bytes());
        ncch[0x18f] = 4;
        let exheader_hash = Sha256::digest(&ncch[0x200..0x600]);
        ncch[0x160..0x180].copy_from_slice(&exheader_hash);
        fs::write(&path, &ncch).unwrap();
        assert_eq!(inspect_file(&path).unwrap().ncch_hash_verified, 1);
        ncch[0x200] ^= 1;
        fs::write(&path, &ncch).unwrap();
        assert_eq!(inspect_file(&path).unwrap().ncch_hash_mismatched, 1);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn exefs_code_hash_detects_corruption() {
        let path = std::env::temp_dir().join(format!("cpf-{}-exefs.cxi", std::process::id()));
        let mut ncch = vec![0u8; 0x600];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&3u32.to_le_bytes());
        ncch[0x18f] = 4;
        ncch[0x1a0..0x1a4].copy_from_slice(&1u32.to_le_bytes());
        ncch[0x1a4..0x1a8].copy_from_slice(&2u32.to_le_bytes());
        ncch[0x1a8..0x1ac].copy_from_slice(&1u32.to_le_bytes());
        ncch[0x200..0x208].copy_from_slice(b".code\0\0\0");
        ncch[0x20c..0x210].copy_from_slice(&0x20u32.to_le_bytes());
        let code_hash = Sha256::digest(&ncch[0x400..0x420]);
        ncch[0x3e0..0x400].copy_from_slice(&code_hash);
        let exefs_hash = Sha256::digest(&ncch[0x200..0x400]);
        ncch[0x1c0..0x1e0].copy_from_slice(&exefs_hash);
        fs::write(&path, &ncch).unwrap();
        assert!(inspect_file(&path).unwrap().exefs_code_verified);
        let mut code = NcchCode::open(&path).unwrap();
        assert_eq!(code.len(), 0x20);
        let mut byte = [0xff];
        code.read_at(0, &mut byte).unwrap();
        assert_eq!(byte, [0]);
        assert!(code.read_at(0x20, &mut byte).is_err());
        drop(code);
        ncch[0x400] ^= 1;
        fs::write(&path, &ncch).unwrap();
        assert_eq!(inspect_file(&path).unwrap().exefs_sections_mismatched, 1);
        assert!(NcchCode::open(&path).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn backward_code_decoder_checks_footer_and_references() {
        let compressed = [
            0x00, 0x80, b'C', b'B', b'A', 0x10, 0x0e, 0, 0, 0x08, 0, 0, 0, 0,
        ];
        let decoded = decompress_exefs_code(&compressed, 14).unwrap();
        assert_eq!(decoded.len(), 14);
        assert_eq!(&decoded[11..14], b"CBA");
        assert!(decompress_exefs_code(&compressed, 15).is_none());
        let mut corrupt = compressed;
        corrupt[0] = 0xff;
        corrupt[1] = 0xff;
        assert!(decompress_exefs_code(&corrupt, 14).is_none());
    }
    #[test]
    fn verified_ncch_code_loads_in_memory() {
        let path = std::env::temp_dir().join(format!("cpf-{}-code-image.cxi", std::process::id()));
        let mut ncch = vec![0u8; 0x1800];
        ncch[0x100..0x104].copy_from_slice(b"NCCH");
        ncch[0x104..0x108].copy_from_slice(&12u32.to_le_bytes());
        ncch[0x180..0x184].copy_from_slice(&0x400u32.to_le_bytes());
        ncch[0x18f] = 4;
        ncch[0x1a0..0x1a4].copy_from_slice(&3u32.to_le_bytes());
        ncch[0x1a4..0x1a8].copy_from_slice(&9u32.to_le_bytes());
        ncch[0x1a8..0x1ac].copy_from_slice(&1u32.to_le_bytes());
        ncch[0x210..0x214].copy_from_slice(&0x0010_0000u32.to_le_bytes());
        ncch[0x214..0x218].copy_from_slice(&1u32.to_le_bytes());
        ncch[0x218..0x21c].copy_from_slice(&0x1000u32.to_le_bytes());
        let exheader_hash = Sha256::digest(&ncch[0x200..0x600]);
        ncch[0x160..0x180].copy_from_slice(&exheader_hash);
        ncch[0x600..0x608].copy_from_slice(b".code\0\0\0");
        ncch[0x60c..0x610].copy_from_slice(&0x1000u32.to_le_bytes());
        let code_hash = Sha256::digest(&ncch[0x800..0x1800]);
        ncch[0x7e0..0x800].copy_from_slice(&code_hash);
        let exefs_hash = Sha256::digest(&ncch[0x600..0x800]);
        ncch[0x1c0..0x1e0].copy_from_slice(&exefs_hash);
        fs::write(&path, ncch).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().executable_layout,
            ExecutableLayoutCheck::UncompressedConsistent
        );
        let mut code = NcchCode::open(&path).unwrap();
        let layout = code.layout().unwrap();
        assert_eq!(layout.text.address, 0x0010_0000);
        assert_eq!(layout.code_bytes(), Some(0x1000));
        assert_eq!(code.read_decoded().unwrap().len(), 0x1000);
        drop(code);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn process_image_enforces_segments_and_keeps_bss_in_memory() {
        let layout = NcchExecutableLayout {
            text: CodeSegment {
                address: 0x1000,
                pages: 1,
                byte_size: 0x1000,
            },
            read_only: CodeSegment {
                address: 0x2000,
                pages: 1,
                byte_size: 0x1000,
            },
            data: CodeSegment {
                address: 0x3000,
                pages: 1,
                byte_size: 0x1000,
            },
            bss_size: 0x80,
            stack_size: 0x4000,
            compressed: false,
        };
        let mut process = ProcessImage::from_parts(layout, vec![0x5a; 0x3000]).unwrap();
        let mut byte = [0];
        process.read(0x1000, &mut byte).unwrap();
        assert_eq!(byte, [0x5a]);
        assert_eq!(process.fetch_u32(0x1000).unwrap(), 0x5a5a_5a5a);
        assert_eq!(process.fetch_u16(0x1000).unwrap(), 0x5a5a);
        assert_eq!(process.fetch_u16(0x1002).unwrap(), 0x5a5a);
        assert!(process.fetch_u16(0x1001).is_err());
        assert!(process.fetch_u16(0x2000).is_err());
        assert!(process.fetch_u16(0x1ffe).is_ok());
        assert!(process.fetch_u32(0x1001).is_err());
        assert!(process.fetch_u32(0x2000).is_err());
        assert!(process.fetch_u32(0x3000).is_err());
        assert!(process.fetch_u32(0x1ffc).is_ok());
        assert!(process.fetch_u32(0x0fff_c000).is_err());
        process.read(0x4000, &mut byte).unwrap();
        assert_eq!(byte, [0]);
        assert!(process.write(0x1000, &[1]).is_err());
        assert!(process.write(0x2000, &[1]).is_err());
        process.write(0x4000, &[7]).unwrap();
        process.read(0x4000, &mut byte).unwrap();
        assert_eq!(byte, [7]);
        assert!(process.read(0x5000, &mut byte).is_err());
        assert_eq!(process.stack_range(), Some((0x0fff_c000, 0x1000_0000)));
        process.read(0x0fff_c000, &mut byte).unwrap();
        assert_eq!(byte, [0]);
        process.write(0x0fff_ffff, &[9]).unwrap();
        process.read(0x0fff_ffff, &mut byte).unwrap();
        assert_eq!(byte, [9]);
        assert!(process.read(0x1000_0000, &mut byte).is_err());
        assert!(process.read(0x0fff_bfff, &mut byte).is_err());
        assert_eq!(process.main_thread_tls_address(), 0x1ff8_2000);
        process.read(0x1ff8_2000, &mut byte).unwrap();
        assert_eq!(byte, [0]);
        process.write(0x1ff8_21ff, &[5]).unwrap();
        process.read(0x1ff8_21ff, &mut byte).unwrap();
        assert_eq!(byte, [5]);
        assert!(process.read(0x1ff8_2200, &mut byte).is_err());
        let overlapping = NcchExecutableLayout {
            data: CodeSegment {
                address: 0x0fff_c000,
                pages: 1,
                byte_size: 0x1000,
            },
            ..layout
        };
        assert!(ProcessImage::from_parts(overlapping, vec![0; 0x3000]).is_err());
        let tls_overlap = NcchExecutableLayout {
            data: CodeSegment {
                address: 0x1ff8_2000,
                pages: 1,
                byte_size: 0x1000,
            },
            ..layout
        };
        assert!(ProcessImage::from_parts(tls_overlap, vec![0; 0x3000]).is_err());
    }
    #[test]
    fn svc_mask_uses_descriptor_group_and_bit() {
        let mut descriptors = [0xff; 0x70];
        descriptors[0..4].copy_from_slice(&0xf000_0001u32.to_le_bytes());
        descriptors[4..8].copy_from_slice(&0xf100_0200u32.to_le_bytes());
        let mask = parse_svc_mask(&descriptors);
        assert!(mask[0]);
        assert!(mask[0x21]);
        assert!(!mask[0x20]);
        assert!(!mask[0x22]);
    }
    #[test]
    fn ncsd_partition_reader_stays_within_bounds() {
        let path = std::env::temp_dir().join(format!("cpf-{}-ncsd.img", std::process::id()));
        let mut data = [0u8; 0x600];
        data[0x100..0x104].copy_from_slice(b"NCSD");
        data[0x104..0x108].copy_from_slice(&4u32.to_le_bytes());
        data[0x120..0x124].copy_from_slice(&1u32.to_le_bytes());
        data[0x124..0x128].copy_from_slice(&1u32.to_le_bytes());
        data[0x200] = 0x5a;
        fs::write(&path, data).unwrap();
        let mut image = NcsdImage::open(&path).unwrap();
        assert_eq!(image.partition_len(0), Some(0x200));
        let mut byte = [0u8; 1];
        image.read_partition(0, 0, &mut byte).unwrap();
        assert_eq!(byte, [0x5a]);
        assert!(image.read_partition(0, 0x200, &mut byte).is_err());
        assert!(image.read_partition(1, 0, &mut byte).is_err());
        drop(image);
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn cia_section_bounds_and_overflow() {
        let mut cia = [0u8; 0x200];
        cia[0..4].copy_from_slice(&0x2020u32.to_le_bytes());
        cia[8..12].copy_from_slice(&0x10u32.to_le_bytes());
        cia[0x0c..0x10].copy_from_slice(&0x20u32.to_le_bytes());
        cia[0x10..0x14].copy_from_slice(&0x30u32.to_le_bytes());
        cia[0x18..0x20].copy_from_slice(&0x80u64.to_le_bytes());
        assert_eq!(validate_cia(&cia, 0x2180), HeaderCheck::BoundsConsistent);
        assert_eq!(validate_cia(&cia, 0x217f), HeaderCheck::InvalidLayout);
        cia[0x18..0x20].copy_from_slice(&u64::MAX.to_le_bytes());
        assert_eq!(validate_cia(&cia, u64::MAX), HeaderCheck::InvalidLayout);
    }
    #[test]
    fn cia_metadata_checks_ticket_and_present_content() {
        let path = std::env::temp_dir().join(format!("cpf-{}-metadata.cia", std::process::id()));
        let mut cia = vec![0u8; 0x2e60];
        cia[0..4].copy_from_slice(&0x2020u32.to_le_bytes());
        cia[0x0c..0x10].copy_from_slice(&0x2b8u32.to_le_bytes());
        cia[0x10..0x14].copy_from_slice(&0xb34u32.to_le_bytes());
        cia[0x18..0x20].copy_from_slice(&0x20u64.to_le_bytes());
        cia[0x20] = 0x80;
        cia[0x2040..0x2044].copy_from_slice(&0x10004u32.to_be_bytes());
        cia[0x2300..0x2304].copy_from_slice(&0x10004u32.to_be_bytes());
        cia[0x22e8..0x22ec].copy_from_slice(&0x14u32.to_be_bytes());
        let title_id = 0x0004_0030_0000_8f02u64.to_be_bytes();
        cia[0x221c..0x2224].copy_from_slice(&title_id);
        cia[0x248c..0x2494].copy_from_slice(&title_id);
        cia[0x24dc..0x24de].copy_from_slice(&42u16.to_be_bytes());
        cia[0x24de..0x24e0].copy_from_slice(&1u16.to_be_bytes());
        cia[0x2e0c..0x2e14].copy_from_slice(&0x20u64.to_be_bytes());
        let content_hash = Sha256::digest(&cia[0x2e40..0x2e60]);
        cia[0x2e14..0x2e34].copy_from_slice(&content_hash);
        cia[0x2506..0x2508].copy_from_slice(&1u16.to_be_bytes());
        let chunk_hash = Sha256::digest(&cia[0x2e04..0x2e34]);
        cia[0x2508..0x2528].copy_from_slice(&chunk_hash);
        let info_hash = Sha256::digest(&cia[0x2504..0x2e04]);
        cia[0x24e4..0x2504].copy_from_slice(&info_hash);
        fs::write(&path, &cia).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().cia_metadata,
            CiaMetadataCheck::Consistent
        );
        assert_eq!(
            inspect_file(&path).unwrap().cia_tmd_hashes,
            CiaMetadataCheck::Consistent
        );
        assert_eq!(inspect_file(&path).unwrap().cia_content_verified, 1);
        assert_eq!(
            title_metadata(&path).unwrap(),
            Some(TitleMetadata {
                title_id: 0x0004_0030_0000_8f02,
                title_version: Some(42),
                ncch_version: None,
                format: TitleFormat::Cia,
            })
        );
        cia[0x2e40] ^= 1;
        fs::write(&path, &cia).unwrap();
        assert_eq!(inspect_file(&path).unwrap().cia_content_mismatched, 1);
        assert!(title_metadata(&path).is_err());
        cia[0x2e40] ^= 1;
        cia[0x2508] ^= 1;
        fs::write(&path, &cia).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().cia_tmd_hashes,
            CiaMetadataCheck::Invalid
        );
        cia[0x2508] ^= 1;
        cia[0x2e0c..0x2e14].copy_from_slice(&0x21u64.to_be_bytes());
        fs::write(&path, &cia).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().cia_metadata,
            CiaMetadataCheck::Invalid
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn ticket_rejects_content_index_past_end() {
        let path = std::env::temp_dir().join(format!("cpf-{}-ticket.tik", std::process::id()));
        let mut ticket = vec![0u8; 0x2ac];
        ticket[..4].copy_from_slice(&0x10004u32.to_be_bytes());
        ticket[0x2a8..0x2ac].copy_from_slice(&8u32.to_be_bytes());
        fs::write(&path, &ticket).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().header_check,
            HeaderCheck::BoundsConsistent
        );
        ticket[0x2a8..0x2ac].copy_from_slice(&9u32.to_be_bytes());
        fs::write(&path, &ticket).unwrap();
        assert_eq!(
            inspect_file(&path).unwrap().header_check,
            HeaderCheck::InvalidLayout
        );
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn cow_preserves_source() {
        let base = std::env::temp_dir().join(format!("cpf-cow-{}", std::process::id()));
        let archive = base.join("archive");
        let overlay = base.join("overlay");
        fs::create_dir_all(&archive).unwrap();
        let source = archive.join("source.img");
        fs::write(&source, [1u8; BLOCK_SIZE]).unwrap();
        assert!(validate_overlay(&archive, &archive.join("bad")).is_err());
        let mut cow = CowStorage::open(&archive, &source, &overlay).unwrap();
        assert_eq!(cow.read_block(0).unwrap()[0], 1);
        cow.write_block(0, &[2u8; BLOCK_SIZE]).unwrap();
        assert_eq!(cow.read_block(0).unwrap()[0], 2);
        assert_eq!(fs::read(&source).unwrap()[0], 1);
        drop(cow);
        fs::remove_dir_all(base).unwrap();
    }
}
