use sov_accounts::AccountConfig;
use sov_bank::TokenConfig;
use sov_blob_storage::{BlobStorage, DEFERRED_SLOTS_COUNT};
use sov_chain_state::{ChainState, ChainStateConfig};
use sov_modules_api::capabilities::{BlobRefOrOwned, BlobSelector};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::hooks::{ApplyBlobHooks, FinalizeHook, SlotHooks, TxHooks};
use sov_modules_api::macros::DefaultRuntime;
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{
    AccessoryWorkingSet, Address, BlobReaderTrait, Context, DaSpec, DispatchCall, MessageCodec,
    Module, Spec, WorkingSet,
};
use sov_modules_stf_template::{AppTemplate, Runtime, SequencerOutcome};
use sov_rollup_interface::mocks::{
    MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaSpec, MockValidityCond, MockZkvm,
};
use sov_sequencer_registry::{SequencerConfig, SequencerRegistry};
use sov_state::{ProverStorage, Storage};

type C = DefaultContext;
type B = MockBlob;
type Da = MockDaSpec;

const LOCKED_AMOUNT: u64 = 200;
const PREFERRED_SEQUENCER_DA: MockAddress = MockAddress::new([10u8; 32]);
const PREFERRED_SEQUENCER_ROLLUP: Address = Address::new(*b"preferred_______________________");
const REGULAR_SEQUENCER_DA: MockAddress = MockAddress::new([30u8; 32]);
const REGULAR_SEQUENCER_ROLLUP: Address = Address::new(*b"regular_________________________");

fn get_bank_config(
    preferred_sequencer: <C as Spec>::Address,
    regular_sequencer: <C as Spec>::Address,
) -> sov_bank::BankConfig<C> {
    let token_config: TokenConfig<C> = TokenConfig {
        token_name: "InitialToken".to_owned(),
        address_and_balances: vec![
            (preferred_sequencer, LOCKED_AMOUNT * 3),
            (regular_sequencer, LOCKED_AMOUNT * 3),
        ],
        authorized_minters: vec![],
        salt: 9,
    };

    sov_bank::BankConfig {
        tokens: vec![token_config],
    }
}

fn make_blobs(blob_num: &mut u8, senders: impl Iterator<Item = MockAddress>) -> Vec<MockBlob> {
    let blobs: Vec<_> = senders
        .enumerate()
        .map(|(offset, sender)| B::new(vec![], sender, [*blob_num + offset as u8; 32]))
        .collect();
    *blob_num += blobs.len() as u8;
    blobs
}

fn make_blobs_by_slot(is_from_preferred_by_slot: &[Vec<bool>]) -> Vec<Vec<MockBlob>> {
    let mut blob_num = 0;
    is_from_preferred_by_slot
        .iter()
        .map(|slot| {
            make_blobs(
                &mut blob_num,
                slot.iter().map(|is_preferred| {
                    if *is_preferred {
                        PREFERRED_SEQUENCER_DA
                    } else {
                        REGULAR_SEQUENCER_DA
                    }
                }),
            )
        })
        .collect()
}

