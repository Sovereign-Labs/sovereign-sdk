use std::env;

use helpers::*;
use sov_attester_incentives::AttesterIncentives;
use sov_bank::IntoPayable;
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::{PriorityFeeBips, Transaction, UnsignedTransaction};
use sov_modules_api::{
    Amount, ApiStateAccessor, DaSpec, FullyBakedTx, Gas, GasArray, ModuleInfo, RawTx, Rewards,
    Spec, TxEffect,
};
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;
use sov_test_utils::{EncodeCall, TestUser};

use super::optimistic_rt::setup;
use crate::stf_blueprint::{get_balance, get_seq_bond, reset_constants, TxStatus, S};

const BOND_AMOUNT: Amount = Amount::new(100);

fn check_unreg_txs(tx_statuses: Vec<TxStatus>, priority_fee_bips: PriorityFeeBips) {
    let (mut runner, users, _) = setup(tx_statuses.len());

    let nb_of_valid_txs = TxStatus::nb_of_valid_txs(&tx_statuses);
    let nb_of_skipped_txs = TxStatus::nb_of_skipped_txs(&tx_statuses);

    // Every potential sequencer gets a unique DA address.
    let mut potential_sequencers_with_statuses = Vec::new();
    for (i, status) in tx_statuses.into_iter().enumerate() {
        let da_address = MockAddress::new([i as u8; 32]);
        let potential_seq = PotentialSequencer {
            user: users[i].clone(),
            da_address,
        };

        potential_sequencers_with_statuses.push((status, potential_seq));
    }

    let blobs_with_pot_sequencers =
        create_blobs_from_unregistered_seq(priority_fee_bips, potential_sequencers_with_statuses);

    let mut valid_tx_count = 0;
    let mut skipped_tx_count = 0;

    for (blob, potential_seq) in blobs_with_pot_sequencers {
        let start = runner.query_visible_state(|state| potential_seq.balances(state));

        let unregistered_blobs = RelevantBlobs {
            proof_blobs: Default::default(),
            batch_blobs: vec![blob],
        };

        let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);

        let batch_receipt = &result.0.batch_receipts[0];
        let gas_price = &batch_receipt.inner.gas_price;

        let tx_receipt = &batch_receipt.tx_receipts[0];

        let gas_value_charged_to_user;
        let seq_fee;
        let bond_amount;
        let mut total_gas = <<S as Spec>::Gas>::zero();

        match &tx_receipt.receipt {
            TxEffect::Successful(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                let gas_value = tx_contents.gas_used.value(gas_price);
                gas_value_charged_to_user = gas_value;
                seq_fee = priority_fee_bips.apply(gas_value).unwrap();
                bond_amount = BOND_AMOUNT;
                valid_tx_count += 1;
            }
            TxEffect::Skipped(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                // The sequencer is not bonded so we can't penalize them for skipped transactions.
                // In this case no one is charged for the failed transaction.
                gas_value_charged_to_user = Amount::ZERO;
                seq_fee = Amount::ZERO;
                bond_amount = Amount::ZERO;
                skipped_tx_count += 1;
            }
            TxEffect::Reverted(tx_contents) => {
                total_gas = total_gas.checked_combine(&tx_contents.gas_used).unwrap();
                let gas_value = tx_contents.gas_used.value(gas_price);
                gas_value_charged_to_user = gas_value;
                seq_fee = Amount::ZERO;
                bond_amount = Amount::ZERO;
                valid_tx_count += 1;
            }
        }

        let end = runner.query_visible_state(|state| potential_seq.balances(state));

        // Sequencer fees are transferred to the bond in the sequencer registry.
        assert_eq!(
            end.potential_seq_bond,
            seq_fee.checked_add(bond_amount).unwrap()
        );
        // The `seq_fee`` is redundant here, but I am leaving it as documentation to explain what is happening.
        // The user acts as a sequencer, transferring the fee from their bank balance to the bond in the sequencer registry.
        assert_eq!(
            end.potential_seq_bank_balance
                .checked_add(end.potential_seq_bond)
                .unwrap()
                .checked_sub(seq_fee)
                .unwrap(),
            start
                .potential_seq_bank_balance
                .checked_sub(gas_value_charged_to_user)
                .unwrap()
                .checked_sub(seq_fee)
                .unwrap()
        );

        assert_eq!(
            end.attester_module_balance,
            start
                .attester_module_balance
                .checked_add(gas_value_charged_to_user)
                .unwrap()
        );

        assert_eq!(end.total_balance(), start.total_balance());

        assert_eq!(
            batch_receipt.inner.outcome,
            sov_modules_api::BatchSequencerOutcome {
                rewards: Rewards {
                    accumulated_reward: seq_fee,
                    accumulated_penalty: Amount::ZERO,
                }
            }
        );

        assert_eq!(batch_receipt.inner.gas_used, total_gas);
        // Ensure that a transaction, including a failed one, still incurs gas costs.
        assert!(<<S as Spec>::Gas>::zero().dim_is_less_than(&total_gas));
    }

    assert_eq!(nb_of_valid_txs, valid_tx_count);
    assert_eq!(nb_of_skipped_txs, skipped_tx_count);
}

