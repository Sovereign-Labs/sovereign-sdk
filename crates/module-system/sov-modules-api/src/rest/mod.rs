//! Traits and utilities for writing REST(ful) APIs that expose rollup data.
//!
//! Sovereign rollup nodes can expose their data over HTTP in two ways:
//!
//! 1. A JSON-RPC interface. This is mostly intended for cross-compatbility with
//!    third-party JSON-RPC interfaces e.g. the Ethereum JSON-RPC API.
//! 2. An [`axum`]-based REST API. Axum is the most flexible and capable
//!    solution out of the two listed here, and integrates most easily with the
//!    rest of the Sovereign SDK ecosystem.
//!
//! This Rust module is exclusively concerned with the latter case, and exposes
//! all the necessary tools for composing Axum routers within the rollup node.
//! If you're looking for JSON-RPC API documentation, please refer to
//! [`crate::macros::expose_rpc`].
//!
//! Nodes expose rollup data using a combination of three traits:
//!
//! | Trait | Derivable | Implemented by |
//! | ----- | --------- | ---- |
//! | [`crate::rest::HasRestApi`] | With [`crate::ModuleRestApi`] and [`crate::macros::RuntimeRestApi`] | Modules and runtimes |
//! | [`crate::rest::HasCustomRestApi`]   | ❌ | Modules |
//!
//! Implementing or deriving *any* of these traits is optional, but types
//! implementing [`crate::rest::HasCustomRestApi`] ought to also derive [`crate::rest::HasRestApi`], or
//! no REST API will be available for them.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{FromRequestParts, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use sov_rest_utils::{json_obj, ErrorObject, Query};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::StateUpdateInfo;
use tokio::sync::watch;
use utoipa::openapi::OpenApi;

use crate::capabilities::{KernelWithSlotMapping, RollupHeight};
use crate::hooks::TxHooks;
use crate::state::VersionReader;
use crate::{ApiStateAccessor, ModuleId, ModuleInfo, Spec, StateCheckpoint};

/// This Rust module is **NOT** part of the public API of the crate, and can
/// change at any time.
#[doc(hidden)]
pub mod __private;

/// A [`tokio::sync::watch::Receiver`] for a [`Spec`]'s storage.
pub type StateUpdateReceiver<S> = tokio::sync::watch::Receiver<StateUpdateInfo<S>>;

pub use sov_modules_macros::{ModuleRestApi, RuntimeRestApi};

/// Utilities for building opinionated REST(ful) APIs with [`axum`].
#[doc(inline)]
pub extern crate sov_rest_utils as utils;

/// This trait is intended to be derived via
/// [`crate::ModuleRestApi`] by modules and via
/// [`crate::macros::RuntimeRestApi`] by runtimes.
/// runtimes, and it provides a fair amount of paths and general information
/// about the rollup, including but not limited to:
///
/// - A list of all modules included in the runtime, and related metadata.
/// - Module state, both latest and historical; per-variable state overview;
///   pagination for structured state items like
///   [`StateVec`](crate::containers::StateVec).
pub trait HasRestApi<S: Spec> {
    /// Returns an [`axum::Router`] on the provided [`StateUpdateReceiver`] instance for the REST API.
    fn rest_api(&self, _state: ApiState<S>) -> axum::Router<()>;

    /// Returns the OpenAPI specification for [`HasRestApi::rest_api`].
    /// [`None`] means there is no known OpenAPI spec for the API.
    fn openapi_spec(&self) -> Option<OpenApi> {
        None
    }
}

/// Makes deriving [`HasRestApi`] for modules optional, with the autoref trick.
impl<M: ModuleInfo> HasRestApi<M::Spec> for &M {
    fn rest_api(&self, _state: ApiState<M::Spec>) -> axum::Router<()> {
        axum::Router::new()
    }

    fn openapi_spec(&self) -> Option<OpenApi> {
        None
    }
}

