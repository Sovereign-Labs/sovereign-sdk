mod jmt;
mod nomt;
use sov_state::{NodeLeaf, OrderedReadsAndWrites, SlotKey, SlotValue, StateAccesses, Storage};
use sov_test_utils::TestHasher;

use crate::state_tests::ForklessStorageManager;

#[derive(Debug)]
pub struct TestCase {
    pub rounds: Vec<StateAccesses>,
}

impl TestCase {
    pub fn single_write() -> Self {
        let key_1 = SlotKey::from_slice(b"key_1");
        let value_a = SlotValue::from("value_a");
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![(key_1, Some(value_a))],
                },
                user: Default::default(),
            }],
        }
    }

    pub fn single_write_both_namespaces() -> Self {
        let key_1 = SlotKey::from_slice(b"key_1");
        let value_a = SlotValue::from("value_a");
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![(key_1.clone(), Some(value_a.clone()))],
                },
                user: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![(key_1, Some(value_a))],
                },
            }],
        }
    }

    pub fn single_read_write_different_key() -> Self {
        let key_2 = SlotKey::from_slice(b"key_2");
        let key_3 = SlotKey::from_slice(b"key_3");
        let value_a = SlotValue::from("value_a");
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![(key_2, None)],
                    ordered_writes: vec![(key_3, Some(value_a))],
                },
                user: Default::default(),
            }],
        }
    }

    pub fn single_read_write_same_key() -> Self {
        let key_1 = SlotKey::from_slice(b"key_1");
        let value_a = SlotValue::from("value_a");
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![(key_1.clone(), None)],
                    ordered_writes: vec![(key_1, Some(value_a))],
                },
                user: Default::default(),
            }],
        }
    }

    pub fn rounds_of_same_key() -> Self {
        let key_1 = SlotKey::from_slice(b"key_1");
        let value_a = SlotValue::from("value_a");
        Self {
            rounds: vec![
                // 1. Read nothing
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![(key_1.clone(), None)],
                        ordered_writes: Vec::new(),
                    },
                    user: Default::default(),
                },
                // 2. Write something
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: Vec::new(),
                        ordered_writes: vec![(key_1.clone(), Some(value_a.clone()))],
                    },
                    user: Default::default(),
                },
                // 3. Read something
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![(
                            key_1.clone(),
                            Some(NodeLeaf::make_leaf::<TestHasher>(&value_a)),
                        )],
                        ordered_writes: Vec::new(),
                    },
                    user: Default::default(),
                },
                // 4. Write nothing
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: Vec::new(),
                        ordered_writes: vec![(key_1.clone(), None)],
                    },
                    user: Default::default(),
                },
                // 5. Read nothing again
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![(key_1.clone(), None)],
                        ordered_writes: Vec::new(),
                    },
                    user: Default::default(),
                },
            ],
        }
    }
}

pub fn run_test<SmProver, Verifier>(
    test_case: TestCase,
    mut sm_prover: SmProver,
    verifier: Verifier,
) where
    SmProver: ForklessStorageManager,
    Verifier: Storage<
        Witness = <SmProver::Storage as Storage>::Witness,
        Root = <SmProver::Storage as Storage>::Root,
    >,
{
    for state_accesses in test_case.rounds {
        let (prover_storage, prev_root) = sm_prover.create_storage_with_root();
        let (root, state_update) =
            compare_compute_state_update(prev_root, state_accesses, &prover_storage, &verifier);
        sm_prover.commit_state_update(prover_storage, state_update, root);
    }
}

pub fn compare_compute_state_update<Prover, Verifier>(
    prev_state_root: <Prover as Storage>::Root,
    state_accesses: StateAccesses,
    prover_storage: &Prover,
    zk_storage: &Verifier,
) -> (<Prover as Storage>::Root, Prover::StateUpdate)
where
    Prover: Storage,
    Verifier: Storage<Witness = Prover::Witness, Root = <Prover as Storage>::Root>,
{
    let witness = Prover::Witness::default();
    let state_accesses_for_zk = StateAccesses {
        user: OrderedReadsAndWrites {
            ordered_reads: state_accesses.user.ordered_reads.clone(),
            ordered_writes: state_accesses.user.ordered_writes.clone(),
        },
        kernel: OrderedReadsAndWrites {
            ordered_reads: state_accesses.kernel.ordered_reads.clone(),
            ordered_writes: state_accesses.kernel.ordered_writes.clone(),
        },
    };

    let (native_root, change_set) = prover_storage
        .compute_state_update(state_accesses, &witness, prev_state_root.clone())
        .expect("state update computation must succeed");

    let (zk_root, _) = zk_storage
        .compute_state_update(state_accesses_for_zk, &witness, prev_state_root)
        .expect("state update computation must succeed");

    assert_eq!(native_root.as_ref(), zk_root.as_ref());

    (native_root, change_set)
}
