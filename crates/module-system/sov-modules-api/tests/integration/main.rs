mod proof_tests;
mod rest_api;
mod state_tests;
mod transaction;

#[test]
fn trybuild() {
    let t = trybuild::TestCases::new();

    t.compile_fail("tests/integration/state_tests/trybuild/state_cannot_mutate_while_borrowed.rs");
    t.compile_fail("tests/integration/state_tests/trybuild/state_cannot_borrow_mut_twice.rs");
    t.compile_fail(
        "tests/integration/state_tests/trybuild/state_cannot_borrow_while_borrowed_mut.rs",
    );
}

#[test]
fn fail_unless_nextest() {
    // See:
    // - https://github.com/Sovereign-Labs/sovereign-sdk-wip/pull/2216
    // - https://nexte.st/docs/configuration/env-vars/#environment-variables-nextest-sets
    if std::env::var("NEXTEST").is_err() {
        panic!("Only cargo nextest is supported for running Sovereign-SDK tests. Use `make test` or `cargo nextest run`");
    }
}
