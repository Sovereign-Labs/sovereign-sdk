#![allow(dead_code)]
mod operator;
mod optimistic;

use std::env;

use sov_bank::Bank;
use sov_mock_da::{MockAddress, MockBlob, MockDaSpec};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::{
    Amount, ApiStateAccessor, DaSpec, FullyBakedTx, Gas, GasArray, GasSpec, Rewards, Spec, TxEffect,
};
use sov_modules_stf_blueprint::{BatchReceipt, Runtime};
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::{AllowedSequencerError, SequencerRegistry};
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::runtime::{config_gas_token_id, Payable, TestRunner};

type S = sov_test_utils::TestSpec;

use sov_modules_api::transaction::{Transaction, TxDetails, UnsignedTransaction, VersionedTx};
use sov_modules_api::{PrivateKey, RawTx};
use sov_test_utils::{EncodeCall, TestUser, TEST_DEFAULT_MAX_FEE};
use sov_value_setter::ValueSetter;

fn get_balance(payable: impl Payable<S>, state: &mut ApiStateAccessor<S>) -> Amount {
    Bank::<S>::default()
        .get_balance_of(payable, config_gas_token_id(), state)
        .unwrap_infallible()
        .unwrap()
}

fn get_seq_bond(
    sequencer_da_address: &<<S as Spec>::Da as DaSpec>::Address,
    state: &mut ApiStateAccessor<S>,
) -> Result<Amount, AllowedSequencerError> {
    let sequencer_module = SequencerRegistry::<S>::default();
    sequencer_module
        .is_sender_allowed(sequencer_da_address, state)
        .map(|s| s.balance)
}

#[derive(PartialEq, Eq)]
pub(crate) enum TxStatus {
    Success,
    BadGeneration,
    BadChainId,
    BadSignature,
    BadSerialization,
    SignerDoesNotExist,
    OutOfGas,
    Reverted,
}

impl TxStatus {
    fn nb_of_valid_txs(statuses: &[TxStatus]) -> usize {
        statuses
            .iter()
            .filter(|s| s.is_valid())
            .collect::<Vec<_>>()
            .len()
    }

    fn nb_of_skipped_txs(statuses: &[TxStatus]) -> usize {
        statuses
            .iter()
            .filter(|s| !s.is_valid())
            .collect::<Vec<_>>()
            .len()
    }

    fn is_valid(&self) -> bool {
        // Reverted transactions pass the authentication process; therefore, we count them as valid.
        matches!(self, TxStatus::Success | TxStatus::Reverted)
    }
}

/// Builds a [`MockBlob`] from a [`Batch`] and a given address.
pub fn new_test_blob_from_batch(
    batch: Vec<FullyBakedTx>,
    address: &[u8],
) -> <MockDaSpec as DaSpec>::BlobTransaction {
    let address = MockAddress::try_from(address).unwrap();
    let data = borsh::to_vec(&batch).unwrap();
    MockBlob::new_with_hash(data, address)
}

/// Builds a new test blob for direct sequencer registration.
pub fn new_test_blob_for_direct_registration(
    tx: FullyBakedTx,
    address: &[u8],
    hash: [u8; 32],
) -> <MockDaSpec as DaSpec>::BlobTransaction {
    let batch = tx;
    let address = MockAddress::try_from(address).unwrap();
    let data = borsh::to_vec(&batch).unwrap();
    MockBlob::new(data, address, hash)
}

/// Checks if the given [`BatchReceipt`] contains any events.
pub fn has_tx_events<S: Spec>(apply_blob_outcome: &BatchReceipt<S>) -> bool {
    let events = apply_blob_outcome
        .tx_receipts
        .iter()
        .flat_map(|receipts| receipts.events.iter());

    events.peekable().peek().is_some()
}

fn default_rewards() -> Rewards {
    Rewards {
        accumulated_reward: Amount::ZERO,
        accumulated_penalty: Amount::ZERO,
    }
}

pub(crate) fn reset_constants() {
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_DEFAULT_GAS_TO_CHARGE_PER_BYTE_BORSH_DESERIALIZATION",
        "[1, 1]",
    );
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "10000000",
    );

    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "10000000",
    );

    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_UNREGISTERED_SEQUENCER_EXEC_GAS_PER_TX",
        "[10000000,10000000]",
    );
}

struct TxsCheckResult<S: Spec> {
    batch_receipt: BatchReceipt<S>,
    seq_fee: Amount,
    seq_penalty: Amount,
    gas_value_charged_to_user: Amount,
    total_gas: <S as GasSpec>::Gas,
}

