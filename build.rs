//! Embeds the app icon and file details into vey.exe so Explorer, the taskbar
//! and the "Details" tab show Vey instead of the generic Windows icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/vey.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    #[cfg(windows)]
    windows_icon();
}

#[cfg(windows)]
fn windows_icon() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/vey.ico");
    res.set("ProductName", "Vey");
    res.set("FileDescription", "Vey - a notch and top bar for Windows");
    res.set("OriginalFilename", "vey.exe");
    res.set("LegalCopyright", "Copyright (c) 2026 Akhilesh Varute. MIT License.");
    if let Err(e) = res.compile() {
        // keep building without an icon rather than failing (for example without the Windows SDK)
        println!("cargo:warning=could not embed the exe icon: {e}");
    }
}
