use std::sync::LazyLock;

const TEST_INNER_ELF_ENV: &str = "SOV_TEST_SP1_GUEST_MOCK_ELF";
const TEST_OUTER_ELF_ENV: &str = "SOV_TEST_SP1_GUEST_AGGREGATION_MOCK_ELF";

/// Attempt to load the ELF file at the given path. If the file is empty or cannot be read,
/// a warning will be printed and the default empty vector will be returned.
/// Debug builds may override the path through `test_override_env` so integration tests can
/// exercise copied or patched guests without modifying the build artifacts in place.
fn load_elf(default_path: &str, test_override_env: &str) -> &'static [u8] {
    #[cfg(debug_assertions)]
    let path = std::env::var(test_override_env).unwrap_or_else(|_| default_path.to_owned());
    #[cfg(not(debug_assertions))]
    let path = {
        let _ = test_override_env;
        default_path.to_owned()
    };

    let elf = std::fs::read(&path).unwrap_or_default();
    if elf.is_empty() {
        println!("Warning: ELF file at '{path}' is empty or could not be read");
    }

    Vec::leak(elf)
}

// Initialize the SP1 guest ELFs. Note: Normally this is done with include_bytes!(PATH_TO_FILE),
// but because we don't include the guest ELFs in the GitHub build, they may potentially not exist.
pub static SP1_GUEST_MOCK_ELF: LazyLock<&'static [u8]> = LazyLock::new(|| {
    load_elf(
        &format!(
            "{}/guest-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-mock-sp1",
            env!("CARGO_MANIFEST_DIR")
        ),
        TEST_INNER_ELF_ENV,
    )
});
pub static SP1_GUEST_AGGREGATION_MOCK_ELF: LazyLock<&'static [u8]> = LazyLock::new(|| {
    load_elf(
        &format!(
            "{}/guest-aggregation-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-aggregated-proof-program",
            env!("CARGO_MANIFEST_DIR")
        ),
        TEST_OUTER_ELF_ENV,
    )
});
