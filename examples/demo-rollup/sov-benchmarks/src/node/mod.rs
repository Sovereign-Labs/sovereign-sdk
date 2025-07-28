use std::collections::HashMap;

use sov_mock_da::MockBlob;
use sov_modules_api::{Amount, BatchSequencerReceipt, CryptoSpec, PublicKey, Spec};
use sov_rollup_interface::crypto::PrivateKey;
use sov_rollup_interface::da::RelevantBlobs;
use sov_test_utils::runtime::{sov_bank, Bank, Coins, TestRunner, TokenId};
use sov_test_utils::{
    AsUser, MockZkvm, TestPrivateKey, TestUser, TransactionType, TxReceiptContents,
};

use crate::{BenchSpec, Roles, RT};

type S = BenchSpec<MockZkvm>;

type BatchReceipt<S> =
    sov_rollup_interface::stf::BatchReceipt<BatchSequencerReceipt<S>, TxReceiptContents<S>>;

type BenchmarkMessages = Vec<RelevantBlobs<MockBlob>>;

/// Builds a simple transfer transaction
pub fn build_send_tx(sender: &TestUser<S>, token_id: TokenId) -> TransactionType<RT<S>, S> {
    let priv_key = TestPrivateKey::generate();
    let to_address: <S as Spec>::Address = priv_key.pub_key().credential_id().into();

    sender.create_plain_message::<_, Bank<S>>(sov_bank::CallMessage::<S>::Transfer {
        to: to_address,
        coins: Coins {
            amount: Amount::new(1),
            token_id,
        },
    })
}

/// Asserts the outcome of the benchmarks
pub fn assert_batch_receipts<S: Spec>(batch_receipts: &[BatchReceipt<S>]) {
    for batch in batch_receipts {
        assert_eq!(Amount::ZERO, batch.inner.outcome.rewards.accumulated_reward);

        for tx in &batch.tx_receipts {
            assert!(
                tx.receipt.is_successful(),
                "Non successful tx: {:?}",
                tx.receipt
            );
        }
    }
}

fn generate_initial_slots(
    roles: &Roles<S>,
    nonces: &mut HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>,
) -> (TokenId, BenchmarkMessages) {
    let token_name = "sov-bench-token";
    let token_id = sov_bank::get_token_id::<S>(token_name, None, &roles.bank_admin.address());

    let create_token_msg = roles.bank_admin.create_plain_message::<_, Bank<S>>(
        sov_bank::CallMessage::<S>::CreateToken {
            token_name: token_name.try_into().unwrap(),
            token_decimals: None,
            initial_balance: Amount::ZERO,
            mint_to_address: roles.bank_admin.address(),
            admins: vec![roles.bank_admin.address()]
                .try_into()
                .expect("Tokens can have at least one minter"),
            supply_cap: None,
        },
    );

    let coins_per_sender = u128::MAX / roles.senders.len() as u128;

    let benchmark_messages = vec![std::iter::once(create_token_msg)
        .chain(roles.senders.iter().map(|sender| {
            roles
                .bank_admin
                .create_plain_message::<_, Bank<S>>(sov_bank::CallMessage::<S>::Mint {
                    coins: Coins {
                        amount: coins_per_sender.into(),
                        token_id,
                    },
                    mint_to_address: sender.address(),
                })
        }))
        .collect::<Vec<_>>()];

    (
        token_id,
        benchmark_messages
            .into_iter()
            .map(|batch| {
                let preferred_batch = roles.preferred_sequencer.build_preferred_batch(batch);
                TestRunner::<RT<S>, S>::soft_confirmation_batches_to_blobs(
                    vec![preferred_batch],
                    nonces,
                )
            })
            .collect::<Vec<_>>(),
    )
}

/// Generate benchmark transactions for the node
pub fn generate_transfers(
    slots_to_process: u64,
    token_id: TokenId,
    roles: &Roles<S>,
    runner: &mut TestRunner<RT<S>, S>,
) -> BenchmarkMessages {
    let send_messages = (0..slots_to_process)
        .map(|_| {
            roles
                .senders
                .iter()
                .map(|sender| build_send_tx(sender, token_id))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let benchmark_messages = send_messages
        .into_iter()
        .map(|batch| {
            let preferred_batch = roles.preferred_sequencer.build_preferred_batch(batch);
            TestRunner::<RT<S>, S>::soft_confirmation_batches_to_blobs(
                vec![preferred_batch],
                runner.nonces_mut(),
            )
        })
        .collect::<Vec<_>>();

    benchmark_messages
}

/// Prefills the state with benchmark transactions
pub fn prefill_state(roles: &Roles<S>, runner: &mut TestRunner<RT<S>, S>) -> TokenId {
    let (token_id, slots) = generate_initial_slots(roles, runner.nonces_mut());
    for blobs in slots {
        let apply_slot_output = runner.execute(blobs);

        assert_batch_receipts(&apply_slot_output.0.batch_receipts);
    }

    token_id
}
