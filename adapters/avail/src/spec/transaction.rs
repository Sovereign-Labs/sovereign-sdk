#[cfg(feature = "native")]
use anyhow::anyhow;
#[cfg(feature = "native")]
use avail_subxt::{
    api::runtime_types::{da_control::pallet::Call, da_runtime::RuntimeCall::DataAvailability},
    primitives::AppUncheckedExtrinsic,
};
use bytes::Bytes;
#[cfg(feature = "native")]
use codec::Encode;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::{BlobReaderTrait, CountedBufReader};

use super::address::AvailAddress;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]

pub struct AvailBlobTransaction {
    blob: CountedBufReader<Bytes>,
    hash: [u8; 32],
    address: AvailAddress,
}

impl BlobReaderTrait for AvailBlobTransaction {
    type Address = AvailAddress;

    fn sender(&self) -> AvailAddress {
        self.address.clone()
    }

    fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn verified_data(&self) -> &[u8] {
        self.blob.accumulator()
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.blob.advance(num_bytes);
        self.verified_data()
    }

    fn total_len(&self) -> usize {
        self.blob.total_len()
    }
}

impl AvailBlobTransaction {
    #[cfg(feature = "native")]
    pub fn new(unchecked_extrinsic: &AppUncheckedExtrinsic) -> anyhow::Result<Self> {
        let address = match &unchecked_extrinsic.signature {
            //TODO: Handle other types of MultiAddress.
            Some((subxt::utils::MultiAddress::Id(id), _, _)) => AvailAddress::from(id.clone().0),
            _ => {
                return Err(anyhow!(
                    "Unsigned extrinsic being used to create AvailBlobTransaction."
                ))
            }
        };
        let blob = match &unchecked_extrinsic.function {
            DataAvailability(Call::submit_data { data }) => {
                CountedBufReader::<Bytes>::new(Bytes::copy_from_slice(&data.0))
            }
            _ => {
                return Err(anyhow!(
                    "Invalid type of extrinsic being converted to AvailBlobTransaction."
                ))
            }
        };

        Ok(AvailBlobTransaction {
            hash: sp_core_hashing::blake2_256(&unchecked_extrinsic.encode()),
            address,
            blob,
        })
    }

    pub fn combine_hash(&self, hash: [u8; 32]) -> [u8; 32] {
        let mut combined_hashes: Vec<u8> = Vec::with_capacity(64);
        combined_hashes.extend_from_slice(hash.as_ref());
        combined_hashes.extend_from_slice(self.hash().as_ref());

        sp_core_hashing::blake2_256(&combined_hashes)
    }
}
