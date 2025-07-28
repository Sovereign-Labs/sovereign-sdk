mod custom_state_codec_builder;
mod derive_wallet;
mod dispatch;
mod hygienic_module_info;

#[cfg(feature = "bench")]
mod cycle_macro;

#[cfg(all(feature = "gas-constant-estimation", feature = "native"))]
mod metrics;

mod module_info;
mod regression_tests;
mod rpc_gen;
mod unrecognized_state_attributes_are_ignored;

// All `trybuild` tests are run on the same `trybuild::TestCases` instance
// because it's faster (`TestCases` internally parallelizes the tests).
//
// See the discussion in <https://github.com/dtolnay/trybuild/pull/285>.
#[test]
fn trybuild() {
    let t = trybuild::TestCases::new();

    // NOTE: We'd like to use `trybuild` only for tests that fail to compile,
    // but in this case we rely on `trybuild` to compile `sov-modules-macros`
    // with `SOV_TEST_MODE_CONST_MANIFEST` set to `1`, so as to use
    // `constants.testing.toml`.
    std::env::set_var("SOV_TEST_MODE_CONST_MANIFEST", "1");
    t.pass("tests/integration/trybuild/constants/valid_constants.rs");

    t.compile_fail("tests/integration/trybuild/constants/bech32_constant_invalid_checksum.rs");
    t.compile_fail("tests/integration/trybuild/constants/bech32_constant_not_a_string.rs");
    t.compile_fail("tests/integration/trybuild/constants/bech32_constant_prefix_too_short.rs");
    t.compile_fail("tests/integration/trybuild/constants/bech32_constant_prefix_too_long.rs");

    t.compile_fail("tests/integration/trybuild/module_info/derive_on_enum_not_supported.rs");
    t.compile_fail("tests/integration/trybuild/module_info/field_missing_attribute.rs");
    t.compile_fail("tests/integration/trybuild/module_info/missing_address.rs");
    t.compile_fail("tests/integration/trybuild/module_info/no_generics.rs");
    t.compile_fail("tests/integration/trybuild/module_info/not_supported_type.rs");
    t.compile_fail("tests/integration/trybuild/module_info/second_addr_not_supported.rs");

    t.compile_fail("tests/integration/trybuild/dispatch/derive_event_no_default_attrs.rs");

    t.compile_fail("tests/integration/trybuild/rpc/derive_rpc_working_set_immutable_reference.rs");
    t.compile_fail("tests/integration/trybuild/rpc/derive_rpc_working_set_no_generic.rs");
    t.compile_fail("tests/integration/trybuild/rpc/expose_rpc_associated_type_not_static.rs");
    t.compile_fail("tests/integration/trybuild/rpc/expose_rpc_first_generic_not_spec.rs");
}
