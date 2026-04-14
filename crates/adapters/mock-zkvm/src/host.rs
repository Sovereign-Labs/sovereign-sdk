use crate::notifier::NotificationManager;
use crate::{MockCodeCommitment, MockProof, MockZkGuest};
use serde::Serialize;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::BlockProof;

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
    pub fn create_serialized_proof<T: Serialize>(is_valid: bool, transition: T) -> Vec<u8> {
        let data = bincode::serialize(&transition).unwrap();
        bincode::serialize(&MockProof {
            is_valid,
            pub_data: data,
        })
        .unwrap()
    }
}

impl Default for MockZkvmHost {
    fn default() -> Self {
        Self::new()
    }
}

impl sov_rollup_interface::zk::ZkvmHost for MockZkvmHost {
    type Guest = MockZkGuest;

    type HostArgs = ();

    fn code_commitment(&self) -> anyhow::Result<<<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment>{
        Ok(MockCodeCommitment::default())
    }

    fn add_hint_and_run<T: Serialize>(&mut self, item: &T) -> anyhow::Result<Vec<u8>> {
        let pub_data = bincode::serialize(item)?;
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        Ok(bincode::serialize(&MockProof {
            is_valid: true,
            pub_data,
        })?)
    }

    fn from_args(_args: &Self::HostArgs) -> Self {
        Self::default()
    }
}

impl sov_rollup_interface::zk::aggregated_proof::OuterZkvmHost for MockZkvmHost {
    fn run_xx<Address: Serialize + Clone, Da: DaSpec, Root: Serialize + Clone>(
        &mut self,
        genesis_state_root: Root,
        headers_with_block_proofs: Vec<(Da::BlockHeader, BlockProof<Address, Da, Root>)>,
    ) -> anyhow::Result<Vec<u8>> {
        use sov_rollup_interface::zk::aggregated_proof::AggregatedProofPublicData;

        let block_proofs_data = headers_with_block_proofs
            .iter()
            .map(|(_, bp)| bp)
            .collect::<Vec<_>>();

        let public_data = AggregatedProofPublicData::from_block_proofs(
            block_proofs_data.as_slice(),
            genesis_state_root,
        );

        <Self as sov_rollup_interface::zk::ZkvmHost>::add_hint_and_run(self, &public_data)
    }
}
