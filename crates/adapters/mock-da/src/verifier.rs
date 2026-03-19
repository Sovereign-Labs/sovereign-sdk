use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::da::{
    BlobReaderTrait, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};

use crate::{MockAddress, MockBlob, MockBlockHeader, MockDaVerifier, MockHash};

impl BlobReaderTrait for MockBlob {
    type Address = MockAddress;
    type BlobHash = MockHash;

    fn sender(&self) -> Self::Address {
        self.address
    }

    fn hash(&self) -> Self::BlobHash {
        self.hash
    }

    fn verified_data(&self) -> &[u8] {
        self.blob.accumulator()
    }

    fn total_len(&self) -> usize {
        self.blob.total_len()
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.blob.advance(num_bytes);
        self.verified_data()
    }
}

/// A [`sov_rollup_interface::da::DaSpec`] suitable for testing.
#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    schemars::JsonSchema,
)]
pub struct MockDaSpec;

impl DaSpec for MockDaSpec {
    type SlotHash = MockHash;
    type BlockHeader = MockBlockHeader;
    type BlobTransaction = MockBlob;
    type TransactionId = MockHash;
    type Address = MockAddress;

    type InclusionMultiProof = [u8; 32];
    type CompletenessProof = ();
    type ChainParams = ();
}

impl DaVerifier for MockDaVerifier {
    type Spec = MockDaSpec;

    type Error = anyhow::Error;

    fn new(_params: <Self::Spec as DaSpec>::ChainParams) -> Self {
        Self {}
    }

    fn verify_relevant_tx_list(
        &self,
        _block_header: &<Self::Spec as DaSpec>::BlockHeader,
        _relevant_blobs: &RelevantBlobs<<Self::Spec as DaSpec>::BlobTransaction>,
        _relevant_proofs: RelevantProofs<
            <Self::Spec as DaSpec>::InclusionMultiProof,
            <Self::Spec as DaSpec>::CompletenessProof,
        >,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}
