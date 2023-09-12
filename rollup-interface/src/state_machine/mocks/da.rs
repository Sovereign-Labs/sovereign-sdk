use std::fmt::Display;
use std::sync::Arc;

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::da::{
    BlobReaderTrait, BlockHashTrait, BlockHeaderTrait, CountedBufReader, DaSpec, DaVerifier,
};
use crate::mocks::MockValidityCond;
use crate::services::da::{DaService, SlotData};
use crate::{BasicAddress, RollupAddress};

/// A mock address type used for testing. Internally, this type is standard 32 byte array.
#[derive(
    Debug,
    PartialEq,
    Clone,
    Eq,
    Copy,
    serde::Serialize,
    serde::Deserialize,
    Hash,
    Default,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
)]
pub struct MockAddress {
    /// Underlying mock address.
    pub addr: [u8; 32],
}

impl core::str::FromStr for MockAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let addr = hex::decode(s)?;
        if addr.len() != 32 {
            return Err(anyhow::anyhow!("Invalid address length"));
        }

        let mut array = [0; 32];
        array.copy_from_slice(&addr);
        Ok(MockAddress { addr: array })
    }
}

impl<'a> TryFrom<&'a [u8]> for MockAddress {
    type Error = anyhow::Error;

    fn try_from(addr: &'a [u8]) -> Result<Self, Self::Error> {
        if addr.len() != 32 {
            anyhow::bail!("Address must be 32 bytes long");
        }
        let mut addr_bytes = [0u8; 32];
        addr_bytes.copy_from_slice(addr);
        Ok(Self { addr: addr_bytes })
    }
}

impl AsRef<[u8]> for MockAddress {
    fn as_ref(&self) -> &[u8] {
        &self.addr
    }
}

impl From<[u8; 32]> for MockAddress {
    fn from(addr: [u8; 32]) -> Self {
        MockAddress { addr }
    }
}

impl Display for MockAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.addr)
    }
}

impl BasicAddress for MockAddress {}
impl RollupAddress for MockAddress {}

#[derive(
    Debug,
    Clone,
    PartialEq,
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
    serde::Serialize,
    serde::Deserialize,
)]

/// A mock BlobTransaction from a DA layer used for testing.
pub struct MockBlob {
    address: MockAddress,
    hash: [u8; 32],
    data: CountedBufReader<Bytes>,
}

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

    #[cfg(feature = "native")]
    fn advance(&mut self, num_bytes: usize) -> &[u8] {
        self.data.advance(num_bytes);
        self.verified_data()
    }

    fn total_len(&self) -> usize {
        self.data.total_len()
    }
}

impl MockBlob {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: MockAddress, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: CountedBufReader::new(bytes::Bytes::from(data)),
            hash,
        }
    }
}

/// A mock hash digest.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    BorshDeserialize,
    BorshSerialize,
)]
pub struct MockHash(pub [u8; 32]);

impl AsRef<[u8]> for MockHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<[u8; 32]> for MockHash {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl From<MockHash> for [u8; 32] {
    fn from(value: MockHash) -> Self {
        value.0
    }
}

impl BlockHashTrait for MockHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone, Copy)]
pub struct MockBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: MockHash,
    /// The hash of this block.
    pub hash: MockHash,
}

impl Default for MockBlockHeader {
    fn default() -> Self {
        Self {
            prev_hash: MockHash([0u8; 32]),
            hash: MockHash([1u8; 32]),
        }
    }
}

impl BlockHeaderTrait for MockBlockHeader {
    type Hash = MockHash;

    fn prev_hash(&self) -> Self::Hash {
        self.prev_hash
    }

    fn hash(&self) -> Self::Hash {
        self.hash
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone)]
pub struct MockBlock {
    /// The header of this block.
    pub header: MockBlockHeader,
    /// The height of this block
    pub height: u64,
    /// Validity condition
    pub validity_cond: MockValidityCond,
    /// Blobs
    pub blobs: Vec<MockBlob>,
}

impl Default for MockBlock {
    fn default() -> Self {
        Self {
            header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: [1; 32].into(),
            },
            height: 0,
            validity_cond: Default::default(),
            blobs: Default::default(),
        }
    }
}

impl SlotData for MockBlock {
    type BlockHeader = MockBlockHeader;
    type Cond = MockValidityCond;

    fn hash(&self) -> [u8; 32] {
        self.header.hash.0
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> MockValidityCond {
        self.validity_cond
    }
}

/// A [`DaSpec`] suitable for testing.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
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

use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex;

#[derive(Clone)]
/// DaService used in tests.
pub struct MockDaService {
    sender: Sender<Vec<u8>>,
    receiver: Arc<Mutex<Receiver<Vec<u8>>>>,
    sequencer_da_address: MockAddress,
}

impl MockDaService {
    /// Creates a new MockDaService.
    pub fn new(sequencer_da_address: MockAddress) -> Self {
        let (sender, receiver) = mpsc::channel(100);
        Self {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
            sequencer_da_address,
        }
    }
}

#[async_trait]
impl DaService for MockDaService {
    type Verifier = MockDaVerifier;
    type Spec = MockDaSpec;
    type FilteredBlock = MockBlock;
    type Error = anyhow::Error;

    async fn get_finalized_at(&self, _height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        let data = self.receiver.lock().await.recv().await;
        let data = data.unwrap();
        let hash = [0; 32];

        let blob = MockBlob::new(data, self.sequencer_da_address, hash);

        Ok(MockBlock {
            blobs: vec![blob],
            ..Default::default()
        })
    }

    async fn get_block_at(&self, height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        self.get_finalized_at(height).await
    }

    fn extract_relevant_txs(
        &self,
        block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
        block.blobs.clone()
    }

    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        ([0u8; 32], ())
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        self.sender.send(blob.to_vec()).await.unwrap();
        Ok(())
    }
}

/// The configuration for mock da
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct MockDaConfig {}

#[derive(Clone, Default)]
/// DaVerifier used in tests.
pub struct MockDaVerifier {}

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
        Ok(MockValidityCond { is_valid: true })
    }
}
