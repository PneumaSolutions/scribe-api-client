// The `extension-module` PyO3 feature intentionally skips linking against
// libpython (required for manylinux compliance and for maturin-built
// wheels), but that leaves `cargo build`/`cargo test` unable to resolve
// Python symbols at link time on macOS. This restores the missing linker
// flag so those commands work without going through maturin.
fn main() {
    pyo3_build_config::add_extension_module_link_args();
}
