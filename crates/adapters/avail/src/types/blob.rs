use avail_rust::avail_core::from_substrate::blake2_256;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlobReaderTrait, CountedBufReader};

use crate::types::{address::AvailAddress, data::AvailData, hash::AvailHash};

#[derive(PartialEq, Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub struct AvailDABlob {
    pub blob: CountedBufReader<bytes::Bytes>,
    pub hash: AvailHash,
    pub sender: AvailAddress,
}

impl BlobReaderTrait for AvailDABlob {
    type Address = AvailAddress;
    type BlobHash = AvailHash;

    fn sender(&self) -> Self::Address {
        self.sender.clone()
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
        self.blob.accumulator()
    }
}

impl From<AvailData> for AvailDABlob {
    fn from(data: AvailData) -> Self {
        let bytes = bytes::Bytes::from(data.data.clone());
        let blob = CountedBufReader::new(bytes.clone());
        AvailDABlob {
            blob,
            hash: AvailHash::try_from(blake2_256(&bytes)).unwrap(),
            sender: AvailAddress(data.signer),
        }
    }
}