// Execute batch of valid registrations.
#[test]
fn execute_seq_registration_success_test() {
    reset_constants();
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![TxStatus::Success, TxStatus::Success];
    check_unreg_txs(tx_statuses, priority_fee_bips);
}

// Execute batch of invalid registrations.
#[test]
fn execute_seq_registration_failure_test() {
    reset_constants();
    let priority_fee_bips = PriorityFeeBips::from_percentage(5);
    let tx_statuses = vec![
        TxStatus::OutOfGas,
        TxStatus::BadSerialization,
        TxStatus::SignerDoesNotExist,
        TxStatus::BadSignature,
        TxStatus::BadSignature,
        TxStatus::BadSerialization,
        TxStatus::BadChainId,
        TxStatus::Reverted,
    ];
    check_unreg_txs(tx_statuses, priority_fee_bips);
}

// Execute a blob that is too expensive to deserialize.
#[test]
fn blob_too_expensive_tests() {
    reset_constants();
    // Set the max amount of gas to be spent on a single blob to a very small value
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_UNREGISTERED_SEQUENCER_EXEC_GAS_PER_TX",
        "[1,1]",
    );

    let (mut runner, _, _) = setup(1);

    let blob = make_blob(FullyBakedTx::new(vec![1, 2, 3]), MockAddress::new([22; 32]));
    let unregistered_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);
    let batch_receipt = &result.0.batch_receipts[0];

    assert!(batch_receipt.tx_receipts.is_empty());
}

// Execute a blob that is too big to be processed.
#[test]
fn blob_test_max_slot_size() {
    reset_constants();
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "1",
    );

    let (mut runner, _, _) = setup(1);

    let blob = make_blob(FullyBakedTx::new(vec![1, 2, 3]), MockAddress::new([22; 32]));
    let unregistered_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);
    // The blob was too big to be deserialized, so it should be rejected.
    assert!(result.0.batch_receipts.is_empty());
}

// Execute a blob that is too big to be returned by the blob storage.
#[test]
fn blob_test_max_allowed_data_size() {
    reset_constants();
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
        "1",
    );

    let (mut runner, _, _) = setup(1);

    let blob = make_blob(FullyBakedTx::new(vec![1, 2, 3]), MockAddress::new([22; 32]));
    let unregistered_blobs = RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![blob],
    };

    let result = runner.execute::<RelevantBlobs<MockBlob>>(unregistered_blobs);
    // The blob was too big to be deserialized, so it should be rejected.
    assert!(result.0.batch_receipts.is_empty());
}

mod helpers {
    use sov_modules_api::FullyBakedTx;
    use sov_modules_stf_blueprint::Runtime;
    use sov_test_utils::TEST_DEFAULT_MAX_FEE;

    use super::*;
    use crate::stf_blueprint::optimistic::optimistic_rt::{IntegTestRuntime, IntegTestRuntimeCall};
    use crate::stf_blueprint::{create_tx_bad_sig, create_tx_out_of_gas, create_tx_valid};

    // A user that is not a registered sequencer and attempts to register itself as one.
    pub(crate) struct PotentialSequencer {
        pub(crate) user: TestUser<S>,
        pub(crate) da_address: MockAddress,
    }

