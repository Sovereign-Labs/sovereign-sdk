use std::ops::Range;

use serde::{Deserialize, Serialize};
use sov_celestia_common::address::CelestiaAddress;
use sov_celestia_common::types::TmHash;
use sov_rollup_interface::da::{BlobReaderTrait, CountedBufReader};
use sov_rollup_interface::Bytes;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "FibreBlobWire", into = "FibreBlobWire")]
pub struct FibreBlob {
    sender: CelestiaAddress,
    commitment: [u8; 32],
    range_in_namespace: Range<usize>,
    data: CountedBufReader<Bytes>,
    total_len: u64,
}

impl FibreBlob {
    pub fn new(
        sender: CelestiaAddress,
        commitment: [u8; 32],
        range_in_namespace: Range<usize>,
        data: Bytes,
    ) -> Self {
        let total_len = u64::try_from(data.len()).expect("Fibre blob length fits in u64");
        Self {
            sender,
            commitment,
            range_in_namespace,
            data: CountedBufReader::new(data),
            total_len,
        }
    }

    pub fn sender(&self) -> CelestiaAddress {
        self.sender
    }

    pub fn commitment(&self) -> [u8; 32] {
        self.commitment
    }

    pub fn range_in_namespace(&self) -> &Range<usize> {
        &self.range_in_namespace
    }

    pub fn verified_data(&self) -> &[u8] {
        self.data.accumulator()
    }
}

impl BlobReaderTrait for FibreBlob {
    type Address = CelestiaAddress;
    type BlobHash = TmHash;

    fn sender(&self) -> CelestiaAddress {
        self.sender
    }

    fn hash(&self) -> Self::BlobHash {
        TmHash::from(self.commitment)
    }

    fn verified_data(&self) -> &[u8] {
        self.data.accumulator()
    }

    fn total_len(&self) -> usize {
        usize::try_from(self.total_len).expect("Fibre blob length fits in usize")
    }

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.data.advance(num_bytes);
        self.verified_data()
    }
}

#[derive(Serialize, Deserialize)]
struct FibreBlobWire {
    accumulator: Vec<u8>,
    total_len: u64,
    sender: CelestiaAddress,
    commitment: [u8; 32],
    range_in_namespace: Range<usize>,
}

impl From<&FibreBlob> for FibreBlobWire {
    fn from(blob: &FibreBlob) -> Self {
        Self {
            accumulator: blob.data.accumulator().to_vec(),
            total_len: blob.total_len,
            sender: blob.sender,
            commitment: blob.commitment,
            range_in_namespace: blob.range_in_namespace.clone(),
        }
    }
}

impl From<FibreBlob> for FibreBlobWire {
    fn from(blob: FibreBlob) -> Self {
        Self::from(&blob)
    }
}

impl TryFrom<FibreBlobWire> for FibreBlob {
    type Error = String;

    fn try_from(wire: FibreBlobWire) -> Result<Self, Self::Error> {
        if wire.total_len > u64::from(u32::MAX) {
            return Err(format!(
                "pruned blob total length {} exceeds the DA sequence-length limit",
                wire.total_len
            ));
        }
        if u64::try_from(wire.accumulator.len()).unwrap_or(u64::MAX) > wire.total_len {
            return Err("pruned blob accumulator exceeds total length".to_owned());
        }
        Ok(Self {
            sender: wire.sender,
            commitment: wire.commitment,
            range_in_namespace: wire.range_in_namespace,
            data: CountedBufReader::from_raw_parts(Bytes::new(), wire.accumulator),
            total_len: wire.total_len,
        })
    }
}
