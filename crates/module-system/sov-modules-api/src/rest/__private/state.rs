//! When auto-generating APIs for state items, we have a few
//! requirements:
//!
//! 1. We require a "blanket" implementation on **ALL** state items,
//!    even the ones that for which we can't provide an API, for
//!    the autoref trick. Without this, compilation would fail unless
//!    all unsuitable state items were marked with `skip`. See
//!    [`StateItemRestApi`].
//! 2. We require a method that is purposefully **NOT**
//!    blanket-implemented to cause a compilation errors when unsuitable
//!    state items are marked with `include`. See
//!    [`StateItemRestApiExists`].se std::marker::PhantomData;

use std::convert::Infallible;
use std::fmt::Display;
use std::marker::PhantomData;
use std::str::FromStr;

use axum::extract::{FromRequestParts, State};
use axum::routing::get;
use serde::Serialize;
use sov_rest_utils::errors::not_found_404;
use sov_rest_utils::{ApiResult, ErrorObject, Path, Query};
use sov_rollup_interface::common::SlotNumber;
use sov_state::{CompileTimeNamespace, Kernel, Namespace, StateCodec, StateItemCodec};
use unwrap_infallible::UnwrapInfallible;

use super::types::StateItemContents;
use super::{ApiState, HeightParam, ModuleSendSync, StateItemInfo};
use crate::map::NamespacedStateMap;
use crate::rest::{json_obj, StatusCode};
use crate::value::NamespacedStateValue;
use crate::vec::NamespacedStateVec;
use crate::{ApiStateAccessor, ModuleInfo, StateReader, VersionedStateValue};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateItemKind {
    StateValue,
    StateVec,
    StateMap,
}

#[derive(derivative::Derivative)]
#[derivative(Clone(bound = ""))]
pub struct StateItemRestApiImpl<M: ModuleInfo, T> {
    pub api_state: ApiState<M::Spec>,
    pub state_item_info: StateItemInfo,
    pub phantom: PhantomData<T>,
}

#[axum::async_trait]
impl<M, T> FromRequestParts<StateItemRestApiImpl<M, T>> for ApiStateAccessor<M::Spec>
where
    M: ModuleSendSync,
    T: Send + Sync + 'static,
{
    type Rejection = crate::rest::utils::ErrorObject;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &StateItemRestApiImpl<M, T>,
    ) -> Result<Self, Self::Rejection> {
        let height_param = Query::<HeightParam>::from_request_parts(parts, state)
            .await
            .ok()
            .map(|q| q.0);

        state
            .api_state
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

pub trait StateItemRestApi {
    fn state_item_rest_api(&self) -> axum::Router<()>;
}

// "Blanket" implementation for all "unsuitable" state items e.g.
// non-`serde` compatible ones, using the autoref trick.
impl<T> StateItemRestApi for &T {
    fn state_item_rest_api(&self) -> axum::Router<()> {
        axum::Router::new()
    }
}

/// It's important for this trait to require [`StateItemRestApi`], otherwise type
/// bounds might get out of sync.
pub trait StateItemRestApiExists: StateItemRestApi {
    /// By calling this method, the proc-macro can "assert" that the type
    /// actually implements [`StateItemRestApi`].
    fn exists(&self) {}
}

impl<N, M, T, Codec> StateItemRestApiImpl<M, NamespacedStateValue<N, T, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    T: Serialize,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    async fn get_state_value_route(
        State(state): State<Self>,
        mut accessor: ApiStateAccessor<M::Spec>,
    ) -> ApiResult<StateItemContents<T, T>> {
        let state_value = NamespacedStateValue::<N, T, Codec>::with_codec(
            state.state_item_info.prefix.0.clone(),
            Codec::default(),
        );

        let value = state_value.get(&mut accessor).unwrap_infallible();
        Ok(StateItemContents::Value { value }.into())
    }
}

impl<N, M, T, Codec> StateItemRestApi for StateItemRestApiImpl<M, NamespacedStateValue<N, T, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    T: Serialize + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    fn state_item_rest_api(&self) -> axum::Router<()> {
        axum::Router::new()
            .route("/", get(Self::get_state_value_route))
            .with_state(self.clone())
    }
}

