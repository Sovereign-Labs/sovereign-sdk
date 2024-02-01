#![no_main]
use demo_stf::runtime::Runtime;
use demo_stf::StfVerifier;
#[cfg(feature = "bench")]
use risc0_zkvm::guest::env;
use sov_mock_da::MockDaVerifier;
use sov_modules_api::default_context::ZkDefaultContext;
use sov_modules_stf_blueprint::kernels::basic::BasicKernel;
use sov_modules_stf_blueprint::StfBlueprint;
use sov_risc0_adapter::guest::Risc0Guest;
use sov_state::ZkStorage;

#[cfg(feature = "bench")]
fn report_bench_metrics(start_cycles: usize, end_cycles: usize) {
    let cycles_per_block = (end_cycles - start_cycles) as u64;
    let tuple = ("Cycles per block".to_string(), cycles_per_block);
    let mut serialized = Vec::new();
    serialized.extend(tuple.0.as_bytes());
    serialized.push(0);
    let size_bytes = tuple.1.to_ne_bytes();
    serialized.extend(&size_bytes);

    // calculate the syscall name.
    let cycle_string = String::from("cycle_metrics\0");
    let metrics_syscall_name =
        risc0_zkvm_platform::syscall::SyscallName::from_bytes_with_nul(cycle_string.as_ptr());

    risc0_zkvm::guest::env::send_recv_slice::<u8, u8>(metrics_syscall_name, &serialized);
}

risc0_zkvm::guest::entry!(main);

pub fn main() {
    let guest = Risc0Guest::new();
    let storage = ZkStorage::new();
    #[cfg(feature = "bench")]
    let start_cycles = env::get_cycle_count();

    let stf: StfBlueprint<ZkDefaultContext, _, _, Runtime<_, _>, BasicKernel<_, _>> =
        StfBlueprint::new();

    let stf_verifier = StfVerifier::new(stf, MockDaVerifier {});

    stf_verifier
        .run_block(guest, storage)
        .expect("Prover must be honest");

    #[cfg(feature = "bench")]
    {
        let end_cycles = env::get_cycle_count();
        report_bench_metrics(start_cycles, end_cycles);
    }
}
