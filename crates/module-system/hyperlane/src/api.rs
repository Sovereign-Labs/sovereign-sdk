use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Json;
use serde::{Deserialize as _, Serialize};
use sov_bank::{config_gas_token_id, Amount};
use sov_modules_api::prelude::axum::http::StatusCode;
use sov_modules_api::prelude::serde_json::json;
use sov_modules_api::prelude::utoipa::openapi::OpenApi;
use sov_modules_api::prelude::{axum, UnwrapInfallible};
use sov_modules_api::rest::utils::{errors, json_obj, ApiResult, Path, Query, ErrorObject};
use sov_modules_api::rest::{ApiState, HasCustomRestApi};
use sov_modules_api::{ApiStateAccessor, CredentialId, HexHash, Spec};

use crate::igp::RelayerWithDomainKey;
use crate::{EthAddress, Ism, Mailbox, Recipient};

fn deserialize_number_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let s = String::deserialize(deserializer)?;
    s.parse().map_err(serde::de::Error::custom)
}

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
    #[serde(deserialize_with = "deserialize_number_from_str")]
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
            .route("/quote-dispatch", get(Self::query_quote_dispatch))
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
    fn get_ism(
        state: &ApiState<S, Self>,
        address: &HexHash,
        mut accessor: ApiStateAccessor<S>,
    ) -> Result<Ism, Response> {
        let ism = state
            .recipients
            .ism(address, &mut accessor)
            .map_err(errors::internal_server_error_response_500)?
            .ok_or_else(|| ErrorObject {
                status: StatusCode::NOT_FOUND,
                message: "Failed to retrieve Recipient ISM".to_string(),
                details: json_obj!({
                    "error": format!("Either the recipient doesn't exist or no ISM is set for the recipient"),
                    "recipient": address.to_string(),
                })
            }.into_response())?;
        Ok(ism)
    }

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
        accessor: ApiStateAccessor<S>,
    ) -> Result<Response, Response> {
        let ism = Self::get_ism(&state, &address, accessor)?;
        let ism_kind = ism.ism_kind() as u8;
        Ok(Json(json!({"ism_kind": ism_kind})).into_response())
    }

    async fn get_recipient_ism_validators_and_threshold(
        state: ApiState<S, Self>,
        Path(address): Path<HexHash>,
        accessor: ApiStateAccessor<S>,
    ) -> ApiResult<ValidatorsAndThreshold> {
        let ism = Self::get_ism(&state, &address, accessor)?;
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

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use sov_modules_api::prelude::axum::http::Uri;

    use super::*;

    #[test]
    fn test_quote_query_params_handles_u128() {
        let uri = Uri::from_static("https://0.0.0.0:12346/modules/mailbox/quote_dispatch?destination_domain=1399811149&gas_limit=500&recipient_address=0x71c7656ec7ab88b098defb751b7401b5f6d8976fae23aeb2342aedac24843ead");
        let query_params: Query<QuoteParams> =
            Query::try_from_uri(&uri).expect("Failed to parse query params");
        assert_eq!(query_params.destination_domain, 1399811149);
        assert_eq!(query_params.gas_limit, 500u128);
        assert_eq!(
            query_params.recipient_address,
            HexHash::from_str("71c7656ec7ab88b098defb751b7401b5f6d8976fae23aeb2342aedac24843ead")
                .unwrap()
        );
    }
}
