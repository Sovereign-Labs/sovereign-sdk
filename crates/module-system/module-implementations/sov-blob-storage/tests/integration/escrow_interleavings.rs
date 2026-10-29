//! Property test: random interleavings of preferred, registered non-preferred, garbage and
//! unregistered blobs must never panic the STF, must never change the user state root in a slot
//! that creates no rollup block, and must account for every token of deferred-blob escrow.

use std::collections::HashMap;

use proptest::prelude::*;
use sov_bank::derived_holder::DerivedHolder;
use sov_bank::{config_gas_token_id, Bank, IntoPayable};
use sov_mock_da::{MockAddress, MockBlob};
use sov_modules_api::{Amount, ModuleInfo, RawTx, VersionReader};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::RelevantBlobs;
use sov_sequencer_registry::SequencerRegistry;
use sov_state::{ProvableNamespace, StateRoot};
use sov_test_utils::runtime::BlobStorage;
use sov_test_utils::{BatchType, SequencerInfo, SoftConfirmationBlobInfo, TransactionType};

use crate::helpers_soft_confirmations::{
    setup_with_registration_soft_confirmation_kernel, SoftConfRT,
};
use crate::{TestData, TestRunner, S};

const UNREGISTERED_SENDER: MockAddress = MockAddress::new([7; 32]);
/// Upper bound on the slots needed for the preferred sequencer to catch the visible slot up to the
/// true slot after the generated slots.
const MAX_DRAIN_SLOTS: usize = 50;

#[derive(Debug, Clone, Copy)]
enum SequenceNumberChoice {
    /// The next expected sequence number.
    Next,
    /// One past the next expected sequence number, so the batch is stored out of order.
    SkipOne,
    /// An already-used sequence number, so the batch is dropped.
    Stale,
}

#[derive(Debug, Clone)]
enum Action {
    Preferred {
        slots_to_advance: u8,
        sequence_number: SequenceNumberChoice,
    },
    Regular {
        num_txs: usize,
        tx_size: usize,
    },
    /// An undecodable blob from the registered non-preferred sequencer, which slashes it.
    RegularGarbage,
    Unregistered {
        size: usize,
    },
}

fn action_strategy() -> impl Strategy<Value = Action> {
    let sequence_number = prop_oneof![
        6 => Just(SequenceNumberChoice::Next),
        1 => Just(SequenceNumberChoice::SkipOne),
        1 => Just(SequenceNumberChoice::Stale),
    ];
    prop_oneof![
        4 => (1u8..=3, sequence_number).prop_map(|(slots_to_advance, sequence_number)| {
            Action::Preferred {
                slots_to_advance,
                sequence_number,
            }
        }),
        4 => (0usize..=3, 0usize..=64)
            .prop_map(|(num_txs, tx_size)| Action::Regular { num_txs, tx_size }),
        1 => Just(Action::RegularGarbage),
        1 => (1usize..=64).prop_map(|size| Action::Unregistered { size }),
    ]
}

struct Harness {
    runner: TestRunner<SoftConfRT>,
    preferred: MockAddress,
    regular: MockAddress,
    next_sequence_number: u64,
    nonces: HashMap<
        <<S as sov_modules_api::Spec>::CryptoSpec as sov_modules_api::CryptoSpec>::PublicKey,
        u64,
    >,
    /// Every deferred-escrow holder observed in storage during the run.
    seen_holders: Vec<DerivedHolder>,
    /// Registry gas balance that belongs to no stake and no pending escrow.
    unowned_registry_balance: Amount,
}

impl Harness {
    fn new() -> Self {
        let (
            TestData {
                preferred_sequencer,
                regular_sequencer,
                ..
            },
            runner,
        ) = setup_with_registration_soft_confirmation_kernel();
        let nonces = runner.nonces().clone();
        let mut harness = Self {
            runner,
            preferred: preferred_sequencer.da_address,
            regular: regular_sequencer.da_address,
            // Sequence number 0 was used to register the regular sequencer.
            next_sequence_number: 1,
            nonces,
            seen_holders: Vec::new(),
            unowned_registry_balance: Amount::ZERO,
        };
        harness.unowned_registry_balance = harness.compute_unowned_registry_balance();
        harness
    }

    fn preferred_blob(&mut self, slots_to_advance: u8, choice: SequenceNumberChoice) -> MockBlob {
        let sequence_number = match choice {
            SequenceNumberChoice::Next => {
                self.next_sequence_number += 1;
                self.next_sequence_number - 1
            }
            SequenceNumberChoice::SkipOne => self.next_sequence_number + 1,
            SequenceNumberChoice::Stale => self.next_sequence_number.saturating_sub(1),
        };
        self.batch_blob(
            self.preferred,
            SequencerInfo::Preferred {
                slots_to_advance,
                sequence_number,
            },
            Vec::new(),
        )
    }

