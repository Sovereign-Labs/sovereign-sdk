use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;

use futures::{Stream, StreamExt, TryStreamExt};
use sov_api_spec::types::{Slot, TxStatus};
use sov_blob_storage::config_deferred_slots_count;
use sov_mock_da::{
    BlockProducingConfig, MockDaConfig, RandomizationBehaviour, RandomizationConfig,
};
use sov_modules_api::prelude::arbitrary::Unstructured;
use sov_modules_api::{Runtime, Spec};
use sov_paymaster::{
    PayeePolicy, PayerGenesisConfig, Paymaster, PaymasterConfig, PaymasterPolicyInitializer,
};
use sov_rollup_interface::common::{HexHash, SafeVec};
use sov_rollup_interface::node::ledger_api::IncludeChildren;
use sov_rollup_interface::zk::CryptoSpec;
use sov_rollup_interface::TxHash;
use sov_sequencer::preferred::PreferredSequencerConfig;
use sov_sequencer::SequencerKindConfig;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;
use sov_test_utils::runtime::genesis::zk::MinimalZkGenesisConfig;
use sov_test_utils::sov_bank::{config_gas_token_id, CallMessageDiscriminants, Coins};
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder};
use sov_test_utils::{
    default_test_tx_details, generate_runtime, RtAgnosticBlueprint, TestSpec, TestUser,
    TransactionType, TEST_DEFAULT_USER_BALANCE,
};
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{
    BasicBankHarness, BasicCallMessageFactory, BasicModuleRef, BasicTag,
};
use sov_transaction_generator::rng_utils::get_random_bytes;
use sov_transaction_generator::{AccountState, Distribution, MessageValidity, Percent, State};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

