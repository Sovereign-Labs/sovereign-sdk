use std::process::Command;

fn try_set_commit_hash_env() {
    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
            println!("cargo:rustc-env=GIT_COMMIT_HASH={hash}");
        }
    }
}

fn main() -> anyhow::Result<()> {
    try_set_commit_hash_env();

    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");

    if sov_zkvm_utils::should_skip_guest_build("sp1") {
        println!("cargo:warning=Skipping SP1 inner guest build for soak testing");
        println!("cargo::rerun-if-changed=build.rs");
        return Ok(());
    }

    println!("cargo::rerun-if-env-changed=OUT_DIR");

    let features = sov_zkvm_utils::collect_features(&[], &["native"]);
    sp1_build::build_program_with_args(
        "./inner-guest-sp1",
        sp1_build::BuildArgs {
            features,
            ..Default::default()
        },
    );

    Ok(())
}
