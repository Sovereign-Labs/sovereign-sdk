mod utils;

#[test]
fn tests() {
    let t = trybuild::TestCases::new();
    t.pass("tests/parse.rs");
    t.pass("tests/mod_and_state.rs");
    t.compile_fail("tests/field_missing_attribute.rs");
    t.compile_fail("tests/not_supported_attribute.rs");
    t.compile_fail("tests/derive_on_enum_not_supported.rs");
    t.compile_fail("tests/not_supported_type.rs");
}
