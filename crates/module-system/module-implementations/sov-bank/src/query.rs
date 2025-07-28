//! Defines REST queries exposed by the bank module, along with the relevant types.

use axum::routing::get;
use axum::Json;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::prelude::utoipa::openapi::OpenApi;
use sov_modules_api::prelude::{axum, serde_yaml, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult, Path, Query};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, Spec};

use crate::{config_gas_token_id, get_token_id, Amount, Bank, Coins, TokenId};

/// Structure returned by the `balance_of` method.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct BalanceResponse {
    /// The balance amount of a given user for a given token. Equivalent to u64.
    pub amount: Option<Amount>,
}

/// Structure returned by the `supply_of` method.
#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
pub struct TotalSupplyResponse {
    /// The amount of token supply for a given token ID. Equivalent to u64.
    pub amount: Option<Amount>,
}

impl<S: Spec> Bank<S> {
    /// Method that returns the balance of the user at the address `user_address` for the token
    /// stored at the address `token_id`.
    pub fn balance_of(
        &self,
        version: Option<RollupHeight>,
        user_address: S::Address,
        token_id: TokenId,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<BalanceResponse, anyhow::Error> {
        let amount = if let Some(v) = version {
            let state = &mut state.get_archival_state(v).map_err(|e| anyhow::anyhow!("Impossible to retrieve the state at the provided height. Please ensure you're querying a valid state. Error: {e}"))?;
            self.get_balance_of(&user_address, token_id, state)
        } else {
            self.get_balance_of(&user_address, token_id, state)
        }
        .unwrap_infallible();
        Ok(BalanceResponse { amount })
    }

    /// Method that returns the supply of a token stored at the address `token_id`.
    pub fn supply_of(
        &self,
        version: Option<RollupHeight>,
        token_id: TokenId,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<TotalSupplyResponse, anyhow::Error> {
        let amount = if let Some(v) = version {
            let mut state = state.get_archival_state(v).map_err(|e| anyhow::anyhow!("Impossible to retrieve the state at the provided height. Please ensure you're querying a valid state. Error: {e}"))?;
            self.get_total_supply_of(&token_id, &mut state)
        } else {
            self.get_total_supply_of(&token_id, state)
        }
        .unwrap_infallible();
        Ok(TotalSupplyResponse { amount })
    }
}

/// Axum routes.
impl<S: Spec> Bank<S> {
    async fn route_gas_token() -> axum::Json<types::TokenIdResponse> {
        Json(types::TokenIdResponse {
            token_id: config_gas_token_id(),
        })
    }

    async fn route_gas_token_balance(
        state: ApiState<S, Self>,
        accessor: ApiStateAccessor<S>,
        Path(user_address): Path<S::Address>,
    ) -> ApiResult<Coins> {
        Self::route_balance(state, accessor, Path((config_gas_token_id(), user_address))).await
    }

    async fn route_balance(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
        Path((token_id, user_address)): Path<(TokenId, S::Address)>,
    ) -> ApiResult<Coins> {
        let amount = state
            .get_balance_of(&user_address, token_id, &mut accessor)
            .unwrap_infallible()
            .ok_or_else(|| errors::not_found_404("Balance", user_address))?;

        Ok(Coins { amount, token_id }.into())
    }

    async fn route_total_supply(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
        Path(token_id): Path<TokenId>,
    ) -> ApiResult<Coins> {
        let amount = state
            .get_total_supply_of(&token_id, &mut accessor)
            .unwrap_infallible()
            .ok_or_else(|| errors::not_found_404("Token", token_id))?;

        Ok(Coins { amount, token_id }.into())
    }

    async fn route_find_token_id(
        params: Query<types::FindTokenIdQueryParams<S::Address>>,
    ) -> ApiResult<types::TokenIdResponse> {
        let token_id = get_token_id::<S>(&params.token_name, params.token_decimals, &params.sender);
        Ok(types::TokenIdResponse { token_id }.into())
    }

    async fn route_admins(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
        Path(token_id): Path<TokenId>,
    ) -> ApiResult<types::AdminsResponse<S>> {
        let admins = state
            .tokens
            .get(&token_id, &mut accessor)
            .unwrap_infallible()
            .ok_or_else(|| errors::not_found_404("Token", token_id))?
            .admins;
        Ok(types::AdminsResponse { admins }.into())
    }
}

impl<S: Spec> HasCustomRestApi for Bank<S> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/tokens/gas_token", get(Self::route_gas_token))
            .route(
                "/tokens/gas_token/balances/:address",
                get(Self::route_gas_token_balance),
            )
            .route(
                "/tokens/:tokenId/balances/:address",
                get(Self::route_balance),
            )
            .route(
                "/tokens/:tokenId/total-supply",
                get(Self::route_total_supply),
            )
            .route("/tokens/:tokenId/admins", get(Self::route_admins))
            .route("/tokens", get(Self::route_find_token_id))
            .with_state(state.with(self.clone()))
    }

    fn custom_openapi_spec(&self) -> Option<OpenApi> {
        let mut open_api: OpenApi =
            serde_yaml::from_str(include_str!("../openapi-v3.yaml")).expect("Invalid OpenAPI spec");
        // Because https://github.com/juhaku/utoipa/issues/972
        for path_item in open_api.paths.paths.values_mut() {
            path_item.extensions = None;
        }
        Some(open_api)
    }
}

#[allow(missing_docs)]
pub mod types {
    use super::*;
    use crate::utils::TokenHolder;

    #[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
    pub struct FindTokenIdQueryParams<Addr> {
        pub token_name: String,
        pub sender: Addr,
        pub token_decimals: Option<u8>,
    }

    #[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
    pub struct TokenIdResponse {
        pub token_id: TokenId,
    }

    #[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize, Clone)]
    #[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
    pub struct AdminsResponse<S: sov_modules_api::Spec> {
        pub admins: Vec<TokenHolder<S>>,
    }
}
