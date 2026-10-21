use serde::Serialize;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
pub struct MediaInfo {
    pub path: String,
    pub title: String,
    pub game_code: String,
    pub maker_code: String,
    pub format: String,
    pub size_bytes: u64,
    pub icon_rgba: Option<Vec<u8>>,
}

fn ascii(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn banner_icon(banner: &[u8; 0x840]) -> Vec<u8> {
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let tile = (y / 8) * 4 + x / 8;
            let pixel = (y % 8) * 8 + x % 8;
            let packed = banner[0x20 + tile * 32 + pixel / 2];
            let index = if pixel & 1 == 0 { packed & 0x0f } else { packed >> 4 } as usize;
            let color = u16::from_le_bytes([
                banner[0x220 + index * 2],
                banner[0x221 + index * 2],
            ]);
            let dst = (y * 32 + x) * 4;
            rgba[dst] = ((color & 0x1f) * 255 / 31) as u8;
            rgba[dst + 1] = (((color >> 5) & 0x1f) * 255 / 31) as u8;
            rgba[dst + 2] = (((color >> 10) & 0x1f) * 255 / 31) as u8;
            rgba[dst + 3] = if index == 0 { 0 } else { 255 };
        }
    }
    rgba
}

pub fn inspect(path: &Path) -> Result<MediaInfo, String> {
    if !path.is_absolute() || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("nds")) {
        return Err("Choose an absolute .nds file path".into());
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("The cartridge image must be a regular file".into());
    }
    if metadata.len() < 0x200 {
        return Err("The image is too small to contain a Nintendo DS header".into());
    }
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut header = [0u8; 0x200];
    file.read_exact(&mut header).map_err(|error| error.to_string())?;
    let code = ascii(&header[0x0c..0x10]);
    let unit = header[0x12];
    let format = match unit {
        0 => "Nintendo DS cartridge",
        2 => "DSi-enhanced cartridge",
        3 => "DSi-only cartridge",
        _ => "Nintendo DS image (unit code unknown)",
    };
    let mut title = ascii(&header[0..12]);
    let offset = u32::from_le_bytes(header[0x68..0x6c].try_into().unwrap()) as u64;
    let mut icon = None;
    if offset >= 0x200 && offset.checked_add(0x840).is_some_and(|end| end <= metadata.len()) {
        file.seek(SeekFrom::Start(offset)).map_err(|error| error.to_string())?;
        let mut banner = [0u8; 0x840];
        file.read_exact(&mut banner).map_err(|error| error.to_string())?;
        let utf16: Vec<u16> = banner[0x340..0x440]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|value| *value != 0)
            .collect();
        let banner_title = String::from_utf16_lossy(&utf16)
            .lines().next().unwrap_or("").trim().to_string();
        if !banner_title.is_empty() { title = banner_title; }
        icon = Some(banner_icon(&banner));
    }
    if title.is_empty() { title = "Title unavailable".into(); }
    Ok(MediaInfo {
        path: path.display().to_string(),
        title,
        game_code: if code.is_empty() { "Unavailable".into() } else { code },
        maker_code: ascii(&header[0x10..0x12]),
        format: format.into(),
        size_bytes: metadata.len(),
        icon_rgba: icon,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_truncated_image() {
        let path = std::env::temp_dir().join(format!("short-{}.nds", std::process::id()));
        fs::write(&path, [0u8; 16]).unwrap();
        assert!(inspect(&path).unwrap_err().contains("too small"));
        fs::remove_file(path).unwrap();
    }
}
