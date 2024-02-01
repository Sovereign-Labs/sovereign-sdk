// use sov_bank::TokenConfig;
// use sov_blob_storage::{BlobStorage, DEFERRED_SLOTS_COUNT};
// use sov_chain_state::ChainStateConfig;
// use sov_mock_da::{MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaSpec};
// use sov_modules_api::da::Time;
// use sov_modules_api::default_context::DefaultContext;
// use sov_modules_api::hooks::SlotHooks;
// use sov_modules_api::macros::DefaultRuntime;
// use sov_modules_api::runtime::capabilities::{BlobRefOrOwned, BlobSelector};
// use sov_modules_api::{
//     Address, BlobReaderTrait, Context, DaSpec, DispatchCall, MessageCodec, Module, Spec, WorkingSet,
// };
// use sov_prover_storage_manager::{new_orphan_storage, SnapshotManager};
// use sov_sequencer_registry::SequencerConfig;
// use sov_state::{DefaultStorageSpec, ProverStorage, Storage};
// TODO: Re-enable these tests

// type C = DefaultContext;
// type B = MockBlob;
// type Da = MockDaSpec;

// const LOCKED_AMOUNT: u64 = 200;
// const PREFERRED_SEQUENCER_DA: MockAddress = MockAddress::new([10u8; 32]);
// const PREFERRED_SEQUENCER_ROLLUP: Address = Address::new(*b"preferred_______________________");
// const REGULAR_SEQUENCER_DA: MockAddress = MockAddress::new([30u8; 32]);
// const REGULAR_SEQUENCER_ROLLUP: Address = Address::new(*b"regular_________________________");
// const REGULAR_REWARD_ROLLUP: Address = Address::new(*b"regular_reward__________________");

// fn get_bank_config(
//     preferred_sequencer: <C as Spec>::Address,
//     regular_sequencer: <C as Spec>::Address,
// ) -> sov_bank::BankConfig<C> {
//     let token_config: TokenConfig<C> = TokenConfig {
//         token_name: "InitialToken".to_owned(),
//         address_and_balances: vec![
//             (preferred_sequencer, LOCKED_AMOUNT * 3),
//             (regular_sequencer, LOCKED_AMOUNT * 3),
//         ],
//         authorized_minters: vec![],
//         salt: 9,
//     };

//     sov_bank::BankConfig {
//         tokens: vec![token_config],
//     }
// }

// fn make_blobs(
//     blob_num: &mut u8,
//     slot: u64,
//     senders_are_preferred: impl Iterator<Item = bool>,
// ) -> Vec<BlobWithAppearance<MockBlob>> {
//     let blobs: Vec<_> = senders_are_preferred
//         .enumerate()
//         .map(|(offset, is_preferred)| {
//             let sender = if is_preferred {
//                 PREFERRED_SEQUENCER_DA
//             } else {
//                 REGULAR_SEQUENCER_DA
//             };

//             BlobWithAppearance {
//                 blob: B::new(vec![], sender, [*blob_num + offset as u8; 32]),
//                 appeared_in_slot: slot,
//                 is_from_preferred: is_preferred,
//             }
//         })
//         .collect();
//     *blob_num += blobs.len() as u8;
//     blobs
// }

// fn make_blobs_by_slot(
//     is_from_preferred_by_slot: &[Vec<bool>],
// ) -> Vec<Vec<BlobWithAppearance<MockBlob>>> {
//     let mut blob_num = 0;
//     is_from_preferred_by_slot
//         .iter()
//         .enumerate()
//         .map(|(slot, senders)| make_blobs(&mut blob_num, slot as u64, senders.iter().cloned()))
//         .collect()
// }

// #[test]
// fn priority_sequencer_flow_general() {
//     let is_from_preferred_by_slot = [
//         vec![false, false, true],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     do_deferred_blob_test(blobs_by_slot, vec![])
// }

