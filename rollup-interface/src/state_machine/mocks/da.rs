use std::fmt::Display;
use std::io::Read;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};

use crate::da::{BlobReaderTrait, BlockHashTrait, BlockHeaderTrait, DaSpec, DaVerifier};
use crate::mocks::MockValidityCond;
use crate::services::batch_builder::BatchBuilder;
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
    data: Bytes,
    #[serde(skip)]
    bytes_read: usize,
}

impl MockBlob {
    /// The number of bytes left to read from this blob
    pub fn remaining(&self) -> usize {
        self.data.len() - self.bytes_read
    }
}

impl Read for MockBlob {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bytes_to_read = std::cmp::min(buf.len(), self.remaining());
        let data = &self.data[self.bytes_read..];
        buf[..bytes_to_read].copy_from_slice(&data[..bytes_to_read]);
        self.bytes_read += bytes_to_read;
        Ok(bytes_to_read)
    }
}

impl BlobReaderTrait for MockBlob {
    type Address = MockAddress;

    fn sender(&self) -> Self::Address {
        self.address
    }

    fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn num_bytes_read(&self) -> usize {
        self.bytes_read
    }
}

impl MockBlob {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: MockAddress, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: bytes::Bytes::from(data),
            hash,
            bytes_read: 0,
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
    borsh::BorshDeserialize,
    borsh::BorshSerialize,
)]
pub struct MockHash(pub [u8; 32]);

impl AsRef<[u8]> for MockHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl BlockHashTrait for MockHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone, Copy)]
pub struct MockBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: MockHash,
    /// The hash of the current
    pub hash: MockHash,
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
                prev_hash: MockHash([0; 32]),
                hash: MockHash([1; 32]),
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
        self.header.hash().0
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> MockValidityCond {
        self.validity_cond
    }
}

/// A [`DaSpec`] suitable for testing.
pub struct MockDaSpec;

impl DaSpec for MockDaSpec {
    type SlotHash = MockHash;
    type BlockHeader = MockBlockHeader;
    type BlobTransaction = MockBlob;
    type ValidityCondition = MockValidityCond;
    type InclusionMultiProof = ();
    type CompletenessProof = ();
    type ChainParams = ();
}

impl core::fmt::Debug for MockDaSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockDaSpec").finish()
    }
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
    ) -> (
        Vec<<Self::Spec as DaSpec>::BlobTransaction>,
        MockValidityCond,
    ) {
        (block.blobs.clone(), Default::default())
    }

    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        ((), ())
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        self.sender.send(blob.to_vec()).await.unwrap();
        Ok(())
    }
}

#[derive(Clone, Default)]
/// DaService used in tests.
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

/// BatchBuilder used in tests.
pub struct MockBatchBuilder {
    /// Mempool with transactions.
    pub mempool: Vec<Vec<u8>>,
}

// It only takes the first byte of the tx, when submits it.
// This allows to show effect of batch builder
impl BatchBuilder for MockBatchBuilder {
    fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<()> {
        self.mempool.push(tx);
        Ok(())
    }

    fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
        if self.mempool.is_empty() {
            anyhow::bail!("Mock mempool is empty");
        }
        let txs = std::mem::take(&mut self.mempool)
            .into_iter()
            .filter_map(|tx| {
                if !tx.is_empty() {
                    Some(vec![tx[0]])
                } else {
                    None
                }
            })
            .collect();
        Ok(txs)
    }
}
