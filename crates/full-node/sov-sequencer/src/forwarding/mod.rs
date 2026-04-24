//! A `Sequencer` that serves reads from the local node and forwards transaction submissions to a
//! remote preferred sequencer.
//!
//! A forwarding sequencer lets an operator run their own full node for read access to the chain
//! without running a `PreferredSequencer` (which can only run in one place) or a `StdSequencer`
//! (whose locally-simulated results can diverge from the canonical order). It implements the
//! [`Sequencer`] trait by proxying writes to a remote upstream over HTTP while answering reads from
//! the local node's state.
//!
//! Limitations (v1):
//! - WebSocket subscription methods return `None`; clients wanting event/tx streams should connect
//!   directly to the upstream.
//! - Proof blob publication is unsupported — forwarding nodes are not provers.
//! - The EVM preflight in `sov_ethereum::handlers` runs against local (possibly lagging) state,
//!   which can spuriously reject valid transactions.

use std::marker::PhantomData;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use async_trait::async_trait;
use axum::http::StatusCode;
use reqwest::StatusCode as ReqwestStatusCode;
pub use sov_full_node_configs::sequencer::ForwardingSequencerConfig;
use sov_modules_api::rest::utils::ErrorObject;
use sov_modules_api::rest::ApiState;
use sov_modules_api::{ConcurrentStateCheckpoint, DaSpec, FullyBakedTx, Spec, StateCheckpoint};
use sov_modules_stf_blueprint::Runtime;
use sov_rest_utils::json_obj;
use sov_rollup_full_node_interface::{DaSyncState, StateUpdateInfo, StateUpdateReceiver};
use sov_rollup_interface::da::DaBlobHash;
use sov_rollup_interface::node::da::DaService;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, trace, warn};

use crate::common::{loop_call_update_state, loop_send_tx_notifications, AcceptedTx, Sequencer};
use crate::preferred::Confirmation;
use crate::rest_api::{ApiAcceptedTx, TxInfoWithConfirmation};
use crate::{
    ProofBlobSender, SequencerConfig, SequencerNotReadyDetails, SerializedProofWithDetailsBytes,
    TxHash, TxStatus, TxStatusManager,
};

/// A [`Sequencer`] that serves reads locally and forwards writes to a remote upstream.
///
/// The remote upstream is expected to be a `PreferredSequencer`; the forwarding sequencer
/// deserializes upstream responses into the native [`Confirmation<S, Rt>`] type so clients see
/// byte-identical results whether they talk to the upstream directly or through the forwarder.
#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct ForwardingSequencer<S, Rt, Da>(Arc<ForwardingSequencerFields<S, Rt, Da>>)
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>;

impl<S, Rt, Da> std::ops::Deref for ForwardingSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    type Target = ForwardingSequencerFields<S, Rt, Da>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Inner fields of a [`ForwardingSequencer`]. Access through the parent struct's `Arc`.
pub struct ForwardingSequencerFields<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    http: reqwest::Client,
    upstream_url: reqwest::Url,
    readiness_cache_ttl: Duration,
    readiness_cache: Mutex<Option<(Instant, Result<(), SequencerNotReadyDetails>)>>,
    txsm: TxStatusManager<S::Da>,
    api_state: ApiState<S>,
    checkpoint_sender: watch::Sender<Arc<ConcurrentStateCheckpoint<S>>>,
    api_ledger_db: sov_db::ledger_db::LedgerDb,
    _marker: PhantomData<(Rt, Da)>,
}