impl<N, M, T, Codec> StateItemRestApiImpl<M, NamespacedStateVec<N, T, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    T: Serialize,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<SlotNumber> + StateItemCodec<u64>,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    fn vec(&self) -> NamespacedStateVec<N, T, Codec> {
        NamespacedStateVec::with_codec(self.state_item_info.prefix.0.clone(), Codec::default())
    }

    async fn get_state_vec_route(
        state: State<Self>,
        mut accessor: ApiStateAccessor<M::Spec>,
    ) -> ApiResult<StateItemContents<T, T>> {
        let state_vec = state.vec();
        let length = state_vec.len(&mut accessor).unwrap_infallible();

        Ok(StateItemContents::Vec { length }.into())
    }

    async fn get_state_vec_item_route(
        state: State<Self>,
        mut accessor: ApiStateAccessor<M::Spec>,
        Path(item_index): Path<u64>,
    ) -> ApiResult<StateItemContents<T, T>> {
        let state_vec = state.vec();

        let value = match state_vec.get(item_index, &mut accessor).unwrap_infallible() {
            None => {
                return Err(not_found_404(&state.state_item_info.name, item_index));
            }
            Some(v) => v,
        };
        Ok(StateItemContents::VecElement {
            index: item_index,
            value,
        }
        .into())
    }
}

impl<N, M, T, Codec> StateItemRestApi for StateItemRestApiImpl<M, NamespacedStateVec<N, T, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    T: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<SlotNumber> + StateItemCodec<u64>,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    fn state_item_rest_api(&self) -> axum::Router<()> {
        axum::Router::new()
            .route("/", get(Self::get_state_vec_route))
            .route("/items/:index", get(Self::get_state_vec_item_route))
            .with_state(self.clone())
    }
}

impl<N, M, K, V, Codec> StateItemRestApiImpl<M, NamespacedStateMap<N, K, V, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    K: Serialize + serde::de::DeserializeOwned + FromStr + Display,
    V: Serialize,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    async fn get_state_map_route(State(state): State<Self>) -> ApiResult<StateItemInfo> {
        Ok(StateItemInfo {
            r#type: StateItemKind::StateMap,
            prefix: state.state_item_info.prefix,
            description: state.state_item_info.description.clone(),
            name: state.state_item_info.name.clone(),
            namespace: state.state_item_info.namespace,
        }
        .into())
    }

    async fn get_state_map_item_route(
        State(state): State<Self>,
        mut accessor: ApiStateAccessor<M::Spec>,
        Path(key): Path<K>,
    ) -> ApiResult<StateItemContents<K, V>> {
        let state_map = NamespacedStateMap::<N, K, V, Codec>::with_codec(
            state.state_item_info.prefix.0.clone(),
            Codec::default(),
        );

        let value = state_map.get(&key, &mut accessor).unwrap_infallible();
        match value {
            // Known issue, will be solved later
            // https://github.com/Sovereign-Labs/sovereign-sdk-wip/blob/f3b934e33833ec3621f46a3b31824a344de7b433/crates/full-node/sov-ledger-apis/src/lib.rs#L387
            None => Err(not_found_404(&state.state_item_info.name, "unknown")),
            Some(value) => Ok(StateItemContents::MapElement { key, value }.into()),
        }
    }
}

impl<M, V, Codec> StateItemRestApiImpl<M, VersionedStateValue<V, Codec>>
where
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<Kernel, Error = Infallible>,
    V: Serialize,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<SlotNumber> + StateItemCodec<u64>,
    Codec::ValueCodec: StateItemCodec<V> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    async fn get_state_value_route(
        State(state): State<Self>,
        mut accessor: ApiStateAccessor<M::Spec>,
    ) -> ApiResult<StateItemContents<V, V>> {
        let state_map = VersionedStateValue::<V, Codec>::with_codec(
            state.state_item_info.prefix.0.clone(),
            Codec::default(),
        );

        let value = state_map.get_current(&mut accessor).unwrap_infallible();
        Ok(StateItemContents::Value { value }.into())
    }
}

