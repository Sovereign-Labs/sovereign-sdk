mod merkle;

use std::collections::BTreeMap;
use anchor_lang::prelude::*;
use merkle::compute_merkle_root;
use anchor_lang::solana_program::sysvar::clock::Clock;
use anchor_lang::solana_program::keccak::hashv;

declare_id!("6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E");

const PREFIX: &str = "chunk_accumulator";

#[program]
pub mod blockroot {
    use super::*;

    pub fn initialize<'info>(ctx: Context<Initialize>) -> Result<()> {
        let accumulator = &mut ctx.accounts.chunk_accumulator;
        accumulator.chunks = BTreeMap::new();
        accumulator.chunk_age = BTreeMap::new();
        Ok(())
    }

    pub fn process_chunk<'info>(ctx: Context<ProcessChunk>, chunk:Chunk) -> Result<()> {
        let chunk_accumulator = &mut ctx.accounts.chunk_accumulator;
        let digest = chunk.digest.clone();
        chunk_accumulator.accumulate(chunk);
        msg!("{}",chunk_accumulator.is_complete(&digest));
        if chunk_accumulator.is_complete(&digest) {
            msg!("{:?}",chunk_accumulator.get_merkle_root(&digest));
        }
        Ok(())
    }
}

#[error_code]
pub enum ErrorCode {
    #[msg("Signer mismatch for accumulator account")]
    IncorrectAccumulator,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(zero)]
    pub chunk_accumulator:  Account<'info, ChunkAccumulator>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ProcessChunk<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(mut)]
    pub chunk_accumulator:  Account<'info, ChunkAccumulator>,
    pub system_program: Program<'info, System>,
}


#[account]
#[derive(Default, Debug)]
pub struct Chunk {
    pub digest: [u8; 32],
    pub num_chunks: u64,
    pub chunk_num: u64,
    pub actual_size: u64,
    pub chunk_body: Vec<u8>,
}

#[account]
#[derive(Default, Debug)]
pub struct ChunkAccumulator {
    pub chunks: BTreeMap<[u8; 32], Vec<Vec<Option<[u8; 32]>>>>,
    pub chunk_age: BTreeMap<[u8;32], u64>
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
            ..
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

        // Store the hash of the chunk body
        let chunk_hash = hashv(&[&chunk_body]).to_bytes();
        levels[0][chunk_num as usize] = Some(chunk_hash);

        let mut current_level = 0;
        let mut current_index = chunk_num as usize;

        while current_level < levels.len() - 1 {
            if current_index % 2 == 1 && levels[current_level][current_index].is_some() && levels[current_level][current_index - 1].is_some() {

                let left = levels[current_level][current_index - 1].as_ref().unwrap();
                let right = levels[current_level][current_index].as_ref().unwrap();
                let merged = hashv(&[left, right]).to_bytes();
                levels[current_level + 1][current_index / 2] = Some(merged);

                current_level += 1;
                current_index /= 2;
            } else if chunk_num == num_chunks - 1 && current_index % 2 == 0 && levels[current_level][current_index].is_some() {
                // Handle the case for unpaired nodes at the end of the level.

                levels[current_level + 1][current_index / 2] = levels[current_level][current_index].clone();

                current_level += 1;
                current_index /= 2;
            } else {
                break;
            }
        }

    }


    // If the digest's tree is complete, this will return the Merkle root against that digest
    pub fn get_merkle_root(&self, digest: &[u8; 32]) -> Option<[u8; 32]> {
        self.chunks.get(digest).and_then(|levels| levels.last()?.first().cloned()).flatten()
    }

    // Check if the Merkle tree for a specific digest is complete
    pub fn is_complete(&self, digest: &[u8; 32]) -> bool {
        self.get_merkle_root(digest).is_some()
    }
}

pub fn get_chunks(raw_data: &[u8], chunk_size: usize) -> Vec<Chunk> {
    let data_length = raw_data.len();
    let num_chunks = (data_length as f64 / chunk_size as f64).ceil() as u64;
    let mut chunks = Vec::new();
    for i in 0..num_chunks {
        let start = i as usize * chunk_size;
        let end = std::cmp::min(start + chunk_size, data_length);
        let mut chunk_body = raw_data[start..end].to_vec();

        // Padding
        while chunk_body.len() < chunk_size {
            chunk_body.push(0);
        }

        chunks.push(Chunk {
            digest: [0u8;32],
            num_chunks,
            chunk_num: i,
            actual_size: (end - start) as u64,
            chunk_body,
        });
    }

    let chunk_bodies: Vec<Vec<u8>> = chunks.iter().map(|x| x.chunk_body.clone()).collect();
    let digest = merkleize(&chunk_bodies);
    for c in &mut chunks {
        c.digest = digest;
    }
    chunks
}

pub fn merkleize(chunk_bodies: &[Vec<u8>]) -> [u8; 32] {
    let mut current_level = chunk_bodies
        .iter()
        .map(|body| hashv(&[body]).0)  // Destructuring to get the [u8; 32]
        .collect::<Vec<_>>();

    while current_level.len() > 1 {
        let mut next_level = Vec::new();

        for pairs in current_level.chunks(2) {
            match pairs.len() {
                2 => {
                    let merged = hashv(&[&pairs[0], &pairs[1]]).0;  // Destructuring again
                    next_level.push(merged);
                },
                1 => {
                    // Just copy the unpaired leaf to the next level.
                    next_level.push(pairs[0]);
                },
                _ => unreachable!(), // chunks() with 2 should only yield 1 or 2 items
            }
        }

        current_level = next_level;
    }

    current_level[0]
}

