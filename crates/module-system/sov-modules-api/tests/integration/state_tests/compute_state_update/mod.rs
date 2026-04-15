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

    pub fn multi_write_distinct_keys() -> Self {
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![
                        (
                            SlotKey::from_slice(b"key_1"),
                            Some(SlotValue::from("value_a")),
                        ),
                        (
                            SlotKey::from_slice(b"key_2"),
                            Some(SlotValue::from("value_b")),
                        ),
                        (
                            SlotKey::from_slice(b"key_3"),
                            Some(SlotValue::from("value_c")),
                        ),
                        (
                            SlotKey::from_slice(b"key_4"),
                            Some(SlotValue::from("value_d")),
                        ),
                        (
                            SlotKey::from_slice(b"key_5"),
                            Some(SlotValue::from("value_e")),
                        ),
                    ],
                },
                user: Default::default(),
            }],
        }
    }

    pub fn multi_write_both_namespaces_distinct() -> Self {
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![
                        (
                            SlotKey::from_slice(b"k_kernel_1"),
                            Some(SlotValue::from("v_a")),
                        ),
                        (
                            SlotKey::from_slice(b"k_kernel_2"),
                            Some(SlotValue::from("v_b")),
                        ),
                        (
                            SlotKey::from_slice(b"k_kernel_3"),
                            Some(SlotValue::from("v_c")),
                        ),
                    ],
                },
                user: OrderedReadsAndWrites {
                    ordered_reads: vec![],
                    ordered_writes: vec![
                        (
                            SlotKey::from_slice(b"k_user_1"),
                            Some(SlotValue::from("v_x")),
                        ),
                        (
                            SlotKey::from_slice(b"k_user_2"),
                            Some(SlotValue::from("v_y")),
                        ),
                        (
                            SlotKey::from_slice(b"k_user_3"),
                            Some(SlotValue::from("v_z")),
                        ),
                    ],
                },
            }],
        }
    }

    pub fn mixed_reads_and_multi_writes() -> Self {
        Self {
            rounds: vec![StateAccesses {
                kernel: OrderedReadsAndWrites {
                    ordered_reads: vec![
                        (SlotKey::from_slice(b"key_read_a"), None),
                        (SlotKey::from_slice(b"key_read_b"), None),
                        (SlotKey::from_slice(b"key_read_c"), None),
                    ],
                    ordered_writes: vec![
                        (
                            SlotKey::from_slice(b"key_write_1"),
                            Some(SlotValue::from("v_1")),
                        ),
                        (
                            SlotKey::from_slice(b"key_write_2"),
                            Some(SlotValue::from("v_2")),
                        ),
                        (
                            SlotKey::from_slice(b"key_write_3"),
                            Some(SlotValue::from("v_3")),
                        ),
                    ],
                },
                user: Default::default(),
            }],
        }
    }

    /// Minimal trigger for the per-path routing bug fixed by commit `1e0fefd0e`.
    ///
    /// Two rounds, each with **two** kernel writes and zero user writes. All four
    /// key/value pairs are captured from the first two rounds of
    /// `test_mock_proof_public_data_matches_witnesses` (genesis + block 1).
    ///
    /// Bisected from a 4-round / 1000+-write capture. The bug's trigger turned out to
    /// be surprisingly narrow:
    /// - 1 kernel write per round: bug does NOT trigger.
    /// - 2 kernel writes per round: bug triggers.
    /// - 3 kernel writes per round (using the third captured write): bug does NOT
    ///   trigger — the witness shape changes again.
    ///
    /// User writes are not needed to reproduce: the pre-fix
    /// `verify_multi_proof_update` diverges from `session.finish().root()` purely on the
    /// kernel namespace's path proofs here. User writes were also not sufficient on
    /// their own (see `HistoricalStateReader::materialize_values` — any user write
    /// requires at least one kernel write, but kernel-only rounds are allowed).
    pub fn routing_bug_minimal() -> Self {
        Self {
            rounds: vec![
                // Round 1 — seed the kernel trie with two leaves.
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![],
                        ordered_writes: vec![
                            (
                                SlotKey::from_slice(&[2, 0]),
                                Some(SlotValue::from(vec![0u8; 8])),
                            ),
                            (
                                SlotKey::from_slice(&[2, 7]),
                                Some(SlotValue::from(vec![0u8; 8])),
                            ),
                        ],
                    },
                    user: Default::default(),
                },
                // Round 2 — overwrite [2, 0] and write a longer new key [2, 6, ...].
                // The witness for this round is where the pre-fix verifier's
                // verify_multi_proof_update routing diverges from session.finish().
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![],
                        ordered_writes: vec![
                            (
                                SlotKey::from_slice(&[2, 0]),
                                Some(SlotValue::from(vec![1, 0, 0, 0, 0, 0, 0, 0])),
                            ),
                            (
                                SlotKey::from_slice(&[2, 6, 1, 0, 0, 0, 0, 0, 0, 0]),
                                Some(SlotValue::from(vec![1, 0, 0, 0, 0, 0, 0, 0])),
                            ),
                        ],
                    },
                    user: Default::default(),
                },
            ],
        }
    }

    pub fn multi_round_multi_write() -> Self {
        let key_1 = SlotKey::from_slice(b"key_1");
        let key_2 = SlotKey::from_slice(b"key_2");
        let key_3 = SlotKey::from_slice(b"key_3");
        let value_a = SlotValue::from("value_a");
        let value_b = SlotValue::from("value_b");
        let value_c = SlotValue::from("value_c");
        Self {
            rounds: vec![
                // Round 1: write three keys
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![],
                        ordered_writes: vec![
                            (key_1.clone(), Some(value_a.clone())),
                            (key_2.clone(), Some(value_b.clone())),
                            (key_3.clone(), Some(value_c.clone())),
                        ],
                    },
                    user: Default::default(),
                },
                // Round 2: read two of them, overwrite one, delete another
                StateAccesses {
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: vec![
                            (
                                key_1.clone(),
                                Some(NodeLeaf::make_leaf::<TestHasher>(&value_a)),
                            ),
                            (
                                key_3.clone(),
                                Some(NodeLeaf::make_leaf::<TestHasher>(&value_c)),
                            ),
                        ],
                        ordered_writes: vec![
                            (key_1, Some(SlotValue::from("value_a2"))),
                            (key_2, None),
                        ],
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
        .compute_state_update(state_accesses, &witness, prev_state_root.clone(), None)
        .expect("state update computation must succeed");

    let (zk_root, _) = zk_storage
        .compute_state_update(state_accesses_for_zk, &witness, prev_state_root, None)
        .expect("state update computation must succeed");

    assert_eq!(native_root.as_ref(), zk_root.as_ref());

    (native_root, change_set)
}
