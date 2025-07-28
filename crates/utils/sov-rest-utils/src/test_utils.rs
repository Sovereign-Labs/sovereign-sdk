//! Testing utilities for Sovereign-flavored JSON API implementations.

use std::fmt::Debug;
use std::str::FromStr;

use axum::http::Uri;

/// Creates a new [`Uri`] with the given query parameters, serialized with
/// [`serde_urlencoded`].
pub fn uri_with_query_params<T>(params: T) -> axum::http::Uri
where
    T: serde::Serialize,
{
    // See
    // <https://github.com/nox/serde_urlencoded/blob/master/tests/test_serialize.rs>
    // for some examples.
    let s = format!(
        "http://example.com?{}",
        serde_urlencoded::to_string(params).unwrap()
    );
    Uri::from_str(&s).expect("Can't create URI from string")
}

/// Serializes, then deserializes a value with [`serde_urlencoded`], then
/// asserts equality.
pub fn test_serialization_roundtrip_equality_urlencoded<T>(item: T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Debug,
{
    let serialized = serde_urlencoded::to_string(&item).unwrap();
    let deserialized: T = serde_urlencoded::from_str(&serialized).unwrap();
    assert_eq!(item, deserialized);
}
