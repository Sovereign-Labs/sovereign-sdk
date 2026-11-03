mod error;

use borsh::{BorshDeserialize, BorshSerialize};
pub use celestia_types::namespace_data::NamespaceData;
pub use celestia_types::nmt::Namespace;
pub use error::*;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::BlockHashTrait;
use sov_universal_wallet::schema::OverrideSchema;
use sov_universal_wallet::UniversalWallet;

#[derive(Debug, PartialEq, PartialOrd, Ord, Clone, Eq, Hash, Serialize, Deserialize)]
pub struct TmHash(pub tendermint::Hash);

// Schema type for TmHash to enable UniversalWallet support
#[derive(UniversalWallet)]
#[allow(dead_code)]
#[doc(hidden)]
pub struct TmHashSchema(#[sov_wallet(display(hex))] [u8; 32]);

impl OverrideSchema for TmHash {
    type Output = TmHashSchema;
}

impl BorshSerialize for TmHash {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> Result<(), std::io::Error> {
        BorshSerialize::serialize(self.inner(), writer)
    }
}

impl BorshDeserialize for TmHash {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, std::io::Error> {
        let bytes = <[u8; 32] as BorshDeserialize>::deserialize(buf)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }

    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(tendermint::Hash::Sha256(bytes)))
    }
}

impl AsRef<[u8]> for TmHash {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl core::fmt::Display for TmHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{}", self.0)
    }
}

impl core::str::FromStr for TmHash {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let stripped = s.strip_prefix("0x").unwrap_or(s);
        let inner = tendermint::Hash::from_str(stripped)?;
        Ok(TmHash(inner))
    }
}

impl TmHash {
    pub fn inner(&self) -> &[u8; 32] {
        match self.0 {
            tendermint::Hash::Sha256(ref h) => h,
            // `Hash::None` is normalized at a higher layer (genesis predecessor placeholder),
            // so `TmHash` should never observe it.
            tendermint::Hash::None => unreachable!("Only the genesis block has a None hash, and we use a placeholder in that corner case")
        }
    }
}

impl BlockHashTrait for TmHash {}

impl From<TmHash> for [u8; 32] {
    fn from(val: TmHash) -> Self {
        *val.inner()
    }
}

impl From<[u8; 32]> for TmHash {
    fn from(value: [u8; 32]) -> Self {
        Self(tendermint::Hash::Sha256(value))
    }
}

/// Data required to extract and prove blobs in one Celestia namespace.
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct NamespaceRelevantData {
    pub namespace: Namespace,
    pub data: NamespaceData,
}

impl NamespaceRelevantData {
    #[cfg(feature = "native")]
    pub fn new(namespace: Namespace, data: NamespaceData) -> Self {
        #[cfg(debug_assertions)]
        {
            for row in data.rows() {
                for share in &row.shares {
                    assert_eq!(
                        share.namespace(),
                        namespace,
                        "Share from different namespace {} (expected {})",
                        String::from_utf8_lossy(share.namespace().as_bytes()),
                        String::from_utf8_lossy(namespace.as_bytes())
                    );
                }
            }
        }
        Self { namespace, data }
    }
}

/// Proof of namespace end boundary in the last relevant row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NamespaceBoundaryProof {
    /// Namespace proof for the boundary.
    /// For presence proofs this is narrowed to the last namespace share.
    /// For absence proofs this proves namespace absence in that row.
    pub last_share_proof: celestia_types::nmt::NamespaceProof,
    /// The last namespace share when `last_share_proof` is of presence; `None` for absence proofs.
    pub last_share: Option<celestia_types::Share>,
}

#[cfg(feature = "native")]
impl NamespaceBoundaryProof {
    pub fn from_namespace_data(namespace_data: &NamespaceRelevantData) -> Option<Self> {
        let last_row = namespace_data.data.rows().last()?;
        if last_row.shares.is_empty() && last_row.proof.is_of_presence() {
            panic!("Incorrect namespace data: last row proof is of presence, but no shares");
        } else if last_row.shares.is_empty() && last_row.proof.is_of_absence() {
            return Some(Self {
                last_share_proof: last_row.proof.clone(),
                last_share: None,
            });
        }
        let all_before_last = &last_row.shares[..last_row.shares.len().saturating_sub(1)];
        let last_share = last_row
            .shares
            .last()
            .expect("Incorrect namespace data: missing shares from last row")
            .clone();
        let last_share_proof = last_row
            .proof
            .narrow_range(all_before_last, &[], *namespace_data.namespace)
            .expect("Incorrect namespace data: cannot narrow range proof last share");
        Some(Self {
            last_share_proof: last_share_proof.into(),
            last_share: Some(last_share),
        })
    }
}
