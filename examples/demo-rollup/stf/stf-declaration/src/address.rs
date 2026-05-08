//! Address types used by the demo STF runtime.
//!
//! We cannot reuse `sov_address::MultiAddress` directly because it supports only two variants:
//! a standard 28-byte rollup address and a single VM-specific address type. For Solana offchain
//! transaction testing we need to support three address forms at once: standard rollup addresses,
//! 20-byte Ethereum addresses, and 32-byte Solana base58 addresses.

use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_hyperlane_integration::HyperlaneAddress;
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{
    address_prefix, Address, AddressBech32, Base58Address, BasicAddress, CredentialId, HexHash,
};

/// An address type which supports standard rollup addresses, EVM addresses, and Solana-style
/// base58 addresses.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    JsonSchema,
    UniversalWallet,
)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[sov_wallet(hide_tag)]
pub enum MultiAddressEvmSolana {
    /// A standard address derived from a SHA-256 hash of a public key.
    Standard(Address),
    /// A 20-byte Ethereum address.
    Evm(EthereumAddress),
    /// A 32-byte Solana-style base58 address.
    Solana(Base58Address),
}

impl BasicAddress for MultiAddressEvmSolana {}

impl From<AddressBech32> for MultiAddressEvmSolana {
    fn from(value: AddressBech32) -> Self {
        Self::Standard(value.into())
    }
}

impl From<CredentialId> for MultiAddressEvmSolana {
    fn from(value: CredentialId) -> Self {
        Self::Standard(Address::from(value))
    }
}

impl From<Address> for MultiAddressEvmSolana {
    fn from(value: Address) -> Self {
        Self::Standard(value)
    }
}

impl From<[u8; 28]> for MultiAddressEvmSolana {
    fn from(value: [u8; 28]) -> Self {
        Self::Standard(Address::from(value))
    }
}

impl From<EthereumAddress> for MultiAddressEvmSolana {
    fn from(value: EthereumAddress) -> Self {
        Self::Evm(value)
    }
}

impl From<Base58Address> for MultiAddressEvmSolana {
    fn from(value: Base58Address) -> Self {
        Self::Solana(value)
    }
}

impl FromVmAddress<EthereumAddress> for MultiAddressEvmSolana {
    fn from_vm_address(value: EthereumAddress) -> Self {
        Self::Evm(value)
    }
}

impl FromVmAddress<Base58Address> for MultiAddressEvmSolana {
    fn from_vm_address(value: Base58Address) -> Self {
        Self::Solana(value)
    }
}

impl std::fmt::Display for MultiAddressEvmSolana {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MultiAddressEvmSolana::Standard(addr) => addr.fmt(f),
            MultiAddressEvmSolana::Evm(addr) => addr.fmt(f),
            MultiAddressEvmSolana::Solana(addr) => addr.fmt(f),
        }
    }
}

#[derive(Serialize, Deserialize)]
enum DeSerHelper {
    Standard(Address),
    Evm(EthereumAddress),
    Solana(Base58Address),
}

impl Serialize for MultiAddressEvmSolana {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            serializer.serialize_str(&self.to_string())
        } else {
            let helper = match self {
                Self::Standard(addr) => DeSerHelper::Standard(*addr),
                Self::Evm(addr) => DeSerHelper::Evm(*addr),
                Self::Solana(addr) => DeSerHelper::Solana(*addr),
            };
            helper.serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for MultiAddressEvmSolana {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s: String = Deserialize::deserialize(deserializer)?;
            MultiAddressEvmSolana::from_str(&s).map_err(serde::de::Error::custom)
        } else {
            let helper = DeSerHelper::deserialize(deserializer)?;
            match helper {
                DeSerHelper::Standard(addr) => Ok(Self::Standard(addr)),
                DeSerHelper::Evm(addr) => Ok(Self::Evm(addr)),
                DeSerHelper::Solana(addr) => Ok(Self::Solana(addr)),
            }
        }
    }
}

impl AsRef<[u8]> for MultiAddressEvmSolana {
    fn as_ref(&self) -> &[u8] {
        match self {
            MultiAddressEvmSolana::Standard(addr) => addr.as_ref(),
            MultiAddressEvmSolana::Evm(addr) => addr.as_ref(),
            MultiAddressEvmSolana::Solana(addr) => addr.as_ref(),
        }
    }
}

impl TryFrom<&[u8]> for MultiAddressEvmSolana {
    type Error = anyhow::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        match bytes.len() {
            20 => Ok(Self::Evm(EthereumAddress::try_from(bytes)?)),
            28 => Ok(Self::Standard(Address::try_from(bytes)?)),
            32 => Ok(Self::Solana(Base58Address::try_from(bytes)?)),
            _ => anyhow::bail!(
                "Invalid address length: expected 20, 28, or 32 bytes, got {}",
                bytes.len()
            ),
        }
    }
}

impl std::str::FromStr for MultiAddressEvmSolana {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with(address_prefix()) {
            return Ok(Self::Standard(Address::from_str(s)?));
        }

        if s.starts_with("0x") || s.starts_with("0X") {
            return Ok(Self::Evm(
                EthereumAddress::from_str(s).map_err(|e| anyhow::anyhow!(e))?,
            ));
        }

        Ok(Self::Solana(Base58Address::from_str(s)?))
    }
}

impl HyperlaneAddress for MultiAddressEvmSolana {
    fn to_sender(&self) -> HexHash {
        match self {
            MultiAddressEvmSolana::Standard(addr) => addr.to_sender(),
            MultiAddressEvmSolana::Evm(addr) => addr.to_sender(),
            MultiAddressEvmSolana::Solana(addr) => addr.to_sender(),
        }
    }

    fn from_sender(recipient: HexHash) -> anyhow::Result<Self> {
        // The 32-byte HexHash cannot encode which enum variant produced it
        // (no room for a discriminant). As the HyperlaneAddress trait docs
        // require: pick one variant and always deserialize into it.
        // Solana (Base58Address) is the only 32-byte variant, so it
        // preserves all bytes with zero information loss.
        Ok(Self::Solana(Base58Address::from_sender(recipient)?))
    }
}