/// Optionally exposes hand-written, custom API routes for a module or
/// runtime.
///
/// This trait cannot be derived, and implementing it is entirely optional.
/// A module that implements this trait will be automatically exposed as part of
/// the runtime API, as [`ModuleRestApi`] will automatically merge the two.
///
/// # Example
///
/// ```
/// use sov_modules_api::prelude::*;
/// use sov_modules_api::{ModuleId, ModuleInfo, StateValue};
/// use sov_modules_api::rest::{HasCustomRestApi, ApiState};
///
/// #[derive(Clone, ModuleInfo, ModuleRestApi)]
/// struct MyModule<S: Spec> {
///     #[id]
///     id: ModuleId,
///     #[state]
///     state_item: StateValue<S::Address>,
/// }
///
/// impl<S: Spec> HasCustomRestApi for MyModule<S> {
///     type Spec = S;
///
///     fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
///         use axum::routing::get;
///
///         axum::Router::new()
///             .route("/", get(|| async { "Hello, world!" }))
///             .with_state(state)
///     }
/// }
/// # // BEGIN MODULE IMPL, COPY-PASTE-ME (https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html#hiding-portions-of-the-example)
/// # impl<S: Spec> sov_modules_api::Module for MyModule<S> {
/// #    type Spec = S;
/// #    type Config = ();
/// #    type CallMessage = ();
/// #    type Event = ();
/// #
/// #    fn call(
/// #        &mut self,
/// #        _msg: Self::CallMessage,
/// #        _context: &Context<Self::Spec>,
/// #        _state: &mut impl sov_modules_api::state::TxState<S>,
/// #    ) -> anyhow::Result<()> {
/// #        unimplemented!()
/// #    }
/// # }
/// # // END MODULE IMPL
/// ```
pub trait HasCustomRestApi: Sized + Clone {
    /// Spec for [`ApiState`]
    type Spec: Spec;

    /// Returns an [`axum::Router`] on the provided [`ApiState`] instance for the REST API.
    fn custom_rest_api(&self, state: ApiState<Self::Spec>) -> axum::Router<()>;

    /// Returns the OpenAPI specification for [`HasCustomRestApi::custom_rest_api`].
    /// [`None`] means there is no known OpenAPI spec for the API.
    fn custom_openapi_spec(&self) -> Option<OpenApi> {
        None
    }
}

/// In case [`HasCustomRestApi`] is implemented for a [`Module`](crate::Module), an
/// empty [`axum::Router`] will be returned instead.
///
/// This "blanket" implementation uses the [Autoref-based stable specialization](https://github.com/dtolnay/case-studies/tree/master/autoref-specialization)
impl<T: ModuleInfo + Default> HasCustomRestApi for &T {
    type Spec = T::Spec;

    fn custom_rest_api(&self, _state: ApiState<Self::Spec>) -> axum::Router<()> {
        tracing::trace!(
            module = T::default().prefix().module_name(),
            "No `HasCustomRestApi` implementation found for module"
        );
        axum::Router::new()
    }
}

/// A wrapper around [`Spec::Storage`] that is appropriate for use as a state
/// type of module and runtime [`axum::Router`]s.
#[derive(derive_more::Deref, derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct ApiState<S: Spec, T = ()> {
    #[deref]
    inner: Arc<T>,
    checkpoint_receiver: watch::Receiver<StateCheckpoint<S>>,
    kernel: Arc<dyn KernelWithSlotMapping<S>>,
    /// The `height` query parameter extracted from the request, when applicable.
    requested_height: Option<HeightParam>,
}

impl<S: Spec, T> ApiState<S, T> {
    /// Creates an [`ApiState`] backed by a Tokio [`watch`] channel of
    /// [`StateCheckpoint`]s.
    pub fn build(
        inner: Arc<T>,
        checkpoint_receiver: watch::Receiver<StateCheckpoint<S>>,
        kernel: Arc<dyn KernelWithSlotMapping<S>>,
        requested_height: Option<HeightParam>,
    ) -> Self {
        Self {
            inner,
            checkpoint_receiver,
            kernel,
            requested_height,
        }
    }

    /// Replaces the inner data with a new value.
    pub fn with<T1>(self, inner: T1) -> ApiState<S, T1> {
        ApiState {
            inner: Arc::new(inner),
            checkpoint_receiver: self.checkpoint_receiver,
            kernel: self.kernel,
            requested_height: self.requested_height,
        }
    }

    /// Returns an [`ApiStateAccessor`] that you can use to read state from within REST API. This accessor
    /// honors the rollup_height query param. If you want to read state from a different height,
    /// use [`Self::build_api_state_accessor`] instead.
    ///
    /// ## Note
    /// This method can return an error if the requested height is invalid (ie the rollup has not reached it yet for instance).
    pub fn default_api_state_accessor(&self) -> ApiStateAccessor<S> {
        self.build_api_state_accessor(self.requested_height).expect(
            "Impossible to build a default api state accessor. This is a bug. Please report it.",
        )
    }

