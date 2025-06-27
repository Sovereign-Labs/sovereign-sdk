use avail_rust::avail_core::DataProof;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{DaSpec, DaVerifier};
use sov_rollup_interface::node::da::{RelevantBlobs, RelevantProofs};
use thiserror::Error;

use crate::types::prelude::*;
use avail_rust::prelude::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, BorshDeserialize, BorshSerialize)]
pub struct AvailDASpec;

impl DaSpec for AvailDASpec {
    type SlotHash = BlockHash;

    type BlockHeader = AvailHeader;

    type BlobTransaction = AvailDABlob;

    type TransactionId = H256;

    type Address = AccountId;

    type InclusionMultiProof = DataProof;

    type CompletenessProof = DataProof;

    type ChainParams = AppId;
}

#[derive(Clone)]
pub struct AvailDAVerifier;

impl DaVerifier for AvailDAVerifier {
    type Spec = AvailDASpec;

    type Error = ();

    // Verify that the given list of blob transactions is complete and correct.
    // NOTE: Function return unit since application client already verifies application data.
    fn verify_relevant_tx_list(
        &self,
        _block_header: &<Self::Spec as DaSpec>::BlockHeader,
        _relevant_blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
        _relevant_proofs: RelevantProofs<
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        >,
    ) -> Result<(), Self::Error> {
        todo!()
    }

    fn new(_params: <Self::Spec as DaSpec>::ChainParams) -> Self {
        Verifier {}
    }
}
