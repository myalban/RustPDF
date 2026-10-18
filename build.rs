fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "RustPDF");
        res.set("FileDescription", "RustPDF - PDF editor");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon not embedded : {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
