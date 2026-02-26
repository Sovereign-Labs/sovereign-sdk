#![allow(missing_docs)]
//! Helpers for property testing.

pub use crate::test_utils::{get_block_hash, ForkDescription, ForkMap};

use proptest::arbitrary::Arbitrary;
use proptest::prelude::*;

impl Arbitrary for ForkDescription {
    type Parameters = ForkGenParams;

    fn arbitrary_with(params: Self::Parameters) -> Self::Strategy {
        // lengths must be >= 1 to satisfy invariants in ForkMap materialization
        assert!(*params.leaf_length.start() >= 1);
        assert!(params.inner_length.start >= 1);

        let leaf = (params.leaf_start_height.clone(), params.leaf_length.clone()).prop_map(
            |(start_height, length)| ForkDescription {
                start_height,
                length,
                child_forks: Vec::new(),
            },
        );

        leaf.prop_recursive(
            params.max_depth,
            params.max_nodes,
            params.max_items_per_collection,
            move |inner| {
                (
                    params.inner_start_height.clone(),
                    params.inner_length.clone(),
                    proptest::collection::vec(inner, params.inner_children.clone()),
                )
                    .prop_map(|(start_height, length, child_forks)| {
                        let child_forks = child_forks
                            .into_iter()
                            .map(|mut child_fork| {
                                // child should start before parent ends
                                let max_child_start = length.saturating_sub(1) as u64;
                                child_fork.start_height =
                                    std::cmp::min(max_child_start, child_fork.start_height);
                                child_fork
                            })
                            .collect();
                        ForkDescription {
                            start_height,
                            length,
                            child_forks,
                        }
                    })
            },
        )
        .boxed()
    }

    type Strategy = BoxedStrategy<Self>;
}

#[derive(Clone, Debug)]
pub struct ForkGenParams {
    pub leaf_start_height: std::ops::RangeInclusive<u64>,
    pub leaf_length: std::ops::RangeInclusive<u8>,
    pub inner_start_height: std::ops::RangeInclusive<u64>,
    // Use exclusive upper-bound to mirror the original 3u8..15
    pub inner_length: std::ops::Range<u8>,
    pub inner_children: std::ops::Range<usize>,
    pub max_depth: u32,
    pub max_nodes: u32,
    pub max_items_per_collection: u32,
}

impl Default for ForkGenParams {
    fn default() -> Self {
        Self {
            leaf_start_height: 1u64..=10u64,
            leaf_length: 5u8..=30u8,
            inner_start_height: 1u64..=10u64,
            // 3..15 (exclusive) matches original behavior
            inner_length: 3u8..15u8,
            inner_children: 0..4,
            // Matches original prop_recursive configuration
            max_depth: 8,
            max_nodes: 256,
            max_items_per_collection: 10,
        }
    }
}

#[test]
fn block_hash_check() {
    let hash1 = get_block_hash(0, 0);
    let hash1a = get_block_hash(0, 0);
    let hash2 = get_block_hash(1, 0);
    let hash3 = get_block_hash(1, 1);

    assert_eq!(hash1, hash1a);
    assert_ne!(hash1, hash2);
    assert_ne!(hash2, hash3);
}