#[test]
fn priority_sequencer_flow() {
    assert_ne!(DEFERRED_SLOTS_COUNT, 7);
    let (app, runtime, genesis_root) = TestRuntime::pre_initialized(true);
    let mut working_set = WorkingSet::new(app.current_storage.clone());

    let register_message = sov_sequencer_registry::CallMessage::Register {
        da_address: REGULAR_SEQUENCER_DA.as_ref().to_vec(),
    };
    runtime
        .sequencer_registry
        .call(
            register_message,
            &C::new(REGULAR_SEQUENCER_ROLLUP),
            &mut working_set,
        )
        .unwrap();

    let is_from_preferred_by_slot = [
        vec![false, false, true],
        vec![false, true, false],
        vec![false, false],
    ];
    let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);

    let slot_1_blobs = blobs_by_slot[0].clone();
    let slot_2_blobs = blobs_by_slot[1].clone();
    let slot_3_blobs = blobs_by_slot[2].clone();
    let blobs = blobs_by_slot.into_iter().flatten().collect::<Vec<_>>();

    // Slot 1: 3rd blob is from preferred sequencer, only it should be executed
    let mut slot_1_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [1; 32].into(),
            height: 1,
        },
        validity_cond: Default::default(),
        blobs: slot_1_blobs,
    };
    let current_root = genesis_root;
    runtime.chain_state.begin_slot_hook(
        &slot_1_data.header,
        &slot_1_data.validity_cond,
        &current_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_1 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_1_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(1, execute_in_slot_1.len());
    blobs_are_equal(blobs[2].clone(), execute_in_slot_1.remove(0), "slot 1");
    // Second attempt to get blobs for slot return existing slots as is.
    let mut execute_in_slot_1_attempt_2 =
        <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
            &runtime.blob_storage,
            &mut slot_1_data.blobs,
            &mut working_set,
        )
        .unwrap();

    // Same as before
    assert_eq!(1, execute_in_slot_1_attempt_2.len());
    // But blob is consumed by previous read comparison, so we compare hash only
    blob_hashes_are_equal(
        blobs[2].clone(),
        execute_in_slot_1_attempt_2.remove(0),
        "slot 1",
    );

    // Slot 2: 5th blob is from preferred sequencer + 2nd and 3rd that were deferred previously
    let mut slot_2_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_1_data.header.hash,
            hash: [2; 32].into(),
            height: 2,
        },
        validity_cond: Default::default(),
        blobs: slot_2_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_2_data.header,
        &slot_2_data.validity_cond,
        &current_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_2 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_2_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(3, execute_in_slot_2.len());
    blobs_are_equal(blobs[4].clone(), execute_in_slot_2.remove(0), "slot 2");
    blobs_are_equal(blobs[0].clone(), execute_in_slot_2.remove(0), "slot 2");
    blobs_are_equal(blobs[1].clone(), execute_in_slot_2.remove(0), "slot 2");

    // Slot 3: no blobs from preferred sequencer, so deferred executed first and then current
    let mut slot_3_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_2_data.header.hash,
            hash: [3; 32].into(),
            height: 3,
        },
        validity_cond: Default::default(),
        blobs: slot_3_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_3_data.header,
        &slot_3_data.validity_cond,
        &current_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_3 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_3_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(2, execute_in_slot_3.len());
    blobs_are_equal(blobs[3].clone(), execute_in_slot_3.remove(0), "slot 3");
    blobs_are_equal(blobs[5].clone(), execute_in_slot_3.remove(0), "slot 3");

    // Slot 4: no blobs at all
    let mut slot_4_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_3_data.header.hash,
            hash: [4; 32].into(),
            height: 4,
        },
        validity_cond: Default::default(),
        blobs: Vec::new(),
    };
    runtime.chain_state.begin_slot_hook(
        &slot_4_data.header,
        &slot_4_data.validity_cond,
        &current_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_4 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_4_data.blobs,
        &mut working_set,
    )
    .unwrap();

    assert_eq!(2, execute_in_slot_4.len());
    blobs_are_equal(blobs[6].clone(), execute_in_slot_4.remove(0), "slot 4");
    blobs_are_equal(blobs[7].clone(), execute_in_slot_4.remove(0), "slot 4");
}

