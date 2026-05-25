use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::marker::PhantomData;
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use alloy_primitives::{Address, Bytes, TxHash, U256};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress};
use sov_blob_storage::PreferredBatchData;
use sov_evm::precompiles::{
    EvmPrecompile, EvmPrecompileEnv, PrecompileError, PrecompileOutput, PrecompileResult,
};
use sov_evm::{
    AccountData, ContractCreationPolicy, Evm, EvmAuthenticatorInput, EvmChainSpec,
    EvmGenesisConfig, SpecId,
};
use sov_evm_test_utils::{PrecompileTester, SolCall};
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::{
    AuthorizationData, GasEnforcer, Guard, HasCapabilities, HasKernel, ProofProcessor,
    SequencerAuthorization, SequencerRemuneration, SequencingDataHandler, TransactionAuthenticator,
    TransactionAuthorizer,
};
use sov_modules_api::sov_universal_wallet::schema::UniversalWallet;
use sov_modules_api::transaction::{
    AuthenticatedTransactionData, ProverReward, RemainingFunds, SequencerReward, Transaction,
};
use sov_modules_api::{
    AggregatedProofPublicData, Amount, BlobReaderTrait, Context, DaSpec, ExecutionContext, Gas,
    GetGasPrice, InfallibleStateAccessor, InvalidProofError, OperatingMode, RawTx, Rewards,
    SequencerType, SovAttestation, SovStateTransitionPublicData, Spec, StateAccessor, StateReader,
    StateWriter, Storage, TxState, VersionReader,
};
use sov_modules_stf_blueprint::{GenesisParams, Runtime as RuntimeTrait};
use sov_paymaster::Paymaster;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_sequencer::SequencerKindConfig;
use sov_state::{Kernel, User};
use sov_stf_runner::processes::RollupProverConfig;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::StandardProvenRollupCapabilities;
use sov_test_utils::test_rollup::{GenesisSource, RollupBuilder, StoragePath, TestRollup};
use sov_test_utils::{generate_runtime_without_capabilities, RtAgnosticBlueprintWithApis};

use crate::common::{
    create_simple_storage_client, DEFAULT_EVM_BALANCE, DEFAULT_FUNDED_EVM_ACCOUNTS, EVM_EXTENSION,
    SENDER_PRIV_KEY,
};
use crate::runtime::{EvmAdditionalApis, EvmTestSpec};

const ORACLE_PRECOMPILE_ADDRESS: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x02, 0x00, 0x01,
]);
const ORACLE_PRECOMPILE_GAS: u64 = 100;
const FINALIZATION_BLOCKS: u32 = 2;

static ORACLE_DATA: LazyLock<Mutex<OracleSequencingData>> =
    LazyLock::new(|| Mutex::new(OracleSequencingData::default()));

#[derive(Clone, Debug, Default, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub struct OracleSequencingData(BTreeMap<u64, u64>);

impl sov_modules_api::capabilities::SequencingDataTrait for OracleSequencingData {
    fn get_maybe_timestamp(self) -> Option<sov_modules_api::HDTimestamp> {
        None
    }
}

#[derive(Clone, Default)]
struct OraclePrecompile<S>(PhantomData<S>);

impl<S: Spec> EvmPrecompile<S> for OraclePrecompile<S> {
    const ADDRESS: Address = ORACLE_PRECOMPILE_ADDRESS;