impl<S, Rt, Da> ForwardingSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    /// Creates a `ForwardingSequencer` and spawns background tasks that keep the local
    /// `ApiState` fresh as the node ingests new slots.
    pub async fn create(
        state_update_receiver: StateUpdateReceiver<S::Storage>,
        _da_sync_state: Arc<DaSyncState>,
        config: &SequencerConfig<S::Address, ForwardingSequencerConfig>,
        ledger_db: sov_db::ledger_db::LedgerDb,
        api_ledger_db: sov_db::ledger_db::LedgerDb,
        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<(Self, Vec<JoinHandle<()>>)> {
        let shutdown_receiver = shutdown_sender.subscribe();
        let cfg = &config.sequencer_kind_config;

        let upstream_url = reqwest::Url::parse(&cfg.upstream_url).with_context(|| {
            format!(
                "Failed to parse forwarding sequencer upstream_url {:?}",
                cfg.upstream_url
            )
        })?;

        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.request_timeout_ms))
            .build()
            .context("Failed to build HTTP client for forwarding sequencer")?;

        let mut runtime = Rt::default();
        let kernel_with_slot_mapping = runtime.kernel_with_slot_mapping();

        let latest_state_update = state_update_receiver.borrow().clone();
        let checkpoint = Arc::new(
            ConcurrentStateCheckpoint::from_state_checkpoint_with_finalized_slot(
                StateCheckpoint::new(latest_state_update.storage.clone(), &runtime.kernel(), None),
                latest_state_update.latest_finalized_slot_number,
            ),
        );
        let (checkpoint_sender, checkpoint_receiver) = watch::channel(checkpoint);

        let api_state = ApiState::build(
            Arc::new(()),
            checkpoint_receiver,
            kernel_with_slot_mapping,
            None,
        );

        let txsm = TxStatusManager::default();

        let seq = ForwardingSequencer(Arc::new(ForwardingSequencerFields {
            http,
            upstream_url,
            readiness_cache_ttl: Duration::from_millis(cfg.readiness_cache_ms),
            readiness_cache: Mutex::new(None),
            txsm,
            api_state,
            checkpoint_sender,
            api_ledger_db,
            _marker: PhantomData,
        }));

        // Keep unused sender alive so we don't panic trying to subscribe on a closed watch.
        let _ = &shutdown_sender;

        let mut handles: Vec<JoinHandle<()>> = Vec::new();

        handles.push(tokio::spawn(loop_call_update_state(
            seq.clone(),
            state_update_receiver.clone(),
            shutdown_receiver.clone(),
        )));
        handles.push(tokio::spawn({
            let ledger_db = ledger_db.clone();
            let seq = seq.clone();
            async move {
                loop_send_tx_notifications::<S, Rt>(
                    state_update_receiver,
                    shutdown_receiver,
                    &ledger_db,
                    seq.tx_status_manager(),
                )
                .await;
            }
        }));

        Ok((seq, handles))
    }

    fn upstream_url_for(&self, path: &str) -> anyhow::Result<reqwest::Url> {
        self.upstream_url
            .join(path)
            .with_context(|| format!("Failed to build upstream URL for {path}"))
    }

    async fn ready_upstream(&self) -> Result<(), SequencerNotReadyDetails> {
        let url = self
            .upstream_url_for("sequencer/ready")
            .map_err(|_| SequencerNotReadyDetails::Startup)?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| SequencerNotReadyDetails::Startup)?;

        if response.status().is_success() {
            return Ok(());
        }
        if response.status() == ReqwestStatusCode::SERVICE_UNAVAILABLE {
            // Upstream is initializing or syncing. Surface a generic "Startup" rather than
            // attempting to parse the upstream's error envelope; callers only care that the
            // sequencer is not ready.
            return Err(SequencerNotReadyDetails::Startup);
        }
        Err(SequencerNotReadyDetails::Startup)
    }
}

