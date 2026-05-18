fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=libs/libshlwapi.a");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("libs");
    println!("cargo:rustc-link-search=native={}", dir.display());

    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        let _ = res.compile();
    }
}