// pub struct SlotTestInfo {
//     pub slot_number: u64,
//     /// Any "requests for early processing" to be sent during this slot
//     pub early_processing_request_with_sender:
//         Option<(sov_blob_storage::CallMessage, Address, Address)>,
//     /// The expected number of blobs to process, if known
//     pub expected_blobs_to_process: Option<usize>,
// }

// // Tests of the "blob deferral" logic tend to have the same structure, which is encoded in this helper:
// // 1. Initialize the rollup
// // 2. Calculate the expected order of blobs to be processed
// // 3. In a loop...
// //   (Optionally) Assert that the correct number of blobs has been processed that slot
// //   (Optionally) Request early processing of some blobs in the next slot
// //   Assert that blobs are pulled out of the queue in the expected order
// // 4. Assert that all blobs have been processed
// fn do_deferred_blob_test(
//     blobs_by_slot: Vec<Vec<BlobWithAppearance<MockBlob>>>,
//     test_info: Vec<SlotTestInfo>,
// ) {
//     let num_slots = blobs_by_slot.len();
//     // Initialize the rollup
//     let (current_storage, runtime, genesis_root) = TestRuntime::pre_initialized(true);
//     let mut working_set = WorkingSet::new(current_storage.clone());

//     // Compute the *expected* order of blob processing.
//     let mut expected_blobs = blobs_by_slot.iter().flatten().cloned().collect::<Vec<_>>();
//     expected_blobs.sort_by_key(|b| b.priority());
//     let mut expected_blobs = expected_blobs.into_iter();
//     let mut slots_iterator = blobs_by_slot
//         .into_iter()
//         .map(|blobs| blobs.into_iter().map(|b| b.blob).collect())
//         .chain(std::iter::repeat(Vec::new()));

//     let mut test_info = test_info.into_iter().peekable();
//     let mut has_processed_blobs_early = false;

//     // Loop  enough times that all provided slots are processed and all deferred blobs expire
//     for slot_number in 0..num_slots as u64 + DEFERRED_SLOTS_COUNT {
//         // Run the blob selector module
//         let slot_number_u8 = slot_number as u8;
//         let mut slot_data = MockBlock {
//             header: MockBlockHeader {
//                 prev_hash: [slot_number_u8; 32].into(),
//                 hash: [slot_number_u8 + 1; 32].into(),
//                 height: slot_number,
//                 time: Time::now(),
//             },
//             validity_cond: Default::default(),
//             blobs: slots_iterator.next().unwrap(),
//         };
//         runtime.chain_state.begin_slot_hook(
//             &slot_data.header,
//             &slot_data.validity_cond,
//             &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
//             &mut working_set,
//         );
//         let blobs_to_execute = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
//             &runtime.blob_storage,
//             &mut slot_data.blobs,
//             &mut working_set,
//         )
//         .unwrap();

//         // Run any extra logic provided by the test for this slot
//         if let Some(next_slot_info) = test_info.peek() {
//             if next_slot_info.slot_number == slot_number {
//                 let next_slot_info = test_info.next().unwrap();
//                 // If applicable, assert that the expected number of blobs was processed
//                 if let Some(expected) = next_slot_info.expected_blobs_to_process {
//                     assert_eq!(expected, blobs_to_execute.len())
//                 }

//                 // If applicable, send the requested call message to the blob_storage module
//                 if let Some((msg, sender, sequencer)) =
//                     next_slot_info.early_processing_request_with_sender
//                 {
//                     runtime
//                         .blob_storage
//                         .call(
//                             msg,
//                             &DefaultContext::new(sender, sequencer, slot_number),
//                             &mut working_set,
//                         )
//                         .unwrap();
//                     has_processed_blobs_early = true;
//                 }
//             }
//         }

//         // Check that the computed list of blobs is the one we expected
//         for blob in blobs_to_execute {
//             let expected: BlobWithAppearance<MockBlob> = expected_blobs.next().unwrap();
//             if !has_processed_blobs_early {
//                 assert_eq!(expected.must_be_processed_by(), slot_number);
//             }
//             assert_blobs_are_equal(expected.blob, blob, &format!("Slot {}", slot_number));
//         }
//     }
//     // Ensure that all blobs have been processed
//     assert!(expected_blobs.next().is_none());
// }

