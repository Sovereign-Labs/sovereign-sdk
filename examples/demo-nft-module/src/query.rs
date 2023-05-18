use crate::NonFungibleToken;
use sov_modules_api::Context;
use sov_state::WorkingSet;

/// This enumeration responsible for querying the nft module.
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetOwner { token_id: u64 },
}

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct OwnerResponse<C: Context> {
    pub owner: Option<C::Address>,
}

impl<C: Context> NonFungibleToken<C> {
    pub(crate) fn get_owner(
        &self,
        token_id: u64,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> OwnerResponse<C> {
        OwnerResponse {
            owner: self.owners.get(&token_id, working_set),
        }
    }
}