generate_runtime! {
    name: TestRuntime,
    modules: [paymaster: Paymaster<S>, value_setter: ValueSetter<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Zk,
    minimal_genesis_config_type: MinimalZkGenesisConfig<S>,
    gas_enforcer: paymaster: Paymaster<S>,
    runtime_trait_impl_bounds: [],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_modules_api::capabilities::RollupAuthenticator<S, TestRuntime<S>>,
    auth_call_wrapper: |call| call,
}

type S = TestSpec;
type RT = TestRuntime<S>;
type RollupBlueprint = RtAgnosticBlueprint<S, RT>;
type TestRollupBuilder = RollupBuilder<RollupBlueprint>;

const TEST_RANDOMIZATION_SEED: HexHash = HexHash::new([10; 32]);
const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

fn setup_genesis(additional_accounts: usize) -> (HighLevelZkGenesisConfig<S>, GenesisConfig<S>) {
    let high_level_genesis_config = HighLevelZkGenesisConfig::generate()
        .add_accounts_with_balance(additional_accounts, TEST_DEFAULT_USER_BALANCE);

    let sequencer = high_level_genesis_config.initial_sequencer.clone();
    let payer = high_level_genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();
    let admin = high_level_genesis_config
        .additional_accounts()
        .get(1)
        .unwrap()
        .clone();
    let genesis_config = GenesisConfig::from_minimal_config(
        high_level_genesis_config.clone().into(),
        PaymasterConfig {
            payers: [PayerGenesisConfig {
                payer_address: payer.address(),
                policy: PaymasterPolicyInitializer {
                    default_payee_policy: PayeePolicy::Allow {
                        max_fee: None,
                        gas_limit: None,
                        max_gas_price: None,
                        transaction_limit: None,
                    },
                    payees: SafeVec::new(),
                    authorized_sequencers: sov_paymaster::AuthorizedSequencers::All,
                    authorized_updaters: [payer.address()].as_ref().try_into().unwrap(),
                },
                sequencers_to_register: [sequencer.da_address].as_ref().try_into().unwrap(),
            }]
            .as_ref()
            .try_into()
            .unwrap(),
        },
        ValueSetterConfig {
            admin: admin.address(),
        },
    );
    (high_level_genesis_config, genesis_config)
}

struct TestState {
    state: State<S, BasicTag>,
    nonces: HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64>,
    txs_to_wait: HashSet<TxHash>,
    /// For now we use a slot subscription until we can reliably receive [`TxStatus::Processed`] from the full-node
    slots_subscription: Pin<Box<dyn Stream<Item = anyhow::Result<Slot>> + Send>>,
}

impl TestState {
    async fn new(client: &sov_api_spec::Client, additional_accounts: &[TestUser<S>]) -> Self {
        let mut state = State::<S, BasicTag>::new();
        for account in additional_accounts {
            let account_state = AccountState {
                balances: vec![Coins {
                    amount: account.available_gas_balance,
                    token_id: config_gas_token_id(),
                }],
                can_mint: Default::default(),
                sequencing_bond: None,
                private_key: account.private_key.clone(),
                additional_info: (),
            };
            state.insert_account(account_state);
        }

        Self {
            state,
            nonces: Default::default(),
            txs_to_wait: Default::default(),
            slots_subscription: client
                .subscribe_finalized_slots_with_children(IncludeChildren::new(true))
                .await
                .expect("Impossible to subscribe to the slots"),
        }
    }

    fn add_tx_to_wait(&mut self, tx_hash: TxHash) {
        self.txs_to_wait.insert(tx_hash);
    }

    async fn gather_all_statuses(
        &mut self,
        client: &sov_api_spec::Client,
        max_slots_to_wait: u64,
    ) -> anyhow::Result<Vec<(TxHash, TxStatus)>> {
        tracing::info!(
            txs_to_wait = self.txs_to_wait.len(),
            "Start gathering statuses"
        );
        let head_slot = client.get_latest_slot(None).await?;
        let head_slot = head_slot.number;
        let wait_to = head_slot + config_deferred_slots_count() + 5;
        tracing::info!(current_head_slot = head_slot, wait_max_till_slot= %wait_to, "Start gathering all statuses");
        let mut last_tx_statuses: Vec<(HexHash, TxStatus)> =
            Vec::with_capacity(self.txs_to_wait.len());

        let mut slots = 0;
        while !self.txs_to_wait.is_empty() {
            slots += 1;
            if slots > max_slots_to_wait {
                anyhow::bail!(
                    "Didn't receive status for all transactions in {} slots. Remaining txs: {:?}",
                    max_slots_to_wait,
                    self.txs_to_wait,
                );
            }
            let Some(next_slot) = self.slots_subscription.try_next().await? else {
                continue;
            };
            tracing::info!(slot_number = next_slot.number, "Received slot");
            if next_slot.number > wait_to {
                break;
            }
            for batch in next_slot.batches {
                for tx in batch.txs {
                    let parsed_hash: TxHash =
                        tx.hash.parse().expect("Impossible to parse tx_hash!");

                    if !self.txs_to_wait.remove(&parsed_hash) {
                        tracing::warn!(tx_hash = %parsed_hash,"Tx wasn't in the wait list");
                    }
                    tracing::info!("Tx {} result: {:?}", parsed_hash, tx.receipt.result);
                    last_tx_statuses.push((parsed_hash, TxStatus::Processed));
                }
            }
        }
        tracing::info!(
            remaining_to_wait = self.txs_to_wait.len(),
            "All statuses from slot subscriptions are gathered, now pulling statuses from the rest"
        );

        // Gather non processed statuses
        for tx_hash in &self.txs_to_wait {
            let last_status_response = client.get_tx_status(&((*tx_hash).into())).await?;
            let received_tx_status = last_status_response.status;
            last_tx_statuses.push((*tx_hash, received_tx_status));
        }
        tracing::info!("All statuses are gathered");
        Ok(last_tx_statuses)
    }
}

/// Test sends a stream of transactions to the sequencer and waits for them to become processed state.
async fn test_stream_of_transactions(
    StreamOfTransactionsArgs {
        block_time_ms,
        finalization_blocks,
        da_slots,
        txs_per_da_slot,
        additional_users,
        randomization_config,
    }: StreamOfTransactionsArgs,
) -> anyhow::Result<()> {
    // ------------
    // Setup rollup
    let block_producing_config = BlockProducingConfig::Periodic { block_time_ms };
    let (high_level_config, genesis_config) = setup_genesis(additional_users);

    let rollup_builder = TestRollupBuilder::new(
        GenesisSource::CustomParams(genesis_config.clone().into_genesis_params()),
        block_producing_config,
        finalization_blocks,
    )
    .set_config(|config| {
        config.sequencer_config = SequencerKindConfig::Preferred(PreferredSequencerConfig {
            minimum_profit_per_tx: 0,
            ..Default::default()
        });
        config.rollup_prover_config = None;
        config.automatic_batch_production = true;
    })
    .set_da_config(|da_config| {
        // We don't need to test restarts, so let's save disk accesses and file descriptors.
        da_config.connection_string = MockDaConfig::sqlite_in_memory();
        da_config.sender_address = genesis_config
            .sequencer_registry
            .sequencer_config
            .seq_da_address;
        da_config.randomization = randomization_config;
    });

    let rollup = rollup_builder
        .start()
        .await
        .expect("Impossible to start rollup");

    let mut finalized_slots = rollup.client.client.subscribe_finalized_slots().await?;
    // Test considers rollup ready when it has produced at least finalized slot with rollup_height = 1
    // TODO: Rollup is incapable to accept txs if it hasn't produced first finalized slot
    for _ in 0..2 {
        let slot = finalized_slots
            .next()
            .await
            .transpose()?
            .expect("Empty response in finalize slots");
        tracing::info!(
            slot_number = slot.number,
            "Test received finalized slot notification"
        );
    }
    tracing::info!("Rollup is ready");

    let mut harness = TestState::new(
        &rollup.client.client,
        high_level_config.additional_accounts(),
    )
    .await;
    let rand_bytes = get_random_bytes(1_000_000, 42);
    let mut u = Unstructured::new(&rand_bytes);

    let bank_generator: BasicModuleRef<S, RT> =
        Arc::new(BasicBankHarness::<S, RT>::new(BankMessageGenerator::new(
            // TODO: Transfers??
            Distribution::with_equiprobable_values(vec![CallMessageDiscriminants::CreateToken]),
            Percent::zero(),
        )));

    let call_message_factory = BasicCallMessageFactory::new();
    let mut da_finalized_headers = rollup.da_service.subscribe_finalized_header().await?;
    for _ in 0..da_slots {
        for _ in 0..txs_per_da_slot {
            let generated_message = call_message_factory.generate_call_message(
                &Distribution::with_equiprobable_values(vec![bank_generator.clone()]),
                &mut u,
                &mut harness.state,
                MessageValidity::Valid,
            )?;

            let msg = TransactionType::<RT, S>::sign(
                generated_message.message,
                generated_message.sender,
                &RT::CHAIN_HASH,
                default_test_tx_details(),
                &mut harness.nonces,
            );

            let accept_tx_response = rollup.client.client.send_tx_to_sequencer(&msg).await?;
            tracing::info!(
                "test received transaction accept response: {:?}",
                accept_tx_response
            );

            let tx_hash = HexHash::from_str(&accept_tx_response.id)?;
            harness.add_tx_to_wait(tx_hash);
        }
        tracing::info!("Set of transactions sent, waiting for finalized header");
        match da_finalized_headers.next().await.transpose()? {
            None => {
                tracing::warn!("No finalized header received, problem with node");
                da_finalized_headers = rollup.da_service.subscribe_finalized_header().await?;
            }
            Some(header) => {
                tracing::info!(%header, "received finalize header");
            }
        }
    }
    tracing::info!("Submission of all transactions to sequencer is done, checking results");
    assert!(
        !rollup.is_rollup_crashed(),
        "Some of the rollup tasks have crashed before shutdown"
    );

    let max_slots = da_slots * 10;
    let all_tx_statuses = harness
        .gather_all_statuses(&rollup.client.client, max_slots as u64)
        .await?;

    for (tx_hash, status) in all_tx_statuses {
        assert_eq!(status, TxStatus::Processed, "tx {tx_hash} wasn't processed");
    }

    // TODO: Check transactions outcomes
    tracing::info!("Test is completed, shutting down the rollup");

    assert!(
        !rollup.is_rollup_crashed(),
        "Some of the rollup tasks have crashed before shutdown"
    );

    rollup.shutdown().await?;
    Ok(())
}

/// Self-check that test works, with long enough block time, not many transactions and without re-orgs
#[tokio::test(flavor = "multi_thread")]
async fn test_check_no_reorgs() -> anyhow::Result<()> {
    tokio::time::timeout(
        TEST_TIMEOUT,
        test_stream_of_transactions(StreamOfTransactionsArgs {
            block_time_ms: 500,
            finalization_blocks: 5,
            da_slots: 3,
            txs_per_da_slot: 10,
            additional_users: 20,
            randomization_config: None,
        }),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
async fn test_check_no_reorgs_longer() -> anyhow::Result<()> {
    tokio::time::timeout(
        TEST_TIMEOUT,
        test_stream_of_transactions(StreamOfTransactionsArgs {
            block_time_ms: 500,
            finalization_blocks: 10,
            da_slots: 50,
            txs_per_da_slot: 10,
            additional_users: 20,
            randomization_config: None,
        }),
    )
    .await?
}

struct StreamOfTransactionsArgs {
    /// StorableMockDa is started in PeriodicBatchProduction mode, so this
    /// parameter defines block time.
    block_time_ms: u64,
    /// Blocks to finality for mock DA.
    finalization_blocks: u32,
    /// `da_slots` and `txs_per_da_slot` are go hand in hand. Client does not
    /// care or know about how many batches rollup is going to create.
    /// But in order to increase probability, that there are several batches at
    /// least, it only sends `txs_per_da_slot` in single before next da slot
    /// appeared.
    /// Total number of transactions sent is `da_slots x txs_per_da_slot`.
    da_slots: usize,
    txs_per_da_slot: usize,
    /// parameter for genesis
    additional_users: usize,
    /// Parameter for randomization for `StorableMockDaLayer`.
    randomization_config: Option<RandomizationConfig>,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Unblock later when state manager is performant and stable"]
async fn test_small_reshuffle_no_drops() -> anyhow::Result<()> {
    let finality = 5;
    let randomization_config = RandomizationConfig {
        seed: TEST_RANDOMIZATION_SEED,
        reorg_interval: 1..finality,
        behaviour: RandomizationBehaviour::only_shuffle(0),
    };
    tokio::time::timeout(
        TEST_TIMEOUT,
        test_stream_of_transactions(StreamOfTransactionsArgs {
            block_time_ms: 500,
            finalization_blocks: finality,
            da_slots: 3,
            txs_per_da_slot: 10,
            additional_users: 20,
            randomization_config: Some(randomization_config),
        }),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Node gets stuck on reorg: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2399"]
async fn test_small_shuffle_rewind() -> anyhow::Result<()> {
    let finality = 20;
    let randomization_config = RandomizationConfig {
        seed: TEST_RANDOMIZATION_SEED,
        reorg_interval: 2..5,
        behaviour: RandomizationBehaviour::ShuffleAndResize {
            drop_percent: 0,
            adjust_head_height: -15..4,
        },
    };
    tokio::time::timeout(
        TEST_TIMEOUT,
        test_stream_of_transactions(StreamOfTransactionsArgs {
            block_time_ms: 500,
            finalization_blocks: finality,
            da_slots: 25,
            txs_per_da_slot: 10,
            additional_users: 20,
            randomization_config: Some(randomization_config),
        }),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Node gets stuck on reorg: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2399"]
async fn test_small_reshuffle_half_dropped() -> anyhow::Result<()> {
    // sov_test_utils::initialize_logging();
    let finality = 10;
    let randomization_config = RandomizationConfig {
        seed: HexHash::new([100; 32]),
        // It can rest for 3 blocks.
        reorg_interval: 3..finality,
        behaviour: RandomizationBehaviour::only_shuffle(50),
    };
    tokio::time::timeout(
        TEST_TIMEOUT,
        test_stream_of_transactions(StreamOfTransactionsArgs {
            block_time_ms: 500,
            finalization_blocks: finality,
            da_slots: 30,
            txs_per_da_slot: 10,
            additional_users: 20,
            randomization_config: Some(randomization_config),
        }),
    )
    .await?
}
