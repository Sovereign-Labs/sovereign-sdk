use sov_rollup_interface::da::DaSpec;

use crate::verifier::ChainValidityCondition;

pub mod address;
pub mod block;
mod hash;
pub mod header;
pub mod transaction;

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct DaLayerSpec;

impl DaSpec for DaLayerSpec {
    type SlotHash = hash::AvailHash;

    type BlockHeader = header::AvailHeader;

    type BlobTransaction = transaction::AvailBlobTransaction;

    type Address = address::AvailAddress;

    type ValidityCondition = ChainValidityCondition;

    type InclusionMultiProof = ();

    type CompletenessProof = ();

    type ChainParams = ();
}
