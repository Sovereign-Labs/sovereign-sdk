//! Property test: random interleavings of preferred, registered non-preferred, garbage,
//! unregistered and emergency-registration blobs (batches and proofs) must never panic the STF,
//! must never change the user state root in a slot that creates no rollup block, and must account
//! for every token of deferred-blob escrow. For every rollup block built from a preferred batch,
//! the preferred sequencer's block builder must also select the same non-preferred blobs, at the
//! same gas price, as the node.

use std::collections::{HashMap, HashSet};
use std::num::NonZero;
use std::sync::Arc;

use proptest::prelude::*;
use sov_bank::derived_holder::DerivedHolder;
use sov_bank::{config_gas_token_id, Bank, IntoPayable};
use sov_blob_storage::{PreferredBatchData, PreferredBlobData, PreferredProofData};
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::capabilities::{BlobSelector, ChainState, HasKernel};
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    Amount, BlobDataWithId, BlobReaderTrait, CryptoSpec, Gas, GasArray, KernelStateAccessor,
    ModuleInfo, NoOpControlFlow, PrivateKey, RawTx, Spec, StateCheckpoint, VersionReader,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::{CallMessage, SequencerRegistry};
use sov_state::{ProvableNamespace, StateRoot};
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::{BlobStorage, Uniqueness};
use sov_test_utils::storage::ForklessStorageManager;
use sov_test_utils::{AsUser, TestUser, TransactionType, TEST_MIN_SEQ_BOND};
use sov_value_setter::ValueSetterConfig;

use crate::helpers_soft_confirmations::{GenesisConfig, SoftConfRT};
use crate::{TestRunner, S};

type Nonces = HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>;
type GasPrice = <<S as Spec>::Gas as Gas>::Price;

/// What the preferred sequencer's block builder selects when it builds a rollup block that
/// advances the visible slot to a given slot.
struct SequencerBlock {
    /// Non-preferred batches (including emergency registrations), as (blob id, sender).
    batches: Vec<([u8; 32], MockAddress)>,
    /// Non-preferred proof blob ids.
    proofs: Vec<[u8; 32]>,
    /// The gas reserved for each registered batch and proof, by blob id.
    reserved_gas: Vec<([u8; 32], Amount)>,
    gas_price: GasPrice,
}

/// A small data limit, so that blob sizes in the generated range can exceed it and exercise the
/// `OutOfCapacity` paths. Non-preferred blobs get 10% of it (`PREFERRED_DATA_FRACTION`), which must
/// still fit unregistered blobs larger than `MAX_EMERGENCY_REGISTRATION_BLOB_SIZE`.
const MAX_BLOB_STORAGE_DATA_SIZE: usize = 20_000;
/// Blobs from unregistered senders larger than this are discarded without being deserialized.
/// Mirrors `MAX_EMERGENCY_REGISTRATION_BLOB_SIZE` in `sov-blob-storage`.
const MAX_EMERGENCY_REGISTRATION_BLOB_SIZE: usize = 1000;
const NUM_EMERGENCY_REGISTRANTS: usize = 4;
const UNREGISTERED_SENDER: MockAddress = MockAddress::new([7; 32]);
/// Upper bound on the slots needed to catch the visible slot up to the true slot after the
/// generated slots.
const MAX_DRAIN_SLOTS: usize = 50;
/// A short deferral window (the default is 120 slots), so that runs reach forced visible slot
/// advancement and withdrawals become ready within a few slots.
const DEFERRED_SLOTS_COUNT: u64 = 5;
/// Upper bound on the slots needed for pending withdrawals to become ready once the visible slot
/// has caught up. A withdrawal is ready `DEFERRED_SLOTS_COUNT + 1` visible slots after it starts.
const MAX_SETTLE_SLOTS: usize = DEFERRED_SLOTS_COUNT as usize + 5;

#[derive(Debug, Clone, Copy)]
enum SequenceNumberChoice {
    /// The next expected sequence number.
    Next,
    /// One past the next expected sequence number, so the blob is stored out of order.
    SkipOne,
    /// An already-used sequence number, so the blob is dropped.
    Stale,
}

/// A registered non-preferred sequencer.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Regular {
    /// Well funded.
    Rich,
    /// Registered with a stake that cannot cover the escrow of most blobs.
    Poor,
    /// Registered with the minimum bond, which cannot even cover deserializing most blobs.
    Dust,
}

/// A registry call sent by one of the regular sequencers inside a preferred batch.
#[derive(Debug, Clone, Copy)]
enum RegistryCall {
    Deposit(Regular, u64),
    InitiateWithdrawal(Regular),
    Withdraw(Regular),
    Register(Regular, u64),
}