// #[test]
// fn bonus_blobs_are_delivered_on_request() {
//     // If blobs are deferred for less than two slots, "early processing" is not possible
//     if DEFERRED_SLOTS_COUNT < 2 {
//         return;
//     }

//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = vec![
//         SlotTestInfo {
//             slot_number: 0,
//             expected_blobs_to_process: Some(1), // The first slot will process the one blob from the preferred sequencer
//             early_processing_request_with_sender: Some((
//                 sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 4 },
//                 PREFERRED_SEQUENCER_ROLLUP,
//                 REGULAR_REWARD_ROLLUP,
//             )),
//         },
//         SlotTestInfo {
//             slot_number: 1,
//             expected_blobs_to_process: Some(5), // The second slot will process four bonus blobs plus the one from the preferred sequencer
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: 2,
//             expected_blobs_to_process: Some(0), // The third slot won't process any blobs
//             early_processing_request_with_sender: None,
//         },
//     ];

//     do_deferred_blob_test(blobs_by_slot, test_info)
// }

// #[test]
// fn test_deferrable_with_small_count() {
//     // If blobs are deferred for less than two slots ensure that "early" processing requests do not alter
//     // the order of blob processing
//     if DEFERRED_SLOTS_COUNT > 1 {
//         return;
//     }

//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = if DEFERRED_SLOTS_COUNT == 1 {
//         vec![
//             SlotTestInfo {
//                 slot_number: 0,
//                 expected_blobs_to_process: Some(1), // The first slot will process the one blob from the preferred sequencer
//                 early_processing_request_with_sender: Some((
//                     sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 8 },
//                     PREFERRED_SEQUENCER_ROLLUP,
//                     REGULAR_REWARD_ROLLUP,
//                 )),
//             },
//             SlotTestInfo {
//                 slot_number: 1,
//                 expected_blobs_to_process: Some(7), // The second slot will process seven bonus blobs plus the one from the preferred sequencer
//                 early_processing_request_with_sender: None,
//             },
//             SlotTestInfo {
//                 slot_number: 2,
//                 expected_blobs_to_process: Some(0), // The third slot won't process any blobs
//                 early_processing_request_with_sender: None,
//             },
//         ]
//     } else {
//         // If the deferred slots count is 0, all blobs are processed as soon as they become available.
//         vec![
//             SlotTestInfo {
//                 slot_number: 0,
//                 expected_blobs_to_process: Some(5),
//                 early_processing_request_with_sender: Some((
//                     sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 4 },
//                     PREFERRED_SEQUENCER_ROLLUP,
//                     REGULAR_REWARD_ROLLUP,
//                 )),
//             },
//             SlotTestInfo {
//                 slot_number: 1,
//                 expected_blobs_to_process: Some(3),
//                 early_processing_request_with_sender: None,
//             },
//             SlotTestInfo {
//                 slot_number: 2,
//                 expected_blobs_to_process: Some(2),
//                 early_processing_request_with_sender: None,
//             },
//         ]
//     };

//     do_deferred_blob_test(blobs_by_slot, test_info)
// }

// // cases to handle:
// // 1. Happy flow (with some bonus blobs)
// // 2. Preferred sequencer exits
// // 3. Too many bonus blobs requested
// // 4. Bonus blobs requested just once
// // 5. Bonus blob requests ignored if not preferred seq
// #[test]
// fn sequencer_requests_more_bonus_blobs_than_possible() {
//     // If blobs are deferred for less than two slots, "early processing" is not possible
//     if DEFERRED_SLOTS_COUNT < 2 {
//         return;
//     }

