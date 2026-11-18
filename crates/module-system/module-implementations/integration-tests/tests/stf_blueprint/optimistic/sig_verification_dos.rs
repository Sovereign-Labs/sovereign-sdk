//! The node must never verify a transaction signature that no sequencer has paid for.
//!
//! Signature verification is the dominant per-transaction cost of authentication. If a node
//! verifies signatures before the economic gates (sequencer escrow, slot gas check, unregistered
//! blob quota) have admitted the transaction, a sequencer can make every full node do unbounded
//! work for the price of the few transactions that actually get charged.
//!
//! Each test sends far more transactions than the gates admit and counts the ed25519
//! verifications the node actually performed.

use std::env;

use sov_mock_da::{MockAddress, MockBlob};
use sov_mock_zkvm::crypto::ed25519_verifications_performed;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::FullyBakedTx;
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::TestUser;

use super::da_simulation::simulate_da_with_multiple_direct_registration_msg;
use super::optimistic_rt::{setup, IntegTestRuntime};
use crate::stf_blueprint::{
    create_tx_bad_sig, encode, encode_message, new_test_blob_for_direct_registration,
    reset_constants, S,
};

type RT = IntegTestRuntime<S>;

/// Signature verifications a batch may perform beyond the transactions it is charged for.
///
/// Raising this (e.g. to allow bounded speculative pre-verification) is a deliberate security
/// decision: each unit is work any sequencer can force on every full node, per batch, for free.
const MAX_UNPAID_VERIFICATIONS_PER_BATCH: u64 = 0;

/// Far more transactions than any gate below admits, so verifying ahead of the gates is obvious.
const ATTACK_TXS: u64 = 500;

fn bad_sig_txs(signer: &TestUser<S>, count: u64) -> Vec<FullyBakedTx> {
    (0..count)
        .map(|nonce| {
            // Distinct nonces give distinct tx hashes, so the signature cache can't hide any work.
            encode(create_tx_bad_sig::<RT>(
                nonce,
                PriorityFeeBips::from_percentage(5),
                signer,
                encode_message::<RT>(None),
            ))
        })
        .collect()
}

fn batch_blob(txs: &[FullyBakedTx], sender: MockAddress) -> RelevantBlobs<MockBlob> {
    RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![MockBlob::new_with_hash(
            borsh::to_vec(&txs).unwrap(),
            sender,
        )],
    }
}

#[test]
fn txs_beyond_slot_gas_are_not_verified() {
    reset_constants();
    // One gas unit above the default `max_tx_check_costs` ([1e9, 1e9]): the slot admits the first
    // tx, after which remaining slot gas can no longer cover a tx check.
    env::set_var(
        "SOV_TEST_CONST_OVERRIDE_BLOCK_GAS_LIMIT",
        "[1000000001,1000000001]",
    );
    let (mut runner, users, _) = setup(1);

    let txs = bad_sig_txs(&users[0], ATTACK_TXS);

    let verifications_before = ed25519_verifications_performed();
    let result = runner.execute(batch_blob(&txs, runner.config.sequencer_da_address));
    let verifications = ed25519_verifications_performed() - verifications_before;

    let batch_receipt = &result.0.batch_receipts[0];
    assert!(
        !batch_receipt.ignored_tx_receipts.is_empty(),
        "Precondition: the slot gas gate should reject part of the batch"
    );
    // Every tx in `tx_receipts` was authenticated and charged; ignored ones were rejected by the gate.
    let charged_txs = batch_receipt.tx_receipts.len() as u64;
    assert!(
        verifications <= charged_txs + MAX_UNPAID_VERIFICATIONS_PER_BATCH,
        "Node verified {verifications} signatures but only {charged_txs} txs fit in the slot gas limit (allowed slack {MAX_UNPAID_VERIFICATIONS_PER_BATCH})"
    );
}

#[test]
fn unescrowed_non_preferred_batch_triggers_no_verification() {
    reset_constants();
    let (mut runner, mut users, _) = setup(1);
    let user = users.pop().unwrap();

    // Register a non-preferred sequencer whose bond can't escrow the check costs of its batch.
    let sequencer_da_address = MockAddress::new([121; 32]);
    let registration = simulate_da_with_multiple_direct_registration_msg(
        vec![sequencer_da_address.as_ref().to_vec()],
        user.private_key.clone(),
    );
    runner.execute(RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs: vec![new_test_blob_for_direct_registration(
            registration[0].clone(),
            MockAddress::new([0; 32]).as_ref(),
            [0; 32],
        )],
    });

    let txs = bad_sig_txs(&user, ATTACK_TXS);

    let verifications_before = ed25519_verifications_performed();
    let result = runner.execute(batch_blob(&txs, sequencer_da_address));
    let verifications = ed25519_verifications_performed() - verifications_before;

    assert!(
        result.0.batch_receipts.is_empty(),
        "Precondition: a batch whose check costs can't be escrowed should be dropped"
    );
    assert_eq!(
        verifications, 0,
        "Node verified signatures from a batch the sequencer could not escrow"
    );
}

#[test]
fn unregistered_blobs_beyond_slot_quota_are_not_verified() {
    reset_constants();
    let (mut runner, users, _) = setup(1);
    let quota = sov_modules_api::macros::config_value!("UNREGISTERED_BLOBS_PER_SLOT");

    let batch_blobs = bad_sig_txs(&users[0], ATTACK_TXS)
        .into_iter()
        .enumerate()
        .map(|(i, tx)| {
            let sender = MockAddress::new([(i % 200) as u8 + 50; 32]);
            new_test_blob_for_direct_registration(tx, sender.as_ref(), [0; 32])
        })
        .collect();

    let verifications_before = ed25519_verifications_performed();
    let result = runner.execute(RelevantBlobs {
        proof_blobs: Default::default(),
        batch_blobs,
    });
    let verifications = ed25519_verifications_performed() - verifications_before;

    assert_eq!(
        result.0.batch_receipts.len() as u64,
        quota,
        "Precondition: only the per-slot quota of unregistered blobs should be processed"
    );
    assert!(
        verifications <= quota,
        "Node verified {verifications} signatures from unregistered blobs, but the per-slot quota is {quota}"
    );
}
