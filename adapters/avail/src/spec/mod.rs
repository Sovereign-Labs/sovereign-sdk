use sov_rollup_interface::da::DaSpec;
use crate::verifier::ChainValidityCondition;

pub mod address;
pub mod block;
mod hash;
pub mod header;
pub mod transaction;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DaLayerSpec;

impl DaSpec for DaLayerSpec {
    type ValidityCondition =  ChainValidityCondition;

    type SlotHash = hash::AvailHash;

    type ChainParams = ();

    type BlockHeader = header::AvailHeader;

    type BlobTransaction = transaction::AvailBlobTransaction;

    type InclusionMultiProof = ();

    type CompletenessProof = ();
}
