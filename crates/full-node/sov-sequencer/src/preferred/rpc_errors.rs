use axum::http::StatusCode;
use sov_modules_api::Spec;
use sov_rest_utils::{json_obj, ErrorObject};

use crate::preferred::{
    batch_size_tracker::BatchSizeTracker, rate_limiter::ResourceLimitExceededError,
};

pub(crate) fn cant_fit_tx(
    current_batch_size: usize,
    max_batch_size: usize,
    tx_len: usize,
) -> ErrorObject {
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "Transaction cannot be included in the batch due to batch size limitations"
            .to_string(),
        details: json_obj!({
            "error": "The transaction is too large.",
            "serialized_tx_size": BatchSizeTracker::serialized_tx_size(tx_len),
            "current_batch_size": current_batch_size,
            "max_batch_size": max_batch_size,
        }),
    }
}

pub(crate) fn rate_limit<S: Spec>(err: ResourceLimitExceededError<S>) -> ErrorObject {
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: format!("The sender was rate-limited by the sequencer: {err}"),
        details: Default::default(),
    }
}

pub(crate) fn replica_mode() -> ErrorObject {
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "The sequencer is running in replica mode and cannot accept transactions"
            .to_string(),
        details: Default::default(),
    }
}

pub(crate) fn shut_down() -> ErrorObject {
    tracing::info!("The sequencer is shutting down. Cannot accept transactions");
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "The sequencer is shutting down".to_string(),
        details: json_obj!({
            "error": "The sequencer is shutting down. Transactions cannot be accepted at this time".to_string(),
        }),
    }
}
