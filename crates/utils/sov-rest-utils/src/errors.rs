//! Common error types.

use axum::extract::OriginalUri;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

use crate::{json_obj, ErrorObject};

/// A 404 response useful as a [`axum::Router::fallback`].
pub async fn global_404(OriginalUri(uri): OriginalUri) -> Response {
    ErrorObject {
        status: StatusCode::NOT_FOUND,
        message: "Not Found".to_string(),
        details: json_obj!({
            "url": uri.to_string(),
        }),
    }
    .into_response()
}

/// Returns a 501 error.
pub fn not_implemented_501() -> Response {
    ErrorObject {
        status: StatusCode::NOT_IMPLEMENTED,
        message: "Not implemented yet".to_string(),
        details: Default::default(),
    }
    .into_response()
}

/// Returns a 404 error when the given resource was not found.
pub fn not_found_404(resource_name_capitalized: &str, resource_id: impl ToString) -> Response {
    ErrorObject {
        status: StatusCode::NOT_FOUND,
        message: format!(
            "{} '{}' not found",
            resource_name_capitalized,
            resource_id.to_string()
        ),
        details: json_obj!({
            "id": resource_id.to_string(),
        }),
    }
    .into_response()
}

/// Returns a custom 400 error.
pub fn bad_request_400(message: &str, err: impl ToString) -> Response {
    ErrorObject {
        status: StatusCode::BAD_REQUEST,
        message: message.to_string(),
        details: json_obj!({
            "error": err.to_string(),
        }),
    }
    .into_response()
}

/// Returns a 503 error to be used when the sequencer is overloaded.
pub fn sequencer_overloaded_503() -> ErrorObject {
    ErrorObject {
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: "The sequencer is temporarily overloaded. Try again in a few seconds".to_string(),
        details: json_obj!({}),
    }
}

/// Returns a 500 error to be used when a database error occurred.
pub fn database_error_500(err: impl ToString) -> ErrorObject {
    // We don't include the database error in the response, because it may
    // contain sensitive information.
    //
    // FIXME(security): logging it is useful, but we should not even be doing that.
    error!(
        error = err.to_string(),
        "Database error while serving request."
    );

    ErrorObject {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: "Internal server error".to_string(),
        details: json_obj!({}),
    }
}

/// Converts [`database_error_500`] into a [`Response`].
pub fn database_error_response_500(err: impl ToString) -> Response {
    database_error_500(err).into_response()
}

/// Returns a 500 internal server error.
pub fn internal_server_error_response_500(err: impl ToString) -> Response {
    tracing::error!(error = err.to_string(), "500 error while serving request");

    ErrorObject {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: "Internal server error".to_string(),
        details: json_obj!({
            "error": err.to_string(),
        }),
    }
    .into_response()
}

#[cfg(test)]
mod tests {

    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn check_404() {
        let r404 = not_found_404("MY RESOURCE", "id-101");
        assert_eq!(StatusCode::from_u16(404).unwrap(), r404.status());
    }

    #[test]
    fn check_500() {
        let r500 = internal_server_error_response_500("check check");
        assert_eq!(StatusCode::from_u16(500).unwrap(), r500.status());
    }

    #[tokio::test]
    async fn test_global_404_fallback() {
        let router = axum::Router::new().fallback(global_404);
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/doesnt-exist-foorbar")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: ErrorObject = serde_json::from_slice(&body).unwrap();

        assert_eq!("Not Found", error.message);
        assert_eq!(
            error.details.get("url").unwrap().to_string(),
            "\"/doesnt-exist-foorbar\"".to_string()
        );
    }
}
