fn main() {
    // This script intentionally ignores `SKIP_GUEST_BUILD`.
    // The nested `sov-aggregated-proof` workspace is not part of the main SDK workspace, so
    // always rebuilding the guest here does not affect normal whole-project build times.
    // This crate is used for manual aggregation-circuit testing, where we want the ELF to be
    // rebuilt instead of accidentally reusing a stale artifact.
    sp1_build::build_program("../program");
}
