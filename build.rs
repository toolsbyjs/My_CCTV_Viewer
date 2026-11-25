//! Embeds the program icon and version details into the Windows program (debug and release).

fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/app.ico")
        .set("ProductName", "MyCCTV")
        .set("FileDescription", "MyCCTV - camera and recorder viewer")
        .set("CompanyName", "MyCCTV")
        .set("LegalCopyright", "MyCCTV")
        .set("OriginalFilename", "mycctv.exe")
        .set("InternalName", "mycctv");
    if let Err(e) = res.compile() {
        // Without the Windows SDK resource compiler the program still builds, just without the icon.
        println!("cargo:warning=program icon not embedded: {e}");
    }
}
