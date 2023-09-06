#[test]
fn module_info_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/module_info/parse.rs");
    t.pass("tests/module_info/mod_and_state.rs");
    t.pass("tests/module_info/use_address_trait.rs");
    t.pass("tests/module_info/not_supported_attribute.rs");
    t.pass("tests/module_info/custom_codec_builder.rs");
    t.pass("tests/custom_codec_must_be_used.rs");
    t.compile_fail("tests/module_info/derive_on_enum_not_supported.rs");
    t.compile_fail("tests/module_info/field_missing_attribute.rs");
    t.compile_fail("tests/module_info/missing_address.rs");
    t.compile_fail("tests/module_info/no_generics.rs");
    t.compile_fail("tests/module_info/not_supported_type.rs");
    t.compile_fail("tests/module_info/second_addr_not_supported.rs");
}

#[test]
fn module_dispatch_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/dispatch/derive_genesis.rs");
    t.pass("tests/dispatch/derive_dispatch.rs");
    t.compile_fail("tests/dispatch/missing_serialization.rs");
}

#[test]
fn rpc_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/rpc/derive_rpc.rs");
    t.pass("tests/rpc/derive_rpc_with_where.rs");
    t.pass("tests/rpc/expose_rpc.rs");
    t.pass("tests/rpc/expose_rpc_associated_types.rs");
    t.pass("tests/rpc/expose_rpc_associated_types_nested.rs");

    t.compile_fail("tests/rpc/expose_rpc_associated_type_not_static.rs");
    t.compile_fail("tests/rpc/expose_rpc_first_generic_not_context.rs");
}

#[test]
fn cli_wallet_arg_tests() {
    let t: trybuild::TestCases = trybuild::TestCases::new();
    t.pass("tests/cli_wallet_arg/derive_enum_named_fields.rs");
    t.pass("tests/cli_wallet_arg/derive_struct_unnamed_fields.rs");
    t.pass("tests/cli_wallet_arg/derive_struct_named_fields.rs");
    t.pass("tests/cli_wallet_arg/derive_enum_mixed_fields.rs");
    t.pass("tests/cli_wallet_arg/derive_enum_unnamed_fields.rs");
    t.pass("tests/cli_wallet_arg/derive_wallet.rs");
}
