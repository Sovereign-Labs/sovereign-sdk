use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::Serialize;
use sov_bank::{config_gas_token_id, Amount};
use sov_modules_api::prelude::serde_json::json;
use sov_modules_api::prelude::utoipa::openapi::OpenApi;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, ApiResult, Path, Query};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, CredentialId, HexHash, Spec};

use crate::igp::RelayerWithDomainKey;
use crate::{EthAddress, Ism, Mailbox, Recipient};

/// A configuration of an [`Ism::MessageIdMultisig`].
#[derive(Serialize)]
pub struct ValidatorsAndThreshold {
    /// The addresses of the validators
    validators: Vec<EthAddress>,
    /// The number of signatures required to accept a message
    threshold: u32,
}

/// Quote params.
#[derive(serde::Deserialize)]
pub struct QuoteParams {
    /// Relayer.
    pub relayer: Option<CredentialId>,
    /// Destination domain.
    pub destination_domain: u32,
    /// Gas limit.
    pub gas_limit: u128,
    /// Recipient address.
    pub recipient_address: HexHash,
}

/// Quote Dispatch rest response
#[derive(serde::Serialize)]
pub struct QuoteDispatchResponse {
    /// Token id (i.e. GAS token)
    pub token_id: String,
    /// Amount.
    pub amount: u128,
}

impl<S: Spec, R: Recipient<S>> HasCustomRestApi for Mailbox<S, R> {
    type Spec = S;

    fn custom_rest_api(&self, state: ApiState<S>) -> axum::Router<()> {
        axum::Router::new()
            .route("/nonce", get(Self::get_nonce))
            .route("/recipient-ism/:address", get(Self::get_recipient_ism))
            .route(
                "/recipient-ism/:address/validators_and_threshold",
                get(Self::get_recipient_ism_validators_and_threshold),
            )
            .route("/quote_dispatch", get(Self::query_quote_dispatch))
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

impl<S: Spec, R: Recipient<S>> Mailbox<S, R> {
    async fn get_nonce(
        state: ApiState<S, Self>,
        mut accessor: ApiStateAccessor<S>,
    ) -> impl IntoResponse {
        let nonce = state
            .dispatch_state
            .get(&mut accessor)
            .unwrap_infallible()
            .map(|dispatch_state| dispatch_state.nonce)
            .unwrap_or_default();
        Json(json!({"nonce": nonce}))
    }

    async fn get_recipient_ism(
        state: ApiState<S, Self>,
        Path(address): Path<HexHash>,
        mut accessor: ApiStateAccessor<S>,
    ) -> Result<Response, Response> {
        let ism = state
            .recipients
            .ism(&address, &mut accessor)
            .map_err(|_| errors::not_found_404("Mailbox", address))?
            .ok_or_else(|| errors::not_found_404("Mailbox", address))?;
        let ism_kind = ism.ism_kind() as u8;
        Ok(Json(json!({"ism_kind": ism_kind})).into_response())
    }

    async fn get_recipient_ism_validators_and_threshold(
        state: ApiState<S, Self>,
        Path(address): Path<HexHash>,
        mut accessor: ApiStateAccessor<S>,
    ) -> ApiResult<ValidatorsAndThreshold> {
        let ism = state
            .recipients
            .ism(&address, &mut accessor)
            .map_err(|_| errors::not_found_404("Mailbox", address))?
            .ok_or_else(|| errors::not_found_404("Mailbox", address))?;

        let Ism::MessageIdMultisig {
            validators,
            threshold,
        } = ism
        else {
            return Err(errors::bad_request_400(
                "Failed getting validators and threshold",
                "Ism is not of type MessageIdMultisig",
            ));
        };

        Ok(ValidatorsAndThreshold {
            validators: validators.into(),
            threshold,
        }
        .into())
    }

    async fn query_quote_dispatch(
        state: ApiState<S, Mailbox<S, R>>,
        mut accessor: ApiStateAccessor<S>,
        Query(params): Query<QuoteParams>,
    ) -> ApiResult<QuoteDispatchResponse> {
        let relayer: Option<S::Address> = params.relayer.map(|r| r.into());
        let relayer = match relayer {
            Some(relayer) => relayer,
            None => state
                .with_default_relayer(relayer, &params.recipient_address, &accessor)
                .map_err(|err| {
                    errors::internal_server_error_response_500(format!(
                        "Internal server error: Failed to get default relayer. Error {err}"
                    ))
                })?,
        };

        let key = RelayerWithDomainKey::new(relayer, params.destination_domain);
        let amount = state
            .interchain_gas_paymaster
            .quote_gas_price(&key, Amount(params.gas_limit), &mut accessor)
            .map_err(|err| {
                errors::internal_server_error_response_500(format!(
                    "Internal server error: Failed to calculate gas price. Error {err}"
                ))
            })?;

        let response = QuoteDispatchResponse {
            amount: amount.0,
            token_id: config_gas_token_id().to_string(),
        };

        Ok(response.into())
    }
}
