use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest};
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::{
    BlockProof, OuterZkvmHost, SerializedAggregatedProof,
};
use sov_rollup_interface::zk::SerializedZkProof;

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    wait_for_proof: bool,
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::add_hint_and_run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
        }
    }

    /// Simulates zk proof generation.
    pub fn make_proof(&self) {
        // We notify the worker thread.
        self.notification_manager.notify();
    }

    /// Create a proof for MockZkvm
    pub fn create_serialized_proof<T: Serialize>(
        is_valid: bool,
        transition: T,
    ) -> SerializedZkProof {
        let data = bincode::serialize(&transition).unwrap();
        let raw_proof = bincode::serialize(&MockProof {
            is_valid,
            pub_data: data,
        })
        .unwrap();
        SerializedZkProof { raw_proof }
    }

    fn add_hint_and_run_inner<T: Serialize>(&self, item: &T) -> anyhow::Result<Vec<u8>> {
        let pub_data = bincode::serialize(item)?;
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        Ok(bincode::serialize(&MockProof {
            is_valid: true,
            pub_data,
        })?)
    }
}

impl Default for MockZkvmHost {
    fn default() -> Self {
        Self::new()
    }
}

impl sov_rollup_interface::zk::ZkvmHost for MockZkvmHost {
    type Guest = MockZkGuest;



    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(MockCodeCommitment::default())
    }

    fn add_hint_deferred_and_run<T: Serialize>(
        &mut self,
        item: &T,
        _agg_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<SerializedZkProof> {
        self.add_hint_and_run_inner(item)
            .map(|raw_proof| SerializedZkProof { raw_proof })
    }
}

impl OuterZkvmHost for MockZkvmHost {
    fn run_proof_aggregation<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<SerializedAggregatedProof> {
        use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;

        let block_proofs_data = headers_with_block_proofs
            .iter()
            .map(|(_, bp)| bp)
            .collect::<Vec<_>>();

        let public_data = AggregatedProofPublicData::from_block_proofs(
            block_proofs_data.as_slice(),
            genesis_state_root,
        );

        self.add_hint_and_run_inner(&public_data)
            .map(|raw_aggregated_proof| SerializedAggregatedProof {
                raw_aggregated_proof,
            })
    }
}
