use std::net::SocketAddr;

use axum::extract::{ConnectInfo, State};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Json;
use sov_modules_api::capabilities::TransactionAuthenticator;
use sov_modules_api::RawTx;
use sov_modules_stf_blueprint::Runtime as RuntimeTrait;
use sov_rollup_interface::da::DaBlobHash;
use sov_rollup_interface::node::da::DaService;
use sov_sequencer::rest_api::{AcceptTx, TxInfoWithConfirmation};
use sov_sequencer::{Sequencer, TxStatus};
use sov_solana_offchain_auth::SolanaOffchainAuthenticatorTrait;

pub(crate) fn solana_offchain_router<Seq>(sequencer: Seq) -> axum::Router
where
    Seq: Sequencer + 'static,
    Seq::Rt: SolanaOffchainAuthenticatorTrait<Seq::Spec>,
    <Seq::Rt as RuntimeTrait<Seq::Spec>>::Auth: TransactionAuthenticator<Seq::Spec>,
{
    axum::Router::new()
        .route(
            "/sequencer/accept-solana-offchain-tx",
            post(accept_solana_offchain_tx::<Seq>),
        )
        .with_state(sequencer)
}

async fn accept_solana_offchain_tx<Seq>(
    connect_info: ConnectInfo<SocketAddr>,
    State(sequencer): State<Seq>,
    tx: Json<AcceptTx>,
) -> Result<
    Json<TxInfoWithConfirmation<DaBlobHash<<Seq::Da as DaService>::Spec>, Seq::Confirmation>>,
    axum::response::Response,
>
where
    Seq: Sequencer + 'static,
    Seq::Rt: SolanaOffchainAuthenticatorTrait<Seq::Spec>,
    <Seq::Rt as RuntimeTrait<Seq::Spec>>::Auth: TransactionAuthenticator<Seq::Spec>,
{
    let raw_tx = RawTx::new(tx.0.body.blob);
    let encoded_tx = Seq::Rt::encode_with_solana_offchain_auth(raw_tx);

    let tx_with_hash = sequencer
        .accept_tx(encoded_tx, connect_info.0.ip())
        .await
        .map_err(|e| {
            if e.status.is_server_error() {
                tracing::error!(error = ?e, "Error accepting Solana offchain transaction");
            }
            IntoResponse::into_response(e)
        })?;

    Ok(Json(TxInfoWithConfirmation {
        id: tx_with_hash.tx_hash,
        confirmation: tx_with_hash.confirmation,
        status: TxStatus::Submitted,
    }))
}