    fn batch_blob(
        &mut self,
        sender: MockAddress,
        sequencer_info: SequencerInfo,
        txs: Vec<RawTx>,
    ) -> MockBlob {
        TestRunner::<SoftConfRT>::soft_confirmation_batches_to_blobs(
            vec![SoftConfirmationBlobInfo {
                batch_type: BatchType(txs.into_iter().map(TransactionType::PreSigned).collect()),
                sequencer_address: sender,
                sequencer_info,
            }],
            &mut self.nonces,
        )
        .batch_blobs
        .pop()
        .expect("One batch was requested")
    }

    fn blob_for(&mut self, action: &Action) -> MockBlob {
        match *action {
            Action::Preferred {
                slots_to_advance,
                sequence_number,
            } => self.preferred_blob(slots_to_advance, sequence_number),
            Action::Regular { num_txs, tx_size } => {
                // Distinct bytes per tx, so that batches with different shapes have different hashes.
                let txs = (0..num_txs)
                    .map(|i| RawTx::new(vec![i as u8; tx_size]))
                    .collect();
                self.batch_blob(self.regular, SequencerInfo::Regular, txs)
            }
            // Random bytes can never deserialize as a batch.
            Action::RegularGarbage => MockBlob::new_with_hash(vec![0xff; 100], self.regular),
            Action::Unregistered { size } => {
                MockBlob::new_with_hash(vec![0x01; size], UNREGISTERED_SENDER)
            }
        }
    }

    fn execute_slot(&mut self, blobs: Vec<MockBlob>) {
        let rollup_height_before = self
            .runner
            .query_state(|state| state.rollup_height_to_access());
        let user_root_before = self
            .runner
            .state_root()
            .namespace_root(ProvableNamespace::User);

        self.runner
            .execute::<RelevantBlobs<MockBlob>>(RelevantBlobs {
                proof_blobs: Vec::new(),
                batch_blobs: blobs,
            });

        let rollup_height_after = self
            .runner
            .query_state(|state| state.rollup_height_to_access());
        if rollup_height_after == rollup_height_before {
            assert_eq!(
                self.runner
                    .state_root()
                    .namespace_root(ProvableNamespace::User),
                user_root_before,
                "The user state root changed in slot {} without a rollup block being created",
                self.runner.true_slot_number()
            );
        }

        let unowned = self.compute_unowned_registry_balance();
        assert!(
            unowned >= self.unowned_registry_balance,
            "Registry funds that belong to no stake and no pending escrow decreased in slot {} from {} to {}. Some escrow was over-refunded or paid out twice.",
            self.runner.true_slot_number(),
            self.unowned_registry_balance,
            unowned
        );
        self.unowned_registry_balance = unowned;
    }

    /// Registry gas balance, minus the stakes of known sequencers, minus the escrow of every
    /// blob still in deferred storage. Also records the escrow holders seen in storage.
    fn compute_unowned_registry_balance(&mut self) -> Amount {
        let (preferred, regular) = (self.preferred, self.regular);
        let true_slot = self.runner.true_slot_number().get();
        let (registry_balance, stakes, holders) = self.runner.query_state(|state| {
            let registry = SequencerRegistry::<S>::default();
            let registry_balance = Bank::<S>::default()
                .get_balance_of(registry.id().to_payable(), config_gas_token_id(), state)
                .unwrap()
                .unwrap_or(Amount::ZERO);
            let stakes = [preferred, regular]
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

    /// Lets the preferred sequencer advance the visible slot until it reaches the true slot, so
    /// that every deferred blob is either executed or dropped.
    fn drain(&mut self) {
        for _ in 0..MAX_DRAIN_SLOTS {
            if self.runner.visible_slot_number().as_true() == self.runner.true_slot_number() {
                return;
            }
            let blob = self.preferred_blob(u8::MAX, SequenceNumberChoice::Next);
            self.execute_slot(vec![blob]);
        }
        panic!(
            "The visible slot did not catch up with the true slot after {MAX_DRAIN_SLOTS} slots"
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
        slots in proptest::collection::vec(proptest::collection::vec(action_strategy(), 0..=4), 1..=30)
    ) {
        let mut harness = Harness::new();
        for actions in &slots {
            let blobs = actions.iter().map(|action| harness.blob_for(action)).collect();
            harness.execute_slot(blobs);
        }
        harness.drain();
        harness.assert_all_escrows_released();
    }
}
