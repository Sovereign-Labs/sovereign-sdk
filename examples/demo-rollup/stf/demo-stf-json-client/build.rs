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

/// Progenitor supports at most 2 response types per endpoint (one success, one error).
/// Hyperlane endpoints define 200, 400, and 404 responses which exceeds this limit.
/// Strip excess non-2xx responses so that at most one error response remains per endpoint.
fn normalize_responses_for_progenitor(spec: &mut serde_json::Value) {
    if let Some(paths) = spec.get_mut("paths").and_then(|p| p.as_object_mut()) {
        for (_path, methods) in paths.iter_mut() {
            if let Some(methods) = methods.as_object_mut() {
                for (_method, operation) in methods.iter_mut() {
                    if let Some(responses) = operation
                        .get_mut("responses")
                        .and_then(|r| r.as_object_mut())
                    {
                        let non_2xx: Vec<String> = responses
                            .keys()
                            .filter(|code| !code.starts_with('2'))
                            .cloned()
                            .collect();
                        // Keep only the first non-2xx response (e.g. 404), drop others (e.g. 400)
                        for code in non_2xx.iter().skip(1) {
                            responses.remove(code);
                        }
                    }
                }
            }
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/module-system/sov-modules-api");
    let runtime = Runtime::<S>::default();

    let spec = runtime.openapi_spec().unwrap();
    let serialized = serde_json::to_string_pretty(&spec).unwrap();
    // crate: openapiv3
    let mut spec: serde_json::Value = serde_json::from_str(&serialized).unwrap();
    normalize_responses_for_progenitor(&mut spec);
    let spec = serde_json::from_value(spec).unwrap();
    let mut generator = progenitor::Generator::default();

    let tokens = generator.generate_tokens(&spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    let content = prettyplease::unparse(&ast);

    let mut out_file = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    out_file.push("codegen.rs");

    std::fs::write(out_file, content).unwrap();
}
