fn main() {
    // This script intentionally ignores `SKIP_GUEST_BUILD`.
    // The guest lives in its own workspace under `provers/sp1/guest-aggregation-mock`, so
    // rebuilding it here does not affect normal whole-project build times.
    // This crate is used for manual aggregation-circuit testing, where we want the ELF to be
    // rebuilt instead of accidentally reusing a stale artifact.
    sp1_build::build_program("../../../../provers/sp1/guest-aggregation-mock");
}
