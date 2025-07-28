use std::collections::VecDeque;

use serde::Serialize;

use crate::notifier::NotificationManager;
use crate::{Empty, Inner, MockCodeCommitment, MockZkGuest, Proof};

/// A mock implementing the zkVM trait.
#[derive(Clone)]
pub struct MockZkvmHost {
    notification_manager: NotificationManager,
    committed_data: VecDeque<Vec<u8>>,
    wait_for_proof: bool,
}

impl MockZkvmHost {
    /// Creates a new MockZkvm.
    pub fn new() -> Self {
        Self {
            wait_for_proof: true,
            notification_manager: Default::default(),
            committed_data: Default::default(),
        }
    }

    /// Creates a new MockZkvm, the `ZkvmHost::run` will return immediately.
    pub fn new_non_blocking() -> Self {
        Self {
            wait_for_proof: false,
            notification_manager: Default::default(),
            committed_data: Default::default(),
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
        bincode::serialize(&Proof::<(), Inner>::PublicData(Inner {
            is_valid,
            pub_data: data,
        }))
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

    fn add_hint<T: Serialize>(&mut self, item: T) {
        let data = bincode::serialize(&item).unwrap();
        self.committed_data.push_back(data);
    }

    fn code_commitment(&self) -> <<Self::Guest as sov_rollup_interface::zk::ZkvmGuest>::Verifier as sov_rollup_interface::zk::ZkVerifier>::CodeCommitment{
        MockCodeCommitment::default()
    }

    fn run(&mut self, _with_proof: bool) -> anyhow::Result<Vec<u8>> {
        if self.wait_for_proof {
            self.notification_manager.wait();
        }
        let data = self.committed_data.pop_front().unwrap_or_default();
        Ok(bincode::serialize(&sov_rollup_interface::zk::Proof::<
            Empty,
            _,
        >::PublicData(Inner {
            is_valid: true,
            pub_data: data,
        }))?)
    }

    fn from_args(_args: &Self::HostArgs) -> Self {
        Self::default()
    }
}
