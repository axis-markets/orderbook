fn main() {
    // Shrink the WASM shadow stack. Native test binaries must not receive this linker flag.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm32") {
        println!("cargo:rustc-link-arg=-zstack-size=4096");
    }
}
