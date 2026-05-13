fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("libs");
    println!("cargo:rustc-link-search=native={}", dir.display());
}