fn do_check_txs<RT>(
    blobs: RelevantBlobs<MockBlob>,
    txs_len: usize,
    priority_fee_bips: PriorityFeeBips,
    runner: &mut TestRunner<RT, S>,
) -> TxsCheckResult<S>
where
    RT: Runtime<S> + MinimalGenesis<S>,
{
    let result = runner.execute::<RelevantBlobs<MockBlob>>(blobs);
    let batch_receipt = result.0.batch_receipts[0].clone();

    let gas_price = &batch_receipt.inner.gas_price;
    let tx_receipts = &batch_receipt.tx_receipts;
    let ignored_tx_receipts = &batch_receipt.ignored_tx_receipts;

    assert_eq!(tx_receipts.len() + ignored_tx_receipts.len(), txs_len);

    let mut seq_fee = Amount::ZERO;
    let mut seq_penalty = Amount::ZERO;
    let mut gas_value_charged_to_user = Amount::ZERO;

    let mut total_gas = <S as GasSpec>::Gas::ZEROED;

    for tx_receipt in tx_receipts {
        match &tx_receipt.receipt {
            TxEffect::Successful(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                let gas_value = tx_contents.gas_used.value(gas_price);
                gas_value_charged_to_user =
                    gas_value_charged_to_user.checked_add(gas_value).unwrap();
                seq_fee = seq_fee
                    .checked_add(priority_fee_bips.apply(gas_value).unwrap())
                    .unwrap();
            }
            TxEffect::Skipped(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                let gas_value = tx_contents.gas_used.value(gas_price);
                // Sequencer doesn't get the fee and is penalized
                seq_penalty = seq_penalty.checked_add(gas_value).unwrap();
            }
            TxEffect::Reverted(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                // From gas usage point of view the `Successful & Reverted` cases are the same.
                let gas_value = tx_contents.gas_used.value(gas_price);
                gas_value_charged_to_user =
                    gas_value_charged_to_user.checked_add(gas_value).unwrap();
                seq_fee = seq_fee
                    .checked_add(priority_fee_bips.apply(gas_value).unwrap())
                    .unwrap();
            }
        }
    }

    for ignored_tx_receipt in ignored_tx_receipts {
        let ignored = &ignored_tx_receipt.ignored;
        let gas_used = &ignored.gas_used;
        total_gas = total_gas.checked_combine(gas_used).unwrap();
        let gas_value = gas_used.value(gas_price);
        seq_penalty = seq_penalty.checked_add(gas_value).unwrap();
    }

    TxsCheckResult {
        batch_receipt,
        seq_fee,
        seq_penalty,
        gas_value_charged_to_user,
        total_gas,
    }
}

pub fn create_blob<RT: Runtime<S> + EncodeCall<ValueSetter<S>>>(
    statuses: &[TxStatus],
    max_priority_fee_bips: PriorityFeeBips,
    admin: &TestUser<S>,
    not_admin: &TestUser<S>,
    seq_da_address: <<S as Spec>::Da as DaSpec>::Address,
) -> MockBlob {
    let txs: Vec<FullyBakedTx> =
        create_txs::<RT>(statuses, max_priority_fee_bips, admin, not_admin);

    let blob = borsh::to_vec(&txs).unwrap();
    MockBlob::new_with_hash(blob, seq_da_address)
}

pub fn create_tx_bad_sig<RT: Runtime<S>>(
    nonce: u64,
    max_priority_fee_bips: PriorityFeeBips,
    signer: &TestUser<S>,
    chain_id: u64,
    message: RT::Decodable,
) -> Transaction<RT, S> {
    let utx = UnsignedTransaction::<RT, S>::new(
        message,
        chain_id,
        max_priority_fee_bips,
        TEST_DEFAULT_MAX_FEE,
        nonce,
        None,
    );

    let signed_tx = Transaction::new_signed_tx(&signer.private_key, &RT::CHAIN_HASH, utx);

    // Create a signature for a different message so it won't verify in the stf.
    let bad_signature = signer.private_key.sign(&[1, 2, 3]);

    match signed_tx.versioned_tx {
        VersionedTx::V0(inner) => Transaction::new_with_details_v0(
            inner.pub_key.clone(),
            inner.runtime_call.clone(),
            bad_signature,
            inner.generation,
            TxDetails {
                max_priority_fee_bips,
                max_fee: Amount::new(200_000),
                gas_limit: None,
                chain_id,
            },
        ),
    }
}

pub fn create_tx_bad_sender<RT: Runtime<S>>(
    nonce: u64,
    max_priority_fee_bips: PriorityFeeBips,
    chain_id: u64,
    message: RT::Decodable,
    chain_hash: &[u8; 32],
) -> Transaction<RT, S> {
    let utx = UnsignedTransaction::new(
        message,
        chain_id,
        max_priority_fee_bips,
        Amount::new(200_000),
        nonce,
        None,
    );

    let signer = TestUser::<S>::generate(Amount::ZERO);
    Transaction::<RT, S>::new_signed_tx(signer.private_key(), chain_hash, utx)
}

