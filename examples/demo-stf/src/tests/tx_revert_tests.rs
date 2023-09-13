use sov_accounts::query::Response;
use sov_data_generators::bank_data::{get_default_private_key, get_default_token_address};
use sov_data_generators::{has_tx_events, new_test_blob_from_batch};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::PrivateKey;
use sov_modules_stf_template::{Batch, SequencerOutcome, SlashingReason, TxEffect};
use sov_rollup_interface::da::BlobReaderTrait;
use sov_rollup_interface::mocks::{MockBlock, MockDaSpec};
use sov_rollup_interface::stf::StateTransitionFunction;
use sov_state::{ProverStorage, WorkingSet};

use super::create_new_demo;
use crate::genesis_config::{create_demo_config, DEMO_SEQUENCER_DA_ADDRESS, LOCKED_AMOUNT};
use crate::runtime::Runtime;
use crate::tests::da_simulation::{
    simulate_da_with_bad_nonce, simulate_da_with_bad_serialization, simulate_da_with_bad_sig,
    simulate_da_with_revert_msg,
};

const SEQUENCER_BALANCE_DELTA: u64 = 1;
const SEQUENCER_BALANCE: u64 = LOCKED_AMOUNT + SEQUENCER_BALANCE_DELTA;
// Assume there was proper address and we converted it to bytes already.
const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];

#[test]
fn test_tx_revert() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(SEQUENCER_BALANCE, &admin_private_key);
    let sequencer_rollup_address = config.sequencer_registry.seq_rollup_address;

    {
        let mut demo = create_new_demo(path);
        // TODO: Maybe complete with actual block data
        let _data = MockBlock::default();
        demo.init_chain(config);

        let txs = simulate_da_with_revert_msg();
        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let mut blobs = [blob];
        let data = MockBlock::default();

        let apply_block_result = demo.apply_slot(
            Default::default(),
            &data.header,
            &data.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Unexpected outcome: Batch execution should have succeeded",
        );

        let txn_receipts = apply_block_result.batch_receipts[0].tx_receipts.clone();
        // 3 transactions
        // create 1000 tokens
        // transfer 15 tokens
        // transfer 5000 tokens // this should be reverted
        assert_eq!(txn_receipts[0].receipt, TxEffect::Successful);
        assert_eq!(txn_receipts[1].receipt, TxEffect::Successful);
        assert_eq!(txn_receipts[2].receipt, TxEffect::Reverted);
    }

    // Checks
    {
        let runtime = &mut Runtime::<DefaultContext, MockDaSpec>::default();
        let storage = ProverStorage::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage);
        let resp = runtime
            .bank
            .balance_of(
                get_default_private_key().default_address(),
                get_default_token_address(),
                &mut working_set,
            )
            .unwrap();

        assert_eq!(resp.amount, Some(985));

        let resp = runtime
            .sequencer_registry
            .sequencer_address(DEMO_SEQUENCER_DA_ADDRESS.to_vec(), &mut working_set)
            .unwrap();
        // Sequencer is not excluded from list of allowed!
        assert_eq!(Some(sequencer_rollup_address), resp.address);
    }
}

#[test]
fn test_nonce_incremented_on_revert() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(SEQUENCER_BALANCE, &admin_private_key);

    {
        let mut demo = create_new_demo(path);
        // TODO: Maybe complete with actual block data
        let _data = MockBlock::default();
        demo.init_chain(config);

        let txs = simulate_da_with_revert_msg();
        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let mut blobs = [blob];
        let data = MockBlock::default();

        let apply_block_result = demo.apply_slot(
            Default::default(),
            &data.header,
            &data.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

        assert_eq!(
            SequencerOutcome::Rewarded(0),
            apply_blob_outcome.inner,
            "Unexpected outcome: Batch execution should have succeeded",
        );

        let txn_receipts = apply_block_result.batch_receipts[0].tx_receipts.clone();
        // 3 transactions
        // create 1000 tokens
        // transfer 15 tokens
        // transfer 5000 tokens // this should be reverted
        assert_eq!(txn_receipts[0].receipt, TxEffect::Successful);
        assert_eq!(txn_receipts[1].receipt, TxEffect::Successful);
        assert_eq!(txn_receipts[2].receipt, TxEffect::Reverted);
    }

    // with 3 transactions, the final nonce should be 3
    // 0 -> 1
    // 1 -> 2
    // 2 -> 3
    {
        let runtime = &mut Runtime::<DefaultContext, MockDaSpec>::default();
        let storage = ProverStorage::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage);
        let nonce = match runtime
            .accounts
            .get_account(get_default_private_key().pub_key(), &mut working_set)
            .unwrap()
        {
            Response::AccountExists { nonce, .. } => nonce,
            Response::AccountEmpty => 0,
        };

        // minter account should have its nonce increased for 3 transactions
        assert_eq!(3, nonce);
    }
}

