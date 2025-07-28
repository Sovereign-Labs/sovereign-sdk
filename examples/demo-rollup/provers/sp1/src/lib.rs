use std::sync::OnceLock;

use lazy_static::lazy_static;

/// Attempt to load the ELF file at the given path. If the file is empty or cannot be read,
/// a warning will be printed and the default empty vector will be returned.
fn load_elf(path: &str) -> &'static [u8] {
    static ELF: OnceLock<Vec<u8>> = OnceLock::new();
    ELF.get_or_init(|| {
        let elf = std::fs::read(path).unwrap_or_default();
        if elf.is_empty() {
            println!("Warning: ELF file at '{path}' is empty or could not be read");
        }
        elf
    })
    .as_slice()
}

// Initialize the SP1 guest ELFs. Note: Normally this is done with include_bytes!(PATH_TO_FILE),
// but because we don't include the guest ELFs in the GitHub build, they may potentially not exist.
lazy_static! {
    pub static ref SP1_GUEST_MOCK_ELF: &'static [u8] = load_elf(&format!(
        "{}/guest-mock/elf/riscv32im-succinct-zkvm-elf",
        env!("CARGO_MANIFEST_DIR")
    ));
    pub static ref SP1_GUEST_CELESTIA_ELF: &'static [u8] = load_elf(&format!(
        "{}/guest-celestia/elf/riscv32im-succinct-zkvm-elf",
        env!("CARGO_MANIFEST_DIR")
    ));
}
