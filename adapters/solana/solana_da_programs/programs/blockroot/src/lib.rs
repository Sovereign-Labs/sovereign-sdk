use std::collections::BTreeMap;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::clock::Clock;
use anchor_lang::solana_program::keccak::hashv;

declare_id!("6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E");

// Prefix for the PDA that will hold the root of the rollup blocks
// and be included in the solana block header as part of the account diff
pub const PREFIX: &str = "chunk_accumulator";

// Solana transaction size is currently capped to 1280 bytes (including frame header)
// We're picking a size that won't push our transaction size beyond 1280
// This can be optimized further
pub const CHUNK_SIZE: u64 = 768;

#[program]
pub mod blockroot {
    use super::*;

    pub fn initialize<'info>(ctx: Context<Initialize>) -> Result<()> {
        let accumulator = &mut ctx.accounts.chunk_accumulator;
        accumulator.chunks = BTreeMap::new();
        Ok(())
    }

    pub fn clear<'info>(ctx: Context<Clear>, digest: Option<[u8;32]>) -> Result<()> {
        let accumulator = &mut ctx.accounts.chunk_accumulator;
        if let Some(d) = digest {
            accumulator.chunks.remove(&d);
        } else {
            accumulator.chunks = BTreeMap::new();
        };
        Ok(())
    }

    #[allow(unused_variables)]
    pub fn process_chunk<'info>(ctx: Context<ProcessChunk>, bump:u8, chunk:Chunk) -> Result<()> {
        let chunk_accumulator = &mut ctx.accounts.chunk_accumulator;
        let blocks_root = &mut ctx.accounts.blocks_root;
        let digest = chunk.digest.clone();
        let current_slot_num = ctx.accounts.clock.slot;
        chunk_accumulator.accumulate(chunk);
        msg!("{}",chunk_accumulator.is_complete(&digest));
        if let Some(merkle_root) = chunk_accumulator.get_merkle_root(&digest) {
            msg!("accumulation blob with digest: {:?} has completed with root {:?}",digest, merkle_root);
            blocks_root.update_root(&merkle_root, current_slot_num);
            msg!("blocks root for slot {}, blob root: {:?} combined root: {:?}",current_slot_num,merkle_root, blocks_root.digest);
            chunk_accumulator.clear_digest(&digest);
        }
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(signer, zero)]
    pub chunk_accumulator:  Account<'info, ChunkAccumulator>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Clear<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(signer, mut)]
    pub chunk_accumulator:  Account<'info, ChunkAccumulator>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ProcessChunk<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(signer, mut)]
    pub chunk_accumulator:  Account<'info, ChunkAccumulator>,
    #[account(init_if_needed, payer=creator, space=8+32+8, seeds= [PREFIX.as_bytes()], bump)]
    pub blocks_root: Account<'info, BlocksRoot>,
    pub system_program: Program<'info, System>,
    pub clock: Sysvar<'info, Clock>,
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
pub struct BlocksRoot {
    pub digest: [u8; 32],
    pub slot: u64,
}

#[account]
#[derive(Default, Debug)]
pub struct ChunkAccumulator {
    pub chunks: BTreeMap<[u8; 32], Vec<Vec<Option<[u8; 32]>>>>,
}

impl BlocksRoot {
    pub fn new() -> Self {
        BlocksRoot {
            digest: [0u8;32],
            slot: 0
        }
    }

    pub fn update_root(&mut self, blockroot: &[u8;32], slot_num: u64) {
        // slot number switched
        if slot_num > self.slot {
            self.digest = *blockroot;
            self.slot = self.slot;
        } else {
            // we're in the same solana slot
            self.digest =  blocks_root_accumulator(&self.digest, blockroot);
        }
    }
}

impl ChunkAccumulator {
    pub fn new() -> Self {
        ChunkAccumulator {
            chunks: BTreeMap::new()
        }
    }

    pub fn clear_digest(&mut self, digest: &[u8;32]) {
        self.chunks.remove(digest);
    }

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
            digest: [0u8;32],
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
                    let merged = hashv(&[&pairs[0], &pairs[1]]).0;  // Destructuring again
                    next_level.push(merged);
                },
                1 => {
                    // Just copy the unpaired leaf to the next level.
                    next_level.push(pairs[0]);
                },
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

fn blocks_root_accumulator(current_root: &[u8;32], block_digest: &[u8;32]) -> [u8;32] {
    let combined = [current_root.as_ref(), block_digest.as_ref()];
    hashv(&combined).0
}

