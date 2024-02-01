#![deny(missing_docs)]
#![doc = include_str!("../../../../README.md")]

use std::collections::BTreeMap;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak::hashv;

/// Prefix for the PDA that will hold the root of the rollup blocks
/// and be included in the solana block header as part of the account diff
pub const PREFIX: &str = "chunk_accumulator";

/// Solana transaction size is currently capped to 1280 bytes (including frame header)
/// We're picking a size that won't push our transaction size beyond 1280
/// This can be optimized further
pub const CHUNK_SIZE: u64 = 768;

/// Represents an individual chunk of data for rollup blobs.
#[account]
#[derive(Default, Debug)]
pub struct Chunk {
    /// Unique identifier for the blob to which this chunk belongs (we currently use Merkle root for simplicity but can be a UUID or even a hash of the entire blob)
    pub digest: [u8; 32],
    /// Total number of chunks that make up the complete blob.
    pub num_chunks: u64,
    /// Position of this chunk in the sequence of chunks that form the blob.
    pub chunk_num: u64,
    /// Actual size of the chunk data, used for handling padding in the final chunk.
    pub actual_size: u64,
    /// The actual data content of this chunk.
    pub chunk_body: Vec<u8>,
}

/// Represents the root account for blocks, typically storing a Merkle root.
#[account]
#[derive(Default, Debug)]
pub struct BlocksRoot {
    /// The accumulated digest for all the merkle roots for each blob that is successfully "accumulated" during that slot
    pub digest: [u8; 32],
    /// The current slot number in Solana when this root is recorded.
    pub slot: u64,
}

/// Represents an accumulator for chunks, allowing for efficient accumulation and retrieval.
#[account]
#[derive(Default, Debug)]
pub struct ChunkAccumulator {
    /// A map where keys are blob digests and values are nested vectors of optional chunk hashes. The nested vectors represent a Merkle tree structure.
    pub chunks: BTreeMap<[u8; 32], Vec<Vec<Option<[u8; 32]>>>>,
}

impl BlocksRoot {
    /// Create a new `BlocksRoot` instance with default values.
    pub fn new() -> Self {
        BlocksRoot {
            digest: [0u8; 32],
            slot: 0,
        }
    }

    /// Updates the root of the `BlocksRoot` with given block root and slot number.
    /// If the slot number is greater than the current slot, the digest is updated.
    /// Otherwise, an accumulator function is used to merge the current digest with the given block root.
    pub fn update_root(&mut self, blockroot: &[u8; 32], slot_num: u64) {
        // slot number switched
        if slot_num > self.slot {
            self.digest = *blockroot;
            self.slot = self.slot;
        } else {
            // we're in the same solana slot
            self.digest = blocks_root_accumulator(&self.digest, blockroot);
        }
    }
}

impl ChunkAccumulator {
    /// Create a new `ChunkAccumulator` instance with an empty chunks BTreeMap.
    pub fn new() -> Self {
        ChunkAccumulator {
            chunks: BTreeMap::new(),
        }
    }
    /// Clear the chunks BTreeMap of a specific digest.
    pub fn clear_digest(&mut self, digest: &[u8; 32]) {
        self.chunks.remove(digest);
    }

    /// Accumulate a given chunk into the chunks BTreeMap.
    /// This involves updating the Merkle tree structure associated with the chunk's digest.
    pub fn accumulate(&mut self, chunk: Chunk) {
        let Chunk {
            digest,
            num_chunks,
            chunk_num,
            actual_size,
            chunk_body,
        } = chunk;

        let levels = self.chunks.entry(digest).or_insert_with(|| {
            let mut vec = Vec::new();
            let mut num = num_chunks as usize;
            while num > 1 {
                vec.push(vec![None; num]);
                num = (num + 1) / 2;
            }
            vec.push(vec![None]);
            vec
        });

        // including the actual size as part of the merkelization. first 8 bytes
        let chunk_hash = {
            let mut combined = Vec::with_capacity(8 + chunk_body.len());
            combined.extend_from_slice(&actual_size.to_le_bytes());
            combined.extend(chunk_body);
            hashv(&[&combined]).to_bytes()
        };
        levels[0][chunk_num as usize] = Some(chunk_hash);

        let mut current_level = 0;
        let mut current_index = chunk_num as usize;

        while current_level < levels.len() - 1 {
            if current_index % 2 == 1
                && levels[current_level][current_index].is_some()
                && levels[current_level][current_index - 1].is_some()
            {
                let left = levels[current_level][current_index - 1].as_ref().unwrap();
                let right = levels[current_level][current_index].as_ref().unwrap();
                let merged = hashv(&[left, right]).to_bytes();

                levels[current_level + 1][current_index / 2] = Some(merged);
                current_level += 1;
                current_index /= 2;
            } else if chunk_num == num_chunks - 1
                && current_index % 2 == 0
                && levels[current_level][current_index].is_some()
            {
                // Handle the case for unpaired nodes at the end of the level.
                levels[current_level + 1][current_index / 2] =
                    levels[current_level][current_index].clone();
                current_level += 1;
                current_index /= 2;
            } else {
                break;
            }
        }
    }

