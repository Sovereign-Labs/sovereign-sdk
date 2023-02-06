#[cfg(test)]
pub mod test_util {
    use std::sync::Arc;

    use crate::{CacheKey, CacheValue};

    pub(crate) fn create_key(key: u8) -> CacheKey {
        CacheKey {
            key: Arc::new(vec![key]),
        }
    }

    pub(crate) fn create_value(v: u8) -> CacheValue {
        CacheValue {
            value: Some(Arc::new(vec![v])),
        }
    }
}