    impl PotentialSequencer {
        pub(crate) fn balances(&self, state: &mut ApiStateAccessor<S>) -> Balances {
            let attester_module = AttesterIncentives::<S>::default();
            Balances {
                potential_seq_bank_balance: get_balance(&self.user.address(), state),
                attester_module_balance: get_balance(attester_module.id().to_payable(), state),
                potential_seq_bond: get_seq_bond(&self.da_address, state).unwrap_or(Amount::ZERO),
            }
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    pub(crate) struct Balances {
        pub(crate) potential_seq_bank_balance: Amount,
        pub(crate) potential_seq_bond: Amount,
        pub(crate) attester_module_balance: Amount,
    }

    impl Balances {
        pub(crate) fn total_balance(&self) -> Amount {
            self.potential_seq_bank_balance
                .checked_add(self.potential_seq_bond)
                .unwrap()
                .checked_add(self.attester_module_balance)
                .unwrap()
        }
    }

    fn create_tx_bad_sender(
        nonce: u64,
        max_priority_fee_bips: PriorityFeeBips,
        chain_id: u64,
        message: IntegTestRuntimeCall<S>,
    ) -> Transaction<IntegTestRuntime<S>, S> {
        let utx = UnsignedTransaction::new(
            message,
            chain_id,
            max_priority_fee_bips,
            Amount::new(200_000),
            nonce,
            None,
        );

        let signer = TestUser::<S>::generate(Amount::ZERO);
        Transaction::<IntegTestRuntime<S>, S>::new_signed_tx(
            signer.private_key(),
            &IntegTestRuntime::<S>::CHAIN_HASH,
            utx,
        )
    }

    // Creates a forced-registration blob to be sent to the sequencer, the transaction will be reverted.
    fn create_tx_reverted(
        nonce: u64,
        max_priority_fee_bips: PriorityFeeBips,
        signer: &TestUser<S>,
        da_address: <<S as Spec>::Da as DaSpec>::Address,
        chain_id: u64,
    ) -> Transaction<IntegTestRuntime<S>, S> {
        // Here, we attempt to bond more funds than are available for a given user, causing the transaction to be reverted.
        let encoded_message = encode_message(
            da_address,
            signer
                .available_gas_balance
                .checked_add(Amount::new(1))
                .unwrap(),
        );

        let utx = UnsignedTransaction::new(
            encoded_message,
            chain_id,
            max_priority_fee_bips,
            TEST_DEFAULT_MAX_FEE,
            nonce,
            None,
        );

        Transaction::<IntegTestRuntime<S>, S>::new_signed_tx(
            signer.private_key(),
            &IntegTestRuntime::<S>::CHAIN_HASH,
            utx,
        )
    }

    pub(crate) fn create_blobs_from_unregistered_seq(
        max_priority_fee_bips: PriorityFeeBips,
        potential_seqs_with_statuses: Vec<(TxStatus, PotentialSequencer)>,
    ) -> Vec<(MockBlob, PotentialSequencer)> {
        let mut blobs = Vec::new();

        for (status, pot_seq) in potential_seqs_with_statuses.into_iter() {
            let blob = create_blob(&status, max_priority_fee_bips, &pot_seq);
            blobs.push((blob, pot_seq));
        }

        blobs
    }

    pub(crate) fn create_blob(
        status: &TxStatus,
        max_priority_fee_bips: PriorityFeeBips,
        potential_seq: &PotentialSequencer,
    ) -> MockBlob {
        let tx = match status {
            TxStatus::Success => encode_tx(create_tx_valid::<IntegTestRuntime<S>>(
                0,
                max_priority_fee_bips,
                &potential_seq.user,
                config_value!("CHAIN_ID"),
                encode_message(potential_seq.da_address, BOND_AMOUNT),
            )),
            TxStatus::BadGeneration => panic!("Unregistered blobs send one transaction per user, any generation number is valid for a user's first transaction"),
            TxStatus::BadChainId => encode_tx(create_tx_valid::<IntegTestRuntime<S>>(
                0,
                max_priority_fee_bips,
                &potential_seq.user,
                config_value!("CHAIN_ID") + 1,
                encode_message(potential_seq.da_address, BOND_AMOUNT),
            )),
            TxStatus::BadSignature => encode_tx(create_tx_bad_sig(
                0,
                max_priority_fee_bips,
                &potential_seq.user,
                config_value!("CHAIN_ID"),
                encode_message(potential_seq.da_address, BOND_AMOUNT),
            )),
            TxStatus::BadSerialization => <IntegTestRuntime<S> as Runtime<S>>::Auth::encode_with_standard_auth(RawTx {
                data: vec![1, 2, 3],
            }),
            TxStatus::OutOfGas => encode_tx(create_tx_out_of_gas::<IntegTestRuntime<S>>(
                0,
                max_priority_fee_bips,
                &potential_seq.user,
                config_value!("CHAIN_ID"),
                encode_message(potential_seq.da_address, BOND_AMOUNT),
            )),
            TxStatus::Reverted => encode_tx(create_tx_reverted(
                0,
                max_priority_fee_bips,
                &potential_seq.user,
                potential_seq.da_address,
                config_value!("CHAIN_ID"),
            )),
            TxStatus::SignerDoesNotExist => encode_tx(create_tx_bad_sender(
                0,
                max_priority_fee_bips,
                config_value!("CHAIN_ID"),
                encode_message(potential_seq.da_address, BOND_AMOUNT),
            )),
        };

        make_blob(tx, potential_seq.da_address)
    }

    fn encode_message(
        da_address: <<S as Spec>::Da as DaSpec>::Address,
        bond_amount: Amount,
    ) -> IntegTestRuntimeCall<S> {
        <IntegTestRuntime<S> as EncodeCall<SequencerRegistry<S>>>::to_decodable(
            sov_sequencer_registry::CallMessage::Register {
                da_address,
                amount: bond_amount,
            },
        )
    }

    fn encode_tx(tx: Transaction<IntegTestRuntime<S>, S>) -> FullyBakedTx {
        let tx_data = borsh::to_vec(&tx).unwrap();
        <IntegTestRuntime<S> as Runtime<S>>::Auth::encode_with_standard_auth(RawTx {
            data: tx_data,
        })
    }

    pub(crate) fn make_blob(
        raw_tx: FullyBakedTx,
        da_address: <<S as Spec>::Da as DaSpec>::Address,
    ) -> MockBlob {
        let tx_blob = borsh::to_vec(&raw_tx).unwrap();
        MockBlob::new_with_hash(tx_blob, da_address)
    }
}