//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = vec![
//         SlotTestInfo {
//             slot_number: 0,
//             expected_blobs_to_process: Some(1), // The first slot will process the one blob from the preferred sequencer
//             early_processing_request_with_sender: Some((
//                 sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 1000 }, // Request a huge number of blobs
//                 PREFERRED_SEQUENCER_ROLLUP,
//                 REGULAR_REWARD_ROLLUP,
//             )),
//         },
//         SlotTestInfo {
//             slot_number: 1,
//             expected_blobs_to_process: Some(7), // The second slot will process all 7 available blobs and then halt
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: 2,
//             expected_blobs_to_process: Some(0), // The third slot won't process any blobs, since none are from the preferred sequencer
//             early_processing_request_with_sender: None,
//         },
//     ];

//     do_deferred_blob_test(blobs_by_slot, test_info)
// }

// // This test ensure that blob storage behaves as expected when it only needs to process a subset of the
// // deferred blobs from a slot.
// #[test]
// fn some_blobs_from_slot_processed_early() {
//     // If blobs are deferred for less than two slots, "early processing" is not possible
//     if DEFERRED_SLOTS_COUNT < 2 {
//         return;
//     }

//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = vec![
//         SlotTestInfo {
//             slot_number: 0,
//             // The first slot will process the one blob from the preferred sequencer
//             expected_blobs_to_process: Some(1),
//             early_processing_request_with_sender: Some((
//                 sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 5 }, // Request 5 bonus blobs
//                 PREFERRED_SEQUENCER_ROLLUP,
//                 REGULAR_REWARD_ROLLUP,
//             )),
//         },
//         SlotTestInfo {
//             slot_number: 1,
//             expected_blobs_to_process: Some(6), // The second slot will process 5 bonus blobs plus the one from the preferred sequencer. One blob from slot two will be deferred again
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: 2,
//             expected_blobs_to_process: Some(0), // The third slot won't process any blobs
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT + 1,
//             expected_blobs_to_process: Some(1), // We process that one re-deferred bob in slot `DEFERRED_SLOTS_COUNT + 1`
//             early_processing_request_with_sender: None,
//         },
//     ];

//     do_deferred_blob_test(blobs_by_slot, test_info)
// }

// #[test]
// fn request_one_blob_early() {
//     // If blobs are deferred for less than two slots, "early processing" is not possible
//     if DEFERRED_SLOTS_COUNT < 2 {
//         return;
//     }

//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = vec![
//         SlotTestInfo {
//             slot_number: 0,
//             // The first slot will process the one blob from the preferred sequencer
//             expected_blobs_to_process: Some(1),
//             early_processing_request_with_sender: Some((
//                 sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 1 }, // Request 1 bonus blob
//                 PREFERRED_SEQUENCER_ROLLUP,
//                 REGULAR_REWARD_ROLLUP,
//             )),
//         },
//         SlotTestInfo {
//             slot_number: 1,
//             expected_blobs_to_process: Some(2), // The second slot will process 1 bonus blob plus the one from the preferred sequencer. Three blobs from slot one will be deferred again
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT,
//             expected_blobs_to_process: Some(3), // We process the 3 re-deferred blobs from slot 0 in slot `DEFERRED_SLOTS_COUNT`
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT + 1,
//             expected_blobs_to_process: Some(2), // We process that two deferred blobs from slot 1 in slot `DEFERRED_SLOTS_COUNT + 1`
//             early_processing_request_with_sender: None,
//         },
//     ];
//     do_deferred_blob_test(blobs_by_slot, test_info)
// }

