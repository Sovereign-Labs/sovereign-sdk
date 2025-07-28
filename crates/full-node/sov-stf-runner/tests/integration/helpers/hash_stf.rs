use sha2::Digest;
use sov_db::storage_manager::NativeChangeSet;
use sov_mock_da::MockAddress;
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier};
use sov_modules_api::{
    AggregatedProofPublicData, ProofOutcome, ProofReceipt, ProofReceiptContents, Storage,
};
use sov_rollup_interface::common::RollupHeight;
use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait, DaSpec, RelevantBlobIters};
use sov_rollup_interface::stf::{ApplySlotOutput, StateTransitionFunction};
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::{ZkVerifier, Zkvm};
use sov_state::namespaces::User;
use sov_state::storage::{NativeStorage, SlotKey, SlotValue};
use sov_state::{
    ArrayWitness, DefaultStorageSpec, OrderedReadsAndWrites, Prefix, ProverStorage, StateAccesses,
    StorageRoot,
};

pub type S = DefaultStorageSpec<sha2::Sha256>;

#[derive(Default, Clone)]
pub struct HashStf;

impl HashStf {
    pub fn new() -> Self {
        Self
    }

    fn hash_key() -> SlotKey {
        let prefix = Prefix::new(b"root".to_vec());
        SlotKey::singleton(&prefix)
    }

    fn save_from_hasher(
        hasher: sha2::Sha256,
        storage: ProverStorage<S>,
        witness: &ArrayWitness,
        root: StorageRoot<S>,
    ) -> (StorageRoot<S>, NativeChangeSet) {
        let result = hasher.finalize();

        let hash_key = HashStf::hash_key();
        let hash_value = SlotValue::from(result.as_slice().to_vec());

        let ordered_reads_writes = OrderedReadsAndWrites {
            ordered_reads: Vec::default(),
            ordered_writes: vec![(hash_key, Some(hash_value))],
        };
        let state_accesses = StateAccesses {
            user: ordered_reads_writes,
            kernel: Default::default(),
        };

        let (jmt_root_hash, state_update) = storage
            .compute_state_update(state_accesses, witness, root)
            .unwrap();

        let change_set = storage.materialize_changes(state_update);

        (jmt_root_hash, change_set)
    }
}

impl<InnerVm: Zkvm, OuterVm: Zkvm, Da: DaSpec> StateTransitionFunction<InnerVm, OuterVm, Da>
    for HashStf
{
    type Address = MockAddress;
    type StateRoot = StorageRoot<S>;
    type GenesisParams = Vec<u8>;
    type PreState = ProverStorage<S>;
    type ChangeSet = NativeChangeSet;
    type TxReceiptContents = ();
    type StorageProof = ();
    type GasPrice = ();
    type BatchReceiptContents = [u8; 32];
    type Witness = ArrayWitness;

    fn init_chain(
        &self,
        _genesis_rollup_header: &Da::BlockHeader,

        genesis_state: Self::PreState,
        params: Self::GenesisParams,
    ) -> (Self::StateRoot, Self::ChangeSet) {
        let mut hasher = sha2::Sha256::new();
        hasher.update(params);

        HashStf::save_from_hasher(
            hasher,
            genesis_state,
            &ArrayWitness::default(),
            <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT,
        )
    }

    #[tracing::instrument(name = "HashStf::apply_slot", skip_all)]
    fn apply_slot(
        &self,
        pre_state_root: &Self::StateRoot,
        pre_state: Self::PreState,
        witness: Self::Witness,
        slot_header: &Da::BlockHeader,
        relevant_blobs: RelevantBlobIters<&mut [Da::BlobTransaction]>,
        _execution_context: sov_modules_api::ExecutionContext,
    ) -> ApplySlotOutput<InnerVm, OuterVm, Da, Self> {
        // Note: Uses native code, so won't work in ZK
        let storage_root_hash = pre_state
            .get_latest_root_hash()
            .expect("pre state always should have root hash");

        tracing::debug!(
            header = %slot_header.display(),
            passed = hex::encode(pre_state_root),
            from_storage = hex::encode(storage_root_hash.root_hash()),
            "HashStf, starting apply slot",
        );

        assert_eq!(
            pre_state_root, &storage_root_hash,
            "Incorrect pre_state_root has been passed"
        );

        let mut hasher = sha2::Sha256::new();

        let hash_key = HashStf::hash_key();
        let existing_cache = pre_state
            .get_historical::<User>(&hash_key, None, &witness)
            .unwrap();
        tracing::debug!(
            pre_state_root = hex::encode(pre_state_root),
            existing_cache = hex::encode(existing_cache.value()),
            "Fetched existing cache value from pre_state"
        );
        hasher.update(existing_cache.value());

        let mut proof_receipts = Vec::new();
        for blob in relevant_blobs.batch_blobs.iter_mut() {
            let data = blob.full_data();
            if !data.is_empty() {
                hasher.update(data);
            }
        }

        for blob in relevant_blobs.proof_blobs.iter_mut() {
            let raw_proof = blob.full_data();
            if raw_proof.is_empty() {
                continue;
            }
            let public_data: AggregatedProofPublicData<Self::Address, Da, Self::StateRoot> =
                match <MockZkVerifier as ZkVerifier>::verify(
                    raw_proof,
                    &MockCodeCommitment::default(),
                ) {
                    Ok(public_data) => public_data,
                    Err(err) => {
                        panic!("Error when processing proof: {err:?}");
                    }
                };

            proof_receipts.push(ProofReceipt {
                blob_hash: [0u8; 32],
                outcome: ProofOutcome::<Self::Address, Da, Self::StateRoot, _>::Valid(
                    ProofReceiptContents::AggregateProof(
                        public_data,
                        SerializedAggregatedProof {
                            raw_aggregated_proof: raw_proof.to_vec(),
                        },
                    ),
                ),
                gas_used: Default::default(),
                gas_price: Default::default(),
            });
        }

        let (state_root, change_set) =
            HashStf::save_from_hasher(hasher, pre_state, &witness, *pre_state_root);

        tracing::debug!(
            from = hex::encode(pre_state_root),
            to = hex::encode(state_root),
            "Post apply slot root hashes",
        );

        ApplySlotOutput::<InnerVm, OuterVm, Da, Self> {
            state_root,
            change_set,
            proof_receipts,
            // TODO: Add batch receipts to inspection
            batch_receipts: vec![],
            discarded_blobs: vec![],
            witness,
            rollup_height: RollupHeight::new(0),
        }
    }
}