#[test]
fn test_blobs_from_non_registered_sequencers_are_not_saved() {
    let (app, runtime, genesis_root) = TestRuntime::pre_initialized(true);
    let mut working_set = WorkingSet::new(app.current_storage.clone());

    let unregistered_sequencer = MockAddress::new([7; 32]);
    let register_message = sov_sequencer_registry::CallMessage::Register {
        da_address: REGULAR_SEQUENCER_DA.as_ref().to_vec(),
    };
    runtime
        .sequencer_registry
        .call(
            register_message,
            &C::new(REGULAR_SEQUENCER_ROLLUP),
            &mut working_set,
        )
        .unwrap();

    let blob_1 = B::new(vec![1], REGULAR_SEQUENCER_DA, [1u8; 32]);
    let blob_2 = B::new(vec![2, 2], unregistered_sequencer, [2u8; 32]);
    let blob_3 = B::new(vec![3, 3, 3], PREFERRED_SEQUENCER_DA, [3u8; 32]);

    let slot_1_blobs = vec![blob_1.clone(), blob_2, blob_3.clone()];

    let mut slot_1_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [1; 32].into(),
            height: 1,
        },
        validity_cond: Default::default(),
        blobs: slot_1_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_1_data.header,
        &slot_1_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_1 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_1_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(1, execute_in_slot_1.len());
    blobs_are_equal(blob_3, execute_in_slot_1.remove(0), "slot 1");

    let mut slot_2_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_1_data.header.hash,
            hash: [2; 32].into(),
            height: 2,
        },
        validity_cond: Default::default(),
        blobs: Vec::new(),
    };
    runtime.chain_state.begin_slot_hook(
        &slot_2_data.header,
        &slot_2_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_2 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_2_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(1, execute_in_slot_2.len());
    blobs_are_equal(blob_1, execute_in_slot_2.remove(0), "slot 2");
}

#[test]
fn test_blobs_no_deferred_without_preferred_sequencer() {
    let (app, runtime, genesis_root) = TestRuntime::pre_initialized(false);
    let mut working_set = WorkingSet::new(app.current_storage.clone());

    let register_message = sov_sequencer_registry::CallMessage::Register {
        da_address: REGULAR_SEQUENCER_DA.as_ref().to_vec(),
    };

    runtime
        .sequencer_registry
        .call(
            register_message,
            &C::new(REGULAR_SEQUENCER_ROLLUP),
            &mut working_set,
        )
        .unwrap();

    let blob_1 = B::new(vec![1], REGULAR_SEQUENCER_DA, [1u8; 32]);
    let blob_2 = B::new(vec![2, 2], REGULAR_SEQUENCER_DA, [2u8; 32]);
    let blob_3 = B::new(vec![3, 3, 3], PREFERRED_SEQUENCER_DA, [3u8; 32]);

    let slot_1_blobs = vec![blob_1.clone(), blob_2.clone(), blob_3.clone()];

    let mut slot_1_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [1; 32].into(),
            height: 1,
        },
        validity_cond: Default::default(),
        blobs: slot_1_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_1_data.header,
        &slot_1_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_1 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_1_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(3, execute_in_slot_1.len());
    blobs_are_equal(blob_1, execute_in_slot_1.remove(0), "slot 1");
    blobs_are_equal(blob_2, execute_in_slot_1.remove(0), "slot 1");
    blobs_are_equal(blob_3, execute_in_slot_1.remove(0), "slot 1");

    let mut slot_2_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_1_data.header.hash,
            hash: [2; 32].into(),
            height: 2,
        },
        validity_cond: Default::default(),
        blobs: Vec::new(),
    };
    runtime.chain_state.begin_slot_hook(
        &slot_2_data.header,
        &slot_2_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let execute_in_slot_2: Vec<BlobRefOrOwned<'_, B>> =
        <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
            &runtime.blob_storage,
            &mut slot_2_data.blobs,
            &mut working_set,
        )
        .unwrap();
    assert!(execute_in_slot_2.is_empty());
}

