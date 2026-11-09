fn main() {
    // Embed the icon and version information into iptv.exe on Windows.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        #[cfg(windows)]
        {
            let icon = "../../packaging/windows/iptv.ico";
            println!("cargo:rerun-if-changed={icon}");
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon)
                .set("ProductName", "IPTV")
                .set("FileDescription", "IPTV - watch TV channels")
                .set("OriginalFilename", "iptv.exe");
            if let Err(e) = res.compile() {
                println!("cargo:warning=could not embed Windows resources: {e}");
            }
        }
    }
}
