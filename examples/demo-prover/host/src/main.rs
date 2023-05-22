use demo_stf::app::create_demo_genesis_config;
use demo_stf::app::{DefaultPrivateKey, NativeAppRunner};
use demo_stf::config::from_toml_path;
use demo_stf::config::Config as RunnerConfig;
use jupiter::da_service::{CelestiaService, DaServiceConfig};
use jupiter::types::NamespaceId;
use jupiter::verifier::RollupParams;

use risc0_adapter::host::Risc0Host;
use risc0_zkvm::serde::to_vec;
use sovereign_core::services::da::DaService;
use sovereign_core::stf::{StateTransitionFunction, StateTransitionRunner};
use sovereign_core::zk::traits::ZkvmHost;

use serde::Deserialize;

use methods::{ROLLUP_ELF, ROLLUP_ID};

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RollupConfig {
    pub start_height: u64,
    pub da: DaServiceConfig,
    pub runner: RunnerConfig,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let rollup_config: RollupConfig = from_toml_path("rollup_config.toml")?;

    let mut demo_runner = NativeAppRunner::<Risc0Host>::new(rollup_config.runner.clone());
    let da_service = CelestiaService::new(
        rollup_config.da.clone(),
        RollupParams {
            namespace: NamespaceId([115, 111, 118, 45, 116, 101, 115, 116]),
        },
    );

    let demo = demo_runner.inner_mut();

    let sequencer_private_key = DefaultPrivateKey::generate();
    let genesis_config = create_demo_genesis_config(
        100000000,
        sequencer_private_key.default_address(),
        vec![
            99, 101, 108, 101, 115, 116, 105, 97, 49, 122, 102, 118, 114, 114, 102, 97, 113, 57,
            117, 100, 54, 103, 57, 116, 52, 107, 122, 109, 115, 108, 112, 102, 50, 52, 121, 115,
            97, 120, 113, 102, 110, 122, 101, 101, 53, 119, 57,
        ],
        &sequencer_private_key,
        &sequencer_private_key,
    );
    demo.init_chain(genesis_config);

    demo.begin_slot(Default::default());
    let (prev_state_root, _, _) = demo.end_slot();
    let mut prev_state_root = prev_state_root.0;

    for height in rollup_config.start_height..=rollup_config.start_height + 5 {
        let mut host = Risc0Host::new(ROLLUP_ELF);
        println!(
            "Requesting data for height {} and prev_state_root 0x{}",
            height,
            hex::encode(&prev_state_root)
        );
        let filtered_block = da_service.get_finalized_at(height).await?;
        let serialized_header = to_vec(&filtered_block.header).unwrap();
        let (blob_txs, inclusion_proof, completeness_proof) =
            da_service.extract_relevant_txs_with_proof(filtered_block);

        // // let mut prover =
        //     Prover::new(ROLLUP_ELF).expect("Prover should be constructed from valid ELF binary");

        // prover.add_input_u32_slice(&serialized_header);
        host.write_to_guest(&serialized_header);
        host.write_to_guest(&blob_txs);
        host.write_to_guest(&inclusion_proof);
        host.write_to_guest(&completeness_proof);
        host.write_to_guest(&prev_state_root);

        demo.begin_slot(Default::default());
        for blob in blob_txs.clone() {
            demo.apply_blob(blob, None);
        }

        let (next_state_root, witness, _) = demo.end_slot();
        host.write_to_guest(witness);
        // prover.add_input_u8_slice(host.hints.borrow().as_slice());

        let receipt = host.run().expect("Prover should run successfully");
        receipt.verify(&ROLLUP_ID).expect("Receipt should be valid");

        prev_state_root = next_state_root.0;
        println!("Completed proving block {}", height);
    }

    Ok(())
}
