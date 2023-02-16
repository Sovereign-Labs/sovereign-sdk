mod call;
mod genesis;
mod query;

#[cfg(test)]
mod tests;

mod types;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;

use self::types::{Candidate, Voter};

#[derive(ModuleInfo)]
pub struct Election<C: sov_modules_api::Context> {
    #[state]
    pub(crate) admin: sov_state::StateValue<C::PublicKey, C::Storage>,

    #[state]
    pub(crate) is_frozen: sov_state::StateValue<bool, C::Storage>,

    // There are two issues here:
    // 1. We use `std::Vec` inside `StateValue` this might be inefficient because
    //       on every get, we are fetching the whole vector. We will add `StateVec` type in the future,
    //       see: https://github.com/Sovereign-Labs/sovereign/issues/33
    //
    // 2. It would be better to use `StateMap`, but it doesn't support iteration,
    //      see: https://github.com/Sovereign-Labs/sovereign/issues/61
    #[state]
    pub(crate) candidates: sov_state::StateValue<Vec<Candidate>, C::Storage>,

    #[state]
    pub(crate) allowed_voters: sov_state::StateMap<C::PublicKey, Voter, C::Storage>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Election<C> {
    type Context = C;

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage;

    fn genesis(&mut self) -> Result<(), Error> {
        Ok(self.init_module()?)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Self::Context,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            Self::CallMessage::SetCandidates { names } => Ok(self.set_candidates(names, context)?),

            Self::CallMessage::AddVoter(voter_pub_key) => {
                Ok(self.add_voter(voter_pub_key, context)?)
            }

            Self::CallMessage::Vote(candidate_index) => {
                Ok(self.make_vote(candidate_index, context)?)
            }

            Self::CallMessage::ClearElection => Ok(self.clear()?),

            Self::CallMessage::FreezeElection => Ok(self.freeze_election(context)?),
        }
    }

    #[cfg(feature = "native")]
    fn query(&self, msg: Self::QueryMessage) -> sov_modules_api::QueryResponse {
        match msg {
            Self::QueryMessage::Result => {
                let response = serde_json::to_vec(&self.results()).unwrap();
                sov_modules_api::QueryResponse { response }
            }
        }
    }
}
