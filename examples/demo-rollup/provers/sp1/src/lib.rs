use std::sync::LazyLock;

/// Attempt to load the ELF file at the given path. If the file is empty or cannot be read,
/// a warning will be printed and the default empty vector will be returned.
fn load_elf(path: &str) -> &'static [u8] {
    let elf = std::fs::read(path).unwrap_or_default();
    if elf.is_empty() {
        println!("Warning: ELF file at '{path}' is empty or could not be read");
    }

    Vec::leak(elf)
}

// Initialize the SP1 guest ELFs. Note: Normally this is done with include_bytes!(PATH_TO_FILE),
// but because we don't include the guest ELFs in the GitHub build, they may potentially not exist.
pub static SP1_GUEST_MOCK_ELF: LazyLock<&'static [u8]> = LazyLock::new(|| {
    load_elf(&format!(
        "{}/guest-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-mock-sp1",
        env!("CARGO_MANIFEST_DIR")
    ))
});
pub static SP1_GUEST_CELESTIA_ELF: LazyLock<&'static [u8]> = LazyLock::new(|| {
    load_elf(&format!(
        "{}/guest-celestia/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-celestia-sp1",
        env!("CARGO_MANIFEST_DIR")
    ))
});