pub fn create_tx_valid<RT: Runtime<S>>(
    nonce: u64,
    max_priority_fee_bips: PriorityFeeBips,
    signer: &TestUser<S>,
    chain_id: u64,
    message: RT::Decodable,
) -> Transaction<RT, S> {
    let utx = UnsignedTransaction::new(
        message,
        chain_id,
        max_priority_fee_bips,
        TEST_DEFAULT_MAX_FEE,
        nonce,
        None,
    );

    Transaction::<RT, S>::new_signed_tx(signer.private_key(), &RT::CHAIN_HASH, utx)
}

// Transaction with zero gas limit.
pub fn create_tx_out_of_gas<RT: Runtime<S>>(
    nonce: u64,
    max_priority_fee_bips: PriorityFeeBips,
    signer: &TestUser<S>,
    chain_id: u64,
    message: RT::Decodable,
) -> Transaction<RT, S> {
    let utx = UnsignedTransaction::new(
        message,
        chain_id,
        max_priority_fee_bips,
        Amount::new(200_000),
        nonce,
        Some(<<S as Spec>::Gas as Gas>::zero()),
    );

    Transaction::<RT, S>::new_signed_tx(signer.private_key(), &RT::CHAIN_HASH, utx)
}

use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::macros::config_value;
use sov_modules_api::DispatchCall;
use sov_value_setter::CallMessage;

pub fn create_txs<RT: Runtime<S> + EncodeCall<ValueSetter<S>>>(
    statuses: &[TxStatus],
    max_priority_fee_bips: PriorityFeeBips,
    admin: &TestUser<S>,
    not_admin: &TestUser<S>,
) -> Vec<FullyBakedTx> {
    let mut generation = 10;
    let mut txs: Vec<FullyBakedTx> = Vec::new();
    for status in statuses {
        match status {
            TxStatus::Success => {
                let tx = create_tx_valid::<RT>(
                    generation,
                    max_priority_fee_bips,
                    admin,
                    config_value!("CHAIN_ID"),
                    encode_message::<RT>(None),
                );
                txs.push(encode(tx));
                generation += 1;
            }
            TxStatus::BadGeneration => {
                if generation == 10 {
                    panic!("The first transaction will always have a valid generation");
                } else {
                    let tx = create_tx_valid::<RT>(
                        0,
                        max_priority_fee_bips,
                        admin,
                        config_value!("CHAIN_ID"),
                        encode_message::<RT>(None),
                    );
                    txs.push(encode(tx));
                }
            }
            TxStatus::BadChainId => {
                let tx = create_tx_valid::<RT>(
                    generation,
                    max_priority_fee_bips,
                    admin,
                    config_value!("CHAIN_ID") + 1,
                    encode_message::<RT>(None),
                );
                txs.push(encode(tx));
            }

            TxStatus::BadSignature => {
                let tx = create_tx_bad_sig::<RT>(
                    generation,
                    max_priority_fee_bips,
                    admin,
                    config_value!("CHAIN_ID"),
                    encode_message::<RT>(None),
                );
                txs.push(encode(tx));
            }
            TxStatus::OutOfGas => {
                let tx = create_tx_out_of_gas::<RT>(
                    generation,
                    max_priority_fee_bips,
                    admin,
                    config_value!("CHAIN_ID"),
                    encode_message::<RT>(None),
                );
                txs.push(encode(tx));
            }
            TxStatus::Reverted => {
                // A call message send by not admin will be reverted.
                let tx = create_tx_valid::<RT>(
                    0,
                    max_priority_fee_bips,
                    not_admin,
                    config_value!("CHAIN_ID"),
                    encode_message::<RT>(None),
                );
                txs.push(encode(tx));
            }
            TxStatus::BadSerialization => {
                let tx = FullyBakedTx::new(vec![1, 2, 3]);
                txs.push(tx);
            }
            TxStatus::SignerDoesNotExist => {
                let tx = create_tx_bad_sender::<RT>(
                    0,
                    max_priority_fee_bips,
                    config_value!("CHAIN_ID"),
                    encode_message::<RT>(None),
                    &RT::CHAIN_HASH,
                );
                txs.push(encode(tx));
            }
        }
    }
    txs
}

pub fn encode_message<RT: Runtime<S> + EncodeCall<ValueSetter<S>>>(
    gas: Option<<S as Spec>::Gas>,
) -> <RT as DispatchCall>::Decodable {
    <RT as EncodeCall<ValueSetter<S>>>::to_decodable(CallMessage::SetValue { value: 8, gas })
}

pub fn encode<RT: Runtime<S>>(tx: Transaction<RT, S>) -> FullyBakedTx {
    <RT as Runtime<S>>::Auth::encode_with_standard_auth(RawTx::new(borsh::to_vec(&tx).unwrap()))
}
