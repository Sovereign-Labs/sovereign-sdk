#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;

use avail_rust_core::AccountId;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::{sov_universal_wallet::UniversalWallet, BasicAddress};
use sp_core::crypto::{AccountId32 as SubstrateAccountId32, Ss58Codec};
use std::hash::{Hash, Hasher};
use std::{
    fmt::{self, Display},
    io::Read,
    str::FromStr,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, UniversalWallet)]
pub struct AvailAddress(#[sov_wallet(as_ty = "AvailAddressSchema")] pub AccountId);

#[derive(sov_rollup_interface::sov_universal_wallet::UniversalWallet)]
#[allow(dead_code)]
#[doc(hidden)]
struct AvailAddressSchema(#[sov_wallet(display(hex))] Vec<u8>);

impl Display for AvailAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let substrate_id = SubstrateAccountId32::from(self.0 .0);
        write!(f, "{}", substrate_id.to_ss58check())
    }
}

impl AsRef<[u8]> for AvailAddress {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl TryFrom<&[u8]> for AvailAddress {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let arr: [u8; 32] = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Invalid slice length"))?;
        Ok(Self(AccountId::from(arr)))
    }
}

impl FromStr for AvailAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let substrate_id = SubstrateAccountId32::from_string(s)?;
        let raw: [u8; 32] = substrate_id.into();
        Ok(Self(AccountId::from(raw)))
    }
}
impl Hash for AvailAddress {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <AccountId as AsRef<[u8]>>::as_ref(&self.0).hash(state)
    }
}

impl schemars::JsonSchema for AvailAddress {
    fn schema_name() -> String {
        "AvailAddress".to_string()
    }

    fn json_schema(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        serde_json::from_value(serde_json::json!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{64}$",
            "description": "A 32-byte hex Avail address"
        }))
        .expect("Invalid schema JSON")
    }
}

impl BorshSerialize for AvailAddress {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_all(&self.0 .0)
    }
}

impl BorshDeserialize for AvailAddress {
    fn deserialize_reader<R: Read>(reader: &mut R) -> std::io::Result<Self> {
        let bytes: [u8; 32] = borsh::BorshDeserialize::deserialize_reader(reader)?;
        Ok(Self(AccountId::from(bytes)))
    }
}

#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for AvailAddress {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bytes: [u8; 32] = u.arbitrary()?;
        Ok(Self(AccountId::from(bytes)))
    }
}

impl BasicAddress for AvailAddress {}
