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

    // Only the mock-DA inner guest and the aggregation guest are used. There is no
    // celestia+sp1 rollup, so the SP1 celestia guest is intentionally not built.
    build_program_with_args(
        "./guest-mock",
        BuildArgs {
            features,
            ..Default::default()
        },
    );
    build_program_with_args("./guest-aggregation-mock", BuildArgs::default());

    Ok(())
}
