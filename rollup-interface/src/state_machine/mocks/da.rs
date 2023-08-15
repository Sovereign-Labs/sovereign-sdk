use std::fmt::Display;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Error};
use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use sha2::Digest;

use crate::da::{BlobReaderTrait, BlockHashTrait, BlockHeaderTrait, CountedBufReader, DaSpec};
use crate::mocks::TestValidityCond;
use crate::services::batch_builder::BatchBuilder;
use crate::services::da::{DaService, SlotData};
use crate::AddressTrait;

/// A mock address type used for testing. Internally, this type is standard 32 byte array.
#[derive(Debug, PartialEq, Clone, Eq, Copy, serde::Serialize, serde::Deserialize, Hash)]
pub struct MockAddress {
    addr: [u8; 32],
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
    type Error = Error;

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

impl AddressTrait for MockAddress {}

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
pub struct TestBlob<Address> {
    address: Address,
    hash: [u8; 32],
    data: CountedBufReader<Bytes>,
}

impl<Address: AddressTrait> BlobReaderTrait for TestBlob<Address> {
    type Data = Bytes;
    type Address = Address;

    fn sender(&self) -> Self::Address {
        self.address.clone()
    }

    fn hash(&self) -> [u8; 32] {
        self.hash
    }

    fn data_mut(&mut self) -> &mut CountedBufReader<Self::Data> {
        &mut self.data
    }

    fn data(&self) -> &CountedBufReader<Self::Data> {
        &self.data
    }
}

impl<Address: AddressTrait> TestBlob<Address> {
    /// Creates a new mock blob with the given data, claiming to have been published by the provided address.
    pub fn new(data: Vec<u8>, address: Address, hash: [u8; 32]) -> Self {
        Self {
            address,
            data: CountedBufReader::new(bytes::Bytes::from(data)),
            hash,
        }
    }
}

/// A mock hash digest.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TestHash(pub [u8; 32]);

impl AsRef<[u8]> for TestHash {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl BlockHashTrait for TestHash {}

/// A mock block header used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone, Copy)]
pub struct TestBlockHeader {
    /// The hash of the previous block.
    pub prev_hash: TestHash,
}

impl BlockHeaderTrait for TestBlockHeader {
    type Hash = TestHash;

    fn prev_hash(&self) -> Self::Hash {
        self.prev_hash
    }

    fn hash(&self) -> Self::Hash {
        TestHash(sha2::Sha256::digest(self.prev_hash.0).into())
    }
}

/// A mock block type used for testing.
#[derive(Serialize, Deserialize, PartialEq, core::fmt::Debug, Clone, Copy)]
pub struct TestBlock {
    /// The hash of this block.
    pub curr_hash: [u8; 32],
    /// The header of this block.
    pub header: TestBlockHeader,
    /// The height of this block
    pub height: u64,
    /// Validity condition
    pub validity_cond: TestValidityCond,
}

impl Default for TestBlock {
    fn default() -> Self {
        Self {
            curr_hash: [0; 32],
            header: TestBlockHeader {
                prev_hash: TestHash([0; 32]),
            },
            height: 0,
            validity_cond: TestValidityCond::default(),
        }
    }
}

impl SlotData for TestBlock {
    type BlockHeader = TestBlockHeader;
    type Cond = TestValidityCond;

    fn hash(&self) -> [u8; 32] {
        self.curr_hash
    }

    fn header(&self) -> &Self::BlockHeader {
        &self.header
    }

    fn validity_condition(&self) -> TestValidityCond {
        self.validity_cond
    }
}

/// A [`DaSpec`] suitable for testing.
pub struct MockDaSpec;

impl DaSpec for MockDaSpec {
    type SlotHash = TestHash;
    type ValidityCondition = TestValidityCond;
    type BlockHeader = TestBlockHeader;
    type BlobTransaction = TestBlob<MockAddress>;
    type InclusionMultiProof = [u8; 32];
    type CompletenessProof = ();
    type ChainParams = ();
}

///TODO
pub struct MockDaService {
    submitted: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl MockDaService {
    ///TODO
    pub fn new() -> Self {
        MockDaService {
            submitted: Arc::new(Mutex::new(Vec::new())),
        }
    }

    ///TODO
    pub fn is_empty(&self) -> bool {
        self.submitted.lock().unwrap().is_empty()
    }

    ///TODO
    pub fn get_submitted(&self) -> Vec<Vec<u8>> {
        self.submitted.lock().unwrap().clone()
    }
}

#[async_trait]
impl DaService for MockDaService {
    type RuntimeConfig = ();
    type Spec = MockDaSpec;
    type FilteredBlock = TestBlock;
    type Error = anyhow::Error;

    async fn new(
        _config: Self::RuntimeConfig,
        _chain_params: <Self::Spec as DaSpec>::ChainParams,
    ) -> Self {
        MockDaService::new()
    }

    async fn get_finalized_at(&self, _height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        todo!()
    }

    async fn get_block_at(&self, _height: u64) -> Result<Self::FilteredBlock, Self::Error> {
        todo!()
    }

    fn extract_relevant_txs(
        &self,
        _block: &Self::FilteredBlock,
    ) -> Vec<<Self::Spec as DaSpec>::BlobTransaction> {
        todo!()
    }

    async fn get_extraction_proof(
        &self,
        _block: &Self::FilteredBlock,
        _blobs: &[<Self::Spec as DaSpec>::BlobTransaction],
    ) -> (
        <Self::Spec as DaSpec>::InclusionMultiProof,
        <Self::Spec as DaSpec>::CompletenessProof,
    ) {
        todo!()
    }

    async fn send_transaction(&self, blob: &[u8]) -> Result<(), Self::Error> {
        self.submitted.lock().unwrap().push(blob.to_vec());
        Ok(())
    }
}
///TODO
pub struct MockBatchBuilder {
    ///TODO
    pub mempool: Vec<Vec<u8>>,
}

/// It only takes the first byte of the tx, when submits it.
/// This allows to show effect of batch builder
impl BatchBuilder for MockBatchBuilder {
    fn accept_tx(&mut self, tx: Vec<u8>) -> anyhow::Result<()> {
        self.mempool.push(tx);
        Ok(())
    }

    fn get_next_blob(&mut self) -> anyhow::Result<Vec<Vec<u8>>> {
        if self.mempool.is_empty() {
            bail!("Mock mempool is empty");
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
