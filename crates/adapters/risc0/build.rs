use anyhow::Context;
use sov_zkvm_utils::{does_rustc_match, should_skip_guest_build, RustComparisonResult};

// Checks that the risc0 toolchain and native toolchain use the same rustc version
fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");

    // Skip the check if we aren't building any guest code
    if should_skip_guest_build("risc0") {
        println!("cargo:warning=Skipping risc0 guest build");
        return Ok(());
    }

    let toolchain_cmp_result = does_rustc_match("risc0")
        .context("Is risc0 installed? If not you can install it with the `rzup` tool")?;

    if let RustComparisonResult::Different {
        native_version,
        zkvm_version,
    } = toolchain_cmp_result
    {
        println!(
            "cargo:warning=Risc0 rustc version {} does not match native rustc version {}. \
            There could be incompatibilities between the two versions. \
            Guest programs are compiled with the Risc0 toolchain, so this is usually fine.",
            zkvm_version, native_version
        );
    }
    Ok(())
}