#[test]
fn deferred_blobs_are_first_after_preferred_sequencer_exit() {
    let (app, runtime, genesis_root) = TestRuntime::pre_initialized(true);
    let mut working_set = WorkingSet::new(app.current_storage.clone());

    let register_message = sov_sequencer_registry::CallMessage::Register {
        da_address: REGULAR_SEQUENCER_DA.as_ref().to_vec(),
    };
    runtime
        .sequencer_registry
        .call(
            register_message,
            &C::new(REGULAR_SEQUENCER_ROLLUP),
            &mut working_set,
        )
        .unwrap();

    let blob_1 = B::new(vec![1], REGULAR_SEQUENCER_DA, [1u8; 32]);
    let blob_2 = B::new(vec![2, 2], REGULAR_SEQUENCER_DA, [2u8; 32]);
    let blob_3 = B::new(vec![3, 3, 3], PREFERRED_SEQUENCER_DA, [3u8; 32]);
    let blob_4 = B::new(vec![4, 4, 4, 4], REGULAR_SEQUENCER_DA, [4u8; 32]);
    let blob_5 = B::new(vec![5, 5, 5, 5, 5], REGULAR_SEQUENCER_DA, [5u8; 32]);

    let slot_1_blobs = vec![blob_1.clone(), blob_2.clone(), blob_3.clone()];
    let slot_2_blobs = vec![blob_4.clone(), blob_5.clone()];

    let mut slot_1_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: [0; 32].into(),
            hash: [1; 32].into(),
            height: 1,
        },
        validity_cond: Default::default(),
        blobs: slot_1_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_1_data.header,
        &slot_1_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_1 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_1_data.blobs,
        &mut working_set,
    )
    .unwrap();

    assert_eq!(1, execute_in_slot_1.len());
    blobs_are_equal(blob_3, execute_in_slot_1.remove(0), "slot 1");

    let exit_message = sov_sequencer_registry::CallMessage::Exit {
        da_address: PREFERRED_SEQUENCER_DA.as_ref().to_vec(),
    };

    runtime
        .sequencer_registry
        .call(
            exit_message,
            &C::new(PREFERRED_SEQUENCER_ROLLUP),
            &mut working_set,
        )
        .unwrap();

    assert!(runtime
        .sequencer_registry
        .get_preferred_sequencer(&mut working_set)
        .is_none());

    let mut slot_2_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_1_data.header.hash,
            hash: [2; 32].into(),
            height: 2,
        },
        validity_cond: Default::default(),
        blobs: slot_2_blobs,
    };
    runtime.chain_state.begin_slot_hook(
        &slot_2_data.header,
        &slot_2_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let mut execute_in_slot_2 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
        &runtime.blob_storage,
        &mut slot_2_data.blobs,
        &mut working_set,
    )
    .unwrap();
    assert_eq!(4, execute_in_slot_2.len());
    blobs_are_equal(blob_1, execute_in_slot_2.remove(0), "slot 2");
    blobs_are_equal(blob_2, execute_in_slot_2.remove(0), "slot 2");
    blobs_are_equal(blob_4, execute_in_slot_2.remove(0), "slot 2");
    blobs_are_equal(blob_5, execute_in_slot_2.remove(0), "slot 2");

    let mut slot_3_data = MockBlock {
        header: MockBlockHeader {
            prev_hash: slot_2_data.header.hash,
            hash: [3; 32].into(),
            height: 3,
        },
        validity_cond: Default::default(),
        blobs: Vec::new(),
    };
    runtime.chain_state.begin_slot_hook(
        &slot_3_data.header,
        &slot_3_data.validity_cond,
        &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
        &mut working_set,
    );
    let execute_in_slot_3: Vec<BlobRefOrOwned<'_, B>> =
        <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
            &runtime.blob_storage,
            &mut slot_3_data.blobs,
            &mut working_set,
        )
        .unwrap();
    assert!(execute_in_slot_3.is_empty());
}

/// Check hashes and data of two blobs.
fn blobs_are_equal<B: BlobReaderTrait>(
    mut expected: B,
    mut actual: BlobRefOrOwned<B>,
    slot_hint: &str,
) {
    let actual_inner = actual.as_mut_ref();
    assert_eq!(
        expected.hash(),
        actual_inner.hash(),
        "incorrect hashes in {}",
        slot_hint
    );

    assert_eq!(
        actual_inner.full_data(),
        expected.full_data(),
        "incorrect data read in {}",
        slot_hint
    );
}