impl<N, M, K, V, Codec> StateItemRestApi
    for StateItemRestApiImpl<M, NamespacedStateMap<N, K, V, Codec>>
where
    N: CompileTimeNamespace,
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<N, Error = Infallible>,
    K: Display + FromStr + Serialize + serde::de::DeserializeOwned + Clone + Send + Sync + 'static,
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
{
    fn state_item_rest_api(&self) -> axum::Router<()> {
        axum::Router::new()
            .route("/", get(Self::get_state_map_route))
            .route("/items/:key", get(Self::get_state_map_item_route))
            .with_state(self.clone())
    }
}

impl<M, V, Codec> StateItemRestApi for StateItemRestApiImpl<M, VersionedStateValue<V, Codec>>
where
    M: ModuleSendSync,
    ApiStateAccessor<M::Spec>: StateReader<Kernel, Error = Infallible>,
    V: Serialize + Clone + Send + Sync + 'static,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<SlotNumber> + StateItemCodec<u64>,
    Codec::ValueCodec: StateItemCodec<V> + StateItemCodec<SlotNumber> + StateItemCodec<u64>,
{
    fn state_item_rest_api(&self) -> axum::Router<()> {
        axum::Router::new()
            .route("/", get(Self::get_state_value_route))
            .with_state(self.clone())
    }
}

impl<N, M, T, Codec> StateItemRestApiExists
    for StateItemRestApiImpl<M, NamespacedStateValue<N, T, Codec>>
where
    M: ModuleInfo,
    Self: StateItemRestApi,
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T>,
{
}

impl<N, M, T, Codec> StateItemRestApiExists
    for StateItemRestApiImpl<M, NamespacedStateVec<N, T, Codec>>
where
    M: ModuleInfo,
    Self: StateItemRestApi,
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<T> + StateItemCodec<u64>,
    Codec::KeyCodec: StateItemCodec<u64>,
{
}

impl<N, M, K, V, Codec> StateItemRestApiExists
    for StateItemRestApiImpl<M, NamespacedStateMap<N, K, V, Codec>>
where
    M: ModuleInfo,
    Self: StateItemRestApi,
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
}

impl<M, V, Codec> StateItemRestApiExists for StateItemRestApiImpl<M, VersionedStateValue<V, Codec>>
where
    M: ModuleInfo,
    Self: StateItemRestApi,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
}

pub trait GetStateItemInfo {
    const STATE_ITEM_KIND: StateItemKind;
    const NAMESPACE: sov_state::Namespace;
}

impl<N, V, Codec> GetStateItemInfo for NamespacedStateValue<N, V, Codec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    const STATE_ITEM_KIND: StateItemKind = StateItemKind::StateValue;
    const NAMESPACE: sov_state::Namespace = N::NAMESPACE;
}

impl<N, V, Codec> GetStateItemInfo for NamespacedStateVec<N, V, Codec>
where
    N: CompileTimeNamespace,
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V> + StateItemCodec<u64>,
    Codec::KeyCodec: StateItemCodec<u64>,
{
    const STATE_ITEM_KIND: StateItemKind = StateItemKind::StateVec;
    const NAMESPACE: sov_state::Namespace = N::NAMESPACE;
}

impl<N, K, V, Codec> GetStateItemInfo for NamespacedStateMap<N, K, V, Codec>
where
    N: CompileTimeNamespace,
    Codec: StateCodec,
    Codec::KeyCodec: StateItemCodec<K>,
    Codec::ValueCodec: StateItemCodec<V>,
    K: FromStr + std::fmt::Display,
{
    const STATE_ITEM_KIND: StateItemKind = StateItemKind::StateMap;
    const NAMESPACE: sov_state::Namespace = N::NAMESPACE;
}

impl<V, Codec> GetStateItemInfo for VersionedStateValue<V, Codec>
where
    Codec: StateCodec,
    Codec::ValueCodec: StateItemCodec<V>,
    Codec::KeyCodec: StateItemCodec<SlotNumber>,
{
    const STATE_ITEM_KIND: StateItemKind = StateItemKind::StateValue;
    const NAMESPACE: sov_state::Namespace = Namespace::Kernel;
}