    /// Fetches the Merkle root for a specific digest.
    /// Returns `None` if the Merkle tree for the given digest is incomplete.
    pub fn get_merkle_root(&self, digest: &[u8; 32]) -> Option<[u8; 32]> {
        self.chunks
            .get(digest)
            .and_then(|levels| levels.last()?.first().cloned())
            .flatten()
    }

    /// Checks if the Merkle tree associated with a given digest is complete.
    pub fn is_complete(&self, digest: &[u8; 32]) -> bool {
        self.get_merkle_root(digest).is_some()
    }
}

/// Splits the given raw data into chunks of the specified size and returns them as a vector.
///
/// Each `Chunk` is initialized with its respective metadata such as its number (`chunk_num`),
/// actual size (`actual_size`), and body (`chunk_body`). Padding is added to the `chunk_body`
/// to ensure all chunks are of uniform size. After all chunks are created, they are merkleized
/// and their digest values are updated.
///
/// # Arguments
///
/// * `raw_data` - The data to be split into chunks.
/// * `chunk_size` - The desired size for each chunk.
///
/// # Returns
///
/// A vector containing the generated `Chunk` objects.
pub fn get_chunks(raw_data: &[u8], chunk_size: u64) -> Vec<Chunk> {
    let data_length = raw_data.len() as u64;
    let num_chunks = (data_length as f64 / chunk_size as f64).ceil() as u64;
    let mut chunks = Vec::new();
    for i in 0..num_chunks {
        let start = i * chunk_size;
        let end = std::cmp::min((start as u64) + chunk_size, data_length);
        let mut chunk_body = raw_data[(start as usize)..(end as usize)].to_vec();

        // Padding
        while (chunk_body.len() as u64) < chunk_size {
            chunk_body.push(0);
        }

        chunks.push(Chunk {
            digest: [0u8; 32],
            num_chunks,
            chunk_num: i,
            actual_size: end - start,
            chunk_body,
        });
    }

    let digest = merkleize(&chunks);
    for c in &mut chunks {
        c.digest = digest;
    }
    chunks
}

/// Computes a Merkle root from a given slice of `Chunk` objects.
///
/// Each `Chunk` is first hashed based on its `actual_size` and `chunk_body`. The resulting hashes
/// are then aggregated level by level until a single Merkle root is derived.
///
/// # Arguments
///
/// * `chunks` - The slices of `Chunk` objects to be merkleized.
///
/// # Returns
///
/// The derived Merkle root as a byte array of length 32.
pub fn merkleize(chunks: &[Chunk]) -> [u8; 32] {
    let mut current_level = chunks
        .iter()
        .map(|chunk| {
            let mut combined = Vec::with_capacity(8 + chunk.chunk_body.len());
            combined.extend_from_slice(&chunk.actual_size.to_le_bytes());
            combined.extend(&chunk.chunk_body);
            hashv(&[&combined]).0
        })
        .collect::<Vec<_>>();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();

        for pairs in current_level.chunks(2) {
            match pairs.len() {
                2 => {
                    let merged = hashv(&[&pairs[0], &pairs[1]]).0; // Destructuring again
                    next_level.push(merged);
                }
                1 => {
                    // Just copy the unpaired leaf to the next level.
                    next_level.push(pairs[0]);
                }
                // chunks() with 2 should only yield 1 or 2 items
                // should never happen, but match completion.
                // TODO: make this logic cleaner
                _ => unreachable!(),
            }
        }

        current_level = next_level;
    }
    current_level[0]
}

/// Combines the current root and a block digest using a cryptographic hash function (keccak hashv syscall)
///
/// This function essentially merges the `current_root` and `block_digest` by hashing them together.
///
/// # Arguments
///
/// * `current_root` - The current root hash.
/// * `block_digest` - The block digest to be combined with the current root.
///
/// # Returns
///
/// The resulting combined hash as a byte array of length 32.
fn blocks_root_accumulator(current_root: &[u8; 32], block_digest: &[u8; 32]) -> [u8; 32] {
    let combined = [current_root.as_ref(), block_digest.as_ref()];
    hashv(&combined).0
}