fn blob_hashes_are_equal<B: BlobReaderTrait>(
    expected: B,
    mut actual: BlobRefOrOwned<B>,
    slot_hint: &str,
) {
    let actual_inner = actual.as_mut_ref();
    assert_eq!(
        expected.hash(),
        actual_inner.hash(),
        "incorrect hashes in {}",
        slot_hint
    );
}

/// A utility struct to allow easy expected ordering of blobs
#[derive(PartialEq, Eq, Ord)]
struct BlobWithAppearance<B> {
    blob: B,
    appeared_in_slot: u64,
    is_from_preferred: bool,
}

impl<B> BlobWithAppearance<B> {
    pub fn gets_processed_in(&self) -> u64 {
        if self.is_from_preferred {
            self.appeared_in_slot
        } else {
            self.appeared_in_slot + DEFERRED_SLOTS_COUNT
        }
    }
}

impl<B: PartialEq> PartialOrd for BlobWithAppearance<B> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.gets_processed_in()
            .partial_cmp(&other.gets_processed_in())
    }
}

#[test]
fn test_blob_priority_sorting() {
    let blob1 = BlobWithAppearance {
        blob: [0u8],
        appeared_in_slot: 1,
        is_from_preferred: true,
    };

    let blob2 = BlobWithAppearance {
        blob: [0u8],
        appeared_in_slot: 1,
        is_from_preferred: false,
    };

    let mut blobs = vec![blob2, blob1];
    assert!(!blobs[0].is_from_preferred);
    blobs.sort();
    if DEFERRED_SLOTS_COUNT != 0 {
        assert!(blobs[0].is_from_preferred);
    }
}

#[derive(sov_modules_api::Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
struct TestRuntime<C: Context, Da: DaSpec> {
    pub bank: sov_bank::Bank<C>,
    pub accounts: sov_accounts::Accounts<C>,
    pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<C, Da>,
    pub chain_state: sov_chain_state::ChainState<C, Da>,
    pub blob_storage: sov_blob_storage::BlobStorage<C, Da>,
}

impl<C: Context, Da: DaSpec> Runtime<C, Da> for TestRuntime<C, Da> {}

impl<C: Context, Da: DaSpec> BlobSelector<Da> for TestRuntime<C, Da> {
    type Context = C;

    fn get_blobs_for_this_slot<'a, I>(
        &self,
        current_blobs: I,
        working_set: &mut WorkingSet<Self::Context>,
    ) -> anyhow::Result<Vec<BlobRefOrOwned<'a, <Da as DaSpec>::BlobTransaction>>>
    where
        I: IntoIterator<Item = &'a mut <Da as DaSpec>::BlobTransaction>,
    {
        todo!()
    }
}

impl<C: Context, Da: DaSpec> TxHooks for TestRuntime<C, Da> {
    type Context = C;

    fn pre_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<<Self::Context as Spec>::Address> {
        // Before executing a transaction, retrieve the sender's address from the accounts module
        // and check the nonce
        self.accounts.pre_dispatch_tx_hook(tx, working_set)
    }

    fn post_dispatch_tx_hook(
        &self,
        tx: &Transaction<Self::Context>,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        // After executing each transaction, update the nonce
        self.accounts.post_dispatch_tx_hook(tx, working_set)
    }
}

impl<C: Context, Da: DaSpec> ApplyBlobHooks<Da::BlobTransaction> for TestRuntime<C, Da> {
    type Context = C;
    type BlobResult =
        SequencerOutcome<<<Da as DaSpec>::BlobTransaction as BlobReaderTrait>::Address>;

    fn begin_blob_hook(
        &self,
        blob: &mut Da::BlobTransaction,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        // Before executing each batch, check that the sender is regsitered as a sequencer
        self.sequencer_registry.begin_blob_hook(blob, working_set)
    }

