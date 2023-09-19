use anyhow::{bail, Result};
use sov_modules_api::{CallResponse, Context, WorkingSet};

use crate::NonFungibleToken;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
/// A transaction handled by the NFT module. Mints, Transfers, or Burns an NFT by id
pub enum CallMessage<C: Context> {
    /// Mint a new token
    Mint {
        /// The id of new token. Caller is an owner
        id: u64,
    },
    /// Transfer existing token to the new owner
    Transfer {
        /// The address to which the token will be transferred.
        to: C::Address,
        /// The token id to transfer
        id: u64,
    },
    /// Burn existing token
    Burn {
        /// The token id to burn
        id: u64,
    },
}

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn mint(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse> {
        if self.owners.get(&id, working_set).is_some() {
            bail!("Token with id {} already exists", id);
        }

        self.owners.set(&id, context.sender(), working_set);

        working_set.add_event("NFT mint", &format!("A token with id {id} was minted"));
        Ok(CallResponse::default())
    }

    pub(crate) fn transfer(
        &self,
        id: u64,
        to: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            bail!("Only token owner can transfer token");
        }
        self.owners.set(&id, &to, working_set);
        working_set.add_event(
            "NFT transfer",
            &format!("A token with id {id} was transferred"),
        );
        Ok(CallResponse::default())
    }

    pub(crate) fn burn(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C>,
    ) -> Result<CallResponse> {
        let token_owner = match self.owners.get(&id, working_set) {
            None => {
                bail!("Token with id {} does not exist", id);
            }
            Some(owner) => owner,
        };
        if &token_owner != context.sender() {
            bail!("Only token owner can burn token");
        }
        self.owners.remove(&id, working_set);

        working_set.add_event("NFT burn", &format!("A token with id {id} was burned"));
        Ok(CallResponse::default())
    }
}
