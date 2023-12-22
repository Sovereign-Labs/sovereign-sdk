use sha2::Digest;
use sov_mock_da::{
    MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaSpec, MockValidityCond,
};
use sov_mock_zkvm::MockZkvm;
use sov_prover_storage_manager::{new_orphan_storage, SnapshotManager};
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaSpec};
use sov_rollup_interface::stf::{SlotResult, StateTransitionFunction};
use sov_rollup_interface::zk::{ValidityCondition, Zkvm};
use sov_state::storage::{NativeStorage, StorageKey, StorageValue};
use sov_state::{
    ArrayWitness, DefaultStorageSpec, OrderedReadsAndWrites, Prefix, ProverStorage, Storage,
};

pub type S = DefaultStorageSpec;
pub type Q = SnapshotManager;

#[derive(Default, Clone)]
pub struct HashStf<Cond> {
    phantom_data: std::marker::PhantomData<Cond>,
}

impl<Cond> HashStf<Cond> {
    pub fn new() -> Self {
        Self {
            phantom_data: std::marker::PhantomData,
        }
    }

    fn hash_key() -> StorageKey {
        let prefix = Prefix::new(b"root".to_vec());
        StorageKey::singleton(&prefix)
    }

    fn save_from_hasher(
        hasher: sha2::Sha256,
        storage: ProverStorage<S, Q>,
        witness: &ArrayWitness,
    ) -> ([u8; 32], ProverStorage<S, Q>) {
        let result = hasher.finalize();

        let hash_key = HashStf::<Cond>::hash_key();
        let hash_value = StorageValue::from(result.as_slice().to_vec());

        let ordered_reads_writes = OrderedReadsAndWrites {
            ordered_reads: Vec::default(),
            ordered_writes: vec![(hash_key.to_cache_key(), Some(hash_value.into_cache_value()))],
        };

        let (jmt_root_hash, state_update) = storage
            .compute_state_update(ordered_reads_writes, witness)
            .unwrap();

        storage.commit(&state_update, &OrderedReadsAndWrites::default());

        let mut root_hash = [0u8; 32];

        for (i, &byte) in jmt_root_hash.as_ref().iter().enumerate().take(32) {
            root_hash[i] = byte;
        }

        (root_hash, storage)
    }
}

impl<Vm: Zkvm, Cond: ValidityCondition, Da: DaSpec> StateTransitionFunction<Vm, Da>
    for HashStf<Cond>
{
    type StateRoot = [u8; 32];
    type GenesisParams = Vec<u8>;
    type PreState = ProverStorage<S, Q>;
    type ChangeSet = ProverStorage<S, Q>;
    type TxReceiptContents = ();
    type BatchReceiptContents = [u8; 32];
    type Witness = ArrayWitness;
    type Condition = Cond;

    fn init_chain(
        &self,
        genesis_state: Self::PreState,
        params: Self::GenesisParams,
    ) -> (Self::StateRoot, Self::ChangeSet) {
        let mut hasher = sha2::Sha256::new();
        hasher.update(params);

        HashStf::<Cond>::save_from_hasher(hasher, genesis_state, &ArrayWitness::default())
    }

    fn apply_slot<'a, I>(
        &self,
        pre_state_root: &Self::StateRoot,
        storage: Self::PreState,
        witness: Self::Witness,
        slot_header: &Da::BlockHeader,
        _validity_condition: &Da::ValidityCondition,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::ChangeSet,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        tracing::debug!(
            "Applying slot in HashStf at height={}",
            slot_header.height()
        );
        let mut hasher = sha2::Sha256::new();

        let hash_key = HashStf::<Cond>::hash_key();
        let existing_cache = storage.get(&hash_key, None, &witness).unwrap();
        tracing::debug!(
            "HashStf provided_state_root={:?}, saved={:?}",
            pre_state_root,
            existing_cache.value()
        );
        hasher.update(existing_cache.value());

        for blob in blobs {
            let data = blob.verified_data();
            hasher.update(data);
        }

        let (state_root, storage) = HashStf::<Cond>::save_from_hasher(hasher, storage, &witness);

        SlotResult {
            state_root,
            change_set: storage,
            // TODO: Add batch receipts to inspection
            batch_receipts: vec![],
            witness,
        }
    }
}

#[test]
fn compare_output() {
    let genesis_params: Vec<u8> = vec![1, 2, 3, 4, 5];

    let raw_blobs: Vec<Vec<Vec<u8>>> = vec![
        // Block A
        vec![vec![1, 1, 1], vec![2, 2, 2]],
        // Block B
        vec![vec![3, 3, 3], vec![4, 4, 4], vec![5, 5, 5]],
        // Block C
        vec![vec![6, 6, 6]],
        // Block D
        vec![vec![7, 7, 7], vec![8, 8, 8]],
    ];

    let mut blocks = Vec::new();

    for (idx, raw_block) in raw_blobs.iter().enumerate() {
        let mut blobs = Vec::new();
        for raw_blob in raw_block.iter() {
            let blob = MockBlob::new(
                raw_blob.clone(),
                MockAddress::new([11u8; 32]),
                [idx as u8; 32],
            );
            blobs.push(blob);
        }

        let block = MockBlock {
            header: MockBlockHeader::from_height((idx + 1) as u64),
            validity_cond: MockValidityCond::default(),
            blobs,
        };
        blocks.push(block);
    }

    let (state_root, root_hash) = get_result_from_blocks(&genesis_params, &blocks);

    assert!(root_hash.is_some());

    let recorded_state_root: [u8; 32] = [
        121, 110, 56, 75, 48, 251, 66, 243, 236, 155, 4, 128, 238, 122, 188, 160, 17, 46, 169, 39,
        160, 142, 220, 208, 15, 213, 221, 250, 108, 52, 7, 46,
    ];

    assert_eq!(recorded_state_root, state_root);
}

// Returns final data hash and root hash
pub fn get_result_from_blocks(
    genesis_params: &[u8],
    blocks: &[MockBlock],
) -> ([u8; 32], Option<<ProverStorage<S, Q> as Storage>::Root>) {
    let tmpdir = tempfile::tempdir().unwrap();

    let storage = new_orphan_storage(tmpdir.path()).unwrap();

    let stf = HashStf::<MockValidityCond>::new();

    let (genesis_state_root, mut storage) =
        <HashStf<MockValidityCond> as StateTransitionFunction<
            MockZkvm<MockValidityCond>,
            MockDaSpec,
        >>::init_chain(&stf, storage, genesis_params.to_vec());

    let mut state_root = genesis_state_root;

    let l = blocks.len();

    for block in blocks {
        let mut blobs = block.blobs.clone();

        let result = <HashStf<MockValidityCond> as StateTransitionFunction<
            MockZkvm<MockValidityCond>,
            MockDaSpec,
        >>::apply_slot::<&mut Vec<MockBlob>>(
            &stf,
            &state_root,
            storage,
            ArrayWitness::default(),
            &block.header,
            &block.validity_cond,
            &mut blobs,
        );

        state_root = result.state_root;
        storage = result.change_set;
    }

    let root_hash = storage.get_root_hash(l as u64).ok();
    (state_root, root_hash)
}
