use sov_address::EvmCryptoSpec;
// use borsh::BorshDeserialize;
use sov_modules_api::CryptoSpec;
use sov_rollup_interface::crypto::PrivateKey;

/// A test crypto spec where we know the admin's private key for testing
#[derive(Debug, Clone, PartialEq)]
pub struct TestCryptoSpec;

impl CryptoSpec for TestCryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = <EvmCryptoSpec as CryptoSpec>::PrivateKey;
    type PublicKey = <EvmCryptoSpec as CryptoSpec>::PublicKey;
    type Hasher = <EvmCryptoSpec as CryptoSpec>::Hasher;
    type Signature = <EvmCryptoSpec as CryptoSpec>::Signature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        let priv_key = serde_json::from_str::<<TestCryptoSpec as CryptoSpec>::PrivateKey>(
            r#""0d87c12ea7c12024b3f70a26d735874608f17c8bce2b48e6fe87389310191264""#,
        )
        .unwrap();
        priv_key.pub_key()
    }
}
