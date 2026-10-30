fn main() {
    // macOS: `-undefined dynamic_lookup`, so Python's symbols resolve at load time.
    pyo3_build_config::add_extension_module_link_args();
}
