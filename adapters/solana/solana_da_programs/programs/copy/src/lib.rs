use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak::hashv;

declare_id!("Fx9d54Cy4RAwYmwgiZf8gDaaUGkFS65diagDX2vvMRqc");

pub const PREFIX: &str = "copy_hash";

#[program]
pub mod copy {
    use super::*;

    #[allow(unused_variables)]
    pub fn copy_hash<'info>(ctx: Context<CopyHash>, bump: u8) -> Result<()> {
        let acc = &ctx.accounts.source_account;
        let lamport_ref = acc.lamports.borrow();
        let data_ref = acc.data.borrow();
        let current_slot_num = ctx.accounts.clock.slot;
        let account_hash = hashv(&[
            acc.key.as_ref(),
            &lamport_ref.to_le_bytes(),
            *data_ref,
            acc.owner.as_ref(),
            &acc.rent_epoch.to_le_bytes(),
        ]);

        let ca = &mut ctx.accounts.copy_account;
        ca.accumulate_hash(&account_hash.to_bytes(), current_slot_num);
        msg!(
            "slot: {:?}, triggering account hash: {:?}, accumulated hash: {:?}",
            current_slot_num,
            account_hash,
            ca.digest
        );
        Ok(())
    }
}

#[derive(Accounts)]
pub struct CopyHash<'info> {
    /// The signer who initiates the chunk processing.
    #[account(mut)]
    pub creator: Signer<'info>,
    /// CHECK: no writes, no deser
    pub source_account: AccountInfo<'info>,
    /// Account (PDA) for storing the Merkle root of the accumulated chunks. Initializes if not already present.
    #[account(init_if_needed, payer=creator, space=8+32+8, seeds= [PREFIX.as_bytes()], bump)]
    pub copy_account: Account<'info, CopyAccount>,

    /// The built-in Solana system program.
    pub system_program: Program<'info, System>,

    /// The Solana sysvar to fetch the current slot number.
    pub clock: Sysvar<'info, Clock>,
}

/// Represents the root account for blocks, typically storing a Merkle root.
#[account]
#[derive(Default, Debug)]
pub struct CopyAccount {
    /// The accumulated digest for all the merkle roots for each blob that is successfully "accumulated" during that slot
    pub digest: [u8; 32],
    /// The current slot number in Solana when this root is recorded.
    pub slot: u64,
}

impl CopyAccount {
    pub fn accumulate_hash(&mut self, account_hash: &[u8; 32], slot_num: u64) {
        // slot number switched
        if slot_num > self.slot {
            self.digest = *account_hash;
            self.slot = self.slot;
        } else {
            // we're in the same solana slot
            self.digest = digest_accumulator(&self.digest, account_hash);
        }
    }
}

fn digest_accumulator(current_hash: &[u8; 32], digest: &[u8; 32]) -> [u8; 32] {
    let combined = [current_hash.as_ref(), digest.as_ref()];
    hashv(&combined).0
}
