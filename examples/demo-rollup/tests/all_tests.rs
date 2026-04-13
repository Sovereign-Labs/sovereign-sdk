mod bank;
mod evm;
mod forced_sequencer_registration;
// prover tests moved to separate [[test]] binary (prover_tests.rs, requires "sp1" feature)
#[cfg(feature = "mock_da_external")]
mod replica;
mod rest_api;
mod restart;
mod resync;
mod test_helpers;
mod wallet;