    fn execute<ST: TxState<S>>(
        &self,
        input: &[u8],
        gas_limit: u64,
        env: &mut EvmPrecompileEnv<'_, S, ST>,
    ) -> PrecompileResult {
        if input.is_empty() || !input.len().is_multiple_of(32) {
            return Err(PrecompileError::InvalidInput(format!(
                "expected one or more 32-byte keys, got {} bytes",
                input.len()
            )));
        }

        let key_count = u64::try_from(input.len() / 32)
            .map_err(|_| PrecompileError::InvalidInput("too many oracle keys".to_string()))?;
        let gas_used = ORACLE_PRECOMPILE_GAS
            .checked_mul(key_count)
            .ok_or(PrecompileError::OutOfGas)?;
        if gas_used > gas_limit {
            return Err(PrecompileError::OutOfGas);
        }

        let sequencing_data = {
            let sequencing_data = env
                .sov_context
                .and_then(|ctx| ctx.sequencing_data().as_ref())
                .ok_or_else(|| PrecompileError::State("missing sequencing data".to_string()))?;
            OracleSequencingData::try_from_slice(sequencing_data).map_err(|err| {
                PrecompileError::State(format!("failed to deserialize sequencing data: {err}"))
            })?
        };

        let mut output = Vec::with_capacity(input.len());
        for raw_key in input.chunks_exact(32) {
            let raw_key = U256::from_be_slice(raw_key);
            if raw_key > U256::from(u64::MAX) {
                return Err(PrecompileError::InvalidInput(
                    "key does not fit in u64".to_string(),
                ));
            }
            let key = raw_key.to::<u64>();
            let value = *sequencing_data.0.get(&key).ok_or_else(|| {
                PrecompileError::InvalidInput(format!("missing oracle key {key}"))
            })?;

            // This crate doesn't have a `"native"` feature, but in a real precompile
            // implementation, key recording is native-only.
            // #[cfg(feature = "native")]
            record_used_oracle_key(env, key)?;

            output.extend_from_slice(&U256::from(value).to_be_bytes::<32>());
        }

        Ok(PrecompileOutput {
            gas_used,
            bytes: output.into(),
        })
    }
}

fn record_used_oracle_key<S: Spec, ST: TxState<S>>(
    env: &EvmPrecompileEnv<'_, S, ST>,
    key: u64,
) -> Result<(), PrecompileError> {
    env.sov_context
        .expect("sov context was checked above")
        .sequencing_scratchpad()
        .with_value(|scratchpad| {
            let mut used_keys = match scratchpad.as_deref() {
                Some(bytes) => Vec::<u64>::try_from_slice(bytes).map_err(|err| {
                    PrecompileError::State(format!("failed to deserialize used oracle keys: {err}"))
                })?,
                None => Vec::new(),
            };
            used_keys.push(key);
            *scratchpad = Some(
                borsh::to_vec(&used_keys)
                    .expect("u64 vector serialization is infallible")
                    .into(),
            );
            Ok(())
        })
}

sov_evm::generate_precompile_set! {
    pub struct OraclePrecompiles<S> {
        oracle: OraclePrecompile<S>,
    }
}

generate_runtime_without_capabilities! {
    name: PruningRuntime,
    modules: [evm: Evm<S, OraclePrecompiles<S>>, paymaster: Paymaster<S>],
    operating_mode: sov_modules_api::runtime::OperatingMode::Optimistic,
    minimal_genesis_config_type: sov_test_utils::runtime::genesis::optimistic::MinimalOptimisticGenesisConfig<S>,
    runtime_trait_impl_bounds: [S::Address: FromVmAddress<EthereumAddress>],
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    auth_type: sov_evm::EvmAuthenticator<S, Self>,
    auth_call_wrapper: |call| match call {
        EvmAuthenticatorInput::Evm(call) => PruningRuntimeCall::Evm(call),
        EvmAuthenticatorInput::Standard(call) => call,
    },
}

impl<S: Spec> sov_evm::EthereumAuthenticator<S> for PruningRuntime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
    Transaction<Self, S>: UniversalWallet,
{
    fn add_ethereum_auth(tx: RawTx) -> <Self::Auth as TransactionAuthenticator<S>>::Input {
        EvmAuthenticatorInput::Evm(tx)
    }
}

type S = EvmTestSpec;
type RT = PruningRuntime<S>;
type PruningBlueprint = RtAgnosticBlueprintWithApis<S, RT, EvmAdditionalApis>;
type StandardCapabilities<'a, S> = StandardProvenRollupCapabilities<'a, S, &'a mut Paymaster<S>>;

pub struct PruningCapabilities<'a, S: Spec> {
    standard: StandardCapabilities<'a, S>,
}

impl<S: Spec> SequencingDataHandler<S> for PruningCapabilities<'_, S> {
    type SequencingData = OracleSequencingData;

    fn handle_sequencing_data(
        &mut self,
        _data: Self::SequencingData,
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn create_sequencing_data(&self) -> Self::SequencingData {
        ORACLE_DATA
            .lock()
            .expect("oracle data mutex was poisoned")
            .clone()
    }

    fn finalize_sequencing_data(
        &mut self,
        mut data: Self::SequencingData,
        scratchpad: Option<sov_rollup_interface::Bytes>,
    ) -> Self::SequencingData {
        let Some(scratchpad) = scratchpad else {
            return data;
        };
        let Ok(used_keys) = Vec::<u64>::try_from_slice(&scratchpad) else {
            return data;
        };
        let used_keys = used_keys.into_iter().collect::<BTreeSet<_>>();
        data.0.retain(|key, _| used_keys.contains(key));
        data
    }
}

