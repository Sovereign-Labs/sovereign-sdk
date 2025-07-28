use alloy_primitives::{Address, AddressError};
use borsh::{BorshDeserialize, BorshSerialize};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::UniversalWallet;
use sov_rollup_interface::common::HexString;
use sov_rollup_interface::crypto::CredentialId;
use sov_rollup_interface::BasicAddress;

use crate::evm::public_key::EthereumPublicKey;
use crate::{MultiAddress, Not28Bytes};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    UniversalWallet,
)]
#[cfg_attr(
    feature = "arbitrary",
    derive(
        sov_modules_api::prelude::arbitrary::Arbitrary,
        sov_modules_api::prelude::proptest_derive::Arbitrary
    )
)]
/// A standard 20-byte Ethereum address with checksum.
pub struct EthereumAddress(#[sov_wallet(as_ty = "[u8;20]", display = "hex")] pub Address);

impl From<CredentialId> for EthereumAddress {
    fn from(credential_id: CredentialId) -> Self {
        credential_id.0.into()
    }
}

impl From<HexString<[u8; 32]>> for EthereumAddress {
    fn from(value: HexString<[u8; 32]>) -> Self {
        Self::try_from(&value.0.as_slice()[12..32]).unwrap()
    }
}

impl<'a> From<&'a EthereumPublicKey> for EthereumAddress {
    fn from(value: &'a EthereumPublicKey) -> Self {
        let uncompressed = value.pub_key.to_encoded_point(false);
        // Construct the address from the 64 bytes of public key material (2x32 byte field elements), stripping
        // out the `UNCOMPRESSED`(0x04 prefix) tag which is the first byte.
        // https://github.com/bitcoin-core/secp256k1/blob/8deef00b33ca81202aca80fe0bcd9730f084fbd2/src/eckey_impl.h#L49
        Self(Address::from_raw_public_key(&uncompressed.as_bytes()[1..]))
    }
}

impl BorshSerialize for EthereumAddress {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.0 .0 .0)
    }
}

impl AsRef<[u8]> for EthereumAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl TryFrom<&[u8]> for EthereumAddress {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(Address::try_from(value)?))
    }
}

impl std::str::FromStr for EthereumAddress {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // NOTE: Why chain id is ignored here?
        Ok(Self(Address::parse_checksummed(s, None)?))
    }
}

impl BorshDeserialize for EthereumAddress {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let mut bytes = [0u8; 20];
        reader.read_exact(&mut bytes)?;
        Ok(Self(bytes.into()))
    }
}

impl From<Address> for EthereumAddress {
    fn from(value: Address) -> Self {
        Self(value)
    }
}

impl From<EthereumAddress> for [u8; 20] {
    fn from(value: EthereumAddress) -> Self {
        value.0.into()
    }
}

impl From<EthereumAddress> for Address {
    fn from(value: EthereumAddress) -> Self {
        value.0
    }
}

impl std::fmt::Display for EthereumAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl schemars::JsonSchema for EthereumAddress {
    fn schema_name() -> String {
        "EthereumAddress".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^0x[a-fA-F0-9]{40}$",
            "description": "20 bytes in hexadecimal format, with `0x` prefix.",
        }))
        .unwrap()
    }
}

pub type MultiAddressEvm = MultiAddress<EthereumAddress>;

impl BasicAddress for EthereumAddress {}
impl Not28Bytes for EthereumAddress {}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use borsh::{BorshDeserialize, BorshSerialize};
    use sov_modules_api::configurable_spec::ConfigurableSpec;
    use sov_modules_api::execution_mode::Native;
    use sov_modules_api::Spec;
    use sov_test_utils::{MockDaSpec, MockZkvm};

    use super::*;
    type S = ConfigurableSpec<MockDaSpec, MockZkvm, MockZkvm, MultiAddressEvm, Native>;

    #[test]
    fn test_serde_json_multi_address_evm_vm() {
        let address = MultiAddressEvm::Vm(
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap(),
        );
        let serialized = serde_json::to_string(&address).unwrap();
        let deserialized: MultiAddressEvm = serde_json::from_str(&serialized).unwrap();
        assert_eq!(address, deserialized);
    }

    #[test]
    fn test_serde_json_multi_address_evm_standard() {
        let address = MultiAddressEvm::Standard(
            sov_modules_api::Address::from_str(
                "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
            )
            .unwrap(),
        );
        let serialized = serde_json::to_string(&address).unwrap();
        let deserialized: MultiAddressEvm = serde_json::from_str(&serialized).unwrap();
        assert_eq!(address, deserialized);
    }

    #[test]
    fn test_bincode_multi_address_evm_vm() {
        let address = MultiAddressEvm::Vm(
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap(),
        );
        let serialized = bincode::serialize(&address).unwrap();
        let deserialized: MultiAddressEvm = bincode::deserialize(&serialized).unwrap();
        assert_eq!(address, deserialized);
    }

    #[test]
    fn test_bincode_multi_address_evm_standard() {
        let address = MultiAddressEvm::Standard(
            sov_modules_api::Address::from_str(
                "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
            )
            .unwrap(),
        );
        let serialized = bincode::serialize(&address).unwrap();
        let deserialized: MultiAddressEvm = bincode::deserialize(&serialized).unwrap();
        assert_eq!(address, deserialized);
    }

    #[test]
    fn test_borsh_allowed_sequencer() {
        #[derive(BorshSerialize, BorshDeserialize, Debug, PartialEq, Eq)]
        pub struct TypeWithFieldAfterAddress<S: Spec> {
            address: S::Address,
            variable: u64,
        }

        let allowed_sequencer = TypeWithFieldAfterAddress {
            address: MultiAddressEvm::Vm(
                EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap(),
            ),
            variable: 90000000000,
        };
        let mut serialized: Vec<u8> = Vec::new();
        BorshSerialize::serialize(&allowed_sequencer, &mut serialized).unwrap();
        let deserialized: TypeWithFieldAfterAddress<S> =
            BorshDeserialize::try_from_slice(&serialized).unwrap();
        assert_eq!(allowed_sequencer, deserialized);
    }

    #[test]
    fn test_borsh_multi_address_evm_vm() {
        let spec_address = MultiAddressEvm::Vm(
            EthereumAddress::from_str("0x71334bf1710D12c9f689cC819476fA589F08C64C").unwrap(),
        );
        let mut spec_address_bytes: Vec<u8> = Vec::new();
        BorshSerialize::serialize(&spec_address, &mut spec_address_bytes).unwrap();
        let deserialized: MultiAddressEvm =
            BorshDeserialize::try_from_slice(spec_address_bytes.as_slice()).unwrap();
        assert_eq!(spec_address, deserialized);
    }

    #[test]
    fn test_borsh_multi_address_evm_standard() {
        let standard_address = MultiAddressEvm::Standard(
            sov_modules_api::Address::from_str(
                "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
            )
            .unwrap(),
        );
        let mut spec_address_bytes: Vec<u8> = Vec::new();
        BorshSerialize::serialize(&standard_address, &mut spec_address_bytes).unwrap();
        let deserialized: MultiAddressEvm =
            BorshDeserialize::try_from_slice(spec_address_bytes.as_slice()).unwrap();
        assert_eq!(standard_address, deserialized);
    }
}
