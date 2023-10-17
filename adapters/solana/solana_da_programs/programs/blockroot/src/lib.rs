pub mod da;

use std::collections::BTreeMap;

use anchor_lang::prelude::*;

use crate::da::{BlocksRoot, Chunk, ChunkAccumulator, CHUNK_SIZE, PREFIX};

declare_id!("6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E");

#[program]
pub mod blockroot {
    use super::*;

    /// Initializes the chunk accumulator with an empty set of chunks.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The context for the `Initialize` operation.
    ///
    /// # Returns
    ///
    /// * A `Result` indicating the success or failure of the initialization.
    pub fn initialize<'info>(ctx: Context<Initialize>) -> Result<()> {
        let accumulator = &mut ctx.accounts.chunk_accumulator;
        accumulator.chunks = BTreeMap::new();
        Ok(())
    }

    /// Clears a specific chunk based on its digest or all chunks if no digest is provided.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The context for the `Clear` operation.
    /// * `digest` - An optional digest of the chunk to be cleared. If `None`, all chunks are cleared.
    ///
    /// # Returns
    ///
    /// * A `Result` indicating the success or failure of the clearing operation.
    pub fn clear<'info>(ctx: Context<Clear>, digest: Option<[u8; 32]>) -> Result<()> {
        let accumulator = &mut ctx.accounts.chunk_accumulator;
        if let Some(d) = digest {
            accumulator.chunks.remove(&d);
        } else {
            accumulator.chunks = BTreeMap::new();
        };
        Ok(())
    }

    /// Processes a given chunk, checks its completion status, and if completed, updates the root.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The context for the `ProcessChunk` operation.
    /// * `bump` - Used to calculate and check the BlocksRoot PDA.
    ///            Unused in the instruction directly, but used by the
    ///            derive macro in the ProcessChunk context
    /// * `chunk` - The chunk data to be processed.
    ///
    /// # Returns
    ///
    /// * A `Result` indicating the success or failure of the chunk processing operation.
    #[allow(unused_variables)]
    pub fn process_chunk<'info>(ctx: Context<ProcessChunk>, bump: u8, chunk: Chunk) -> Result<()> {
        if chunk.chunk_body.len() > CHUNK_SIZE as usize {
            return Err(error!(ErrorCode::ChunkSizeTooLarge));
        }
        let chunk_accumulator = &mut ctx.accounts.chunk_accumulator;
        let blocks_root = &mut ctx.accounts.blocks_root;
        let digest = chunk.digest.clone();
        let current_slot_num = ctx.accounts.clock.slot;
        chunk_accumulator.accumulate(chunk);
        msg!("{}", chunk_accumulator.is_complete(&digest));
        if let Some(merkle_root) = chunk_accumulator.get_merkle_root(&digest) {
            msg!(
                "accumulation blob with digest: {:?} has completed with root {:?}",
                digest,
                merkle_root
            );
            blocks_root.update_root(&merkle_root, current_slot_num);
            msg!(
                "blocks root for slot {}, blob root: {:?} combined root: {:?}",
                current_slot_num,
                merkle_root,
                blocks_root.digest
            );
            chunk_accumulator.clear_digest(&digest);
        }
        Ok(())
    }
}

/// Represents the accounts to be utilized during the initialization of the chunk accumulator.
#[derive(Accounts)]
pub struct Initialize<'info> {
    /// The signer/payer for the transaction. Sequencer identity.
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The key pair account for storing chunk data for in-flight blobs. Must be zeroed for init. Must be a signer.
    #[account(signer, zero)]
    pub chunk_accumulator: Account<'info, ChunkAccumulator>,

    /// The built-in Solana system program.
    pub system_program: Program<'info, System>,
}

/// Represents the accounts to be utilized during the clearing operation of the chunk accumulator.
#[derive(Accounts)]
pub struct Clear<'info> {
    /// The signer who initiates the clearing operation.
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The key pair account for storing chunk data for in-flight blobs. Must be a signer.
    #[account(signer, mut)]
    pub chunk_accumulator: Account<'info, ChunkAccumulator>,

    /// The built-in Solana system program.
    pub system_program: Program<'info, System>,
}

/// Represents the accounts to be utilized during the chunk processing operation.
#[derive(Accounts)]
pub struct ProcessChunk<'info> {
    /// The signer who initiates the chunk processing.
    #[account(mut)]
    pub creator: Signer<'info>,

    /// The key pair account for storing chunk data for in-flight blobs. Must be a signer.
    #[account(signer, mut)]
    pub chunk_accumulator: Account<'info, ChunkAccumulator>,

    /// Account (PDA) for storing the Merkle root of the accumulated chunks. Initializes if not already present.
    #[account(init_if_needed, payer=creator, space=8+32+8, seeds= [PREFIX.as_bytes()], bump)]
    pub blocks_root: Account<'info, BlocksRoot>,

    /// The built-in Solana system program.
    pub system_program: Program<'info, System>,

    /// The Solana sysvar to fetch the current slot number.
    pub clock: Sysvar<'info, Clock>,
}

/// Errors for the Block root DA program
#[error_code]
pub enum ErrorCode {
    /// Each chunk needs a fixed size and it cannot be exceeded
    #[msg(format!("Max chunk size is {}",CHUNK_SIZE))]
    ChunkSizeTooLarge,
}
