use sov_rollup_interface::da::{BlobReaderTrait, DaSpec, DaVerifier};

use crate::{MockAddress, MockBlob, MockBlockHeader, MockDaVerifier, MockHash, MockValidityCond};

impl BlobReaderTrait for MockBlob {
    type Address = MockAddress;

    fn sender(&self) -> Self::Address {
        self.address
    }

    fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn verified_data(&self) -> &[u8] {
        self.data.accumulator()
    }

    fn total_len(&self) -> usize {
        self.data.total_len()
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.data.advance(num_bytes);
        self.verified_data()
    }
}

/// A [`sov_rollup_interface::da::DaSpec`] suitable for testing.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct MockDaSpec;

impl DaSpec for MockDaSpec {
    type SlotHash = MockHash;
    type BlockHeader = MockBlockHeader;
    type BlobTransaction = MockBlob;
    type Address = MockAddress;
    type ValidityCondition = MockValidityCond;
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
        _txs: &[<Self::Spec as DaSpec>::BlobTransaction],
        _inclusion_proof: <Self::Spec as DaSpec>::InclusionMultiProof,
        _completeness_proof: <Self::Spec as DaSpec>::CompletenessProof,
    ) -> Result<<Self::Spec as DaSpec>::ValidityCondition, Self::Error> {
        Ok(Default::default())
    }
}
