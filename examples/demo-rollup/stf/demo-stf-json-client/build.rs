use demo_stf::runtime::Runtime;
use sov_address::MultiAddressEvm;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::rest::HasRestApi;

type S = sov_modules_api::configurable_spec::ConfigurableSpec<
    MockDaSpec,
    MockZkvm,
    MockZkvm,
    MultiAddressEvm,
    Native,
>;

fn main() {
    println!("cargo:rerun-if-changed=../../../../crates/module-system/sov-modules-api");
    let runtime = Runtime::<S>::default();

    let spec = runtime.openapi_spec().unwrap();
    let serialized = serde_json::to_string_pretty(&spec).unwrap();
    // crate: openapiv3"
    let spec = serde_json::from_str(&serialized).unwrap();
    let mut generator = progenitor::Generator::default();

    let tokens = generator.generate_tokens(&spec).unwrap();
    let ast = syn::parse2(tokens).unwrap();
    let content = prettyplease::unparse(&ast);

    let mut out_file = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).to_path_buf();
    out_file.push("codegen.rs");

    std::fs::write(out_file, content).unwrap();
}
