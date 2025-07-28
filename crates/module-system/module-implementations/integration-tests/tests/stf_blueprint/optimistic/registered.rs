use std::env;

use sov_mock_da::MockBlob;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{
    BlobReaderTrait, Gas, GasArray, GasSpec, GasUnit, Rewards, Spec, TransactionReceipt, TxEffect,
};
use sov_rollup_interface::da::RelevantBlobs;

use super::optimistic_rt::helpers::*;
use crate::stf_blueprint::optimistic::optimistic_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{
    create_blob, create_tx_valid, create_txs, do_check_txs, encode, encode_message, TxStatus,
    TxsCheckResult, S,
};

fn check_txs(tx_statuses: Vec<TxStatus>, priority_fee_bips: PriorityFeeBips) {
    let (mut runner, users, sequencer_account) = setup(2);

    let actors = Actors {
        admin_account: users[0].clone(),
        not_admin_account: users[1].clone(),
        sequencer_account,
    };

    let start = runner.query_visible_state(|state| actors.balances(state));

    let txs_len = tx_statuses.len();

    let mock_blob = create_blob::<IntegTestRuntime<S>>(
        &tx_statuses,
        priority_fee_bips,
        &actors.admin_account,
        &actors.not_admin_account,
        runner.config.sequencer_da_address,
    );
    // The gas amount burned by the sequencer to submit the blob. See #2491
    let seq_burn_gas = <S as GasSpec>::gas_to_charge_per_byte_borsh_deserialization()
        .checked_scalar_product(mock_blob.total_len() as u64)
        .unwrap();

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    let TxsCheckResult {
        batch_receipt,
        seq_fee,
        seq_penalty,
        gas_value_charged_to_user,
        total_gas,
    } = do_check_txs(blobs, txs_len, priority_fee_bips, &mut runner);

    let gas_price = &batch_receipt.inner.gas_price;
    let seq_burn = seq_burn_gas.checked_value(gas_price).unwrap();
    let end = runner.query_state(|state| actors.balances(state));

    // Check user balances.
    assert_eq!(
        end.admin_balance
            .checked_add(end.not_admin_balance)
            .unwrap(),
        start
            .admin_balance
            .checked_add(start.not_admin_balance)
            .unwrap()
            .checked_sub(seq_fee)
            .unwrap()
            .checked_sub(gas_value_charged_to_user)
            .unwrap()
    );

    // Check sequencer rewards.
    assert_eq!(
        end.sequencer_bond,
        start
            .sequencer_bond
            .checked_add(seq_fee)
            .unwrap()
            .checked_sub(seq_penalty)
            .unwrap()
            .checked_sub(seq_burn)
            .unwrap()
    );

    // Check prover rewards.
    assert_eq!(
        end.attester_module_balance,
        start
            .attester_module_balance
            .checked_add(gas_value_charged_to_user)
            .unwrap()
            .checked_add(seq_penalty)
            .unwrap()
    );

    // This has already been tested by previous assertions, but here we explicitly clarify that no money is created or lost.
    // except for the gas burned by the sequencer to submit the blob.
    assert_eq!(
        end.total_balance(),
        start.total_balance().saturating_sub(seq_burn)
    );

    assert_eq!(
        batch_receipt.inner.outcome,
        sov_modules_api::BatchSequencerOutcome {
            rewards: Rewards {
                accumulated_reward: seq_fee,
                accumulated_penalty: seq_penalty,
            }
        }
    );

    assert_eq!(batch_receipt.inner.gas_used, total_gas);
}

// Execute a batch of valid transactions and ensure that the relevant balances ware updated correctly
#[test]
fn execute_many_successful_tx_test() {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
    ];
    check_txs(tx_statuses, priority_fee_bips);
}

// Execute a batch of mixed transactions and ensure that the relevant balances were updated correctly
#[test]
fn execute_batch_of_valid_and_invalid_tx_test() {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![
        TxStatus::BadSerialization,
        TxStatus::SignerDoesNotExist,
        TxStatus::Success,
        TxStatus::BadSignature,
        TxStatus::Success,
        TxStatus::BadChainId,
        TxStatus::BadGeneration,
        TxStatus::Success,
        TxStatus::Reverted,
    ];
    check_txs(tx_statuses, priority_fee_bips);
}

