#[cfg(feature = "evm")]
mod evm;

use std::str::FromStr;

use borsh::{BorshDeserialize, BorshSerialize};
#[cfg(feature = "evm")]
pub use evm::{EthereumAddress, EvmCryptoSpec, MultiAddressEvm};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{address_prefix, Address, BasicAddress, CredentialId};

/// A generic VM-compatible address enum, enabling supporting for both Sov SDK standard SHA-256 derived addresses and VM-specific addresses.
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
pub enum MultiAddress<VmAddress> {
    /// A standard address derived from a SHA-256 hash of a public key.
    Standard(Address),
    /// A VM-specific address type, allowing native support of the VM's address format.
    Vm(VmAddress),
}

impl<VmAddress: Not28Bytes + BasicAddress + core::str::FromStr<Err: Into<anyhow::Error>>>
    BasicAddress for MultiAddress<VmAddress>
{
}

impl<VmAddress> From<sov_modules_api::AddressBech32> for MultiAddress<VmAddress> {
    fn from(value: sov_modules_api::AddressBech32) -> Self {
        Self::Standard(value.into())
    }
}

impl<VmAddress> From<CredentialId> for MultiAddress<VmAddress> {
    fn from(value: CredentialId) -> Self {
        Self::Standard(Address::from(value))
    }
}

impl<VmAddress: std::fmt::Display> std::fmt::Display for MultiAddress<VmAddress> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MultiAddress::Standard(addr) => addr.fmt(f),
            MultiAddress::Vm(addr) => std::fmt::Display::fmt(&addr, f),
        }
    }
}

#[derive(Serialize, Deserialize)]
enum DeSerHelper<VmAddress> {
    Standard(Address),
    Vm(VmAddress),
}

impl<VmAddress: Serialize + std::fmt::Display> Serialize for MultiAddress<VmAddress> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            // For the human readable format, we rely on `Display` impl of the `VmAddress` to
            // be distinct from the `Standard` address, which should be prefixed with something like `sov`.
            serializer.serialize_str(&self.to_string())
        } else {
            let helper = match self {
                Self::Standard(addr) => DeSerHelper::Standard(*addr),
                Self::Vm(addr) => DeSerHelper::Vm(addr),
            };
            helper.serialize(serializer)
        }
    }
}

impl<'de, VmAddress> Deserialize<'de> for MultiAddress<VmAddress>
where
    VmAddress: Deserialize<'de>,
    MultiAddress<VmAddress>: std::str::FromStr,
    <MultiAddress<VmAddress> as std::str::FromStr>::Err: std::fmt::Display,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let s: String = Deserialize::deserialize(deserializer)?;
            MultiAddress::<VmAddress>::from_str(&s).map_err(serde::de::Error::custom)
        } else {
            let helper = DeSerHelper::deserialize(deserializer)?;
            match helper {
                DeSerHelper::Standard(addr) => Ok(MultiAddress::Standard(addr)),
                DeSerHelper::Vm(addr) => Ok(MultiAddress::Vm(addr)),
            }
        }
    }
}

impl<VmAddress> From<[u8; 28]> for MultiAddress<VmAddress> {
    fn from(value: [u8; 28]) -> Self {
        Self::Standard(Address::from(value))
    }
}

impl<VmAddress: AsRef<[u8]>> AsRef<[u8]> for MultiAddress<VmAddress> {
    fn as_ref(&self) -> &[u8] {
        match self {
            MultiAddress::Standard(addr) => addr.as_ref(),
            MultiAddress::Vm(addr) => addr.as_ref(),
        }
    }
}

impl<VmAddress: for<'a> TryFrom<&'a [u8], Error = anyhow::Error>> TryFrom<&[u8]>
    for MultiAddress<VmAddress>
{
    type Error = anyhow::Error;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        if bytes.len() == 28 {
            Ok(Self::Standard(sov_modules_api::Address::try_from(bytes)?))
        } else {
            Ok(Self::Vm(VmAddress::try_from(bytes)?))
        }
    }
}

impl<VmAddress: core::str::FromStr<Err: Into<anyhow::Error>>> std::str::FromStr
    for MultiAddress<VmAddress>
{
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with(address_prefix()) {
            let std_addr = sov_modules_api::Address::from_str(s)?;
            Ok(MultiAddress::Standard(std_addr))
        } else {
            let vm_addr = VmAddress::from_str(s).map_err(|e| anyhow::anyhow!(e))?;
            Ok(MultiAddress::Vm(vm_addr))
        }
    }
}

impl<VmAddress> FromVmAddress<VmAddress> for MultiAddress<VmAddress> {
    fn from_vm_address(value: VmAddress) -> Self {
        Self::Vm(value)
    }
}

impl<VmAddress> From<Address> for MultiAddress<VmAddress> {
    fn from(value: Address) -> Self {
        Self::Standard(value)
    }
}
pub trait Not28Bytes {}
pub trait FromVmAddress<VmAddress> {
    fn from_vm_address(value: VmAddress) -> Self;
}
