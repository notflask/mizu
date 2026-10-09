fn main() {
    // Embed the application icon into the Windows executable.
    #[cfg(windows)]
    {
        let ico = "assets/icons/generated/windows/mizu.ico";
        if std::path::Path::new(ico).exists() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon(ico);
            res.set("ProductName", "mizu");
            res.set("FileDescription", "mizu PDF viewer");
            if let Err(e) = res.compile() {
                println!("cargo:warning=failed to embed icon: {e}");
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/generated/windows/mizu.ico");
}
