use std::fs::{self, File};
use std::io::Cursor;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=imgs/icon.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    create_windows_icon().expect("could not build CPF preview icon");
}

fn create_windows_icon() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(feature = "public-release"))]
    let icon = include_bytes!("imgs/icon.png").as_slice();
    #[cfg(feature = "public-release")]
    let icon = include_bytes!("release/assets/icon.png").as_slice();
    let mut decoder = png::Decoder::new(Cursor::new(icon));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut raw = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw)?;
    if info.color_type != png::ColorType::Rgba || info.width > 2048 || info.height > 2048 {
        return Err("icon.png must be an RGBA image of at most 2048 pixels per side".into());
    }
    let width = info.width;
    let height = info.height;
    raw.truncate(info.buffer_size());

    let mut bounds = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            let offset = ((y * width + x) * 4) as usize;
            if raw[offset + 3] > 16 {
                bounds.0 = bounds.0.min(x);
                bounds.1 = bounds.1.min(y);
                bounds.2 = bounds.2.max(x + 1);
                bounds.3 = bounds.3.max(y + 1);
            }
        }
    }
    if bounds.0 >= bounds.2 || bounds.1 >= bounds.3 {
        return Err("icon.png has no visible pixels".into());
    }
    let side = (bounds.2 - bounds.0).max(bounds.3 - bounds.1) + 32;
    let left = (bounds.0 as i32 + bounds.2 as i32 - side as i32) / 2;
    let top = (bounds.1 as i32 + bounds.3 as i32 - side as i32) / 2;

    let mut icons = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 32, 48, 256] {
        let pixels = resize_icon(&raw, width, height, left, top, side, size);
        let image = ico::IconImage::from_rgba_data(size, size, pixels);
        icons.add_entry(ico::IconDirEntry::encode(&image)?);
    }

    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR unavailable")?);
    let icon_path = output.join("cpf-preview.ico");
    icons.write(File::create(&icon_path)?)?;
    let resource_path = output.join("cpf-preview.rc");
    let resource = format!(
        "1 ICON \"{}\"\n",
        icon_path.to_string_lossy().replace('\\', "/")
    );
    fs::write(&resource_path, resource)?;
    embed_resource::compile_for(
        &resource_path,
        &["cpf-preview", "cpf-2ds"],
        embed_resource::NONE,
    )
    .manifest_required()
    .map_err(|result| format!("resource compiler failed: {result:?}"))?;
    Ok(())
}

fn resize_icon(
    source: &[u8],
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    side: u32,
    size: u32,
) -> Vec<u8> {
    let mut output = vec![0; (size * size * 4) as usize];
    for dy in 0..size {
        for dx in 0..size {
            let x0 = left + (dx * side / size) as i32;
            let x1 = left + ((dx + 1) * side / size).max(dx * side / size + 1) as i32;
            let y0 = top + (dy * side / size) as i32;
            let y1 = top + ((dy + 1) * side / size).max(dy * side / size + 1) as i32;
            let mut alpha_sum = 0u64;
            let mut color_sum = [0u64; 3];
            let mut samples = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    samples += 1;
                    if sx < 0 || sy < 0 || sx >= width as i32 || sy >= height as i32 {
                        continue;
                    }
                    let offset = ((sy as u32 * width + sx as u32) * 4) as usize;
                    let alpha = u64::from(source[offset + 3]);
                    alpha_sum += alpha;
                    for channel in 0..3 {
                        color_sum[channel] += u64::from(source[offset + channel]) * alpha;
                    }
                }
            }
            let offset = ((dy * size + dx) * 4) as usize;
            if alpha_sum > 0 {
                for channel in 0..3 {
                    output[offset + channel] = (color_sum[channel] / alpha_sum) as u8;
                }
            }
            output[offset + 3] = (alpha_sum / samples) as u8;
        }
    }
    output
}
