use crate::NonFungibleToken;
use anyhow::{bail, Result};
use sov_modules_api::{CallResponse, Context};
use sov_state::WorkingSet;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<C: Context> {
    Mint {
        /// The id of new token. Caller is an owner
        id: u64,
    },
    Transfer {
        /// The address to which the token will be transferred.
        to: C::Address,
        /// The token id to transfer
        id: u64,
    },
    Burn {
        id: u64,
    },
}

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn mint(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        if self.owners.get(&id, working_set).is_some() {
            bail!("Token with id {} already exists", id);
        }

        self.owners.set(&id, context.sender().clone(), working_set);
        Ok(CallResponse::default())
    }

    pub(crate) fn transfer(
        &self,
        id: u64,
        to: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
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
        self.owners.set(&id, to, working_set);
        Ok(CallResponse::default())
    }

    pub(crate) fn burn(
        &self,
        id: u64,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
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
        Ok(CallResponse::default())
    }
}