#[derive(Debug, Clone)]
enum Action {
    PreferredBatch {
        slots_to_advance: u8,
        /// Whether to cap `slots_to_advance` at the last slot before the one the batch lands in.
        /// A real preferred sequencer only advances the visible slot to DA slots it has seen, so
        /// it never reaches the slot its own batch lands in. Uncapped batches can, which exercises
        /// the node's handling of the current slot's blobs, but has no sequencer equivalent.
        only_seen_slots: bool,
        sequence_number: SequenceNumberChoice,
        calls: Vec<RegistryCall>,
        /// Size of an extra junk tx, used to push the batch over the data limit.
        padding: Option<usize>,
    },
    PreferredProof {
        sequence_number: SequenceNumberChoice,
    },
    RegularBatch {
        sender: Regular,
        num_txs: usize,
        tx_size: usize,
    },
    RegularProof {
        sender: Regular,
        size: usize,
    },
    /// An undecodable blob from a regular sequencer, which slashes it.
    RegularGarbage(Regular),
    /// Random bytes from an unregistered sender. Sizes above the emergency registration limit
    /// are discarded without being deserialized.
    Unregistered {
        size: usize,
    },
    /// Several small blobs from the unregistered sender in a row, to reach the per-slot cap on
    /// unregistered blobs.
    UnregisteredBurst {
        count: usize,
    },
    /// A valid emergency registration from a funded account with no registered DA address.
    EmergencyRegistration {
        registrant: usize,
        amount: u64,
    },
}

fn regular_strategy() -> impl Strategy<Value = Regular> {
    prop_oneof![3 => Just(Regular::Rich), 1 => Just(Regular::Poor), 1 => Just(Regular::Dust)]
}

fn sequence_number_strategy() -> impl Strategy<Value = SequenceNumberChoice> {
    prop_oneof![
        6 => Just(SequenceNumberChoice::Next),
        1 => Just(SequenceNumberChoice::SkipOne),
        1 => Just(SequenceNumberChoice::Stale),
    ]
}

fn registry_call_strategy() -> impl Strategy<Value = RegistryCall> {
    // Deposits up to ~5 escrows' worth, so that a poor sequencer can come back.
    let amount = 1_000u64..=200_000_000_000;
    prop_oneof![
        3 => (regular_strategy(), amount.clone()).prop_map(|(r, a)| RegistryCall::Deposit(r, a)),
        1 => regular_strategy().prop_map(RegistryCall::InitiateWithdrawal),
        1 => regular_strategy().prop_map(RegistryCall::Withdraw),
        1 => (regular_strategy(), amount).prop_map(|(r, a)| RegistryCall::Register(r, a)),
    ]
}

fn action_strategy() -> impl Strategy<Value = Action> {
    prop_oneof![
        6 => (
            1u8..=3,
            proptest::bool::weighted(0.85),
            sequence_number_strategy(),
            proptest::collection::vec(registry_call_strategy(), 0..=2),
            proptest::option::weighted(0.1, 1usize..=2 * MAX_BLOB_STORAGE_DATA_SIZE),
        )
            .prop_map(|(slots_to_advance, only_seen_slots, sequence_number, calls, padding)| {
                Action::PreferredBatch {
                    slots_to_advance,
                    only_seen_slots,
                    sequence_number,
                    calls,
                    padding,
                }
            }),
        1 => sequence_number_strategy()
            .prop_map(|sequence_number| Action::PreferredProof { sequence_number }),
        6 => (regular_strategy(), 0usize..=3, 0usize..=1200).prop_map(
            |(sender, num_txs, tx_size)| Action::RegularBatch {
                sender,
                num_txs,
                tx_size,
            }
        ),
        1 => (regular_strategy(), 0usize..=1200)
            .prop_map(|(sender, size)| Action::RegularProof { sender, size }),
        1 => regular_strategy().prop_map(Action::RegularGarbage),
        2 => (1usize..=2 * MAX_EMERGENCY_REGISTRATION_BLOB_SIZE)
            .prop_map(|size| Action::Unregistered { size }),
        1 => (6usize..=7).prop_map(|count| Action::UnregisteredBurst { count }),
        // Amounts below the minimum bond make the registration revert, so its reward is refunded.
        1 => (0..NUM_EMERGENCY_REGISTRANTS, 0u64..=100_000_000_000)
            .prop_map(|(registrant, amount)| Action::EmergencyRegistration { registrant, amount }),
    ]
}

#[derive(Debug, Clone)]
struct Scenario {
    /// The actions of each slot.
    slots: Vec<Vec<Action>>,
    /// The slot (if any) at the start of which the preferred sequencer posts garbage and is
    /// slashed, switching the rollup to recovery mode.
    slash_preferred_at: Option<usize>,
    /// The rollup height at which setup mode ends, if enabled. Gas is free during setup mode, so
    /// blobs deferred then reserve no gas and are dropped and refunded when retrieved after it.
    setup_mode_termination_height: Option<u64>,
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    (1usize..=30).prop_flat_map(|num_slots| {
        (
            proptest::collection::vec(
                proptest::collection::vec(action_strategy(), 0..=8),
                num_slots,
            ),
            proptest::option::weighted(0.2, 0..num_slots),
            proptest::option::weighted(0.3, 2u64..=6),
        )
            .prop_map(
                |(slots, slash_preferred_at, setup_mode_termination_height)| Scenario {
                    slots,
                    slash_preferred_at,
                    setup_mode_termination_height,
                },
            )
    })
}

