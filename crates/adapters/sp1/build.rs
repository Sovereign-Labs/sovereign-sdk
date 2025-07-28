use anyhow::Context;
use sov_zkvm_utils::{does_rustc_match, should_skip_guest_build, RustComparisonResult};

fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");
    if should_skip_guest_build("sp1") {
        return Ok(());
    }

    let toolchain_cmp_result = does_rustc_match("succinct")
        .context("Failed to get SP1 rustc version. Is SP1 installed? If not you can install it with the `sp1up` tool. Try running:\n\n   curl -L https://sp1.succinct.xyz | bash && sp1up")?;

    if let RustComparisonResult::Different {
        native_version,
        zkvm_version,
    } = toolchain_cmp_result
    {
        anyhow::bail!(
            "SP1 rustc version {} does not match native rustc version {}. Please \
            update your SP1 toolchain or use a rust-toolchain.toml file to force your \
            native compiler to the correct version.\n\n   To install a specific version of the SP1 \
            rust toolchain, use the command `sp1up -C {{commit}}` where {{commit}} is a release commit hash.\n   You can find a \
            list of available versions at https://github.com/succinctlabs/sp1/releases\
            For reproducible builds, use `sp1up -C <commit>` with a specific commit hash.\n",
            zkvm_version,
            native_version,
        );
    }

    Ok(())
}
