//! Read-only, non-recursive catalog of locally supplied title exports.
use crate::{signature_body_start, title_metadata, TitleMetadata};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

pub struct TitleCatalog {
    pub titles: Vec<TitleMetadata>,
    pub rejected: usize,
}

pub fn read_title_directory(directory: &Path) -> io::Result<TitleCatalog> {
    let mut titles = Vec::new();
    let mut rejected = 0;
    for item in fs::read_dir(directory)? {
        let Ok(item) = item else {
            rejected += 1;
            continue;
        };
        // Keep this scan within the supplied directory, including when it
        // contains links to another private archive or a newer NAND set.
        let Ok(kind) = item.file_type() else {
            rejected += 1;
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let path = item.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !["cia", "cxi", "cfa", "app"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            continue;
        }
        match title_metadata(&path) {
            Ok(Some(metadata)) => titles.push(metadata),
            Ok(None) | Err(_) => rejected += 1,
        }
    }
    titles.sort_by_key(|title| (title.title_id, title.title_version, title.ncch_version));
    Ok(TitleCatalog { titles, rejected })
}

/// Audits an Azahar-style writable title tree without following links or changing files.
/// The caller supplies the `title` directory, keeping runtime paths outside this module.
pub fn read_installed_title_tree(directory: &Path) -> io::Result<TitleCatalog> {
    let mut titles = Vec::new();
    let mut rejected = 0;
    for high in fs::read_dir(directory)? {
        let high = high?;
        if !high.file_type()?.is_dir() || !is_hex_component(&high.file_name()) {
            continue;
        }
        for low in fs::read_dir(high.path())? {
            let low = low?;
            if !low.file_type()?.is_dir() || !is_hex_component(&low.file_name()) {
                continue;
            }
            let id_text = format!(
                "{}{}",
                high.file_name().to_string_lossy(),
                low.file_name().to_string_lossy()
            );
            let Ok(expected_id) = u64::from_str_radix(&id_text, 16) else {
                rejected += 1;
                continue;
            };
            let content = low.path().join("content");
            if !content.is_dir() || content.symlink_metadata()?.file_type().is_symlink() {
                rejected += 1;
                continue;
            }
            let mut found = false;
            for entry in fs::read_dir(content)? {
                let entry = entry?;
                if !entry.file_type()?.is_file()
                    || !entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
                {
                    continue;
                }
                match title_metadata(&entry.path()) {
                    Ok(Some(metadata)) if metadata.title_id == expected_id => {
                        titles.push(metadata);
                        found = true;
                    }
                    _ => rejected += 1,
                }
            }
            if !found {
                rejected += 1;
            }
        }
    }
    titles.sort_by_key(|title| title.title_id);
    Ok(TitleCatalog { titles, rejected })
}

fn is_hex_component(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|part| part.len() == 8 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchLinkStatus {
    Ready,
    FallbackVerified,
    MissingMetadata,
    InvalidMetadata,
    MissingContent,
    ContentMismatch,
}

impl LaunchLinkStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "TMD main content hash matches",
            Self::FallbackVerified => "NCCH fallback internally verified (no TMD)",
            Self::MissingMetadata => "base TMD missing",
            Self::InvalidMetadata => "base TMD invalid",
            Self::MissingContent => "TMD main content missing",
            Self::ContentMismatch => "TMD main content mismatch",
        }
    }
}

pub struct InstalledLaunchLink {
    pub title_id: u64,
    pub status: LaunchLinkStatus,
}

/// Checks the lowest numbered TMD-to-main-content link for each installed title,
/// matching Azahar's base metadata selection.
/// It checks local metadata, file length, SHA-256, and the NCCH title ID without
/// reading the archived NAND or attempting an applet launch. Azahar can load
/// some titles through a fallback path when this base TMD is absent.
pub fn read_installed_launch_links(directory: &Path) -> io::Result<Vec<InstalledLaunchLink>> {
    let mut links = Vec::new();
    for high in fs::read_dir(directory)? {
        let high = high?;
        if !high.file_type()?.is_dir() || !is_hex_component(&high.file_name()) {
            continue;
        }
        for low in fs::read_dir(high.path())? {
            let low = low?;
            if !low.file_type()?.is_dir() || !is_hex_component(&low.file_name()) {
                continue;
            }
            let id_text = format!(
                "{}{}",
                high.file_name().to_string_lossy(),
                low.file_name().to_string_lossy()
            );
            let Ok(title_id) = u64::from_str_radix(&id_text, 16) else {
                continue;
            };
            links.push(InstalledLaunchLink {
                title_id,
                status: check_installed_launch_link(&low.path().join("content"), title_id),
            });
        }
    }
    links.sort_by_key(|link| link.title_id);
    Ok(links)
}

