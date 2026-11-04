#[path = "src/overlay.rs"]
mod overlay;

fn main() {
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo must supply CARGO_MANIFEST_DIR");
    if let Err(error) = overlay::apply(std::path::Path::new(&manifest_dir), |path| {
        println!("cargo::rerun-if-changed={}", path.display());
    }) {
        panic!("ViaHorizon source patch refused: {error}");
    }
}