impl<S: Spec> GasEnforcer<S> for PruningCapabilities<'_, S> {
    fn try_reserve_gas(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: <S::Gas as Gas>::Price,
        ctx: &mut Context<S>,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        self.standard.try_reserve_gas(tx, gas_price, ctx, state)
    }

    fn try_reserve_gas_for_proof(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: <S::Gas as Gas>::Price,
        sender: &S::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        self.standard
            .try_reserve_gas_for_proof(tx, gas_price, sender, state)
    }

    fn reward_prover(
        &mut self,
        prover_rewards: &ProverReward,
        operating_mode: OperatingMode,
        state: &mut impl InfallibleStateAccessor,
    ) {
        self.standard
            .reward_prover(prover_rewards, operating_mode, state);
    }

    fn refund_remaining_gas(
        &mut self,
        recipient: &S::Address,
        remaining_funds: &RemainingFunds,
        state: &mut impl InfallibleStateAccessor,
    ) {
        self.standard
            .refund_remaining_gas(recipient, remaining_funds, state);
    }

    fn reward_prover_from_sequencer_balance(
        &mut self,
        amount: Amount,
        sequencer: &S::Address,
        operating_mode: OperatingMode,
        state: &mut impl InfallibleStateAccessor,
    ) -> anyhow::Result<()> {
        self.standard
            .reward_prover_from_sequencer_balance(amount, sequencer, operating_mode, state)
    }

    fn return_escrowed_funds_to_sequencer<
        Accessor: StateReader<Kernel, Error = Infallible>
            + StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<User, Error = Infallible>
            + VersionReader,
    >(
        &mut self,
        bond_amount: Amount,
        reward: Rewards,
        sequencer: &<S::Da as DaSpec>::Address,
        state: &mut Accessor,
    ) {
        self.standard
            .return_escrowed_funds_to_sequencer(bond_amount, reward, sequencer, state);
    }
}

impl<S: Spec> SequencerAuthorization<S> for PruningCapabilities<'_, S> {
    fn is_preferred_sequencer(
        &self,
        sequencer: &<S::Da as DaSpec>::Address,
        state: &mut impl InfallibleStateAccessor,
    ) -> bool {
        self.standard.is_preferred_sequencer(sequencer, state)
    }
}

impl<S: Spec> TransactionAuthorizer<S> for PruningCapabilities<'_, S> {
    fn resolve_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<S::Da as DaSpec>::Address,
        sequencer_rollup_address: S::Address,
        state: &mut impl StateReader<User>,
        sequencing_data: Option<sov_rollup_interface::Bytes>,
        execution_context: ExecutionContext,
        sequencer_type: SequencerType,
    ) -> anyhow::Result<Context<S>> {
        self.standard.resolve_context(
            auth_data,
            sequencer,
            sequencer_rollup_address,
            state,
            sequencing_data,
            execution_context,
            sequencer_type,
        )
    }

    fn resolve_unregistered_context(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<S::Da as DaSpec>::Address,
        state: &mut impl StateReader<User>,
        execution_context: ExecutionContext,
    ) -> anyhow::Result<Context<S>> {
        self.standard
            .resolve_unregistered_context(auth_data, sequencer, state, execution_context)
    }

    fn check_uniqueness(
        &self,
        auth_data: &AuthorizationData<S>,
        context: &Context<S>,
        execution_context: &ExecutionContext,
        state: &mut impl StateReader<User>,
    ) -> anyhow::Result<()> {
        self.standard
            .check_uniqueness(auth_data, context, execution_context, state)
    }

    fn mark_tx_attempted(
        &mut self,
        auth_data: &AuthorizationData<S>,
        sequencer: &<S::Da as DaSpec>::Address,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()> {
        self.standard.mark_tx_attempted(auth_data, sequencer, state)
    }
}

impl<'a, S: Spec> ProofProcessor<S> for PruningCapabilities<'a, S> {
    type BondingProofService<K: HasKernel<S>> =
        <StandardCapabilities<'a, S> as ProofProcessor<S>>::BondingProofService<K>;

