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
        anyhow::bail!(
            "Risc0 rustc version {} does not match native rustc version {}. Please \
            update your Risc0 toolchain or use a rust-toolchain.toml file to force your \
            native compiler to the correct version.\n\n   To install a specific version of the Risc0 \
            rust toolchain, use the command `rzup install rust {{tag}}`.\n You can find a \
            list of available versions at https://github.com/risc0/rust/releases.\n",
            zkvm_version,
            native_version
        );
    }
    Ok(())
}