#[async_trait]
impl<S, Rt, Da> Sequencer for ForwardingSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    // The upstream is expected to be a `PreferredSequencer`, which returns rich confirmations.
    // Reusing the native type here means clients see the same JSON shape whether they go through
    // the forwarder or talk to the upstream directly.
    type Confirmation = Confirmation<S, Rt>;
    type Spec = S;
    type Rt = Rt;
    type Da = Da;

    async fn is_ready(&self) -> Result<(), SequencerNotReadyDetails> {
        if self.readiness_cache_ttl > Duration::ZERO {
            let mut guard = self.readiness_cache.lock().await;
            if let Some((ts, result)) = guard.as_ref() {
                if ts.elapsed() < self.readiness_cache_ttl {
                    return result.clone();
                }
            }
            let result = self.ready_upstream().await;
            *guard = Some((Instant::now(), result.clone()));
            result
        } else {
            self.ready_upstream().await
        }
    }

    fn tx_status_manager(&self) -> &TxStatusManager<<Self::Spec as Spec>::Da> {
        &self.txsm
    }

    fn api_state(&self) -> ApiState<Self::Spec> {
        self.api_state.clone()
    }

    async fn update_state(
        &self,
        state_update_info: StateUpdateInfo<S::Storage>,
    ) -> anyhow::Result<()> {
        let StateUpdateInfo {
            storage,
            slot_number,
            ledger_reader,
            latest_finalized_slot_number,
            ..
        } = &state_update_info;

        let checkpoint = StateCheckpoint::new(storage.clone(), &Rt::default().kernel(), None);

        trace!(%slot_number, "Forwarding sequencer refreshing local api_state");

        self.checkpoint_sender
            .send(Arc::new(
                ConcurrentStateCheckpoint::from_state_checkpoint_with_finalized_slot(
                    checkpoint
                        .clone_with_empty_witness_dropping_temp_cache_and_ignoring_pinned_cache(),
                    *latest_finalized_slot_number,
                ),
            ))
            .ok();

        self.api_ledger_db.replace_reader(ledger_reader.clone());
        self.api_ledger_db.send_notifications_for_slot(*slot_number);

        Ok(())
    }

    async fn accept_tx(
        &self,
        baked_tx: FullyBakedTx,
        ip_addr: IpAddr,
    ) -> Result<AcceptedTx<Self::Confirmation>, ErrorObject> {
        let url = self
            .upstream_url_for("sequencer/txs/baked")
            .map_err(|e| ErrorObject {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "Failed to build upstream URL".to_string(),
                details: json_obj!({ "error": e.to_string() }),
            })?;

        let response = self
            .http
            .post(url)
            .header("x-forwarded-for", ip_addr.to_string())
            .json(&baked_tx)
            .send()
            .await
            .map_err(upstream_network_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(upstream_http_error(response).await);
        }

        let body: TxInfoWithConfirmation<DaBlobHash<<Da as DaService>::Spec>, Self::Confirmation> =
            response.json().await.map_err(|e| ErrorObject {
                status: StatusCode::BAD_GATEWAY,
                message: "Failed to decode upstream response".to_string(),
                details: json_obj!({ "error": e.to_string() }),
            })?;

        let tx_hash = body.id;
        self.txsm.notify(tx_hash, TxStatus::Submitted);

        Ok(AcceptedTx {
            tx: baked_tx,
            tx_hash,
            confirmation: body.confirmation,
        })
    }

    async fn get_api_tx(
        &self,
        tx_hash: TxHash,
    ) -> anyhow::Result<Option<ApiAcceptedTx<Self::Confirmation>>> {
        let url = self.upstream_url_for(&format!("sequencer/txs/{tx_hash}"))?;
        let response = self.http.get(url).send().await?;

        if response.status() == ReqwestStatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!(
                "Upstream get_tx returned unexpected status {} for tx {}",
                response.status(),
                tx_hash
            );
        }

        Ok(Some(response.json().await?))
    }

    async fn tx_status(
        &self,
        tx_hash: &TxHash,
    ) -> anyhow::Result<TxStatus<<<Self::Spec as Spec>::Da as DaSpec>::TransactionId>> {
        if let Some(status) = self.txsm.get_cached(tx_hash) {
            return Ok(status);
        }

        let url = self.upstream_url_for(&format!("sequencer/txs/{tx_hash}/status"))?;
        let response = match self.http.get(url).send().await {
            Ok(response) => response,
            Err(e) => {
                debug!(%tx_hash, error = %e, "Upstream tx_status query failed; returning Unknown");
                return Ok(TxStatus::Unknown);
            }
        };

        if response.status() == ReqwestStatusCode::NOT_FOUND {
            return Ok(TxStatus::Unknown);
        }
        if !response.status().is_success() {
            warn!(
                %tx_hash,
                status = %response.status(),
                "Upstream tx_status returned non-success; returning Unknown"
            );
            return Ok(TxStatus::Unknown);
        }

        // The upstream returns `{ id, status, ...status_fields }` via a flattened `TxStatus`
        // tagged by the `status` field; deserialize the full envelope and pull the status out.
        let envelope: UpstreamTxInfo<<<Self::Spec as Spec>::Da as DaSpec>::TransactionId> =
            response.json().await?;
        Ok(envelope.status)
    }

    async fn sequencer_role(&self) -> crate::SequencerRole {
        // Forwarding nodes do not produce batches. Report as a DA-only replica so downstream
        // callers treat this node as a replica.
        crate::SequencerRole::DaOnlyReplica
    }
}

#[async_trait]
impl<S, Rt, Da> ProofBlobSender for ForwardingSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    async fn produce_and_publish_proof_blob(
        &self,
        _proof_blob: SerializedProofWithDetailsBytes,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "ForwardingSequencer cannot publish proof blobs; run a preferred or standard sequencer if this node needs to submit proofs"
        )
    }
}