// #[test]
// fn bonus_blobs_request_ignored_if_not_from_preferred_seq() {
//     // If blobs are deferred for less than two slots, "early processing" is not possible
//     if DEFERRED_SLOTS_COUNT < 2 {
//         return;
//     }
//     let is_from_preferred_by_slot = [
//         vec![false, false, true, false, false],
//         vec![false, true, false],
//         vec![false, false],
//     ];
//     let blobs_by_slot: Vec<_> = make_blobs_by_slot(&is_from_preferred_by_slot);
//     let test_info = vec![
//         SlotTestInfo {
//             slot_number: 0,
//             // The first slot will process the one blob from the preferred sequencer
//             expected_blobs_to_process: Some(1),
//             early_processing_request_with_sender: Some((
//                 sov_blob_storage::CallMessage::ProcessDeferredBlobsEarly { number: 1 }, // Request 1 bonus blob, but send the request from the *WRONG* address
//                 REGULAR_SEQUENCER_ROLLUP,
//                 REGULAR_REWARD_ROLLUP,
//             )),
//         },
//         SlotTestInfo {
//             slot_number: 1,
//             expected_blobs_to_process: Some(1), // The second slot will one blob from the preferred sequencer but no bonus blobs
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT,
//             expected_blobs_to_process: Some(4), // We process the 4 deferred blobs from slot 0 in slot `DEFERRED_SLOTS_COUNT`
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT + 1,
//             expected_blobs_to_process: Some(2), // We process that two deferred blobs from slot 1 in slot `DEFERRED_SLOTS_COUNT + 1`
//             early_processing_request_with_sender: None,
//         },
//         SlotTestInfo {
//             slot_number: DEFERRED_SLOTS_COUNT + 2,
//             expected_blobs_to_process: Some(2), // We process that two deferred blobs from slot 2 in slot `DEFERRED_SLOTS_COUNT + 2`
//             early_processing_request_with_sender: None,
//         },
//     ];
//     do_deferred_blob_test(blobs_by_slot, test_info);
// }

// #[test]
// fn test_blobs_from_non_registered_sequencers_are_not_saved() {
//     let (current_storage, runtime, genesis_root) = TestRuntime::pre_initialized(true);
//     let mut working_set = WorkingSet::new(current_storage.clone());

//     let unregistered_sequencer = MockAddress::from([7; 32]);
//     let blob_1 = B::new(vec![1], REGULAR_SEQUENCER_DA, [1u8; 32]);
//     let blob_2 = B::new(vec![2, 2], unregistered_sequencer, [2u8; 32]);
//     let blob_3 = B::new(vec![3, 3, 3], PREFERRED_SEQUENCER_DA, [3u8; 32]);

//     let slot_1_blobs = vec![blob_1.clone(), blob_2, blob_3.clone()];
//     let mut blobs_processed = 0;

//     for slot_number in 0..DEFERRED_SLOTS_COUNT + 1 {
//         let slot_number_u8 = slot_number as u8;
//         let mut slot_data = MockBlock {
//             header: MockBlockHeader {
//                 prev_hash: [slot_number_u8; 32].into(),
//                 hash: [slot_number_u8 + 1; 32].into(),
//                 height: slot_number,
//                 time: Time::now(),
//             },
//             validity_cond: Default::default(),
//             blobs: if slot_number == 0 {
//                 slot_1_blobs.clone()
//             } else {
//                 vec![]
//             },
//         };
//         runtime.chain_state.begin_slot_hook(
//             &slot_data.header,
//             &slot_data.validity_cond,
//             &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
//             &mut working_set,
//         );
//         let blobs_to_execute = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
//             &runtime.blob_storage,
//             &mut slot_data.blobs,
//             &mut working_set,
//         )
//         .unwrap();

//         for blob in blobs_to_execute {
//             blobs_processed += 1;
//             let sender = match blob {
//                 BlobRefOrOwned::Ref(b) => b.sender(),
//                 BlobRefOrOwned::Owned(b) => b.sender(),
//             };
//             assert_ne!(sender, unregistered_sequencer)
//         }
//     }
//     assert_eq!(blobs_processed, 2)
// }

// #[test]
// fn test_blobs_not_deferred_without_preferred_sequencer() {
//     let (current_storage, runtime, genesis_root) = TestRuntime::pre_initialized(false);
//     let mut working_set = WorkingSet::new(current_storage.clone());