    fn create_bonding_proof_service<K: HasKernel<S>>(
        &self,
        attester_address: S::Address,
        storage_receiver: tokio::sync::watch::Receiver<S::Storage>,
    ) -> Self::BondingProofService<K> {
        self.standard
            .create_bonding_proof_service::<K>(attester_address, storage_receiver)
    }

    fn process_aggregated_proof<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedAggregatedProof,
        prover_address: &S::Address,
        execution_context: ExecutionContext,
        state: &mut ST,
    ) -> Result<
        (
            AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
            SerializedAggregatedProof,
        ),
        InvalidProofError,
    > {
        self.standard
            .process_aggregated_proof(proof, prover_address, execution_context, state)
    }

    fn process_attestation<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedAttestation,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<SovAttestation<S>, InvalidProofError> {
        self.standard
            .process_attestation(proof, prover_address, state)
    }

    fn process_challenge<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedChallenge,
        rollup_height: SlotNumber,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<SovStateTransitionPublicData<S>, InvalidProofError> {
        self.standard
            .process_challenge(proof, rollup_height, prover_address, state)
    }
}

impl<S: Spec> SequencerRemuneration<S> for PruningCapabilities<'_, S> {
    fn reward_sequencer_or_refund<
        Accessor: StateReader<Kernel, Error = Infallible>
            + StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &mut self,
        sequencer: &<S::Da as DaSpec>::Address,
        sequencer_rollup_address: &S::Address,
        reward: SequencerReward,
        state: &mut Accessor,
    ) {
        self.standard.reward_sequencer_or_refund(
            sequencer,
            sequencer_rollup_address,
            reward,
            state,
        );
    }

    fn preferred_sequencer(
        &self,
        state: &mut impl InfallibleStateAccessor,
    ) -> Option<<S::Da as DaSpec>::Address> {
        self.standard.preferred_sequencer(state)
    }
}

