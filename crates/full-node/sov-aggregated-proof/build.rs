use sov_zkvm_utils::should_skip_guest_build;
use sp1_build::{build_program_with_args, BuildArgs};

fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");

    if should_skip_guest_build("sp1") {
        println!("cargo:warning=Skipping sp1 guest build");
        println!("cargo::rerun-if-changed=build.rs");
        return Ok(());
    }

    println!("cargo::rerun-if-env-changed=OUT_DIR");

    build_program_with_args(
        "./guest",
        BuildArgs {
            ..Default::default()
        },
    );

    Ok(())
}
