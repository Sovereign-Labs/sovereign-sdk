use std::marker::PhantomData;
use std::sync::Arc;

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json;
use serde::{Deserialize, Serialize};
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::capabilities::{HasCapabilities, HasKernel, TransactionAuthenticator};
use sov_modules_api::execution_mode::Native;
use sov_modules_api::prelude::axum::async_trait;
use sov_modules_api::rest::{HasRestApi, StateUpdateReceiver};
use sov_modules_api::{NodeEndpoints, RawTx, Spec, SyncStatus};
use sov_modules_rollup_blueprint::pluggable_traits::PluggableSpec;
use sov_modules_rollup_blueprint::{FullNodeBlueprint, RollupBlueprint, SequencerCreationReceipt};
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_rest_utils::{errors, ApiResult};
use sov_rollup_interface::da::DaBlobHash;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::TxHash;
use sov_sequencer::rest_api::{AcceptTx, TxInfoWithConfirmation};
use sov_sequencer::{ProofBlobSender, Sequencer, TxStatus};
use sov_stf_runner::RollupConfig;
use sov_test_utils::RtAgnosticBlueprint;

use sov_solana_offchain_auth::capabilities::SolanaOffchainAuthenticatorTrait;

/// A test blueprint that extends RtAgnosticBlueprint with Solana offchain auth endpoints
pub struct SolanaOffchainAuthBlueprint<S: Spec, R: RuntimeTrait<S>> {
    inner: RtAgnosticBlueprint<S, R>,
}

impl<S: Spec, R: RuntimeTrait<S>> Default for SolanaOffchainAuthBlueprint<S, R> {
    fn default() -> Self {
        Self {
            inner: RtAgnosticBlueprint::default(),
        }
    }
}

impl<S, R> RollupBlueprint<Native> for SolanaOffchainAuthBlueprint<S, R>
where
    S: Spec + PluggableSpec,
    R: RuntimeTrait<S> + HasKernel<S> + HasCapabilities<S>,
    RtAgnosticBlueprint<S, R>: RollupBlueprint<Native>,
{
    type Spec = <RtAgnosticBlueprint<S, R> as RollupBlueprint<Native>>::Spec;
    type Runtime = <RtAgnosticBlueprint<S, R> as RollupBlueprint<Native>>::Runtime;
}

#[async_trait]
impl<S, R> FullNodeBlueprint<Native> for SolanaOffchainAuthBlueprint<S, R>
where
    S: Spec + PluggableSpec,
    R: RuntimeTrait<S>
        + HasRestApi<S>
        + HasCapabilities<S>
        + HasKernel<S>
        + SolanaOffchainAuthenticatorTrait<S>
        + 'static,
    RtAgnosticBlueprint<S, R>: FullNodeBlueprint<Native>,
    // Add constraint that our Runtime is compatible with SolanaOffchainAuthenticatorTrait
    <RtAgnosticBlueprint<S, R> as RollupBlueprint<Native>>::Runtime:
        SolanaOffchainAuthenticatorTrait<
            <RtAgnosticBlueprint<S, R> as RollupBlueprint<Native>>::Spec,
        >,
{
    type DaService = <RtAgnosticBlueprint<S, R> as FullNodeBlueprint<Native>>::DaService;
    type StorageManager = <RtAgnosticBlueprint<S, R> as FullNodeBlueprint<Native>>::StorageManager;
    type ProverService = <RtAgnosticBlueprint<S, R> as FullNodeBlueprint<Native>>::ProverService;
    type ProofSender = <RtAgnosticBlueprint<S, R> as FullNodeBlueprint<Native>>::ProofSender;

    fn create_outer_code_commitment(
        &self,
    ) -> <<Self::ProverService as sov_stf_runner::processes::ProverService>::Verifier as sov_modules_api::ZkVerifier>::CodeCommitment
    {
        self.inner.create_outer_code_commitment()
    }

    async fn create_endpoints(
        &self,
        state_update_receiver: StateUpdateReceiver<<Self::Spec as Spec>::Storage>,
        sync_status_receiver: tokio::sync::watch::Receiver<SyncStatus>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
        ledger_db: &LedgerDb,
        sequencer: &SequencerCreationReceipt<Self::Spec>,
        da_service: &Self::DaService,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints> {
        self.inner
            .create_endpoints(
                state_update_receiver,
                sync_status_receiver,
                shutdown_receiver,
                ledger_db,
                sequencer,
                da_service,
                rollup_config,
            )
            .await
    }

    async fn sequencer_additional_apis<Seq>(
        &self,
        sequencer: Arc<Seq>,
        _rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<NodeEndpoints>
    where
        Seq: Sequencer<Spec = Self::Spec, Rt = Self::Runtime, Da = Self::DaService>,
    {
        let router = axum::Router::new()
            .route(
                "/sequencer/accept_solana_offchain_tx",
                post(accept_solana_offchain_tx::<Seq>),
            )
            .with_state(sequencer);

        Ok(NodeEndpoints {
            axum_router: router,
            jsonrpsee_module: jsonrpsee::RpcModule::new(()),
            background_handles: Vec::new(),
        })
    }

    async fn create_da_service(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> Self::DaService {
        self.inner
            .create_da_service(rollup_config, shutdown_receiver)
            .await
    }

    async fn create_prover_service(
        &self,
        prover_config: sov_stf_runner::processes::RollupProverConfig<
            <Self::Spec as Spec>::InnerZkvm,
        >,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        da_service: &Self::DaService,
    ) -> Self::ProverService {
        self.inner
            .create_prover_service(prover_config, rollup_config, da_service)
            .await
    }

    fn create_storage_manager(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
    ) -> anyhow::Result<Self::StorageManager> {
        self.inner.create_storage_manager(rollup_config)
    }

    fn create_proof_sender(
        &self,
        rollup_config: &RollupConfig<<Self::Spec as Spec>::Address, Self::DaService>,
        proof_blob_sender: Arc<dyn ProofBlobSender>,
    ) -> anyhow::Result<Self::ProofSender> {
        self.inner
            .create_proof_sender(rollup_config, proof_blob_sender)
    }
}

/// Handler for accepting Solana offchain authenticated transactions
async fn accept_solana_offchain_tx<Seq>(
    State(sequencer): State<Arc<Seq>>,
    tx: Json<AcceptTx>,
) -> ApiResult<TxInfoWithConfirmation<DaBlobHash<<Seq::Da as DaService>::Spec>, Seq::Confirmation>>
where
    Seq: Sequencer + 'static,
    Seq::Rt: SolanaOffchainAuthenticatorTrait<Seq::Spec>,
    <Seq::Rt as RuntimeTrait<Seq::Spec>>::Auth: TransactionAuthenticator<Seq::Spec>,
{
    let raw_tx = RawTx::new(tx.0.body.blob);
    let encoded_tx = Seq::Rt::encode_with_solana_offchain_auth(raw_tx);

    // Submit to sequencer (similar to axum_accept_tx but with Solana auth)
    let tx_with_hash = tokio::spawn(async move { sequencer.accept_tx(encoded_tx).await })
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "A panic occurred while accepting a Solana offchain transaction");
            sov_rest_utils::errors::internal_server_error_response_500(
                "An internal error occurred while processing the transaction",
            )
        })?
    .map_err(|e| {
        if e.status.is_server_error() {
            tracing::error!(error = ?e, "Error accepting Solana offchain transaction");
        }
        IntoResponse::into_response(e)
    })?;

    Ok(TxInfoWithConfirmation {
        id: tx_with_hash.tx_hash,
        confirmation: tx_with_hash.confirmation,
        status: TxStatus::Submitted,
    }
    .into())
}