#[test]
fn test_tx_bad_sig() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(SEQUENCER_BALANCE, &admin_private_key);

    {
        let mut demo = create_new_demo(path);
        // TODO: Maybe complete with actual block data
        let _data = MockBlock::default();
        demo.init_chain(config);

        let txs = simulate_da_with_bad_sig();

        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let blob_sender = blob.sender();
        let mut blobs = [blob];

        let data = MockBlock::default();
        let apply_block_result = demo.apply_slot(
            Default::default(),
            &data.header,
            &data.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

        assert_eq!(
            SequencerOutcome::Slashed{
                reason:SlashingReason::StatelessVerificationFailed,
                sequencer_da_address: blob_sender,
            },
            apply_blob_outcome.inner,
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));
    }
}

#[test]
fn test_tx_bad_nonce() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();
    let admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(SEQUENCER_BALANCE, &admin_private_key);

    {
        let mut demo = create_new_demo(path);
        // TODO: Maybe complete with actual block data
        let _data = MockBlock::default();
        demo.init_chain(config);

        let txs = simulate_da_with_bad_nonce();

        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let mut blobs = [blob];

        let data = MockBlock::default();
        let apply_block_result = demo.apply_slot(
            Default::default(),
            &data.header,
            &data.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let tx_receipts = apply_block_result.batch_receipts[0].tx_receipts.clone();
        // Bad nonce means that the transaction has to be reverted
        assert_eq!(tx_receipts[0].receipt, TxEffect::Reverted);

        // We don't expect the sequencer to be slashed for a bad nonce
        // The reason for this is that in cases such as based sequencing, the sequencer can
        // still post under the assumption that the nonce is valid (It doesn't know other sequencers
        // are also doing this) so it needs to be rewarded.
        // We're asserting that here to track if the logic changes
        assert_eq!(
            apply_block_result.batch_receipts[0].inner,
            SequencerOutcome::Rewarded(0)
        );
    }
}

#[test]
fn test_tx_bad_serialization() {
    let tempdir = tempfile::tempdir().unwrap();
    let path = tempdir.path();

    let value_setter_admin_private_key = DefaultPrivateKey::generate();

    let config = create_demo_config(SEQUENCER_BALANCE, &value_setter_admin_private_key);
    let sequencer_rollup_address = config.sequencer_registry.seq_rollup_address;
    let sequencer_balance_before = {
        let mut demo = create_new_demo(path);
        demo.init_chain(config);

        let mut working_set = WorkingSet::new(demo.current_storage);
        let coins = demo
            .runtime
            .sequencer_registry
            .get_coins_to_lock(&mut working_set)
            .unwrap();

        demo.runtime
            .bank
            .get_balance_of(
                sequencer_rollup_address,
                coins.token_address,
                &mut working_set,
            )
            .unwrap()
    };

    {
        // TODO: Maybe complete with actual block data
        let _data = MockBlock::default();

        let mut demo = create_new_demo(path);

        let txs = simulate_da_with_bad_serialization();
        let blob = new_test_blob_from_batch(Batch { txs }, &DEMO_SEQUENCER_DA_ADDRESS, [0; 32]);
        let blob_sender = blob.sender();
        let mut blobs = [blob];

        let data = MockBlock::default();
        let apply_block_result = demo.apply_slot(
            Default::default(),
            &data.header,
            &data.validity_cond,
            &mut blobs,
        );

        assert_eq!(1, apply_block_result.batch_receipts.len());
        let apply_blob_outcome = apply_block_result.batch_receipts[0].clone();

        assert_eq!(
            SequencerOutcome::Slashed {
                reason: SlashingReason::InvalidTransactionEncoding ,
                sequencer_da_address: blob_sender,
            },
            apply_blob_outcome.inner,
            "Unexpected outcome: Stateless verification should have failed due to invalid signature"
        );

        // The batch receipt contains no events.
        assert!(!has_tx_events(&apply_blob_outcome));
    }

    {
        let runtime = &mut Runtime::<DefaultContext, MockDaSpec>::default();
        let storage = ProverStorage::with_path(path).unwrap();
        let mut working_set = WorkingSet::new(storage);

        // Sequencer is not in the list of allowed sequencers

        let allowed_sequencer = runtime
            .sequencer_registry
            .sequencer_address(SEQUENCER_DA_ADDRESS.to_vec(), &mut working_set)
            .unwrap();
        assert!(allowed_sequencer.address.is_none());

        // Balance of sequencer is not increased
        let coins = runtime
            .sequencer_registry
            .get_coins_to_lock(&mut working_set)
            .unwrap();
        let sequencer_balance_after = runtime
            .bank
            .get_balance_of(
                sequencer_rollup_address,
                coins.token_address,
                &mut working_set,
            )
            .unwrap();
        assert_eq!(sequencer_balance_before, sequencer_balance_after);
    }
}
