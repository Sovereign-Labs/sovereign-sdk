#![allow(missing_docs)]
//! Helpers for property testing.

use std::collections::{HashMap, VecDeque};

use proptest::arbitrary::Arbitrary;
use proptest::prelude::*;
use sov_mock_da::{MockBlockHeader, MockHash};
use sov_rollup_interface::da::BlockHeaderTrait;

/// A description of the test case, so we can implement arbitrary for it.
#[derive(Clone, Debug)]
pub struct ForkDescription {
    /// Relative to parent.
    pub start_height: u64,
    /// How many blocks this fork has.
    pub length: u8,
    /// All forks that start from given Fork.
    /// Note: `start_height` of the child's fork should be below the length of the parent.
    pub child_forks: Vec<ForkDescription>,
}

impl Arbitrary for ForkDescription {
    type Parameters = ();

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        let leaf = (1u64..=10u64, 5u8..=30).prop_map(|(start_height, length)| ForkDescription {
            start_height,
            length,
            child_forks: Vec::new(),
        });

        leaf.prop_recursive(
            8,   // 8 levels deep
            256, // Shoot for maximum size of 256 nodes
            10,  // Each collection can have up to 10 items
            |inner| {
                // TODO: Parametrize it
                (
                    1u64..=10u64,
                    3u8..15,
                    proptest::collection::vec(inner, 0..4),
                )
                    .prop_map(|(start_height, length, child_forks)| {
                        let child_forks = child_forks
                            .into_iter()
                            .map(|mut child_fork| {
                                // child should start before parent ends
                                child_fork.start_height =
                                    std::cmp::min((length - 1) as u64, child_fork.start_height);
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
struct ForkMapBlockInfo {
    header: MockBlockHeader,
    forks: Vec<MockHash>,
    parent: Option<MockHash>,
}

/// It is "materialized" [`ForkDescription`] with all blocks pointing to each other.
/// Can be used in actual tests, guaranteeing the correct blockchain.
#[derive(Clone, Default, Debug)]
pub struct ForkMap {
    blocks: HashMap<MockHash, ForkMapBlockInfo>,
}

impl ForkMap {
    pub fn get_start(&self) -> Option<MockHash> {
        // Start from any, all should traverse back to original start
        let mut start = self.blocks.keys().next();
        while let Some(block_hash) = start {
            let block = self.blocks.get(block_hash).unwrap();
            if block.parent.is_none() {
                break;
            }
            start = block.parent.as_ref();
        }

        start.cloned()
    }

    pub fn get_block_header(&self, block_hash: &MockHash) -> Option<&MockBlockHeader> {
        self.blocks.get(block_hash).map(|i| &i.header)
    }

    pub fn blocks_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn get_child_hashes(&self, block_hash: &MockHash) -> Vec<MockHash> {
        self.blocks
            .get(block_hash)
            .cloned()
            .map(|x| x.forks)
            .unwrap_or_default()
    }

    // Builds a whole chain of block headers, up to given hash (inclusive)
    pub fn get_chain_up_to(&self, up_to: MockBlockHeader) -> Vec<MockBlockHeader> {
        let mut chain = Vec::with_capacity(up_to.height() as usize);

        let mut current_hash = up_to.hash();
        while let Some(block_info) = self.blocks.get(&current_hash) {
            chain.push(block_info.header.clone());
            if let Some(parent_hash) = block_info.parent.as_ref() {
                current_hash = *parent_hash;
            } else {
                break;
            }
        }

        chain.reverse();
        chain
    }

    fn insert_to_parent(&mut self, prev_hash: MockHash, current_hash: MockHash) {
        if let std::collections::hash_map::Entry::Vacant(_) = self
            .blocks
            .entry(prev_hash)
            .and_modify(|parent_block_info| parent_block_info.forks.push(current_hash))
        {
            panic!("Parent should be always inserted before updating its forks");
        }
    }
}

impl From<ForkDescription> for ForkMap {
    fn from(fork: ForkDescription) -> Self {
        let mut chain_map = ForkMap::default();
        let mut forks_to_process = VecDeque::new();
        // starting height, parent_fork_id and original fork.
        forks_to_process.push_back((0u64, 0u64, fork));

        // For debugging purposes.
        let mut nodes_count_1: usize = 0;

        // Flat ids of all forks
        let mut next_fork_id = 0;

        while let Some((height, parent_fork_id, fork)) = forks_to_process.pop_front() {
            next_fork_id += 1;
            let fork_id = next_fork_id;
            nodes_count_1 += fork.length as usize;
            let fork_total_height = height + fork.start_height;
            let prev_hash =
                get_block_hash(parent_fork_id, fork_total_height.checked_sub(1).unwrap());
            let parent = if height > 0 { Some(prev_hash) } else { None };
            let current_hash = get_block_hash(fork_id, fork_total_height);
            let fork_start = MockBlockHeader {
                prev_hash,
                hash: current_hash,
                height: fork_total_height,
                ..Default::default()
            };
            let fork_start = ForkMapBlockInfo {
                header: fork_start,
                forks: Vec::new(),
                parent,
            };
            chain_map.blocks.insert(current_hash, fork_start);
            if height > 0 {
                chain_map.insert_to_parent(prev_hash, current_hash);
            }
            for child_rel_height in 1..fork.length {
                let child_total_height = fork_total_height + child_rel_height as u64;
                let prev_hash = get_block_hash(fork_id, child_total_height - 1);
                let current_hash = get_block_hash(fork_id, child_total_height);
                chain_map.insert_to_parent(prev_hash, current_hash);
                let block_header = MockBlockHeader {
                    prev_hash,
                    hash: current_hash,
                    height: child_total_height,
                    ..Default::default()
                };
                let block_info = ForkMapBlockInfo {
                    header: block_header,
                    forks: Vec::new(),
                    parent: Some(prev_hash),
                };
                if let Some(b) = chain_map.blocks.insert(current_hash, block_info) {
                    panic!("duplicate block on fork_id={fork_id} on {current_hash} from={b:?}");
                };
            }
            for child_fork in fork.child_forks {
                forks_to_process.push_back((fork_total_height, fork_id, child_fork));
            }
        }
        assert_eq!(nodes_count_1, chain_map.blocks.len());
        chain_map
    }
}

// Gets deterministic MockHash for given height and `fork_id`.
pub fn get_block_hash(fork_id: u64, height: u64) -> MockHash {
    let mut raw_hash: [u8; 32] = [0; 32];
    let fork_id_bytes = fork_id.to_be_bytes();
    raw_hash[..fork_id_bytes.len()].copy_from_slice(&fork_id_bytes);
    let height_bytes = height.to_be_bytes();
    raw_hash[fork_id_bytes.len()..fork_id_bytes.len() + height_bytes.len()]
        .copy_from_slice(&height_bytes);
    MockHash(raw_hash)
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
