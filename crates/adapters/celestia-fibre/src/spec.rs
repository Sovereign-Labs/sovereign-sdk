use borsh::{BorshDeserialize, BorshSerialize};
use sov_celestia_common::address::CelestiaAddress;
use sov_celestia_common::header::CelestiaHeader;
use sov_celestia_common::types::{NamespaceBoundaryProof, TmHash};
use sov_celestia_common::verifier::RollupParams;
use sov_rollup_interface::da::DaSpec;

use crate::blob::FibreBlob;
use crate::proof::FibreBlobProof;

#[derive(
    Default,
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    BorshSerialize,
    BorshDeserialize,
    schemars::JsonSchema,
)]
pub struct CelestiaFibreSpec;

impl DaSpec for CelestiaFibreSpec {
    type SlotHash = TmHash;
    type BlockHeader = CelestiaHeader;
    type BlobTransaction = FibreBlob;
    type TransactionId = TmHash;
    type Address = CelestiaAddress;
    type InclusionMultiProof = Vec<FibreBlobProof>;
    type CompletenessProof = Option<NamespaceBoundaryProof>;
    type ChainParams = RollupParams;
}
