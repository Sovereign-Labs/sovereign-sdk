use anyhow::Result;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::{GasMeter, HexHash, HexString, SizedSafeString, Spec};

use crate::crypto::keccak256_hash;

pub type Domain = u32;

/// These are returned from `hook_type` to indicate to the caller (usually a relayer) what type of metadata
/// to pass into `post_dispatch/quote_dispatch`. These are defined by the hyperlane protocol here:
/// <https://github.com/eigerco/hyperlane-monorepo/blob/b68fe264b3585ecd9d95a5ec2ec2d7defbe907d2/solidity/contracts/interfaces/hooks/IPostDispatchHook.sol#L18>
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum HookType {
    Unused = 0,
    Routing = 1,
    Aggregation = 2,
    MerkleTree = 3,
    InterchainGasPaymaster = 4,
    FallbackRouting = 5,
    IdAuthIsm = 6,
    Pausable = 7,
    ProtocolFee = 8,
    LayerZeroV1 = 9,
    RateLimited = 10,
    ArbL2ToL1 = 11,
    OpL2ToL1 = 12,
}

/// Message struct used for interchain communication.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(proptest_derive::Arbitrary, arbitrary::Arbitrary)
)]
pub struct Message {
    /// Version of the message format.
    pub version: u8,
    /// A nonce used to ensure message ids are unique even if the recipient and contents are identical
    pub nonce: u32,
    /// Domain of the origin chain.
    pub origin_domain: Domain,
    /// Address of the sender.
    pub sender: HexHash,
    /// Domain of the destination chain.
    pub dest_domain: Domain,
    /// Address of the recipient.
    pub recipient: HexHash,
    /// Some application-specific message to be deserialized and processed by the recipient.
    pub body: HexString,
}

impl Message {
    /// Decode a message from a hex string.
    pub fn decode(message: &[u8]) -> Result<Self> {
        anyhow::ensure!(
            message.len() >= 77,
            "Message is too short. A valid message must be at least 77 bytes"
        );
        let sender: [u8; 32] = message[9..41].try_into().unwrap();
        let recipient: [u8; 32] = message[45..77].try_into().unwrap();
        Ok(Self {
            version: message[0],
            nonce: u32::from_be_bytes(message[1..5].try_into().unwrap()),
            origin_domain: u32::from_be_bytes(message[5..9].try_into().unwrap()),
            sender: sender.into(),
            dest_domain: u32::from_be_bytes(message[41..45].try_into().unwrap()),
            recipient: recipient.into(),
            body: message[77..].to_vec().into(),
        })
    }

    /// Encode a message into a hex string.
    pub fn encode(&self) -> HexString {
        self.version
            .to_be_bytes()
            .iter()
            .chain(self.nonce.to_be_bytes().iter())
            .chain(self.origin_domain.to_be_bytes().iter())
            .chain(self.sender.0.iter())
            .chain(self.dest_domain.to_be_bytes().iter())
            .chain(self.recipient.0.iter())
            .chain(self.body.0.iter())
            .copied()
            .collect::<Vec<u8>>()
            .into()
    }

    /// Generate a unique identifier for the message.
    pub fn id<S: Spec>(&self, gas_meter: &mut impl GasMeter<Spec = S>) -> Result<HexHash> {
        let hex: HexString = self.encode();
        keccak256_hash(hex.as_ref(), gas_meter)
    }
}

/// A storage location of a validator. Allows up to 256 characters.
pub type StorageLocation = SizedSafeString<256>;

/// Representation of ethereum address.
pub type EthAddress = HexString<[u8; 20]>;

/// Represantation of a single validator signature.
pub type ValidatorSignature = HexString<[u8; 65]>;

#[cfg(test)]
mod tests {
    use proptest::prelude::any;
    use proptest::proptest;
    use sov_modules_api::prelude::proptest;

    use super::*;
    #[test]
    fn test_encode_decode() {
        use std::str::FromStr;
        let encode_expected = HexString::from_str("0x00000021500000aef3000000000000000000000000477d860f8f41bc69ddd32821f2bf2c2af0243f1600aa36a70000000000000000000000005d56b8a669f50193b54319442c6eee5edd66238148656c6c6f21").unwrap();

        let decode_actual: Message = Message::decode(encode_expected.as_ref()).unwrap();
        let decode_expected = Message {
            version: 0,
            nonce: 8528,
            origin_domain: 44787,
            sender: HexString::from_str(
                "0x000000000000000000000000477d860f8f41bc69ddd32821f2bf2c2af0243f16",
            )
            .unwrap(),
            dest_domain: 11155111,
            recipient: HexString::from_str(
                "0x0000000000000000000000005d56b8a669f50193b54319442c6eee5edd662381",
            )
            .unwrap(),
            body: HexString::from_str("0x48656c6c6f21").unwrap(),
        };
        let encode_actual: HexString = decode_expected.encode();

        assert_eq!(decode_expected, decode_actual);
        assert_eq!(encode_expected, encode_actual);
    }

    #[test]
    fn test_overflow() {
        use std::str::FromStr;
        let no: HexString = HexString::from_str("0x00000021500000aef3000000000000000000000000477d860f8f41bc69ddd32821f2bf2c2af0243f1600aa36a70000000000000000000000005d56b8a669f50193b543").unwrap();

        let Err(e) = Message::decode(no.as_ref()) else {
            panic!("Expected an error");
        };
        assert_eq!(
            e.to_string(),
            "Message is too short. A valid message must be at least 77 bytes"
        );
    }

    proptest! {
        #[test]
        fn prop_encode_decode(message in any::<Message>()) {
            let encoded = message.encode();
            let decoded = Message::decode(encoded.as_ref()).unwrap();
            assert_eq!(message, decoded);
        }
    }
}