//     let blob_1 = B::new(vec![1], REGULAR_SEQUENCER_DA, [1u8; 32]);
//     let blob_2 = B::new(vec![2, 2], REGULAR_SEQUENCER_DA, [2u8; 32]);
//     let blob_3 = B::new(vec![3, 3, 3], PREFERRED_SEQUENCER_DA, [3u8; 32]);

//     let slot_1_blobs = vec![blob_1.clone(), blob_2.clone(), blob_3.clone()];

//     let mut slot_1_data = MockBlock {
//         header: MockBlockHeader {
//             prev_hash: [0; 32].into(),
//             hash: [1; 32].into(),
//             height: 1,
//             time: Time::now(),
//         },
//         validity_cond: Default::default(),
//         blobs: slot_1_blobs,
//     };
//     runtime.chain_state.begin_slot_hook(
//         &slot_1_data.header,
//         &slot_1_data.validity_cond,
//         &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
//         &mut working_set,
//     );
//     let mut execute_in_slot_1 = <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
//         &runtime.blob_storage,
//         &mut slot_1_data.blobs,
//         &mut working_set,
//     )
//     .unwrap();
//     assert_eq!(3, execute_in_slot_1.len());
//     assert_blobs_are_equal(blob_1, execute_in_slot_1.remove(0), "slot 1");
//     assert_blobs_are_equal(blob_2, execute_in_slot_1.remove(0), "slot 1");
//     assert_blobs_are_equal(blob_3, execute_in_slot_1.remove(0), "slot 1");

//     let mut slot_2_data = MockBlock {
//         header: MockBlockHeader {
//             prev_hash: slot_1_data.header.hash,
//             hash: [2; 32].into(),
//             height: 2,
//             time: Time::now(),
//         },
//         validity_cond: Default::default(),
//         blobs: Vec::new(),
//     };
//     runtime.chain_state.begin_slot_hook(
//         &slot_2_data.header,
//         &slot_2_data.validity_cond,
//         &genesis_root, // For this test, we don't actually execute blocks - so keep reusing the genesis root hash as a placeholder
//         &mut working_set,
//     );
//     let execute_in_slot_2: Vec<BlobRefOrOwned<'_, B>> =
//         <BlobStorage<C, Da> as BlobSelector<Da>>::get_blobs_for_this_slot(
//             &runtime.blob_storage,
//             &mut slot_2_data.blobs,
//             &mut working_set,
//         )
//         .unwrap();
//     assert!(execute_in_slot_2.is_empty());
// }

// /// Check hashes and data of two blobs.
// fn assert_blobs_are_equal<B: BlobReaderTrait>(
//     mut expected: B,
//     mut actual: BlobRefOrOwned<B>,
//     slot_hint: &str,
// ) {
//     let actual_inner = actual.as_mut_ref();
//     assert_eq!(
//         expected.hash(),
//         actual_inner.hash(),
//         "incorrect hashes in {}",
//         slot_hint
//     );

//     assert_eq!(
//         actual_inner.full_data(),
//         expected.full_data(),
//         "incorrect data read in {}",
//         slot_hint
//     );
// }

// /// A utility struct to allow easy expected ordering of blobs
// #[derive(PartialEq, Clone)]
// struct BlobWithAppearance<B> {
//     pub blob: B,
//     appeared_in_slot: u64,
//     is_from_preferred: bool,
// }

// impl<B> BlobWithAppearance<B> {
//     pub fn must_be_processed_by(&self) -> u64 {
//         if self.is_from_preferred {
//             self.appeared_in_slot
//         } else {
//             self.appeared_in_slot + DEFERRED_SLOTS_COUNT
//         }
//     }

//     /// A helper for sorting blobs be expected order. Blobs are ordered first by the slot in which the must be processed
//     /// Then by whether they're from the preferred sequencer. (Lower score means that an item is sorted first)
//     pub fn priority(&self) -> u64 {
//         if self.is_from_preferred {
//             self.appeared_in_slot * 10
//         } else {
//             (self.appeared_in_slot + DEFERRED_SLOTS_COUNT) * 10 + 1
//         }
//     }
// }

