use sov_bank::{config_gas_token_id, Amount, Bank};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{BlobReaderTrait, Gas, GasArray, GasSpec, Rewards};
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TestAddress, TestUser, TEST_DEFAULT_USER_BALANCE};

use crate::stf_blueprint::operator::operator_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{
    create_blob, do_check_txs, get_seq_bond, PriorityFeeBips, TxStatus, TxsCheckResult, S,
};

fn check_txs(tx_statuses: Vec<TxStatus>) {
    let priority_fee_bips = PriorityFeeBips::from_percentage(0);
    let reward_user = TestUser::<S>::generate(TEST_DEFAULT_USER_BALANCE);
    let reward_address = reward_user.address();
    let (mut runner, users, sequencer_account) = setup(reward_user, 2);

    let admin_account = &users[0];
    let not_admin_account = &users[1];

    let seq_bond_start = runner
        .query_visible_state(|state| get_seq_bond(&sequencer_account.da_address, state))
        .unwrap();

    let start_reward_address_balance = get_balance(reward_address, &runner).unwrap();

    let mock_blob = create_blob::<IntegTestRuntime<S>>(
        &tx_statuses,
        priority_fee_bips,
        admin_account,
        not_admin_account,
        runner.config.sequencer_da_address,
    );

    let seq_burn_gas = <S as GasSpec>::gas_to_charge_per_byte_borsh_deserialization()
        .checked_scalar_product(mock_blob.total_len() as u64)
        .unwrap();

    let blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![mock_blob],
    };

    let txs_len = tx_statuses.len();

    let TxsCheckResult {
        seq_fee,
        seq_penalty,
        gas_value_charged_to_user,
        batch_receipt,
        total_gas,
    } = do_check_txs(blobs, txs_len, priority_fee_bips, &mut runner);

    let gas_price = &batch_receipt.inner.gas_price;
    let seq_bond_end = runner
        .query_visible_state(|state| get_seq_bond(&sequencer_account.da_address, state))
        .unwrap();

    let seq_burn = seq_burn_gas.checked_value(gas_price).unwrap();

    assert_eq!(
        seq_bond_end
            .checked_add(seq_penalty)
            .unwrap()
            .checked_add(seq_burn)
            .unwrap(),
        seq_bond_start
    );

    // Seq fee is 0 because we set PriorityFeeBips = 0.
    assert_eq!(seq_fee, Amount::ZERO);

    // In the operator mode only the pre-set address gets all the rewards.
    let end_reward_address_balance = get_balance(reward_address, &runner).unwrap();
    assert!(end_reward_address_balance > start_reward_address_balance);
    assert_eq!(
        end_reward_address_balance,
        gas_value_charged_to_user
            .checked_add(seq_penalty)
            .unwrap()
            .checked_add(start_reward_address_balance)
            .unwrap()
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

#[test]
fn execute_many_successful_tx_test_operator() {
    let tx_statuses = vec![
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
        TxStatus::Success,
    ];
    check_txs(tx_statuses);
}

// Execute a batch of mixed transactions and ensure that the relevant balances were updated correctly
#[test]
fn execute_batch_of_valid_and_invalid_tx_test_operator() {
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
    check_txs(tx_statuses);
}

#[test]
fn execute_batch_of_invalid_tx_test_operator() {
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
    check_txs(tx_statuses);
}

fn get_balance(
    reward_address: TestAddress,
    runner: &TestRunner<IntegTestRuntime<S>, S>,
) -> Option<Amount> {
    runner.query_visible_state(|state| {
        Bank::<S>::default()
            .get_balance_of(&reward_address, config_gas_token_id(), state)
            .unwrap_infallible()
    })
}
