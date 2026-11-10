// Embeds the app icon (assets/truss.ico) in truss.exe, so Explorer, shortcuts and the taskbar
// show it. The window's own icon is set at runtime in main.rs.
fn main() {
    println!("cargo:rerun-if-changed=assets/truss.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/truss.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=Couldn't embed the icon: {e}");
        }
    }
}
