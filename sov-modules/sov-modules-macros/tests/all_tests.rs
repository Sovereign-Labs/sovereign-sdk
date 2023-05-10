#[test]
fn module_info_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/module_info/parse.rs");
    t.pass("tests/module_info/mod_and_state.rs");
    t.compile_fail("tests/module_info/field_missing_attribute.rs");
    t.compile_fail("tests/module_info/not_supported_attribute.rs");
    t.compile_fail("tests/module_info/derive_on_enum_not_supported.rs");
    t.compile_fail("tests/module_info/not_supported_type.rs");
    t.compile_fail("tests/module_info/second_addr_not_supported.rs");
    t.compile_fail("tests/module_info/missing_address.rs");
}

#[test]
fn module_dispatch_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/dispatch/derive_genesis.rs");
    t.pass("tests/dispatch/derive_rpc.rs");
    t.pass("tests/dispatch/derive_dispatch.rs");
    t.compile_fail("tests/dispatch/missing_serialization.rs");
}

#[test]
fn rpc_tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/dispatch/derive_rpc.rs");
}