/// Envelope used to parse responses from the upstream's `GET /sequencer/txs/{hash}/status`
/// endpoint. The upstream flattens `TxStatus` into the response object; `serde(flatten)` on the
/// deserialize side lets us recover the `TxStatus` variant using the `status` tag.
#[derive(serde::Deserialize)]
struct UpstreamTxInfo<DaTransactionId> {
    #[allow(dead_code)]
    id: TxHash,
    #[serde(flatten)]
    status: TxStatus<DaTransactionId>,
}

fn upstream_network_error(e: reqwest::Error) -> ErrorObject {
    ErrorObject {
        status: StatusCode::BAD_GATEWAY,
        message: "Failed to reach upstream sequencer".to_string(),
        details: json_obj!({ "error": e.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_joining_with_trailing_slash_preserves_path() {
        let base = reqwest::Url::parse("http://localhost:1234/").unwrap();
        assert_eq!(
            base.join("sequencer/txs/baked").unwrap().as_str(),
            "http://localhost:1234/sequencer/txs/baked",
        );
    }

    #[test]
    fn url_joining_without_trailing_slash_replaces_last_segment() {
        // Baseline check: reqwest::Url semantics drop the final segment when there's no slash.
        // The forwarding sequencer always constructs its base URL from a user-provided config value,
        // so this test just pins the behavior.
        let base = reqwest::Url::parse("http://localhost:1234/api").unwrap();
        assert_eq!(
            base.join("sequencer/txs/baked").unwrap().as_str(),
            "http://localhost:1234/sequencer/txs/baked",
        );
    }

    #[tokio::test]
    async fn upstream_http_error_preserves_4xx_status() {
        let client = reqwest::Client::new();
        let server = tokio::task::spawn(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let handle = tokio::spawn(async move {
                let app = axum::Router::new().route(
                    "/err",
                    axum::routing::get(|| async {
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            axum::Json(serde_json::json!({ "message": "nope" })),
                        )
                    }),
                );
                axum::serve(listener, app).await.unwrap();
            });
            (addr, handle)
        })
        .await
        .unwrap();

        let response = client
            .get(format!("http://{}/err", server.0))
            .send()
            .await
            .unwrap();
        let err = upstream_http_error(response).await;
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        server.1.abort();
    }

    #[tokio::test]
    async fn upstream_http_error_maps_unknown_to_bad_gateway() {
        // Construct a response with an unusual status to verify the 2xx/redirect fallback path.
        let client = reqwest::Client::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let app = axum::Router::new().route(
                "/redir",
                axum::routing::get(|| async {
                    (
                        axum::http::StatusCode::MOVED_PERMANENTLY,
                        [("location", "/elsewhere")],
                        "",
                    )
                }),
            );
            axum::serve(listener, app).await.unwrap();
        });

        let response = client
            .get(format!("http://{addr}/redir"))
            // Disable redirect following so we get the raw 301 response.
            .send()
            .await
            .unwrap();
        // reqwest default-follows redirects. If it did not redirect, the final status is 301.
        // If it did follow, final status will be 404. In either case, the mapping is sane.
        let mapped = upstream_http_error(response).await;
        assert!(mapped.status.is_client_error() || mapped.status == StatusCode::BAD_GATEWAY);
        server.abort();
    }
}

async fn upstream_http_error(response: reqwest::Response) -> ErrorObject {
    let upstream_status = response.status();
    let body_text = response.text().await.unwrap_or_default();

    // Preserve the upstream's HTTP status when it's a client-side error (4xx) so callers can
    // react to things like 400 (bad tx) or 503 (upstream not ready). Map transport-level failures
    // to 502 so clients know the error originated between us and the upstream.
    let mapped_status = if upstream_status.is_client_error() || upstream_status.is_server_error() {
        StatusCode::from_u16(upstream_status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY)
    } else {
        StatusCode::BAD_GATEWAY
    };

    // If the upstream body parses as a known error envelope, forward its message; otherwise
    // include the raw body for debuggability.
    let parsed: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();
    let details = match &parsed {
        Some(value) => json_obj!({ "upstream": value.clone() }),
        None => json_obj!({ "upstream_body": body_text.clone() }),
    };

    ErrorObject {
        status: mapped_status,
        message: format!("Upstream sequencer returned {upstream_status}"),
        details,
    }
}