struct Account {
    user: TestUser<S>,
    da_address: MockAddress,
}

struct Harness {
    runner: TestRunner<SoftConfRT>,
    preferred: MockAddress,
    rich: Account,
    poor: Account,
    dust: Account,
    registrants: Vec<Account>,
    next_sequence_number: u64,
    nonces: Nonces,
    /// Every deferred-escrow holder observed in storage during the run.
    seen_holders: Vec<DerivedHolder>,
    /// Registry gas balance that belongs to no stake and no pending escrow.
    unowned_registry_balance: Amount,
    /// Regular sequencers with a pending withdrawal. Every preferred batch tries to complete them.
    pending_withdrawals: Vec<Regular>,
    /// Hashes of every blob posted by the preferred sequencer, to tell its proofs apart from
    /// non-preferred ones in receipts.
    preferred_blob_hashes: HashSet<[u8; 32]>,
    /// Rollup blocks for which the sequencer's selection was compared with the node's.
    compared_blocks: usize,
    /// The number of pre-execution checks each registered non-preferred blob needs (one per tx,
    /// or one for a proof), by blob hash.
    pre_exec_checks: HashMap<[u8; 32], u64>,
}

impl Harness {
    fn new() -> Self {
        let genesis_config = HighLevelZkGenesisConfig::generate_with_additional_accounts(
            4 + NUM_EMERGENCY_REGISTRANTS,
        );
        let preferred = genesis_config.initial_sequencer.da_address;
        let accounts = genesis_config.additional_accounts().clone();
        let genesis = GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: accounts[0].address(),
            },
        );
        let runner = TestRunner::<SoftConfRT>::new_with_genesis(
            genesis.into_genesis_params(),
            SoftConfRT::default(),
        );
        let account = |idx: usize, da_byte: u8| Account {
            user: accounts[idx].clone(),
            da_address: MockAddress::new([da_byte; 32]),
        };

        let mut harness = Self {
            nonces: runner.nonces().clone(),
            runner,
            preferred,
            rich: account(1, 42),
            poor: account(2, 43),
            dust: account(3, 44),
            registrants: (0..NUM_EMERGENCY_REGISTRANTS)
                .map(|i| account(4 + i, 100 + i as u8))
                .collect(),
            next_sequence_number: 0,
            seen_holders: Vec::new(),
            unowned_registry_balance: Amount::ZERO,
            pending_withdrawals: Vec::new(),
            preferred_blob_hashes: HashSet::new(),
            compared_blocks: 0,
            pre_exec_checks: HashMap::new(),
        };

        // Register the regular sequencers through the preferred sequencer. The rich one can afford
        // many escrows, the poor one fewer than one escrow of a typical batch, and the dusty one
        // not even the deserialization charge of most blobs.
        // Priced at the initial base fee rather than the current one, which is zero in setup mode.
        let max_tx_cost = <S as Spec>::Gas::from(config_value!("MAX_SEQUENCER_EXEC_GAS_PER_TX"))
            .value(<S as sov_modules_api::GasSpec>::initial_base_fee_per_gas());
        let calls = vec![
            harness.registry_tx(
                Regular::Rich,
                CallMessage::Register {
                    da_address: harness.rich.da_address,
                    amount: max_tx_cost.checked_mul(Amount::new(100)).unwrap(),
                },
            ),
            harness.registry_tx(
                Regular::Poor,
                CallMessage::Register {
                    da_address: harness.poor.da_address,
                    amount: max_tx_cost.checked_div(Amount::new(5)).unwrap(),
                },
            ),
            harness.registry_tx(
                Regular::Dust,
                CallMessage::Register {
                    da_address: harness.dust.da_address,
                    amount: TEST_MIN_SEQ_BOND,
                },
            ),
        ];
        let blob = harness.preferred_batch_blob(1, SequenceNumberChoice::Next, calls);
        harness
            .runner
            .execute::<RelevantBlobs<MockBlob>>(RelevantBlobs {
                proof_blobs: Vec::new(),
                batch_blobs: vec![blob],
            });

        harness.unowned_registry_balance = harness.compute_unowned_registry_balance();
        harness
    }

    fn regular(&self, which: Regular) -> &Account {
        match which {
            Regular::Rich => &self.rich,
            Regular::Poor => &self.poor,
            Regular::Dust => &self.dust,
        }
    }

    fn registry_tx(
        &self,
        signer: Regular,
        message: CallMessage<S>,
    ) -> TransactionType<SoftConfRT, S> {
        self.regular(signer)
            .user
            .create_plain_message::<SoftConfRT, SequencerRegistry<S>>(message)
    }

    fn registry_call_tx(&self, call: RegistryCall) -> TransactionType<SoftConfRT, S> {
        let (signer, message) = match call {
            RegistryCall::Deposit(r, amount) => (
                r,
                CallMessage::Deposit {
                    da_address: self.regular(r).da_address,
                    amount: Amount::new(amount.into()),
                },
            ),
            RegistryCall::InitiateWithdrawal(r) => (
                r,
                CallMessage::InitiateWithdrawal {
                    da_address: self.regular(r).da_address,
                },
            ),
            RegistryCall::Withdraw(r) => (
                r,
                CallMessage::Withdraw {
                    da_address: self.regular(r).da_address,
                },
            ),
            RegistryCall::Register(r, amount) => (
                r,
                CallMessage::Register {
                    da_address: self.regular(r).da_address,
                    amount: Amount::new(amount.into()),
                },
            ),
        };
        self.registry_tx(signer, message)
    }

    /// Sets the nonces used to sign transactions to the ones the rollup expects next. They drift
    /// when a preferred batch is dropped or stored out of order, and the rollup rejects
    /// transactions whose nonce doesn't match exactly.
    fn sync_nonces(&mut self) {
        let users = [&self.rich, &self.poor, &self.dust]
            .into_iter()
            .chain(&self.registrants)
            .map(|account| &account.user)
            .collect::<Vec<_>>();
        let nonces = self.runner.query_state(|state| {
            users
                .iter()
                .map(|user| {
                    let nonce = Uniqueness::<S>::default()
                        .next_nonce(&user.credential_id(), state)
                        .unwrap();
                    (user.private_key.pub_key(), nonce)
                })
                .collect::<Vec<_>>()
        });
        self.nonces.extend(nonces);
    }

    fn sequence_number(&mut self, choice: SequenceNumberChoice) -> u64 {
        match choice {
            SequenceNumberChoice::Next => {
                self.next_sequence_number += 1;
                self.next_sequence_number - 1
            }
            SequenceNumberChoice::SkipOne => self.next_sequence_number + 1,
            SequenceNumberChoice::Stale => self.next_sequence_number.saturating_sub(1),
        }
    }

    fn preferred_batch_blob(
        &mut self,
        slots_to_advance: u8,
        choice: SequenceNumberChoice,
        txs: Vec<TransactionType<SoftConfRT, S>>,
    ) -> MockBlob {
        self.sync_nonces();
        let sequence_number = self.sequence_number(choice);
        let data = txs
            .into_iter()
            .map(|tx| tx.to_serialized_authenticated_tx(&mut self.nonces))
            .collect::<Vec<_>>();
        let batch = PreferredBatchData {
            sequence_number,
            data: Arc::new(data),
            visible_slots_to_advance: NonZero::new(slots_to_advance).unwrap(),
        };
        let blob = MockBlob::new_with_hash(borsh::to_vec(&batch).unwrap(), self.preferred);
        self.preferred_blob_hashes.insert(blob.hash().into());
        blob
    }

    fn regular_batch_blob(&mut self, sender: MockAddress, txs: Vec<RawTx>) -> MockBlob {
        let num_txs = txs.len() as u64;
        let data = txs
            .into_iter()
            .map(|tx| {
                TransactionType::<SoftConfRT, S>::PreSigned(tx)
                    .to_serialized_authenticated_tx(&mut self.nonces)
            })
            .collect::<Vec<_>>();
        let blob = MockBlob::new_with_hash(borsh::to_vec(&data).unwrap(), sender);
        self.pre_exec_checks.insert(blob.hash().into(), num_txs);
        blob
    }

    fn emergency_registration_blob(&mut self, registrant: usize, amount: u64) -> MockBlob {
        let Account { user, da_address } = &self.registrants[registrant];
        let message = CallMessage::<S>::Register {
            da_address: *da_address,
            amount: Amount::new(amount.into()),
        };
        // Same encoding as a tx in a batch: a signed tx wrapped in the standard authenticator input.
        let mut fully_baked_tx = user
            .create_plain_message::<SoftConfRT, SequencerRegistry<S>>(message)
            .to_serialized_authenticated_tx(&mut self.nonces);
        fully_baked_tx.sequencing_data =
            Some(sov_modules_api::capabilities::new_tx_sequencing_data::<
                S,
                SoftConfRT,
            >());
        MockBlob::new_with_hash(borsh::to_vec(&fully_baked_tx).unwrap(), *da_address)
    }

    /// Returns the blob for the action, and whether it is a proof.
    fn blob_for(&mut self, action: &Action) -> (MockBlob, bool) {
        match action {
            Action::PreferredBatch {
                slots_to_advance,
                only_seen_slots,
                sequence_number,
                calls,
                padding,
            } => {
                let slots_to_advance = if *only_seen_slots {
                    // The batch lands in the next slot, so the last slot it can have seen is the
                    // current true slot. `execute_actions` skips these batches when there is no
                    // unseen slot.
                    let unseen = self.runner.true_slot_number().get()
                        - self.runner.visible_slot_number().get();
                    (*slots_to_advance)
                        .min(u8::try_from(unseen).unwrap_or(u8::MAX))
                        .max(1)
                } else {
                    *slots_to_advance
                };
                let mut txs = calls
                    .iter()
                    .copied()
                    .chain(
                        self.pending_withdrawals
                            .iter()
                            .map(|r| RegistryCall::Withdraw(*r)),
                    )
                    .map(|call| self.registry_call_tx(call))
                    .collect::<Vec<_>>();
                if let Some(size) = padding {
                    txs.push(TransactionType::PreSigned(RawTx::new(vec![0xab; *size])));
                }
                let blob = self.preferred_batch_blob(slots_to_advance, *sequence_number, txs);
                (blob, false)
            }
            Action::PreferredProof { sequence_number } => {
                let proof = PreferredProofData {
                    sequence_number: self.sequence_number(*sequence_number),
                    data: vec![0xcd; 64],
                };
                let blob = MockBlob::new_with_hash(borsh::to_vec(&proof).unwrap(), self.preferred);
                self.preferred_blob_hashes.insert(blob.hash().into());
                (blob, true)
            }
            Action::RegularBatch {
                sender,
                num_txs,
                tx_size,
            } => {
                // Distinct bytes per tx, so that batches with different shapes have different hashes.
                let txs = (0..*num_txs)
                    .map(|i| RawTx::new(vec![i as u8; *tx_size]))
                    .collect();
                let sender = self.regular(*sender).da_address;
                (self.regular_batch_blob(sender, txs), false)
            }
            Action::RegularProof { sender, size } => {
                // At least one byte, so that an empty proof can't share a hash with an empty batch
                // (both would encode as an empty borsh vector).
                let blob = MockBlob::new_with_hash(
                    borsh::to_vec(&vec![0xef_u8; *size + 1]).unwrap(),
                    self.regular(*sender).da_address,
                );
                self.pre_exec_checks.insert(blob.hash().into(), 1);
                (blob, true)
            }
            // Random bytes can never deserialize as a batch.
            Action::RegularGarbage(sender) => (
                MockBlob::new_with_hash(vec![0xff; 100], self.regular(*sender).da_address),
                false,
            ),
            Action::Unregistered { size } => (
                MockBlob::new_with_hash(vec![0x01; *size], UNREGISTERED_SENDER),
                false,
            ),
            Action::EmergencyRegistration { registrant, amount } => (
                self.emergency_registration_blob(*registrant, *amount),
                false,
            ),
            Action::UnregisteredBurst { .. } => {
                unreachable!("Bursts produce several blobs and are expanded by `execute_actions`")
            }
        }
    }

    fn execute_actions(&mut self, actions: &[Action]) {
        let mut blobs = RelevantBlobs {
            proof_blobs: Vec::new(),
            batch_blobs: Vec::new(),
        };
        let caught_up =
            self.runner.visible_slot_number().get() == self.runner.true_slot_number().get();
        for action in actions {
            // A real preferred sequencer only starts a block once it sees a new DA slot, so it has
            // no batch to post while the visible slot is caught up.
            if caught_up
                && matches!(
                    action,
                    Action::PreferredBatch {
                        only_seen_slots: true,
                        ..
                    }
                )
            {
                continue;
            }
            if let Action::UnregisteredBurst { count } = action {
                // Distinct bytes per blob, so that they have different hashes.
                blobs.batch_blobs.extend(
                    (0..*count)
                        .map(|i| MockBlob::new_with_hash(vec![i as u8; 32], UNREGISTERED_SENDER)),
                );
                continue;
            }
            match self.blob_for(action) {
                (blob, true) => blobs.proof_blobs.push(blob),
                (blob, false) => blobs.batch_blobs.push(blob),
            }
        }
        self.execute_slot(blobs);
    }

    fn execute_slot(&mut self, blobs: RelevantBlobs<MockBlob>) {
        self.execute_slot_inner(blobs, true);
    }

    /// Executes a slot and checks the invariants. `check_user_root` can be disabled for slots
    /// that are allowed to change the user state root without creating a rollup block.
    fn execute_slot_inner(&mut self, blobs: RelevantBlobs<MockBlob>, check_user_root: bool) {
        let rollup_height_before = self
            .runner
            .query_state(|state| state.rollup_height_to_access());
        // The sequencer builds its next block on the node's state before this slot, without
        // knowing how far the block will advance the visible slot. So compute its selection for
        // every visible slot the block could advance to, and compare the right one afterwards.
        let sequencer_blocks = if check_user_root && self.preferred_is_registered() {
            let visible = self.runner.visible_slot_number().get();
            (visible + 1..=self.runner.true_slot_number().get())
                .map(|next_visible| (next_visible, self.sequencer_block(next_visible)))
                .collect()
        } else {
            HashMap::new()
        };
        let user_root_before = self
            .runner
            .state_root()
            .namespace_root(ProvableNamespace::User);

        self.runner.execute::<RelevantBlobs<MockBlob>>(blobs);

        let rollup_height_after = self
            .runner
            .query_state(|state| state.rollup_height_to_access());
        if check_user_root && rollup_height_after == rollup_height_before {
            assert_eq!(
                self.runner
                    .state_root()
                    .namespace_root(ProvableNamespace::User),
                user_root_before,
                "The user state root changed in slot {} without a rollup block being created",
                self.runner.true_slot_number()
            );
        }

        if rollup_height_after != rollup_height_before {
            self.compare_with_sequencer_block(&sequencer_blocks);
        }

        let unowned = self.compute_unowned_registry_balance();
        assert!(
            unowned >= self.unowned_registry_balance,
            "Registry funds that belong to no stake and no pending escrow decreased in slot {} from {} to {}. Some escrow or stake was over-credited or paid out twice.",
            self.runner.true_slot_number(),
            self.unowned_registry_balance,
            unowned
        );
        self.unowned_registry_balance = unowned;
        self.update_pending_withdrawals();
    }

    /// Runs what the preferred sequencer's block builder does to start a rollup block advancing
    /// the visible slot to `next_visible`, on a throwaway copy of the current state.
    fn sequencer_block(&self, next_visible: u64) -> SequencerBlock {
        let mut rt = SoftConfRT::default();
        let mut kernel = rt.kernel();
        let mut checkpoint = StateCheckpoint::<S>::new(
            self.runner.storage_manager().create_prover_storage(),
            &kernel,
        );
        let mut accessor = KernelStateAccessor::from_checkpoint(&kernel, &mut checkpoint);
        let old_visible = accessor.visible_slot_number();
        // `advance` mutates its receiver, so advance a copy.
        let next_visible = { old_visible }.advance(next_visible - old_visible.get());
        kernel.increment_rollup_height(&mut accessor, next_visible);
        let gas_price = kernel.base_fee_per_gas(&mut accessor).unwrap();
        let selected = kernel.get_non_preferred_blobs(
            old_visible
                .as_true()
                .next()
                .range_inclusive(next_visible.as_true()),
            &mut accessor,
            NoOpControlFlow,
        );

        let mut block = SequencerBlock {
            batches: Vec::new(),
            proofs: Vec::new(),
            reserved_gas: Vec::new(),
            gas_price,
        };
        for blob in selected {
            assert_eq!(
                blob.reserved_gas_tokens.is_none(),
                matches!(blob.blob_data, BlobDataWithId::EmergencyRegistration { .. }),
                "Only emergency registrations should be selected without reserved gas"
            );
            let id = match &blob.blob_data {
                BlobDataWithId::Batch(batch) => batch.id,
                BlobDataWithId::Proof { id, .. }
                | BlobDataWithId::EmergencyRegistration { id, .. } => *id,
            };
            if let Some(reserved) = blob.reserved_gas_tokens {
                block.reserved_gas.push((id, reserved));
            }
            match blob.blob_data {
                BlobDataWithId::Proof { .. } => block.proofs.push(id),
                BlobDataWithId::Batch(_) | BlobDataWithId::EmergencyRegistration { .. } => {
                    block.batches.push((id, blob.sender));
                }
            }
        }
        block
    }

    /// If the last slot created a rollup block from a preferred batch, checks that the sequencer
    /// would have selected the same non-preferred blobs, at the same gas price, as the node.
    fn compare_with_sequencer_block(&mut self, sequencer_blocks: &HashMap<u64, SequencerBlock>) {
        let receipt = self.runner.receipts().last().unwrap();
        // Blocks without a preferred batch are created by forced visible slot advancement, which
        // the sequencer is not involved in.
        let Some(preferred_batch) = receipt
            .batch_receipts
            .iter()
            .find(|batch| batch.inner.da_address == self.preferred)
        else {
            return;
        };
        let new_visible = self.runner.visible_slot_number().get();
        // A preferred batch can only advance the visible slot to a slot its sequencer has seen,
        // which is always before the slot the batch lands in. The generator doesn't model this,
        // so blocks that advance to the batch's own slot have no sequencer equivalent.
        let Some(sequencer_block) = sequencer_blocks.get(&new_visible) else {
            return;
        };
        self.compared_blocks += 1;

        let node_batches = receipt
            .batch_receipts
            .iter()
            .filter(|batch| batch.inner.da_address != self.preferred)
            .map(|batch| (batch.batch_hash, batch.inner.da_address))
            .collect::<Vec<_>>();
        let node_proofs = receipt
            .proof_receipts
            .iter()
            .map(|proof| proof.blob_hash)
            .filter(|hash| !self.preferred_blob_hashes.contains(hash))
            .collect::<Vec<_>>();
        let slot = self.runner.true_slot_number();
        assert_eq!(
            sequencer_block.batches, node_batches,
            "The sequencer and the node selected different non-preferred batches in slot {slot}"
        );
        assert_eq!(
            sequencer_block.proofs, node_proofs,
            "The sequencer and the node selected different non-preferred proofs in slot {slot}"
        );
        assert_eq!(
            sequencer_block.gas_price, preferred_batch.inner.gas_price,
            "The sequencer and the node used different gas prices for the block in slot {slot}"
        );
        // The node reserves one `max_tx_check_costs` per pre-execution check at the block's price.
        for (id, reserved) in &sequencer_block.reserved_gas {
            let checks = self.pre_exec_checks[id];
            let expected = <S as sov_modules_api::GasSpec>::max_tx_check_costs()
                .checked_scalar_product(checks)
                .unwrap()
                .checked_value(preferred_batch.inner.gas_price)
                .unwrap();
            assert_eq!(
                *reserved, expected,
                "The sequencer reserved a different amount of gas than the node for blob {} in slot {slot}",
                hex::encode(id)
            );
        }
    }

    fn update_pending_withdrawals(&mut self) {
        let pending = [Regular::Rich, Regular::Poor, Regular::Dust]
            .into_iter()
            .filter(|r| {
                let da_address = self.regular(*r).da_address;
                self.runner.query_state(|state| {
                    SequencerRegistry::<S>::default()
                        .is_sender_known(&da_address, state)
                        .is_ok_and(|sequencer| sequencer.balance_state.is_pending_withdrawal())
                })
            })
            .collect();
        self.pending_withdrawals = pending;
    }

    fn known_da_addresses(&self) -> Vec<MockAddress> {
        [
            self.preferred,
            self.rich.da_address,
            self.poor.da_address,
            self.dust.da_address,
            UNREGISTERED_SENDER,
        ]
        .into_iter()
        .chain(self.registrants.iter().map(|r| r.da_address))
        .collect()
    }

    /// Registry gas balance, minus the stakes of known sequencers, minus the escrow of every
    /// blob still in deferred storage. Also records the escrow holders seen in storage.
    fn compute_unowned_registry_balance(&mut self) -> Amount {
        let da_addresses = self.known_da_addresses();
        let true_slot = self.runner.true_slot_number().get();
        let (registry_balance, stakes, holders) = self.runner.query_state(|state| {
            let registry = SequencerRegistry::<S>::default();
            let registry_balance = Bank::<S>::default()
                .get_balance_of(registry.id().to_payable(), config_gas_token_id(), state)
                .unwrap()
                .unwrap_or(Amount::ZERO);
            let stakes = da_addresses
                .iter()
                .filter_map(|da| registry.get_sender_balance_via_api(da, state))
                .collect::<Vec<_>>();
            let mut holders = Vec::new();
            for slot in 0..=true_slot {
                for holder in BlobStorage::<S>::default()
                    .get_deferred_escrow_holders(SlotNumber::new(slot), state)
                    .unwrap()
                {
                    let amount = registry
                        .get_escrowed_amount_via_api(&holder, state)
                        .unwrap_or_else(|| {
                            panic!("Deferred blob with holder {holder} has no escrow entry")
                        });
                    holders.push((holder, amount));
                }
            }
            (registry_balance, stakes, holders)
        });

        let mut owned = Amount::ZERO;
        for amount in stakes
            .into_iter()
            .chain(holders.iter().map(|(_, amount)| *amount))
        {
            owned = owned.checked_add(amount).unwrap();
        }
        for (holder, _) in holders {
            if !self.seen_holders.contains(&holder) {
                self.seen_holders.push(holder);
            }
        }
        registry_balance.checked_sub(owned).unwrap_or_else(|| {
            panic!("The registry holds {registry_balance} but owes {owned} in stakes and pending escrow")
        })
    }

    fn preferred_is_registered(&self) -> bool {
        self.runner.query_state(|state| {
            SequencerRegistry::<S>::default()
                .get_sender_balance_via_api(&self.preferred, state)
                .is_some()
        })
    }

    /// Builds a preferred batch that advances the visible slot to the last slot before the one the
    /// batch lands in (as far as a real preferred sequencer can), with the sequence number the
    /// rollup expects next. Generated scenarios can desync the harness's
    /// sequence numbers from the rollup's (e.g. a dropped oversized batch never uses its number),
    /// so this reads the rollup's state instead. Returns `None` if a stored batch will already
    /// advance the visible slot in the next slot.
    fn catch_up_preferred_batch(
        &mut self,
        txs: Vec<TransactionType<SoftConfRT, S>>,
    ) -> Option<MockBlob> {
        let sequence_number = self.runner.query_state(|state| {
            let blob_storage = BlobStorage::<S>::default();
            let mut sequence_number = blob_storage.next_sequence_number(state);
            // Stored proofs are processed in sequence until the next batch.
            loop {
                match blob_storage
                    .get_deferred_preferred_sequencer_blob(sequence_number, state)
                    .unwrap()
                    .map(|blob| blob.inner)
                {
                    Some(PreferredBlobData::Proof(_)) => sequence_number += 1,
                    Some(PreferredBlobData::Batch(_)) => return None,
                    None => return Some(sequence_number),
                }
            }
        })?;
        self.next_sequence_number = sequence_number;
        let unseen = self.runner.true_slot_number().get() - self.runner.visible_slot_number().get();
        let slots_to_advance = u8::try_from(unseen).unwrap_or(u8::MAX).max(1);
        Some(self.preferred_batch_blob(slots_to_advance, SequenceNumberChoice::Next, txs))
    }

    /// Catches the visible slot up with every slot that has deferred blobs, so that they are all
    /// either executed or dropped. With a preferred sequencer, it advances the visible slot up to
    /// the slot before each catch-up slot. Catch-up slots have no non-preferred blobs, so one
    /// catch-up slot that ends one slot behind leaves nothing deferred. In recovery mode, the
    /// visible slot catches up on its own.
    fn drain(&mut self) {
        for drained_slots in 0..MAX_DRAIN_SLOTS {
            let behind =
                self.runner.true_slot_number().get() - self.runner.visible_slot_number().get();
            if behind == 0 || (behind == 1 && drained_slots > 0) {
                return;
            }
            let batch_blobs = if self.preferred_is_registered() {
                self.catch_up_preferred_batch(Vec::new())
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            self.execute_slot(RelevantBlobs {
                proof_blobs: Vec::new(),
                batch_blobs,
            });
        }
        panic!(
            "The visible slot did not catch up with the true slot after {MAX_DRAIN_SLOTS} slots"
        );
    }

    /// Lets the preferred sequencer advance the visible slot until every pending withdrawal is
    /// ready, and completes them. Withdrawals need transactions, so this is skipped in recovery
    /// mode.
    fn settle_withdrawals(&mut self) {
        if !self.preferred_is_registered() {
            return;
        }
        for _ in 0..MAX_SETTLE_SLOTS {
            if self.pending_withdrawals.is_empty() {
                return;
            }
            let calls = self
                .pending_withdrawals
                .iter()
                .map(|r| self.registry_call_tx(RegistryCall::Withdraw(*r)))
                .collect();
            let batch_blobs = self.catch_up_preferred_batch(calls).into_iter().collect();
            self.execute_slot(RelevantBlobs {
                proof_blobs: Vec::new(),
                batch_blobs,
            });
        }
        assert_eq!(
            self.pending_withdrawals,
            Vec::new(),
            "Withdrawals did not complete after {MAX_SETTLE_SLOTS} slots"
        );
    }

    fn assert_all_escrows_released(&self) {
        self.runner.query_state(|state| {
            let registry = SequencerRegistry::<S>::default();
            for holder in &self.seen_holders {
                assert_eq!(
                    registry.get_escrowed_amount_via_api(holder, state),
                    None,
                    "Escrow for holder {holder} was not released after its blob left storage"
                );
            }
        });
    }
}

