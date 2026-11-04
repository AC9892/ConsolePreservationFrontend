//! Local shell artwork for the native Rust preview. No console dump is opened.
use std::{
    io::{Cursor, Read},
    path::Path,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellModel {
    Original2ds,
    New2dsXl,
}

impl ShellModel {
    fn png(self) -> &'static [u8] {
        #[cfg(feature = "public-release")]
        return match self {
            Self::Original2ds => include_bytes!("../release/assets/slate.png"),
            Self::New2dsXl => include_bytes!("../release/assets/fold.png"),
        };
        #[cfg(not(feature = "public-release"))]
        match self {
            Self::Original2ds => include_bytes!("../imgs/2ds.png"),
            Self::New2dsXl => include_bytes!("../imgs/New2ds.png"),
        }
    }

    pub fn source_size(self) -> (u32, u32) {
        match self {
            Self::Original2ds => (1536, 1024),
            Self::New2dsXl => (2000, 2000),
        }
    }

    pub fn screens(self) -> [(u32, u32, u32, u32); 2] {
        // Inset pixel bounds measured from the two user-supplied shell images.
        match self {
            Self::Original2ds => [(463, 145, 610, 348), (520, 546, 494, 342)],
            Self::New2dsXl => [(450, 225, 1344, 656), (558, 1118, 884, 660)],
        }
    }
}

pub struct ShellImage {
    pub width: u32,
    pub height: u32,
    /// Top-down premultiplied BGRA pixels for a per-pixel-alpha Windows DIB.
    pub bgra: Vec<u8>,
}

pub fn load_shell(model: ShellModel) -> Result<ShellImage, String> {
    #[cfg(feature = "public-release")]
    if model == ShellModel::Original2ds {
        if let Some(path) = std::env::var_os("CPF_2DS_SHELL") {
            return load_shell_from_path(model, Path::new(&path));
        }
    }
    decode_shell(model, model.png())
}

pub fn load_shell_from_path(model: ShellModel, path: &Path) -> Result<ShellImage, String> {
    const MAX: u64 = 12 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| "Unable to read selected shell artwork")?
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Unable to read selected shell artwork")?;
    if bytes.len() as u64 > MAX {
        return Err("Selected shell artwork exceeds the 12 MiB limit".into());
    }
    decode_shell(model, &bytes)
}

fn decode_shell(model: ShellModel, bytes: &[u8]) -> Result<ShellImage, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    if (reader.info().width, reader.info().height) != model.source_size() {
        return Err("Artwork dimensions must match the selected device shell".into());
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut raw)
        .map_err(|error| error.to_string())?;
    if info.color_type != png::ColorType::Rgba || info.width > 2048 || info.height > 2048 {
        return Err("unsupported shell image format or dimensions".into());
    }
    let length = usize::try_from(info.width).unwrap() * usize::try_from(info.height).unwrap() * 4;
    if info.buffer_size() != length {
        return Err("shell image length mismatch".into());
    }

    let mut bgra = vec![0; length];
    for (source, target) in raw[..length].chunks_exact(4).zip(bgra.chunks_exact_mut(4)) {
        let alpha = u16::from(source[3]);
        let premultiply = |color: u8| -> u8 { ((u16::from(color) * alpha + 127) / 255) as u8 };
        target[0] = premultiply(source[2]);
        target[1] = premultiply(source[1]);
        target[2] = premultiply(source[0]);
        target[3] = source[3];
    }

    for (x, y, width, height) in model.screens() {
        if x + width > info.width || y + height > info.height {
            return Err("screen rectangle outside shell image".into());
        }
        for row in y..y + height {
            for column in x..x + width {
                let offset = ((row * info.width + column) * 4) as usize;
                bgra[offset..offset + 4].copy_from_slice(&[0x10, 0x10, 0x10, 255]);
            }
        }
    }

    Ok(ShellImage {
        width: info.width,
        height: info.height,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_artwork_requires_matching_geometry_and_preserves_source() {
        let path = std::env::temp_dir().join(format!("cpf-shell-input-{}.png", std::process::id()));
        let source = ShellModel::Original2ds.png();
        std::fs::write(&path, source).unwrap();
        let loaded = load_shell_from_path(ShellModel::Original2ds, &path).unwrap();
        assert_eq!((loaded.width, loaded.height), (1536, 1024));
        assert_eq!(std::fs::read(&path).unwrap(), source);
        assert!(load_shell_from_path(ShellModel::New2dsXl, &path).is_err());
        std::fs::write(&path, b"invalid PNG").unwrap();
        assert!(load_shell_from_path(ShellModel::Original2ds, &path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn supplied_shells_decode_and_have_requested_screen_fill() {
        for (model, width, height, screen_points) in [
            (
                ShellModel::Original2ds,
                1536,
                1024,
                [(768, 300), (768, 700)],
            ),
            (
                ShellModel::New2dsXl,
                2000,
                2000,
                [(1000, 500), (1000, 1400)],
            ),
        ] {
            let image = load_shell(model).unwrap();
            assert_eq!((image.width, image.height), (width, height));
            for (x, y) in screen_points {
                let offset = ((y * image.width + x) * 4) as usize;
                assert_eq!(&image.bgra[offset..offset + 4], &[0x10, 0x10, 0x10, 255]);
            }
        }
    }
}
