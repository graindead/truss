//! The app icon in a theme's colours, and .ico files of it. Also compiled into build.rs.

/// The base icon (Rust orange on Rust's background).
pub const BASE_PNG: &[u8] = include_bytes!("../assets/truss-256.png");

/// The icon recoloured: the orange truss becomes `accent` and the dark tile `bg` (0–255 RGB),
/// keeping the tile's shading, the truss's highlights and the smoothed edges. RGBA, w, h.
pub fn recolor(accent: [f32; 3], bg: [f32; 3]) -> Option<(Vec<u8>, u32, u32)> {
    let img = ::image::load_from_memory(BASE_PNG).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    // The original's colours: Rust orange on Rust's background.
    let (o, b) = ([226.0, 98.0, 42.0], [27.0, 23.0, 21.0]);
    let lum = |c: [f32; 3]| 0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2];
    let mut out = img.into_raw();
    for px in out.chunks_exact_mut(4) {
        let c = [px[0] as f32, px[1] as f32, px[2] as f32];
        // How much of the pixel is orange: red minus blue is ~8 on the tile, 184 on the truss.
        // Past 1 it's one of the truss's lighter highlights, so the accent brightens too.
        let raw = (c[0] - c[2] - 8.0) / (o[0] - o[2] - 8.0);
        let a = raw.clamp(0.0, 1.0);
        let lift = raw.clamp(1.0, 1.3);
        // The tile part's shade relative to the darkest tile colour (the tile is a gradient).
        let tile = [0, 1, 2].map(|i| if a < 1.0 { (c[i] - a * o[i]) / (1.0 - a) } else { b[i] });
        let shade = (lum(tile) / lum(b)).clamp(0.5, 2.0);
        // On a light background a gentle shade the other way (darker at the top) keeps it subtle.
        let shade = if lum(bg) < 128.0 { shade } else { 1.0 - (shade - 1.0) * 0.12 };
        for i in 0..3 {
            let base = bg[i] * shade;
            px[i] = (base * (1.0 - a) + (accent[i] * lift).min(255.0) * a).round().clamp(0.0, 255.0) as u8;
        }
    }
    Some((out, w, h))
}

/// A multi-size .ico (PNG entries, which the shell reads) from a square RGBA image.
#[allow(dead_code)] // used by build.rs
pub fn ico_bytes(rgba: &[u8], size: u32) -> Option<Vec<u8>> {
    let img = ::image::RgbaImage::from_raw(size, size, rgba.to_vec())?;
    let mut entries = Vec::new();
    for s in [16u32, 20, 24, 32, 40, 48, 64, 256] {
        let scaled = if s == size { img.clone() } else { ::image::imageops::resize(&img, s, s, ::image::imageops::FilterType::Lanczos3) };
        let mut png = Vec::new();
        ::image::DynamicImage::ImageRgba8(scaled)
            .write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png)
            .ok()?;
        entries.push((s, png));
    }
    let mut out = Vec::new();
    out.extend(0u16.to_le_bytes()); // reserved
    out.extend(1u16.to_le_bytes()); // icon
    out.extend((entries.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * entries.len() as u32;
    for (s, png) in &entries {
        let dim = if *s >= 256 { 0u8 } else { *s as u8 }; // 0 means 256
        out.extend([dim, dim, 0, 0]);
        out.extend(1u16.to_le_bytes()); // planes
        out.extend(32u16.to_le_bytes()); // bits per pixel
        out.extend((png.len() as u32).to_le_bytes());
        out.extend(offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in entries {
        out.extend(png);
    }
    Some(out)
}
