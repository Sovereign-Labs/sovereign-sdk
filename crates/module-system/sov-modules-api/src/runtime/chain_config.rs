//! Lazily loaded chain configuration embedded in native and SP1 guest binaries.

use std::sync::{LazyLock, OnceLock};

use sov_chain_config::decode_chain_config_record;
pub use sov_chain_config::{ChainConfig, ChainHashOverride};
use sov_universal_wallet::schema::{ChainData, ChainHashTemplate};

sov_modules_macros::embed_chain_config!(CHAIN_CONFIG_BYTES);

static EMBEDDED_CHAIN_CONFIG: LazyLock<ChainConfig> = LazyLock::new(|| {
    // The bytes are intentionally mutable after linking, which is outside the memory model the
    // compiler assumes for an immutable static. The volatile read forces the record to be loaded
    // at runtime instead of having its build-time contents folded into the executable code.
    // SAFETY: `CHAIN_CONFIG_BYTES` is a static array, so the pointer is valid, aligned, and
    // readable for the full record length.
    let record = unsafe { std::ptr::read_volatile(std::ptr::addr_of!(CHAIN_CONFIG_BYTES)) };
    decode_chain_config_record(&record)
        .unwrap_or_else(|error| panic!("invalid embedded chain config: {error}"))
});

/// Returns the lazily decoded chain configuration shared by native and ZK execution.
pub fn chain_config() -> &'static ChainConfig {
    &EMBEDDED_CHAIN_CONFIG
}

/// Chain ID retained as a compatibility accessor for existing SDK consumers.
pub static CHAIN_ID: LazyLock<u64> = LazyLock::new(|| {
    #[cfg(debug_assertions)]
    if std::env::var_os("SOV_TEST_CONST_OVERRIDE_CHAIN_ID").is_some() {
        return sov_modules_macros::config_value_private!("CHAIN_ID");
    }
    chain_config().chain_id
});

/// Chain name retained as a compatibility accessor for existing SDK consumers.
pub static CHAIN_NAME: LazyLock<String> = LazyLock::new(|| {
    #[cfg(debug_assertions)]
    if std::env::var_os("SOV_TEST_CONST_OVERRIDE_CHAIN_NAME").is_some() {
        return sov_modules_macros::config_value_private!("CHAIN_NAME").to_owned();
    }
    chain_config().chain_name.clone()
});

/// Returns the test-only chain hash override schedule from the environment, if set.
///
/// The variable is re-read on every call so tests that configure different schedules in the same
/// process (e.g. under plain `cargo test`) each observe their own value, matching the behavior
/// from before the schedule was embedded in the binary. The most recently parsed schedule is
/// cached keyed by the raw environment string, so repeated calls with an unchanged value do not
/// leak memory.
#[cfg(debug_assertions)]
fn test_override_chain_hash_overrides() -> Option<&'static [ChainHashOverride]> {
    static CACHE: std::sync::Mutex<Option<(String, &'static [ChainHashOverride])>> =
        std::sync::Mutex::new(None);

    let value = std::env::var("SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES").ok()?;
    let mut cache = CACHE.lock().unwrap();
    if let Some((cached_value, overrides)) = cache.as_ref() {
        if *cached_value == value {
            return Some(overrides);
        }
    }
    let overrides: &'static [ChainHashOverride] =
        sov_chain_config::parse_chain_hash_overrides_toml(&value)
            .unwrap_or_else(|error| {
                panic!("invalid SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES: {error}")
            })
            .leak();
    *cache = Some((value, overrides));
    Some(overrides)
}

/// Returns the dynamically embedded chain hash override schedule.
pub fn chain_hash_overrides() -> &'static [ChainHashOverride] {
    #[cfg(debug_assertions)]
    if let Some(overrides) = test_override_chain_hash_overrides() {
        return overrides;
    }
    &chain_config().chain_hash_overrides
}

/// Constructs the current chain hash from an immutable Borsh-encoded runtime template and the
/// dynamically embedded chain data.
///
/// This recomputes the hash on every call; runtimes should cache the result through
/// [`LazyChainHash`] rather than calling this from a hot path such as `Runtime::chain_hash()`.
pub fn construct_chain_hash(template_borsh: &[u8]) -> [u8; 32] {
    let template = ChainHashTemplate::from_borsh_bytes(template_borsh)
        .expect("invalid generated chain-hash template");
    template
        .chain_hash(&ChainData {
            chain_id: *CHAIN_ID,
            chain_name: CHAIN_NAME.to_string(),
        })
        .expect("failed to construct runtime chain hash")
}

/// A memoized chain hash bound to one runtime's immutable Borsh-encoded chain-hash template,
/// constructed from the dynamically embedded chain data on first use.
///
/// Intended to back `Runtime::chain_hash()` implementations:
///
/// ```rust,ignore
/// static RUNTIME_CHAIN_HASH: LazyChainHash =
///     LazyChainHash::new(&__generated::CHAIN_HASH_TEMPLATE_BORSH);
///
/// fn chain_hash() -> [u8; 32] {
///     RUNTIME_CHAIN_HASH.get()
/// }
/// ```
pub struct LazyChainHash {
    template_borsh: &'static [u8],
    hash: OnceLock<[u8; 32]>,
}

impl LazyChainHash {
    /// Creates a lazy chain hash for the given template, typically the generated
    /// `CHAIN_HASH_TEMPLATE_BORSH`.
    pub const fn new(template_borsh: &'static [u8]) -> Self {
        Self {
            template_borsh,
            hash: OnceLock::new(),
        }
    }

    /// Returns the chain hash, constructing it on first use.
    pub fn get(&self) -> [u8; 32] {
        *self
            .hash
            .get_or_init(|| construct_chain_hash(self.template_borsh))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macros::config_value;

    #[test]
    fn embedded_defaults_come_from_constants_toml() {
        assert_eq!(chain_config().chain_id, config_value!("CHAIN_ID"));
        assert_eq!(chain_config().chain_name, config_value!("CHAIN_NAME"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn chain_hash_override_env_values_are_reread_per_access() {
        let schedule = |end_height: u64, byte: &str| {
            format!(
                "[{{ start_height = 0, end_height = {end_height}, chain_hash = \"0x{}\" }}]",
                byte.repeat(32)
            )
        };
        std::env::set_var(
            "SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES",
            schedule(10, "aa"),
        );
        assert_eq!(
            chain_hash_overrides(),
            [ChainHashOverride {
                start_height: 0,
                end_height: 10,
                chain_hash: [0xaa; 32],
                grace_period: 0,
            }]
            .as_slice(),
            "the first override schedule set in this process should be visible"
        );

        std::env::set_var(
            "SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES",
            schedule(20, "bb"),
        );
        assert_eq!(
            chain_hash_overrides(),
            [ChainHashOverride {
                start_height: 0,
                end_height: 20,
                chain_hash: [0xbb; 32],
                grace_period: 0,
            }]
            .as_slice(),
            "changing the override schedule later in the same process should take effect"
        );
        std::env::remove_var("SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES");
    }
}
