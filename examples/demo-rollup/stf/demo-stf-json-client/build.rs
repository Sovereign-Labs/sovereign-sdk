use demo_stf::runtime::Runtime;
use demo_stf::MultiAddressEvmSolana;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::rest::HasRestApi;

type S = sov_modules_api::configurable_spec::ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvmSolana,
    Native,
>;

/// Recursively downconvert an OpenAPI 3.1 spec to 3.0.3 for progenitor compatibility.
/// Handles: version string, `type: "null"` in oneOf arrays.
/// See: oxidecomputer/progenitor#762
fn downconvert_to_3_0(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            // Downgrade version string
            if let Some(v) = map.get_mut("openapi") {
                if v.as_str() == Some("3.1.0") {
                    *v = serde_json::Value::String("3.0.3".to_string());
                }
            }
            // Convert 3.1 `type: "null"` in oneOf to 3.0 `nullable: true`
            if let Some(serde_json::Value::Array(one_of)) = map.get_mut("oneOf") {
                let had_null = one_of.iter().any(|item| {
                    matches!(item.get("type"), Some(serde_json::Value::String(t)) if t == "null")
                });
                if had_null {
                    one_of.retain(|item| {
                        !matches!(item.get("type"), Some(serde_json::Value::String(t)) if t == "null")
                    });
                    map.insert("nullable".to_string(), serde_json::Value::Bool(true));
                }
            }
            // Recurse
            for v in map.values_mut() {
                downconvert_to_3_0(v);
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                downconvert_to_3_0(v);
            }
        }
        _ => {}
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/module-system/sov-modules-api");
    let runtime = Runtime::<S>::default();

    let spec = runtime.openapi_spec().unwrap();
    let mut serialized = serde_json::to_value(&spec).unwrap();
    downconvert_to_3_0(&mut serialized);
    // crate: openapiv3
    let spec: openapiv3::OpenAPI = serde_json::from_value(serialized).unwrap();
    let mut generator = progenitor::Generator::default();

    let tokens = generator.generate_tokens(&spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    let content = prettyplease::unparse(&ast);

    let mut out_file = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    out_file.push("codegen.rs");

    std::fs::write(out_file, content).unwrap();
}
