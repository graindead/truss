// Embeds the app icon (assets/truss.ico) in truss.exe, so Explorer, shortcuts and the taskbar
// show it, plus a recoloured copy for every theme (resource IDs from themes::ICON_ID_BASE), which
// a taskbar pin is pointed at when you pick that theme (see src/taskbar_icon.rs). The window's
// own icon is set at runtime in main.rs.

extern crate iced_core as iced;

#[path = "src/themes.rs"]
mod themes;
#[path = "src/icon_paint.rs"]
mod icon_paint;

fn main() {
    for f in ["assets/truss.ico", "assets/truss-256.png", "src/themes.rs", "src/icon_paint.rs"] {
        println!("cargo:rerun-if-changed={f}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/truss.ico");
    // The first theme (Rust) is the main icon above.
    for (i, theme) in themes::all().iter().enumerate().skip(1) {
        let (accent, bg) = themes::icon_colors(theme);
        let Some((px, w, _)) = icon_paint::recolor(accent, bg) else { continue };
        let Some(ico) = icon_paint::ico_bytes(&px, w) else { continue };
        let path = out.join(format!("theme-icon-{i}.ico"));
        std::fs::write(&path, ico).unwrap();
        res.set_icon_with_id(path.to_str().unwrap(), &(themes::ICON_ID_BASE as usize + i).to_string());
    }
    if let Err(e) = res.compile() {
        println!("cargo:warning=Couldn't embed the icons: {e}");
    }
}
