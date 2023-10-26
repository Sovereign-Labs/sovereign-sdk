#[cfg(test)]
pub mod test_util {
    use alloc::sync::Arc;
    use alloc::vec;

    use crate::{CacheKey, CacheValue};

    pub(crate) fn create_key(key: u8) -> CacheKey {
        CacheKey {
            key: Arc::new(vec![key]),
        }
    }

    pub(crate) fn create_value(v: u8) -> Option<CacheValue> {
        Some(CacheValue {
            value: Arc::new(vec![v]),
        })
    }
}