proptest! {
    // Each case runs up to ~80 slots through the STF, so keep the local default small.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(32)
    ))]

    #[test]
    fn interleaved_blobs_keep_escrow_accounting_consistent(
        scenario in scenario_strategy()
    ) {
        // nextest runs each test in its own process, so these overrides don't leak. They are read
        // at every use, so each case can set its own.
        std::env::set_var(
            "SOV_TEST_CONST_OVERRIDE_MAX_ALLOWED_DATA_SIZE_RETURNED_BY_BLOB_STORAGE",
            MAX_BLOB_STORAGE_DATA_SIZE.to_string(),
        );
        std::env::set_var(
            "SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT",
            DEFERRED_SLOTS_COUNT.to_string(),
        );
        const SETUP_MODE_OVERRIDE: &str = "SOV_TEST_CONST_OVERRIDE_SETUP_MODE_TERMINATION_HEIGHT";
        match scenario.setup_mode_termination_height {
            Some(height) => std::env::set_var(SETUP_MODE_OVERRIDE, height.to_string()),
            None => std::env::remove_var(SETUP_MODE_OVERRIDE),
        }

        let mut harness = Harness::new();
        for (slot, actions) in scenario.slots.iter().enumerate() {
            if scenario.slash_preferred_at == Some(slot) {
                // Random bytes can never deserialize as a preferred batch, so this slashes the
                // preferred sequencer.
                // Slashing deletes the user-space `SequencerRegistry::preferred_sequencer` value
                // without creating a rollup block, which is accepted.
                let preferred = harness.preferred;
                harness.execute_slot_inner(
                    RelevantBlobs {
                        proof_blobs: Vec::new(),
                        batch_blobs: vec![MockBlob::new_with_hash(vec![0xff; 100], preferred)],
                    },
                    false,
                );
            }
            harness.execute_actions(actions);
        }
        harness.drain();
        harness.settle_withdrawals();
        harness.assert_all_escrows_released();
    }
}
