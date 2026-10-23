fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("FileDescription", "GoldWatcher - Global Gold Monitor");
        res.set("ProductName", "GoldWatcher");
        res.set("LegalCopyright", "MIT License");
        res.set("OriginalFilename", "GoldWatcher.exe");
        let _ = res.compile();
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
}
