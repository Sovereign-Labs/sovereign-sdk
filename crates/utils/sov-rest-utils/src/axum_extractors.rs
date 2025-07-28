//! Custom [`axum`] extractors for opinionated JSON APIs.
//!
//! See also: <https://docs.rs/axum/latest/axum/extract/index.html#customizing-extractor-responses>.

#![deny(missing_docs)]

use std::fmt::Debug;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{StatusCode, Uri};
use serde::de::DeserializeOwned;

use crate::{json_obj, ErrorObject};

/// An alternative to the built-in Axum extractor [`axum::extract::Query`],
/// which handles properly formatted JSON errors upon deserialization failure
/// according to the standard API convention followed by [`ErrorObject`].
///
/// See:
/// - <https://github.com/tokio-rs/axum/issues/1116>
/// - <https://github.com/tokio-rs/axum/blob/main/examples/customize-extractor-error/src/derive_from_request.rs>
/// - <https://docs.rs/axum/latest/axum/extract/index.html#customizing-extractor-responses>
#[derive(Debug, Copy, Clone, derive_more::Deref)]
pub struct Query<T>(pub T);

impl<T: DeserializeOwned> Query<T> {
    /// Attempts to deserialize and then validate the query string from the
    /// given [`Uri`].
    pub fn try_from_uri(uri: &Uri) -> Result<Self, ErrorObject> {
        axum::extract::Query::<T>::try_from_uri(uri)
            .map(|q| Self(q.0))
            .map_err(|err| ErrorObject {
                status: StatusCode::BAD_REQUEST,
                message: "Invalid query string".to_string(),
                details: json_obj!({
                    "error": err.to_string(),
                }),
            })
    }
}

#[axum::async_trait]
impl<S, T: DeserializeOwned> FromRequestParts<S> for Query<T> {
    type Rejection = ErrorObject;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Self::try_from_uri(&parts.uri)
    }
}

/// An alternative to the built-in Axum extractor [`axum::extract::Path`], which
/// handles errors gracefully and returns error responses in a `JSON:API`-like
/// format.
#[derive(Debug, derive_more::Deref)]
pub struct Path<T>(pub T);

#[axum::async_trait]
impl<S, T> FromRequestParts<S> for Path<T>
where
    axum::extract::Path<T>: FromRequestParts<S>,
    <axum::extract::Path<T> as FromRequestParts<S>>::Rejection: ToString + Debug,
    S: Send + Sync,
{
    type Rejection = ErrorObject;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Path::from_request_parts(parts, state).await {
            Ok(query) => Ok(Path(query.0)),
            Err(err) => Err(ErrorObject {
                status: StatusCode::BAD_REQUEST,
                message: "Failed to deserialize path string parameter(s)".to_string(),
                details: json_obj!({
                    "error": err.to_string(),
                }),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: add tests for `Path`.

    mod query {
        use super::*;
        use crate::test_utils::uri_with_query_params;

        #[derive(Debug, serde::Deserialize)]
        struct TestQuery {
            #[allow(unused)]
            integer: u8,
        }

        #[test]
        fn query_serde_error() {
            let uri = uri_with_query_params([("integer", "foo")]);
            let result = Query::<TestQuery>::try_from_uri(&uri);
            let err = result.unwrap_err();

            assert_eq!(
                err,
                ErrorObject {
                    status: StatusCode::BAD_REQUEST,
                    message: "Invalid query string".to_string(),
                    details: json_obj!({
                        "error": "Failed to deserialize query string"
                    }),
                }
            );
        }

        #[test]
        fn query_ok() {
            let uri = uri_with_query_params([("integer", 42)]);
            let result = Query::<TestQuery>::try_from_uri(&uri);
            assert!(result.is_ok());
        }
    }
}