fn check_installed_launch_link(content: &Path, title_id: u64) -> LaunchLinkStatus {
    if !content
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return LaunchLinkStatus::MissingMetadata;
    }
    let Ok(entries) = fs::read_dir(content) else {
        return LaunchLinkStatus::MissingMetadata;
    };
    let mut base_tmd = None;
    for entry in entries {
        let Ok(entry) = entry else {
            return LaunchLinkStatus::InvalidMetadata;
        };
        let path = entry.path();
        if !path.extension().is_some_and(|ext| ext == "tmd") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .filter(|name| is_hex_component(name))
            .and_then(|name| u32::from_str_radix(name.to_str()?, 16).ok())
        else {
            return LaunchLinkStatus::InvalidMetadata;
        };
        if base_tmd.as_ref().is_none_or(|(current, _)| id < *current) {
            base_tmd = Some((id, path));
        }
    }
    let Some((_, tmd_path)) = base_tmd else {
        let fallback = content.join("00000000.app");
        if fallback
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            && title_metadata(&fallback).is_ok_and(|metadata| {
                metadata.is_some_and(|metadata| metadata.title_id == title_id)
            })
        {
            return LaunchLinkStatus::FallbackVerified;
        }
        return LaunchLinkStatus::MissingMetadata;
    };
    let Ok(tmd_metadata) = tmd_path.symlink_metadata() else {
        return LaunchLinkStatus::MissingMetadata;
    };
    if !tmd_metadata.is_file() || tmd_metadata.len() > 1024 * 1024 {
        return LaunchLinkStatus::InvalidMetadata;
    }
    let Ok(tmd) = fs::read(&tmd_path) else {
        return LaunchLinkStatus::InvalidMetadata;
    };
    if tmd.len() > 1024 * 1024 {
        return LaunchLinkStatus::InvalidMetadata;
    }
    let Some((content_id, expected_size, expected_hash)) = parse_main_content(&tmd, title_id)
    else {
        return LaunchLinkStatus::InvalidMetadata;
    };
    let app_path = content.join(format!("{content_id:08x}.app"));
    let Ok(metadata) = app_path.symlink_metadata() else {
        return LaunchLinkStatus::MissingContent;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return LaunchLinkStatus::MissingContent;
    }
    if metadata.len() != expected_size {
        return LaunchLinkStatus::ContentMismatch;
    }
    let Ok(Some(ncch)) = title_metadata(&app_path) else {
        return LaunchLinkStatus::ContentMismatch;
    };
    if ncch.title_id != title_id {
        return LaunchLinkStatus::ContentMismatch;
    }
    let Ok(mut app) = File::open(&app_path) else {
        return LaunchLinkStatus::MissingContent;
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let Ok(size) = app.read(&mut buffer) else {
            return LaunchLinkStatus::ContentMismatch;
        };
        if size == 0 {
            break;
        }
        hasher.update(&buffer[..size]);
    }
    if hasher.finalize().as_slice() == expected_hash {
        LaunchLinkStatus::Ready
    } else {
        LaunchLinkStatus::ContentMismatch
    }
}

fn parse_main_content(tmd: &[u8], title_id: u64) -> Option<(u32, u64, &[u8])> {
    let signature = u32::from_be_bytes(tmd.get(..4)?.try_into().ok()?);
    let body = usize::try_from(signature_body_start(signature)?).ok()?;
    let header_end = body.checked_add(0x9c4)?;
    let body_header = tmd.get(body..header_end)?;
    if u64::from_be_bytes(body_header.get(0x4c..0x54)?.try_into().ok()?) != title_id {
        return None;
    }
    let count = u16::from_be_bytes(body_header.get(0x9e..0xa0)?.try_into().ok()?) as usize;
    if count == 0 || header_end.checked_add(count.checked_mul(0x30)?)? > tmd.len() {
        return None;
    }
    let chunk = tmd.get(header_end..header_end + 0x30)?;
    let content_id = u32::from_be_bytes(chunk.get(..4)?.try_into().ok()?);
    let size = u64::from_be_bytes(chunk.get(8..16)?.try_into().ok()?);
    Some((content_id, size, chunk.get(0x10..0x30)?))
}

#[cfg(test)]
mod launch_link_tests {
    use super::{check_installed_launch_link, parse_main_content, LaunchLinkStatus};

    #[test]
    fn parses_main_content_only_for_matching_tmd_title() {
        let mut tmd = vec![0u8; 0x140 + 0x9c4 + 0x30];
        tmd[..4].copy_from_slice(&0x10004u32.to_be_bytes());
        tmd[0x140 + 0x4c..0x140 + 0x54].copy_from_slice(&0x0004_0030_0000_9702u64.to_be_bytes());
        tmd[0x140 + 0x9e..0x140 + 0xa0].copy_from_slice(&1u16.to_be_bytes());
        let chunk = 0x140 + 0x9c4;
        tmd[chunk..chunk + 4].copy_from_slice(&0x10u32.to_be_bytes());
        tmd[chunk + 8..chunk + 16].copy_from_slice(&823296u64.to_be_bytes());
        assert_eq!(
            parse_main_content(&tmd, 0x0004_0030_0000_9702).unwrap().0,
            0x10
        );
        assert!(parse_main_content(&tmd, 0x0004_0030_0000_9602).is_none());
        tmd.truncate(chunk + 0x2f);
        assert!(parse_main_content(&tmd, 0x0004_0030_0000_9702).is_none());
    }

    #[test]
    fn chooses_lowest_tmd_even_when_zero_is_absent() {
        let root = std::env::temp_dir().join(format!(
            "cpf-tmd-selection-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let mut tmd = vec![0u8; 0x140 + 0x9c4 + 0x30];
        tmd[..4].copy_from_slice(&0x10004u32.to_be_bytes());
        tmd[0x140 + 0x4c..0x140 + 0x54].copy_from_slice(&0x0004_0030_0000_9702u64.to_be_bytes());
        tmd[0x140 + 0x9e..0x140 + 0xa0].copy_from_slice(&1u16.to_be_bytes());
        std::fs::write(root.join("00000001.tmd"), tmd).unwrap();
        std::fs::write(root.join("00000002.tmd"), b"invalid newer metadata").unwrap();
        // Selecting 01 reaches its missing content; selecting 02 would fail parsing.
        let selected = check_installed_launch_link(&root, 0x0004_0030_0000_9702);
        std::fs::write(root.join("00000000.tmd"), b"invalid older metadata").unwrap();
        let older = check_installed_launch_link(&root, 0x0004_0030_0000_9702);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(selected, LaunchLinkStatus::MissingContent);
        assert_eq!(older, LaunchLinkStatus::InvalidMetadata);
    }
}
