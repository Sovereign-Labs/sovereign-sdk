use sov_zkvm_utils::should_skip_guest_build;
use sp1_build::{build_program_with_args, BuildArgs};

fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");

    if should_skip_guest_build("sp1") {
        println!("cargo:warning=Skipping sp1 guest build");
        // When skipping, only rerun if build.rs changes
        println!("cargo::rerun-if-changed=build.rs");
        return Ok(());
    }

    // When building guests, track OUT_DIR to detect guest dependency changes
    println!("cargo::rerun-if-env-changed=OUT_DIR");
    let features = sov_zkvm_utils::collect_features(&["bench"], &["native"]);

    build_program_with_args(
        "./guest-mock",
        BuildArgs {
            features: features.clone(),
            ..Default::default()
        },
    );
    build_program_with_args(
        "./guest-celestia",
        BuildArgs {
            features,
            ..Default::default()
        },
    );

    Ok(())
}
