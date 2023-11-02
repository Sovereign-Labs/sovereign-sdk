#![no_main]
use demo_stf::runtime::Runtime;
use demo_stf::AppVerifier;
use sov_mock_da::MockDaVerifier;
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_stf_template::kernels::basic::BasicKernel;
use sov_modules_stf_template::AppTemplate;
use sov_risc0_adapter::guest::Risc0Guest;
use sov_state::ZkStorage;

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let storage = ZkStorage::new();
    let app: AppTemplate<ZkDefaultContext, _, _, Runtime<_, _>, BasicKernel<_>> =
        AppTemplate::new();

    let mut stf_verifier = AppVerifier::new(app, MockDaVerifier {});

    stf_verifier
        .run_block(guest, storage)
        .expect("Prover must be honest");
}
