use sov_mock_da::MockBlob;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{Gas, GasArray, Spec, TransactionReceipt};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::TestRunner;

use super::optimistic_rt::helpers::*;
use super::optimistic_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{create_txs, TxStatus, S};

fn create_runner_and_blobs(
    tx_statuses: &[TxStatus],
) -> (TestRunner<IntegTestRuntime<S>, S>, RelevantBlobs<MockBlob>) {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let (runner, users, sequencer_account) = setup(2);

    let actors = Actors {
        admin_account: users[0].clone(),
        not_admin_account: users[1].clone(),
        sequencer_account,
    };

    let txs = create_txs::<IntegTestRuntime<S>>(
        tx_statuses,
        priority_fee_bips,
        &actors.admin_account,
        &actors.not_admin_account,
    );

    let seq_da_address = runner.config.sequencer_da_address;

    let batch_blobs = vec![MockBlob::new_with_hash(
        borsh::to_vec(&txs).unwrap(),
        seq_da_address,
    )];

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs,
    };

    (runner, blobs)
}

// The sequencer should see only the effects of successful transactions.
// This test verifies that unsuccessful transactions do not consume any gas.
#[test]
fn test_sequencer_process_only_sucessfull_tx() {
    // This sequence consumes the same amount of gas in the SEQUENCER, as a single TxStatus::Success in the NODE, so the cache after the initial revert is invalidated.
    check_seq_and_node_gas(vec![TxStatus::Reverted, TxStatus::Success]);
    // This sequence consumes the same amount of gas in the SEQUENCER as two TxStatus::Success in the NODE, so the revert doesn’t overly invalidate the cache
    // (i.e., the revert doesn’t wipe out the cache from the earlier successful transactions).
    check_seq_and_node_gas(vec![
        TxStatus::Success,
        TxStatus::Reverted,
        TxStatus::Success,
    ]);

    // Some other cases.
    check_seq_and_node_gas(vec![
        TxStatus::Reverted,
        TxStatus::Success,
        TxStatus::OutOfGas,
        TxStatus::BadGeneration,
        TxStatus::BadSerialization,
        TxStatus::SignerDoesNotExist,
        TxStatus::BadChainId,
        TxStatus::Reverted,
        TxStatus::Success,
        TxStatus::Reverted,
        TxStatus::Reverted,
        TxStatus::Reverted,
        TxStatus::Success,
    ]);
}

fn check_seq_and_node_gas(txs: Vec<TxStatus>) {
    // Each successful tx in txs has a different generation number.
    // Gas used in sequencer
    let gas_used_by_sequencer = {
        let (mut runner, blobs) = create_runner_and_blobs(&txs);
        let result = runner.execute_as_sequencer::<RelevantBlobs<MockBlob>>(blobs.clone());
        let batch_receipt_1 = result.0.batch_receipts[0].clone();
        get_gas_from_txs(&batch_receipt_1.tx_receipts)
    };

    // Gas used in node only for successful transactions.
    let sucess_txs: Vec<_> = txs
        .into_iter()
        .filter(|tx| matches!(tx, TxStatus::Success))
        .collect();

    let gas_used_by_node = {
        let (mut runner, blobs) = create_runner_and_blobs(&sucess_txs);
        let result = runner.execute::<RelevantBlobs<MockBlob>>(blobs.clone());
        let batch_receipt_1 = result.0.batch_receipts[0].clone();
        get_gas_from_txs(&batch_receipt_1.tx_receipts)
    };
    assert_eq!(gas_used_by_node, gas_used_by_sequencer);
}

/// Check if reverted tx is ignored by the sequecner.
#[test]
fn test_sequencer_inores_reverted_tx() {
    let (mut runner, blobs) = create_runner_and_blobs(&[TxStatus::Reverted]);
    let result = runner.execute_as_sequencer::<RelevantBlobs<MockBlob>>(blobs);
    let batch_receipt_1 = result.0.batch_receipts[0].clone();

    // Sequencer ignores reverted txs so the `tx_receipts` should be empty.
    assert!(&batch_receipt_1.tx_receipts.is_empty());
}

fn get_gas_from_txs(receipts: &[TransactionReceipt<S>]) -> <S as Spec>::Gas {
    let mut gas_in_batch = <<S as Spec>::Gas>::zero();

    for receipt in receipts {
        match &receipt.receipt {
            sov_modules_api::TxEffect::Successful(tx_contents) => {
                gas_in_batch = gas_in_batch.checked_combine(&tx_contents.gas_used).unwrap();
            }
            sov_modules_api::TxEffect::Reverted(tx_contents) => {
                gas_in_batch = gas_in_batch.checked_combine(&tx_contents.gas_used).unwrap();
            }
            sov_modules_api::TxEffect::Skipped(tx_contents) => {
                gas_in_batch = gas_in_batch.checked_combine(&tx_contents.gas_used).unwrap();
            }
        }
    }
    gas_in_batch
}
