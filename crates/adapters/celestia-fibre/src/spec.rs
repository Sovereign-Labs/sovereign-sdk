use sov_celestia_adapter::types::TmHash;
use sov_celestia_adapter::verifier::address::CelestiaAddress;
use sov_celestia_adapter::verifier::RollupParams;
use sov_celestia_adapter::CelestiaHeader;
use sov_rollup_interface::da::{
    BlobReaderTrait, CountedBufReader, DaSpec, DaVerifier, RelevantBlobs, RelevantProofs,
};

#[derive(
    Default,
    serde::Serialize,
    serde::Deserialize,
    Debug,
    PartialEq,
    Eq,
    Clone,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    schemars::JsonSchema,
)]
pub struct CelestiaFibreSpec;

impl DaSpec for CelestiaFibreSpec {
    type SlotHash = TmHash;
    type BlockHeader = CelestiaHeader;
    type BlobTransaction = FibreBlob;
    type TransactionId = TmHash;
    type Address = CelestiaAddress;
    // shortcut: no proofs yet, replace with real types together with `CelestiaFibreVerifier`.
    type InclusionMultiProof = ();
    type CompletenessProof = ();
    type ChainParams = RollupParams;
}

/// Rollup blob downloaded from Fibre, identified by the commitment from its v2 share.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FibreBlob {
    sender: CelestiaAddress,
    hash: TmHash,
    blob: CountedBufReader<bytes::Bytes>,
}

impl FibreBlob {
    pub fn new(sender: CelestiaAddress, commitment: [u8; 32], data: Vec<u8>) -> Self {
        Self {
            sender,
            hash: TmHash::from(commitment),
            blob: CountedBufReader::new(bytes::Bytes::from(data)),
        }
    }
}

impl BlobReaderTrait for FibreBlob {
    type Address = CelestiaAddress;
    type BlobHash = TmHash;

    fn sender(&self) -> Self::Address {
        self.sender
    }

    fn hash(&self) -> Self::BlobHash {
        self.hash.clone()
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

#[derive(Clone)]
pub struct CelestiaFibreVerifier;

impl DaVerifier for CelestiaFibreVerifier {
    type Spec = CelestiaFibreSpec;
    type Error = anyhow::Error;

    fn new(_params: RollupParams) -> Self {
        Self
    }

    fn verify_relevant_tx_list(
        &self,
        _block_header: &CelestiaHeader,
        _relevant_blobs: &RelevantBlobs<FibreBlob>,
        _relevant_proofs: RelevantProofs<(), ()>,
    ) -> Result<(), Self::Error> {
        // Fail closed: blobs must be checked against the v2 share commitments before this can pass.
        anyhow::bail!("Celestia Fibre blob verification is not implemented")
    }
}
