use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;
use sov_modules_api::prelude::serde_json::json;
use sov_modules_api::prelude::utoipa::openapi::OpenApi;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, HexHash, Spec};

use crate::merkle::MerkleTreeHook;

/// A checkpoint of a merkle tree, being it's root and index.
///
/// Checkpoints are indexed since first insertion, i.e.
/// empty tree doesn't have a checkpoint.
#[derive(Serialize)]
pub struct Checkpoint {
    /// A root of the merkle tree.
    root: HexHash,
    /// A count of the checkpoint.
    index: u32,
}

impl<S: Spec> HasCustomRestApi for MerkleTreeHook<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/checkpoint", get(Self::get_checkpoint))
            .route("/count", get(Self::get_count))
            .with_state(state.with(self.clone()))
    }

    fn custom_openapi_spec(&self) -> Option<OpenApi> {
        let mut open_api: OpenApi =
            serde_yaml::from_str(include_str!("openapi-v3.yaml")).expect("Invalid OpenAPI spec");
        // Because https://github.com/juhaku/utoipa/issues/972
        for path_item in open_api.paths.paths.values_mut() {
            path_item.extensions = None;
        }
        Some(open_api)
    }
}

impl<S: Spec> MerkleTreeHook<S> {
    async fn get_count(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
    ) -> Result<Response, Response> {
        let count = state
            .tree
            .get(&mut accessor)
            .unwrap_infallible()
            // we could `.unwrap_or_default` there, but this matches the behavior
            // of /module/merkle-tree-hook/state/tree
            .ok_or_else(|| errors::not_found_404("MerkleTreeHook", "count"))?
            .count;

        Ok(axum::Json(json!({"count": count})).into_response())
    }

    async fn get_checkpoint(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
    ) -> ApiResult<Checkpoint> {
        let tree = state
            .tree
            .get(&mut accessor)
            .unwrap_infallible()
            .ok_or_else(|| errors::not_found_404("MerkleTreeHook", "checkpoint"))?;

        // checkpoint's index start from first insertion
        let index = tree
            .count
            .checked_sub(1)
            .ok_or_else(|| errors::not_found_404("MerkleTreeHook", "checkpoint"))?;
        let root = tree
            .root(&mut accessor)
            .expect("Should not fail charging gas");
        let checkpoint = Checkpoint { root, index };
        Ok(checkpoint.into())
    }
}
