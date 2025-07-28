use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sov_rollup_interface::zk::CryptoSpec;

use crate::evm::public_key::EthereumPublicKey;
use crate::evm::signature::EthereumSignature;

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    BorshDeserialize,
    BorshSerialize,
)]
/// A [`CryptoSpec`] implementation for EVM rollups.
/// Uses the secp256k1 signature scheme with keccak256 hashes for signatures,
/// and sha256 as the default hasher for other operations.
pub struct EvmCryptoSpec;

impl CryptoSpec for EvmCryptoSpec {
    #[cfg(feature = "native")]
    type PrivateKey = crate::evm::private_key::EthereumPrivateKey;

    type PublicKey = EthereumPublicKey;

    type Hasher = Sha256;

    type Signature = EthereumSignature;

    fn sovereign_admin_pubkey() -> Self::PublicKey {
        let admin_pubkey_bytes: [u8; 65] = [
            0x04, 0xa3, 0xc4, 0x96, 0xaa, 0xff, 0x0c, 0x16, 0xeb, 0xb4, 0x7e, 0x85, 0x37, 0xef,
            0xe1, 0x05, 0x96, 0x35, 0x8b, 0x43, 0x3a, 0x48, 0xf4, 0x8a, 0x08, 0xa3, 0xf0, 0xc5,
            0xb6, 0x84, 0xf0, 0xe8, 0x33, 0xee, 0xaf, 0x6c, 0x45, 0xac, 0x40, 0x6b, 0x82, 0xa8,
            0xa5, 0x1f, 0x09, 0x65, 0xdf, 0x1b, 0x37, 0xea, 0xde, 0xd5, 0x42, 0x8f, 0xb5, 0xac,
            0x32, 0xfc, 0xeb, 0x5f, 0x60, 0x99, 0x1d, 0xf3, 0x42,
        ];

        // This will panic if the bytes are invalid, which is fine for a hardcoded constant
        let pub_key = k256::PublicKey::from_sec1_bytes(&admin_pubkey_bytes)
            .expect("Invalid admin public key bytes");

        EthereumPublicKey { pub_key }
    }
}

#[test]
fn test_sovereign_admin_pubkey() {
    use sov_rollup_interface::zk::CryptoSpec;

    use crate::{EthereumAddress, EvmCryptoSpec};

    let pub_key = EvmCryptoSpec::sovereign_admin_pubkey();
    assert_eq!(
        EthereumAddress::from(&pub_key).to_string(),
        "0xD9Ab2169e1CF41B9E2F4486D974207FEB1E38902"
    );
}