// #[test]
// fn test_blob_priority_sorting() {
//     let blob1 = BlobWithAppearance {
//         blob: [0u8],
//         appeared_in_slot: 1,
//         is_from_preferred: true,
//     };

//     let blob2 = BlobWithAppearance {
//         blob: [0u8],
//         appeared_in_slot: 1,
//         is_from_preferred: false,
//     };

//     let mut blobs = vec![blob2, blob1];
//     assert!(!blobs[0].is_from_preferred);
//     blobs.sort_by_key(|b| b.must_be_processed_by());
//     if DEFERRED_SLOTS_COUNT == 0 {
//         assert!(blobs[1].is_from_preferred);
//     } else {
//         assert!(blobs[0].is_from_preferred);
//     }
// }

// #[derive(sov_modules_api::Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
// #[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
// struct TestRuntime<C: Context, Da: DaSpec> {
//     pub bank: sov_bank::Bank<C>,
//     pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<C, Da>,
//     pub chain_state: sov_chain_state::ChainState<C, Da>,
//     pub blob_storage: BlobStorage<C, Da>,
// }

// impl TestRuntime<DefaultContext, MockDaSpec> {
//     pub fn pre_initialized(
//         with_preferred_sequencer: bool,
//     ) -> (
//         ProverStorage<DefaultStorageSpec, SnapshotManager>,
//         Self,
//         jmt::RootHash,
//     ) {
//         use sov_modules_api::Genesis;
//         let tmpdir = tempfile::tempdir().unwrap();
//         let storage = new_orphan_storage(tmpdir.path()).unwrap();

//         let genesis_config = Self::build_genesis_config(with_preferred_sequencer);
//         let runtime: Self = Default::default();

//         let mut working_set = WorkingSet::new(storage.clone());
//         runtime.genesis(&genesis_config, &mut working_set).unwrap();

//         // In addition to "genesis", register one non-preferred sequencer
//         let register_message = sov_sequencer_registry::CallMessage::Register {
//             da_address: REGULAR_SEQUENCER_DA.as_ref().to_vec(),
//         };
//         runtime
//             .sequencer_registry
//             .call(
//                 register_message,
//                 &C::new(REGULAR_SEQUENCER_ROLLUP, REGULAR_REWARD_ROLLUP, 1),
//                 &mut working_set,
//             )
//             .unwrap();

//         let (reads_writes, witness) = working_set.checkpoint().freeze();
//         let genesis_root = storage.validate_and_commit(reads_writes, &witness).unwrap();

//         // let root = storage.validate_and_commit()
//         (storage, runtime, genesis_root)
//     }

//     fn build_genesis_config(
//         with_preferred_sequencer: bool,
//     ) -> GenesisConfig<DefaultContext, MockDaSpec> {
//         let bank_config = get_bank_config(PREFERRED_SEQUENCER_ROLLUP, REGULAR_SEQUENCER_ROLLUP);

//         let token_address = sov_bank::get_genesis_token_address::<C>(
//             &bank_config.tokens[0].token_name,
//             bank_config.tokens[0].salt,
//         );

//         let sequencer_registry_config = SequencerConfig {
//             seq_rollup_address: PREFERRED_SEQUENCER_ROLLUP,
//             seq_da_address: PREFERRED_SEQUENCER_DA,
//             coins_to_lock: sov_bank::Coins {
//                 amount: LOCKED_AMOUNT,
//                 token_address,
//             },
//             is_preferred_sequencer: with_preferred_sequencer,
//         };

//         let initial_slot_height = 0;
//         let chain_state_config = ChainStateConfig {
//             initial_slot_height,
//             current_time: Default::default(),
//         };

//         GenesisConfig {
//             bank: bank_config,
//             sequencer_registry: sequencer_registry_config,
//             chain_state: chain_state_config,
//             blob_storage: (),
//         }
//     }
// }
