mod merkle;

use anchor_lang::prelude::*;
use merkle::compute_merkle_root;

declare_id!("6YQGvP866CHpLTdHwmLqj2Vh5q7T1GF4Kk9gS9MCta8E");

#[program]
pub mod blockroot {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        let data1= &[1u8; 10000];
        let data2 = &[2u8; 10000];
        let data_vec = vec![&data1[..],&data2[..]];
        let root = compute_merkle_root(data_vec);
        msg!("{:?}",root);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