    fn end_blob_hook(
        &self,
        result: Self::BlobResult,
        working_set: &mut WorkingSet<C>,
    ) -> anyhow::Result<()> {
        // After processing each blob, reward or slash the sequencer if appropriate
        match result {
            SequencerOutcome::Rewarded(_reward) => {
                // TODO: Process reward here or above.
                <SequencerRegistry<C, Da> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
                    &self.sequencer_registry,
                    sov_sequencer_registry::SequencerOutcome::Completed,
                    working_set,
                )
            }
            SequencerOutcome::Ignored => Ok(()),
            SequencerOutcome::Slashed {
                reason,
                sequencer_da_address,
            } => <SequencerRegistry<C, Da> as ApplyBlobHooks<Da::BlobTransaction>>::end_blob_hook(
                &self.sequencer_registry,
                sov_sequencer_registry::SequencerOutcome::Slashed {
                    sequencer: sequencer_da_address,
                },
                working_set,
            ),
        }
    }
}

impl<C: Context, Da: DaSpec> SlotHooks<Da> for TestRuntime<C, Da> {
    type Context = C;

    fn begin_slot_hook(
        &self,
        _slot_header: &Da::BlockHeader,
        _validity_condition: &Da::ValidityCondition,
        _pre_state_root: &<<Self::Context as Spec>::Storage as Storage>::Root,
        _working_set: &mut sov_modules_api::WorkingSet<C>,
    ) {
    }

    fn end_slot_hook(&self, _working_set: &mut sov_modules_api::WorkingSet<C>) {}
}

impl<C: Context, Da: sov_modules_api::DaSpec> FinalizeHook<Da> for TestRuntime<C, Da> {
    type Context = C;

    fn finalize_hook(
        &self,
        _root_hash: &<<Self::Context as Spec>::Storage as Storage>::Root,
        _accessory_working_set: &mut AccessoryWorkingSet<C>,
    ) {
    }
}

impl TestRuntime<DefaultContext, MockDaSpec> {
    pub fn pre_initialized(
        with_preferred_sequencer: bool,
    ) -> (
        AppTemplate<DefaultContext, MockDaSpec, MockZkvm, Self>,
        Self,
        jmt::RootHash,
    ) {
        use sov_rollup_interface::stf::StateTransitionFunction;
        let tmpdir = tempfile::tempdir().unwrap();
        let storage = ProverStorage::with_path(tmpdir.path()).unwrap();

        let genesis_config = Self::build_genesis_config(with_preferred_sequencer);
        let mut app = AppTemplate::new(storage, Self::default());
        let root = app.init_chain(genesis_config);
        (app, Default::default(), root)
    }

    fn build_genesis_config(
        with_preferred_sequencer: bool,
    ) -> GenesisConfig<DefaultContext, MockDaSpec> {
        let bank_config = get_bank_config(PREFERRED_SEQUENCER_ROLLUP, REGULAR_SEQUENCER_ROLLUP);

        let token_address = sov_bank::get_genesis_token_address::<C>(
            &bank_config.tokens[0].token_name,
            bank_config.tokens[0].salt,
        );

        let sequencer_registry_config = SequencerConfig {
            seq_rollup_address: PREFERRED_SEQUENCER_ROLLUP,
            seq_da_address: PREFERRED_SEQUENCER_DA,
            coins_to_lock: sov_bank::Coins {
                amount: LOCKED_AMOUNT,
                token_address,
            },
            is_preferred_sequencer: with_preferred_sequencer,
        };

        let initial_slot_height = 0;
        let chain_state_config = ChainStateConfig {
            initial_slot_height,
            current_time: Default::default(),
        };

        let accounts_config = AccountConfig { pub_keys: vec![] };

        GenesisConfig {
            bank: bank_config,
            accounts: accounts_config,
            sequencer_registry: sequencer_registry_config,
            chain_state: chain_state_config,
            blob_storage: (),
        }
    }
}
