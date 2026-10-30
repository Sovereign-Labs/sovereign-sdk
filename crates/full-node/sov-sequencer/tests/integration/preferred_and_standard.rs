//! A node running the preferred sequencer and a node running a registered, non-preferred
//! standard sequencer, side by side on the same mock DA layer.

use std::future::Future;
use std::time::Duration;

use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::{FutureExt, StreamExt};
use sov_api_spec::types::{self as api_types, IntOrHash, TxReceiptResult};
use sov_mock_da::{BlockProducingConfig, MockAddress};
use sov_modules_api::{Amount, RawTx, Runtime};
use sov_modules_stf_blueprint::GenesisParams;
use sov_rollup_interface::TxHash;
use sov_sequencer::SequencerKindConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, TestRollup};
use sov_test_utils::{
    generate_optimistic_runtime_with_kernel, RtAgnosticBlueprint, TestSpec, TestUser,
    TEST_FINALIZATION_BLOCKS,
};

use crate::utils::{encode_call, tempdir_inside_codebase_dir};

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [],
);

type S = TestSpec;
type RT = TestRuntime<S>;
type TestBlueprint = RtAgnosticBlueprint<S, RT>;

const STANDARD_SEQUENCER_DA_ADDRESS: MockAddress = MockAddress::new([0x5e; 32]);
const PROGRESS_TIMEOUT: Duration = Duration::from_secs(60);

struct SideBySide {
    preferred: TestRollup<TestBlueprint>,
    standard: TestRollup<TestBlueprint>,
    /// Transfer sent through the preferred sequencer, by an account that touches nothing else.
    preferred_tx_hash: TxHash,
    /// Two transfers sent through the standard sequencer after it got registered, by an account
    /// that touches nothing else. Two, so they land in one multi-tx batch.
    standard_tx_hashes: [TxHash; 2],
}

fn create_genesis_params() -> (GenesisParams<GenesisConfig<S>>, Vec<TestUser<S>>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(5);
    let users = genesis_config.additional_accounts().to_vec();
    let runtime = <RT as Runtime<S>>::GenesisConfig::from_minimal_config(genesis_config.into());
    (GenesisParams { runtime }, users)
}

fn transfer(sender: &TestUser<S>, recipient: &TestUser<S>, generation: u64) -> RawTx {
    encode_call::<RT>(
        &sender.private_key,
        generation,
        &TestRuntimeCall::Bank(sov_test_utils::sov_bank::CallMessage::Transfer {
            to: recipient.address(),
            coins: sov_test_utils::runtime::Coins {
                amount: Amount::new(0x1234),
                token_id: sov_test_utils::runtime::config_gas_token_id(),
            },
        }),
    )
}

async fn accept_tx(client: &sov_api_spec::Client, tx: &RawTx) -> TxHash {
    let accepted = client
        .accept_tx(&api_types::AcceptTxBody {
            body: BASE64_STANDARD.encode(&tx.data),
        })
        .await
        .expect("sequencer should accept the transaction");
    accepted.id.parse().unwrap()
}

/// Returns `None` while the node hasn't executed the transaction yet.
async fn ledger_tx(
    rollup: &TestRollup<TestBlueprint>,
    tx_hash: &TxHash,
) -> Option<api_types::LedgerTx> {
    let hash = tx_hash.to_string().parse().unwrap();
    match rollup
        .api_client()
        .get_tx_by_id(&IntOrHash::Hash(hash), None)
        .await
    {
        Ok(response) => Some(response.into_inner()),
        Err(e) if e.status() == Some(axum::http::StatusCode::NOT_FOUND) => None,
        Err(e) => panic!("failed to query tx {tx_hash}: {e}"),
    }
}

async fn slot_number_containing(rollup: &TestRollup<TestBlueprint>, tx_hash: &TxHash) -> u64 {
    let tx = ledger_tx(rollup, tx_hash).await.unwrap();
    rollup
        .api_client()
        .get_batch_by_id(&IntOrHash::Integer(tx.batch_number), None)
        .await
        .unwrap()
        .into_inner()
        .slot_number
}

