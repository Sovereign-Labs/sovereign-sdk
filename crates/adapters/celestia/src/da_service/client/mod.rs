use crate::da_service::client::twinkle::TwinkleClient;
use crate::da_service::client::vanilla::VanillaClient;
use crate::types::{RollupNamespace, TmHash};
use crate::verifier::address::CelestiaAddress;
use sov_rollup_interface::node::da::SubmitBlobReceipt;
use tokio::sync::oneshot;

/// Celestia client implementations
pub mod twinkle;
pub mod vanilla;

#[derive(Debug, Clone)]
pub enum CelestiaClient {
    Vanilla(VanillaClient),
    Twinkle(TwinkleClient),
}

impl CelestiaClient {
    pub async fn submit_blob_to_namespace(
        &self,
        blob: &[u8],
        namespace: RollupNamespace,
        signer: &CelestiaAddress,
    ) -> oneshot::Receiver<anyhow::Result<SubmitBlobReceipt<TmHash>>> {
        match self {
            CelestiaClient::Vanilla(vanilla_client) => {
                vanilla_client
                    .submit_blob_to_namespace(blob, namespace, signer)
                    .await
            }
            CelestiaClient::Twinkle(twinkle_client) => {
                twinkle_client
                    .submit_blob_to_namespace_inner(blob, namespace, signer)
                    .await
            }
        }
    }
}