impl<S: Spec> HasCapabilities<S> for PruningRuntime<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    type Capabilities<'a>
        = PruningCapabilities<'a, S>
    where
        Self: 'a;
    type SequencingData = OracleSequencingData;

    fn capabilities(&mut self) -> Guard<Self::Capabilities<'_>> {
        Guard::new(PruningCapabilities {
            standard: StandardProvenRollupCapabilities {
                bank: &mut self.bank,
                gas_payer: &mut self.paymaster,
                sequencer_registry: &mut self.sequencer_registry,
                accounts: &mut self.accounts,
                uniqueness: &mut self.uniqueness,
                chain_state: &mut self.chain_state,
                operator_incentives: &mut self.operator_incentives,
                prover_incentives: &mut self.prover_incentives,
                attester_incentives: &mut self.attester_incentives,
            },
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn evm_precompile_can_prune_sequencing_data_and_replay_from_da() -> anyhow::Result<()> {
    let test_rollup = setup_pruning_rollup().await;
    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_sequencer_ready().await?;

    let evm_client = create_simple_storage_client(test_rollup.http_addr, SENDER_PRIV_KEY).await;
    let tester_address = deploy_precompile_tester(&evm_client).await?;

    let oracle_entries = BTreeMap::from([(11, 1_100), (22, 2_200), (33, 3_300)]);
    set_oracle_data(oracle_entries.clone());

    let mut expected_by_hash: BTreeMap<[u8; 32], BTreeSet<u64>> = BTreeMap::new();
    for &key in oracle_entries.keys() {
        let keys = [key];
        let tx_hash =
            assert_precompile_values(&evm_client, tester_address, &keys, &oracle_entries).await?;
        expected_by_hash.insert(tx_hash_bytes(tx_hash), keys.into_iter().collect());
    }
    for keys in [vec![11, 22], vec![33, 11, 22]] {
        let tx_hash =
            assert_precompile_values(&evm_client, tester_address, &keys, &oracle_entries).await?;
        expected_by_hash.insert(tx_hash_bytes(tx_hash), keys.into_iter().collect());
    }

    let last_checked_height = test_rollup.da_service.get_head_block_header().await?.height;
    test_rollup.force_close_batch().await?;

    wait_for_pruned_txs_on_da(
        &test_rollup,
        &expected_by_hash,
        &oracle_entries,
        last_checked_height,
    )
    .await?;

    test_rollup.produce_enough_finalized_slots().await;
    test_rollup.wait_for_node_synced().await?;

    for tx_hash in expected_by_hash.keys() {
        let receipt = evm_client
            .wait_for_finalized_receipt(TxHash::from(*tx_hash))
            .await;
        assert!(receipt.status(), "oracle assertion tx should be finalized");
    }

    let (resynced_rollup, _keep_da_storage_alive) =
        restart_from_scratch_on_same_da(test_rollup).await?;
    let resynced_client =
        create_simple_storage_client(resynced_rollup.http_addr, SENDER_PRIV_KEY).await;
    resynced_rollup.wait_for_node_synced().await?;

    for tx_hash in expected_by_hash.keys() {
        let receipt = resynced_client
            .wait_for_finalized_receipt(TxHash::from(*tx_hash))
            .await;
        assert!(
            receipt.status(),
            "oracle assertion tx should replay on resync"
        );
    }

    resynced_rollup.shutdown().await?;
    Ok(())
}

async fn setup_pruning_rollup() -> TestRollup<PruningBlueprint> {
    let (genesis, seq_da_address) = pruning_genesis();
    let storage = StoragePath::Tmp(Arc::new(tempfile::tempdir().expect("create tempdir")));

    let mut builder = RollupBuilder::<PruningBlueprint>::new_with_storage_path(
        GenesisSource::CustomParams(GenesisParams { runtime: genesis }),
        BlockProducingConfig::Periodic {
            block_time_ms: 1_000,
        },
        FINALIZATION_BLOCKS,
        storage,
        false,
    );

    builder = builder
        .set_config(|c| {
            c.max_concurrent_batch_blobs = 65536;
            c.rollup_prover_config = RollupProverConfig::Disabled;
            c.aggregated_proof_block_jump = 5;
            c.max_infos_in_db = 30;
            c.max_channel_size = 20;
            c.extension = Some(EVM_EXTENSION);
            if let SequencerKindConfig::Preferred(ref mut seq) = c.sequencer_config {
                seq.ideal_lag_behind_finalized_slot = 3;
            }
        })
        .set_da_config(|c| {
            c.sender_address = seq_da_address;
        });

    builder.start().await.expect("start pruning test rollup")
}

fn pruning_genesis() -> (
    GenesisConfig<S>,
    <<S as Spec>::Da as sov_rollup_interface::da::DaSpec>::Address,
) {
    let hl_genesis = HighLevelOptimisticGenesisConfig::<S>::generate();
    let seq_da_address = hl_genesis.initial_sequencer.da_address;
    let admin = hl_genesis.initial_sequencer.user_info.address();

    let evm_chain_spec = EvmChainSpec {
        block_gas_limit: 100_000_000_000,
        tx_gas_limit: Some(30_000_000),
        hardforks: vec![(0, SpecId::CANCUN)],
        ..EvmChainSpec::default()
    };

    let evm_config = EvmGenesisConfig::<S> {
        accounts: DEFAULT_FUNDED_EVM_ACCOUNTS
            .iter()
            .map(|addr| AccountData::empty_with_address(Address::from_str(addr).unwrap()))
            .collect(),
        initial_base_fee: 10,
        genesis_timestamp: 0,
        chain_spec: evm_chain_spec,
        contract_creation_policy: ContractCreationPolicy::Everyone,
        enabled_custom_precompiles: BTreeSet::from([ORACLE_PRECOMPILE_ADDRESS]),
        admin,
    };

    let mut genesis = GenesisConfig::from_minimal_config(
        hl_genesis.into(),
        evm_config,
        sov_paymaster::PaymasterConfig::default(),
    );
    if let Some(token_cfg) = genesis.bank.gas_token_config.as_mut() {
        for addr in DEFAULT_FUNDED_EVM_ACCOUNTS {
            token_cfg.address_and_balances.push((
                MultiAddress::Vm(EthereumAddress::from(Address::from_str(addr).unwrap())),
                Amount::new(DEFAULT_EVM_BALANCE),
            ));
        }
    }

    (genesis, seq_da_address)
}

async fn deploy_precompile_tester(
    client: &sov_eth_client::SimpleStorageClient,
) -> anyhow::Result<Address> {
    set_oracle_data(BTreeMap::new());
    let tx = client.make_tx(None, Some(Bytes::from(PrecompileTester::BYTECODE.to_vec())));
    let receipt = client
        .send_tx_and_wait(tx)
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    assert!(
        receipt.status(),
        "precompile tester deployment should succeed"
    );
    Ok(receipt
        .contract_address
        .expect("deployment receipt should include contract address"))
}

async fn assert_precompile_values(
    client: &sov_eth_client::SimpleStorageClient,
    tester_address: Address,
    keys: &[u64],
    oracle_entries: &BTreeMap<u64, u64>,
) -> anyhow::Result<TxHash> {
    let expected_values = keys.iter().map(|key| {
        oracle_entries
            .get(key)
            .copied()
            .unwrap_or_else(|| panic!("missing test oracle value for key {key}"))
    });
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: ORACLE_PRECOMPILE_ADDRESS,
        input: u256_words(keys.iter().copied()),
        expectedOutput: u256_words(expected_values),
    };
    let tx = client.make_tx(Some(tester_address), Some(Bytes::from(call.abi_encode())));
    let tx_hash = client
        .send_tx(tx)
        .await
        .map_err(|err| anyhow::anyhow!("{err}"))?;
    let receipt = client.wait_for_receipt(tx_hash).await;
    assert!(
        receipt.status(),
        "precompile assertion tx for keys {keys:?} should succeed"
    );
    Ok(tx_hash)
}

async fn wait_for_pruned_txs_on_da(
    test_rollup: &TestRollup<PruningBlueprint>,
    expected_by_hash: &BTreeMap<[u8; 32], BTreeSet<u64>>,
    oracle_entries: &BTreeMap<u64, u64>,
    mut last_checked_height: u64,
) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut found = BTreeMap::new();
        loop {
            test_rollup.da_service.produce_block_now().await?;
            let head_height = test_rollup.da_service.get_head_block_header().await?.height;
            for height in last_checked_height + 1..=head_height {
                let mut block = test_rollup.da_service.get_block_at(height).await?;
                for blob in block.batch_blobs.iter_mut() {
                    let batch = PreferredBatchData::try_from_slice(blob.full_data())?;
                    for tx in batch.data.iter() {
                        let tx_hash = <RT as RuntimeTrait<S>>::Auth::compute_tx_hash(tx)?;
                        let tx_hash = <[u8; 32]>::from(tx_hash);
                        let Some(expected_keys) = expected_by_hash.get(&tx_hash) else {
                            continue;
                        };
                        let sequencing_data = tx
                            .sequencing_data
                            .as_ref()
                            .expect("oracle tx should include sequencing data");
                        let sequencing_data =
                            OracleSequencingData::try_from_slice(sequencing_data)?;
                        let actual_keys =
                            sequencing_data.0.keys().copied().collect::<BTreeSet<_>>();
                        assert_eq!(
                            actual_keys, *expected_keys,
                            "published tx should retain only the used oracle keys"
                        );
                        for expected_key in expected_keys {
                            assert_eq!(
                                sequencing_data.0.get(expected_key),
                                oracle_entries.get(expected_key),
                                "published tx should retain the used oracle value"
                            );
                        }
                        found.insert(tx_hash, expected_keys.clone());
                    }
                }
            }
            if found.len() == expected_by_hash.len() {
                return anyhow::Ok(());
            }
            last_checked_height = head_height;
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for oracle txs to be published")
}

async fn restart_from_scratch_on_same_da(
    test_rollup: TestRollup<PruningBlueprint>,
) -> anyhow::Result<(TestRollup<PruningBlueprint>, StoragePath)> {
    let builder = test_rollup.shutdown().await?;
    let keep_da_storage_alive = builder.storage_path();
    let fresh_storage = StoragePath::Tmp(Arc::new(tempfile::tempdir()?));
    let rollup = builder
        .set_config(|c| c.storage = fresh_storage)
        .start()
        .await?;
    Ok((rollup, keep_da_storage_alive))
}

fn set_oracle_data(data: BTreeMap<u64, u64>) {
    *ORACLE_DATA.lock().expect("oracle data mutex was poisoned") = OracleSequencingData(data);
}

fn u256_words(values: impl IntoIterator<Item = u64>) -> Bytes {
    let mut bytes = Vec::new();
    for value in values {
        bytes.extend_from_slice(&U256::from(value).to_be_bytes::<32>());
    }
    Bytes::from(bytes)
}

fn tx_hash_bytes(tx_hash: TxHash) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes.copy_from_slice(tx_hash.as_slice());
    bytes
}