/// Produces DA blocks through the preferred node (the only block producer on the shared layer)
/// until `done` returns true. Fails if either node crashes, including while `done` is evaluated.
async fn produce_blocks_until<F, Fut>(
    preferred: &TestRollup<TestBlueprint>,
    standard: &TestRollup<TestBlueprint>,
    description: &str,
    mut done: F,
) where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let assert_no_crash = || {
        for node in [preferred, standard] {
            assert!(
                !node.is_rollup_crashed(),
                "a node crashed while waiting for {description}"
            );
        }
    };
    tokio::time::timeout(PROGRESS_TIMEOUT, async {
        loop {
            assert_no_crash();
            if done().await {
                assert_no_crash();
                return;
            }
            preferred.tenderly_produce_blocks(1).await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {description}"));
}

/// Starts both nodes, registers the standard sequencer through the preferred one, then sends one
/// transfer through each sequencer and waits until both nodes have executed both.
///
/// Every account involved (operator, the two senders, the two recipients) is distinct, so the
/// two sequencers never contend on nonces or balances.
async fn run_side_by_side() -> SideBySide {
    let preferred_dir = tempdir_inside_codebase_dir();
    let standard_dir = tempdir_inside_codebase_dir();
    let (genesis_params, users) = create_genesis_params();
    let preferred_seq_config = &genesis_params.runtime.sequencer_registry.sequencer_config;
    let (preferred_da_address, sequencer_bond) = (
        preferred_seq_config.seq_da_address,
        preferred_seq_config.seq_bond,
    );
    let [standard_operator, preferred_sender, standard_sender, preferred_recipient, standard_recipient] =
        &users[..]
    else {
        panic!("expected 5 genesis accounts");
    };

    let preferred = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params.clone()),
        BlockProducingConfig::Manual,
        TEST_FINALIZATION_BLOCKS,
    )
    .set_config(|c| {
        c.storage = sov_test_utils::test_rollup::StoragePath::Tmp(preferred_dir);
        if let SequencerKindConfig::Preferred(p) = &mut c.sequencer_config {
            p.ideal_lag_behind_finalized_slot = 3;
        }
    })
    .set_da_config(|c| c.sender_address = preferred_da_address)
    .start()
    .await
    .unwrap();
    preferred.produce_enough_finalized_slots().await;
    preferred.wait_for_sequencer_ready().await.unwrap();

    let da_layer = preferred.da_service.da_layer();
    let standard = RollupBuilder::<TestBlueprint>::new(
        GenesisSource::CustomParams(genesis_params),
        BlockProducingConfig::Manual,
        TEST_FINALIZATION_BLOCKS,
    )
    .set_config(|c| {
        c.storage = sov_test_utils::test_rollup::StoragePath::Tmp(standard_dir);
        c.sequencer_address = standard_operator.address().to_string();
    })
    .set_da_config(|c| {
        c.da_layer = Some(da_layer);
        c.sender_address = STANDARD_SEQUENCER_DA_ADDRESS;
    })
    .with_standard_sequencer()
    .start()
    .await
    .unwrap();
    standard.wait_for_node_synced().await.unwrap();

    let register_tx = encode_call::<RT>(
        &standard_operator.private_key,
        0,
        &TestRuntimeCall::SequencerRegistry(
            sov_test_utils::sov_sequencer_registry::CallMessage::Register {
                da_address: STANDARD_SEQUENCER_DA_ADDRESS,
                // A bond at the registry minimum is too small to cover batch gas reservations, so
                // match the genesis sequencer.
                amount: sequencer_bond,
            },
        ),
    );
    let register_hash = accept_tx(preferred.api_client(), &register_tx).await;
    preferred.force_close_batch().await.unwrap();
    produce_blocks_until(
        &preferred,
        &standard,
        "registration to execute on the standard node",
        || async { ledger_tx(&standard, &register_hash).await.is_some() },
    )
    .await;
    assert_eq!(
        ledger_tx(&standard, &register_hash).await.unwrap().receipt.result,
        TxReceiptResult::Successful,
        "standard sequencer registration must succeed, otherwise its batches go down the unregistered path"
    );

    let mut standard_tx_hashes = Vec::new();
    for generation in 0..2 {
        standard_tx_hashes.push(
            accept_tx(
                standard.api_client(),
                &transfer(standard_sender, standard_recipient, generation),
            )
            .await,
        );
    }
    let standard_tx_hashes: [TxHash; 2] = standard_tx_hashes.try_into().unwrap();
    let preferred_tx_hash = accept_tx(
        preferred.api_client(),
        &transfer(preferred_sender, preferred_recipient, 0),
    )
    .await;
    preferred.force_close_batch().await.unwrap();
    produce_blocks_until(
        &preferred,
        &standard,
        "both transfers to execute on both nodes",
        || async {
            for node in [&preferred, &standard] {
                for hash in std::iter::once(&preferred_tx_hash).chain(&standard_tx_hashes) {
                    if ledger_tx(node, hash).await.is_none() {
                        return false;
                    }
                }
            }
            true
        },
    )
    .await;

    SideBySide {
        preferred,
        standard,
        preferred_tx_hash,
        standard_tx_hashes,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn txs_from_both_sequencers_succeed_on_both_nodes() {
    let run = run_side_by_side().await;

    for (node_name, node) in [("preferred", &run.preferred), ("standard", &run.standard)] {
        for (seq_name, hash) in std::iter::once(("preferred", &run.preferred_tx_hash))
            .chain(run.standard_tx_hashes.iter().map(|h| ("standard", h)))
        {
            assert_eq!(
                ledger_tx(node, hash).await.unwrap().receipt.result,
                TxReceiptResult::Successful,
                "tx sent through the {seq_name} sequencer should succeed on the {node_name} node"
            );
        }
    }
}

/// The preferred sequencer replays non-preferred batches and checks its state root against the
/// node's, panicking on mismatch. Both happen when it processes the node's state update, which can
/// be after the node's ledger already shows the transaction, so wait for that update explicitly.
#[tokio::test(flavor = "multi_thread")]
async fn preferred_sequencer_stays_ready_after_non_preferred_batch() {
    let run = run_side_by_side().await;
    let target_slot = slot_number_containing(&run.preferred, &run.standard_tx_hashes[0]).await;
    let mut state_updates = run
        .preferred
        .subscribe_state_updates()
        .await
        .expect("should subscribe to preferred sequencer state updates");

    // The sequencer processes slots in order, so any update at or past `target_slot` means it has
    // processed `target_slot`.
    produce_blocks_until(
        &run.preferred,
        &run.standard,
        "the preferred sequencer to process the slot with the non-preferred batch",
        || {
            let mut reached = false;
            while let Some(notification) = state_updates.next().now_or_never() {
                let notification = notification
                    .expect("state update stream ended")
                    .expect("state update notification should deserialize");
                reached |= notification.slot_number.get() >= target_slot;
            }
            std::future::ready(reached)
        },
    )
    .await;

    assert!(
        run.preferred.is_sequencer_ready().await,
        "preferred sequencer should stay ready after replaying a non-preferred batch"
    );
}
