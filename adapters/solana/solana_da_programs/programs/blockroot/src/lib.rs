mod merkle;

use std::collections::BTreeMap;
use anchor_lang::prelude::*;
use merkle::compute_merkle_root;
use anchor_lang::solana_program::sysvar::clock::Clock;
use anchor_lang::solana_program::keccak::hashv;

declare_id!("6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E");

#[program]
pub mod blockroot {
    use super::*;

    pub fn initialize<'info>(ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(init_if_needed,payer=creator, space=10000000)]
    pub chunk_accumulator: Account<'info, ChunkAccumulator>,
    pub clock: Sysvar<'info, Clock>,
    pub system_program: Program<'info, System>,
}


#[account]
#[derive(Default, Debug)]
pub struct Chunk {
    pub digest: [u8; 32],
    pub num_chunks: u64,
    pub chunk_num: u64,
    pub chunk_body: Vec<u8>,
}

#[account]
#[derive(Default, Debug)]
pub struct ChunkAccumulator {
    chunks: BTreeMap<[u8; 32], Vec<Vec<Option<[u8; 32]>>>>,
    chunk_age: BTreeMap<[u8;32], u64>
}

impl ChunkAccumulator {
    pub fn new() -> Self {
        ChunkAccumulator {
            chunks: BTreeMap::new(),
            chunk_age: BTreeMap::new()
        }
    }

    pub fn accumulate(&mut self, chunk: Chunk) {
        let Chunk {
            digest,
            num_chunks,
            chunk_num,
            chunk_body,
        } = chunk;

        let levels = self.chunks.entry(digest).or_insert_with(|| {
            let mut vec = Vec::new();
            let mut num = num_chunks as usize;
            while num > 0 {
                vec.push(vec![None; num]);
                num = (num + 1) / 2;
            }
            vec
        });

        // Store the hash of the chunk body
        let chunk_hash = hashv(&[&chunk_body]).to_bytes();
        levels[0][chunk_num as usize] = Some(chunk_hash);

        // Merge and promote as necessary
        for i in 0..levels.len() - 1 {
            for j in (0..levels[i].len()).step_by(2) {
                if j + 1 < levels[i].len() && levels[i][j].is_some() && levels[i][j + 1].is_some() {
                    let merged = hashv(&[levels[i][j].as_ref().unwrap(), levels[i][j + 1].as_ref().unwrap()]).to_bytes();
                    levels[i + 1][j / 2] = Some(merged);
                }
            }
        }
    }

    // If the digest's tree is complete, this will return the Merkle root.
    pub fn get_merkle_root(&self, digest: [u8; 32]) -> Option<[u8; 32]> {
        self.chunks.get(&digest).and_then(|levels| levels.last()?.first().cloned()).flatten()
    }

    // Check if the Merkle tree for a specific digest is complete
    pub fn is_complete(&self, digest: [u8; 32]) -> bool {
        self.get_merkle_root(digest).is_some()
    }
}