// Execute a batch of invalid transactions and ensure that the relevant balances ware updated correctly
#[test]
fn execute_batch_of_invalid_tx_test() {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    // BadGeneration is only possible if an account already had at least one valid tx, so we cannot
    // test it here
    let tx_statuses = vec![
        TxStatus::OutOfGas,
        TxStatus::BadChainId,
        TxStatus::BadChainId,
        TxStatus::BadSignature,
        TxStatus::SignerDoesNotExist,
        TxStatus::BadChainId,
        TxStatus::OutOfGas,
        TxStatus::BadSignature,
    ];
    check_txs(tx_statuses, priority_fee_bips);
}

// The batch from an unregistered sequencer is ignored, and no batch receipt is returned.
#[test]
fn non_existing_seq_da_tests() {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![TxStatus::Success];

    let (mut runner, users, sequencer_account) = setup(2);

    let actors = Actors {
        admin_account: users[0].clone(),
        not_admin_account: users[1].clone(),
        sequencer_account,
    };

    let bad_da_address: [u8; 32] = [33u8; 32];

    let mock_blob = create_blob::<IntegTestRuntime<S>>(
        &tx_statuses,
        priority_fee_bips,
        &actors.admin_account,
        &actors.not_admin_account,
        bad_da_address.into(),
    );

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    assert!(result.0.batch_receipts.is_empty());
}

#[test]
fn sequencer_run_out_of_gas() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_DEFAULT_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION",
        "[100000, 100000]",
    );

    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![TxStatus::Success];
    check_txs(tx_statuses, priority_fee_bips);
}

// If the slot runs out of gas during transaction execution, the transaction is reverted.
#[test]
fn slot_out_of_gas_tests() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_INITIAL_GAS_LIMIT",
        "[10000000000, 10000000000]",
    );
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);

    let (mut runner, users, sequencer_account) = setup(2);

    let actors = Actors {
        admin_account: users[0].clone(),
        not_admin_account: users[1].clone(),
        sequencer_account,
    };

    // The transaction uses more gas than the slot gas limit.
    let gas = GasUnit::from([10000000001, 2]);
    let tx = create_tx_valid::<IntegTestRuntime<S>>(
        10,
        priority_fee_bips,
        &actors.admin_account,
        config_value!("CHAIN_ID"),
        encode_message::<IntegTestRuntime<S>>(Some(gas)),
    );

    let blob = borsh::to_vec(&vec![encode(tx)]).unwrap();
    let mock_blob = MockBlob::new_with_hash(blob, runner.config.sequencer_da_address);

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    let tx_receipt = &result.0.batch_receipts[0].tx_receipts[0].receipt;

    assert!(matches!(tx_receipt, TxEffect::Reverted(_)));
}

// This test verifies that the gas used for executing a batch matches the total gas consumed for executing all transactions within the batch.
#[test]
fn test_batch_gas_used() {
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let (mut runner, users, sequencer_account) = setup(2);

    let actors = Actors {
        admin_account: users[0].clone(),
        not_admin_account: users[1].clone(),
        sequencer_account,
    };

    let mut txs = create_txs::<IntegTestRuntime<S>>(
        &[
            TxStatus::Success,
            TxStatus::Success,
            TxStatus::Success,
            TxStatus::Success,
        ],
        priority_fee_bips,
        &actors.admin_account,
        &actors.not_admin_account,
    );

    let seq_da_address = runner.config.sequencer_da_address;

    // Create two batches with two transactions each.
    let batch_blobs = vec![
        MockBlob::new_with_hash(
            borsh::to_vec(&vec![txs.remove(0), txs.remove(0)]).unwrap(),
            seq_da_address,
        ),
        MockBlob::new_with_hash(
            borsh::to_vec(&vec![txs.remove(0), txs.remove(0)]).unwrap(),
            seq_da_address,
        ),
    ];

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs,
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    let batch_receipt_1 = result.0.batch_receipts[0].clone();
    let batch_receipt_2 = result.0.batch_receipts[1].clone();

    let gas_used = get_gas_from_txs(&batch_receipt_1.tx_receipts);
    assert_eq!(batch_receipt_1.inner.gas_used, gas_used);

    let gas_used = get_gas_from_txs(&batch_receipt_2.tx_receipts);
    assert_eq!(batch_receipt_2.inner.gas_used, gas_used);

    fn get_gas_from_txs(receipts: &[TransactionReceipt<S>]) -> <S as Spec>::Gas {
        let mut gas_in_batch = <<S as Spec>::Gas>::zero();
        for receipt in receipts {
            match &receipt.receipt {
                sov_modules_api::TxEffect::Successful(tx_contents) => {
                    gas_in_batch = gas_in_batch.checked_combine(&tx_contents.gas_used).unwrap();
                }
                _ => panic!("Transactions should succeed"),
            }
        }
        gas_in_batch
    }
}
