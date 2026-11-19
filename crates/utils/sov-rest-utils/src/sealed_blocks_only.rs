use axum::http::{HeaderMap, HeaderName};

/// HTTP header that asks the EVM JSON-RPC methods to treat the newest *sealed* block as the
/// chain head, hiding the synthetic blocks built from the in-progress batch.
///
/// Intended for indexers such as Blockscout, which see every new synthetic block as a reorg.
/// Set the header on the websocket upgrade request to apply it to a whole websocket connection.
///
/// Exceptions, which still include the in-progress batch:
/// - State reads (`eth_call`, `eth_getBalance`, ...), including reads at a synthetic block hash.
/// - Responses to transaction submission (`eth_sendRawTransactionSync`,
///   `realtime_sendRawTransaction`): they return the soft-confirmed receipt, which may reference a
///   synthetic block newer than this view's `latest`.
pub static SEALED_BLOCKS_ONLY_HEADER: HeaderName =
    HeaderName::from_static("x-sov-sealed-blocks-only");

/// Request extension marking a request that carries [`SEALED_BLOCKS_ONLY_HEADER`] set to `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SealedBlocksOnly;

impl SealedBlocksOnly {
    /// Returns the marker if the headers request sealed blocks only.
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        headers
            .get(&SEALED_BLOCKS_ONLY_HEADER)
            .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"true"))
            .then_some(Self)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers_with(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(&SEALED_BLOCKS_ONLY_HEADER, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn header_set_to_true_enables_sealed_blocks_only() {
        assert_eq!(
            SealedBlocksOnly::from_headers(&headers_with("True")),
            Some(SealedBlocksOnly)
        );
    }

    #[test]
    fn header_set_to_other_value_is_ignored() {
        assert_eq!(SealedBlocksOnly::from_headers(&headers_with("false")), None);
    }

    #[test]
    fn missing_header_is_ignored() {
        assert_eq!(SealedBlocksOnly::from_headers(&HeaderMap::new()), None);
    }
}