    /// Returns an [`ApiStateAccessor`] that you can use to read state from within REST
    /// API. The new accessor can be set to read any historical rollup state available to the node,
    /// or to read the rollup's latest state (by passing `None` as the height parameter).
    ///
    /// ## Note
    /// This method tries to retrieve the base fee per gas at the requested height. In case of failure, it
    /// uses a zeroed gas price.
    pub fn build_api_state_accessor(
        &self,
        height_param: Option<HeightParam>,
    ) -> Result<ApiStateAccessor<S>, anyhow::Error> {
        let checkpoint = self.checkpoint_receiver.borrow();

        let kernel = self.kernel.clone();

        tracing::trace!(?height_param, "Building an API state accessor");
        let state = match height_param {
            Some(HeightParam::RollupHeight(height)) => {
                let mut state =
                    ApiStateAccessor::new_archival(&checkpoint, kernel.clone(), height)?;
                // This is not a security isse and this code runs offchain.
                // TODO: Move this inside the constructor
                // <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2244>
                let gas_price = self
                .kernel
                .base_fee_per_gas_at(height, &mut state)
                .ok_or_else(|| {
                    anyhow::anyhow!("Impossible to get the rollup state at the specified height. Please ensure you have queried the correct height.")
                })?;

                state.set_gas_price(gas_price);

                state
            }
            Some(HeightParam::SlotNumber(slot_number)) => {
                // This constructor sets the gas price correctly, so we don't need to do it manually.
                ApiStateAccessor::new_archival_with_true_slot_number(
                    &checkpoint,
                    kernel.clone(),
                    slot_number,
                )?
            }
            None => {
                let height = checkpoint.rollup_height_to_access();
                let mut state = ApiStateAccessor::new(&checkpoint, kernel.clone());
                // This is not a security isse and this code runs offchain.
                // TODO: Move this inside the constructor
                // <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/2244>
                let gas_price = self
                .kernel
                .base_fee_per_gas_at(height, &mut state)
                .ok_or_else(|| {
                    anyhow::anyhow!("Impossible to get the rollup state at the specified height. Please ensure you have queried the correct height.")
                })?;

                state.set_gas_price(gas_price);

                state
            }
        };

        Ok(state)
    }
}

/// The height parameter for REST API requests. This can be a rollup height or a slot number.
#[derive(Debug, Clone, Copy)]
pub enum HeightParam {
    /// The rollup height to access.
    RollupHeight(RollupHeight),
    /// The slot number to access.
    SlotNumber(SlotNumber),
}

// Custom deserializer to work with serde_url_encoded, which chokes on enums
impl<'de> Deserialize<'de> for HeightParam {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        #[derive(Deserialize)]
        struct HeightParamInner {
            rollup_height: Option<RollupHeight>,
            slot_number: Option<SlotNumber>,
        }

        let input = HeightParamInner::deserialize(deserializer)?;
        match (input.rollup_height, input.slot_number) {
            (Some(rollup_height), None) => Ok(HeightParam::RollupHeight(rollup_height)),
            (None, Some(slot_number)) => Ok(HeightParam::SlotNumber(slot_number)),
            (Some(_), Some(_)) => Err(D::Error::custom(
                "Only one of rollup_height or slot_number can be provided",
            )),
            (None, None) => Err(D::Error::custom("No height parameter provided")),
        }
    }
}
#[axum::async_trait]
impl<S, T> FromRequestParts<ApiState<S, T>> for ApiState<S, T>
where
    S: Spec,
    T: Send + Sync,
{
    type Rejection = utils::ErrorObject;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &ApiState<S, T>,
    ) -> Result<Self, Self::Rejection> {
        let height_param = Query::<HeightParam>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|q| q.0);

        let mut output = state.clone();
        output.requested_height = height_param;
        Ok(output)
    }
}

#[axum::async_trait]
impl<S, T> FromRequestParts<ApiState<S, T>> for ApiStateAccessor<S>
where
    T: Send + Sync,
    S: Spec,
{
    type Rejection = utils::ErrorObject;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &ApiState<S, T>,
    ) -> Result<Self, Self::Rejection> {
        let height_param = Query::<HeightParam>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|q| q.0);

        state
            .build_api_state_accessor(height_param)
            .map_err(|e| ErrorObject {
                status: StatusCode::NOT_FOUND,
                message: "invalid rollup height".to_string(),
                details: json_obj!({
                    "error": e.to_string(),
                }),
            })
    }
}
