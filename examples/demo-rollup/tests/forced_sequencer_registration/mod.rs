use std::sync::Arc;

use demo_stf::runtime::{Runtime, RuntimeCall};
use futures::StreamExt;
use sov_blob_storage::config_deferred_slots_count;
use sov_cli::NodeClient;
use sov_demo_rollup::{mock_da_risc0_host_args, MockDemoRollup};
use sov_mock_da::{MockAddress, MockDaSpec};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::{Amount, CryptoSpec, OperatingMode, RawTx, Runtime as RuntimeT, Spec};
use sov_modules_macros::config_value;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_test_utils::test_rollup::{read_private_key, RollupBuilder};
use sov_test_utils::TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING;

use crate::test_helpers::{test_genesis_source, DemoRollupSpec, CHAIN_HASH};

type TestSpec = DemoRollupSpec;
type TestPrivateKey = <<TestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;

const MAX_TX_FEE: Amount = Amount::new(100_000_000);
const UNREGISTERED_SENDER: MockAddress = MockAddress::new([121; 32]);
const MINIMUM_BOND: Amount = Amount::new(100_000_000);
const FINALIZATION_BLOCKS: u32 = 1;

// Verifies that a rollup with a preferred sequencer can handle forced registration from a different DA address.
// Steps:
// 1. Start the rollup with the preferred sequencer and demo-stf.
// 2. Submit a forced registration from another DA address.
// 3. Wait until the forced registration is processed.
// 4. Use the REST API to confirm that the new sequencer is registered.
#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_forced_sequencer_registration() -> anyhow::Result<()> {
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "50");

    let rollup = RollupBuilder::<MockDemoRollup<Native>>::new(
        test_genesis_source(OperatingMode::Zk),
        // We need to set the block producing mode to periodic to ensure that the forced registration
        // eventually succeeds because the registration batch is deferred.
        TEST_DEFAULT_MOCK_DA_PERIODIC_PRODUCING,
        FINALIZATION_BLOCKS,
    )
    .with_zkvm_host_args(mock_da_risc0_host_args())
    .set_config(|c| {
        c.max_concurrent_blobs = 65536;
        c.automatic_batch_production = true;
        c.rollup_prover_config = None;
        c.max_channel_size = 1;
        c.max_infos_in_db = 1;
    })
    .start()
    .await?;

    let da_service = Arc::new(
        rollup
            .da_service
            .another_on_the_same_layer(UNREGISTERED_SENDER)
            .await,
    );

    tokio::select! {
        err = rollup.rollup_task => err??,
        res = forced_sequencer_registration_test_case(da_service, &rollup.client) => res?,
    };
    Ok(())
}

async fn forced_sequencer_registration_test_case(
    da_service: Arc<impl DaService>,
    client: &NodeClient,
) -> anyhow::Result<()> {
    let key_and_address = read_private_key::<TestSpec>("tx_signer_private_key.json");
    let key = key_and_address.private_key;

    let tx = build_register_sequencer_tx(&key, 0);
    let blob = transaction_into_blob(tx);

    let _receipt = da_service
        .send_transaction(&blob)
        .await
        .await?
        .expect("Failed to submit forced sequencer registration to DA");

    let wait_for_slots = config_deferred_slots_count() + FINALIZATION_BLOCKS as u64 + 5;

    let mut slots = client
        .client
        .subscribe_finalized_slots_with_children(IncludeChildren::new(true))
        .await?;

    for _i in 0..wait_for_slots {
        let _slot = slots
            .next()
            .await
            .transpose()?
            .expect("slot data is missing");
        // This optimization could be enabled, when update of API state on sequencer side will be stable
        // if !slot.batches.is_empty() {
        //     break;
        // }
    }

    let allowed_sequencer = client
        .sequencer_rollup_address::<TestSpec, MockDaSpec>(&UNREGISTERED_SENDER)
        .await?
        .expect("Allowed sequencer response should contain data");
    assert_eq!(allowed_sequencer.balance, MINIMUM_BOND);
    assert_eq!(allowed_sequencer.address, key_and_address.address);
    Ok(())
}

fn build_register_sequencer_tx(
    key: &TestPrivateKey,
    nonce: u64,
) -> Transaction<Runtime<TestSpec>, TestSpec> {
    let msg =
        RuntimeCall::<TestSpec>::SequencerRegistry(sov_sequencer_registry::CallMessage::Register {
            da_address: UNREGISTERED_SENDER,
            amount: MINIMUM_BOND,
        });
    let chain_id = config_value!("CHAIN_ID");
    let max_priority_fee_bips = PriorityFeeBips::ZERO;
    let max_fee = MAX_TX_FEE;
    let gas_limit = None;
    Transaction::<Runtime<TestSpec>, TestSpec>::new_signed_tx(
        key,
        &CHAIN_HASH,
        UnsignedTransaction::new(
            msg,
            chain_id,
            max_priority_fee_bips,
            max_fee,
            nonce,
            gas_limit,
        ),
    )
}

fn transaction_into_blob(transaction: Transaction<Runtime<TestSpec>, TestSpec>) -> Vec<u8> {
    borsh::to_vec(
        &<Runtime<TestSpec> as RuntimeT<TestSpec>>::Auth::encode_with_standard_auth(RawTx {
            data: borsh::to_vec(&transaction).unwrap(),
        }),
    )
    .unwrap()
}
