use avail_rust::{
    block::DataSubmission, subxt::utils::MultiAddress, AccountId, BlockHash, MultiAddress, H256,
};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::da::{BlockHashTrait, CountedBufReader};

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AvailDABlob {
    pub blob: CountedBufReader<bytes::Bytes>,
    pub hash: H256,
    pub sender: AccountId,
}

impl BlobReaderTrait for AvailDABlob {
    type Address = AccountId;
    type BlobHash = H256;

    fn sender(&self) -> AccountId {
        self.sender.clone()
    }

    fn hash(&self) -> Self::BlobHash {
        H256(self.hash)
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
        self.blob.accumulator()
    }
}
