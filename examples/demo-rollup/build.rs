use sov_zkvm_utils::should_skip_guest_build;

fn main() -> anyhow::Result<()> {
    println!("cargo::rerun-if-env-changed=SKIP_GUEST_BUILD");
    println!("cargo::rerun-if-env-changed=SOV_PROVER_MODE");
    println!("cargo::rustc-check-cfg=cfg(skip_guest_build)");
    println!("cargo:rerun-if-changed=NULL");

    // It will be true only if SKIP_GUEST_BUILD is 1 or true
    if should_skip_guest_build("any-zkvm") {
        println!("cargo::rustc-cfg=skip_guest_build");
    }

    Ok(())
}
