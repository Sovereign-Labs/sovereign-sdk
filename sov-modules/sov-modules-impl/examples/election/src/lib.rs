pub mod call;
pub mod genesis;
pub mod query;

#[cfg(test)]
mod tests;

mod types;

use sov_state::WorkingSet;
pub use types::Candidate;

use sov_modules_api::Error;
use sov_modules_macros::ModuleInfo;
use types::Voter;

#[derive(ModuleInfo)]
pub struct Election<C: sov_modules_api::Context> {
    #[address]
    pub address: C::Address,

    #[state]
    pub(crate) admin: sov_state::StateValue<C::Address>,

    #[state]
    pub(crate) is_frozen: sov_state::StateValue<bool>,

    // There are two issues here:
    // 1. We use `std::Vec` inside `StateValue` this might be inefficient because
    //       on every get, we are fetching the whole vector. We will add `StateVec` type in the future,
    //       see: https://github.com/Sovereign-Labs/sovereign/issues/33
    //
    // 2. It would be better to use `StateMap`, but it doesn't support iteration,
    //      see: https://github.com/Sovereign-Labs/sovereign/issues/61
    #[state]
    pub(crate) candidates: sov_state::StateValue<Vec<Candidate>>,

    #[state]
    pub(crate) allowed_voters: sov_state::StateMap<C::Address, Voter>,

    #[state]
    pub(crate) number_of_votes: sov_state::StateValue<u64>,
}

impl<C: sov_modules_api::Context> sov_modules_api::Module for Election<C> {
    type Context = C;

    type Config = ();

    type CallMessage = call::CallMessage<C>;

    type QueryMessage = query::QueryMessage;

    fn genesis(
        &self,
        _config: &Self::Config,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<(), Error> {
        Ok(self.init_module(working_set)?)
    }

    fn call(
        &self,
        msg: Self::CallMessage,
        context: &Self::Context,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, Error> {
        match msg {
            Self::CallMessage::SetCandidates { names } => {
                Ok(self.set_candidates(names, context, working_set)?)
            }

            Self::CallMessage::AddVoter(voter_address) => {
                Ok(self.add_voter(voter_address, context, working_set)?)
            }

            Self::CallMessage::Vote(candidate_index) => {
                Ok(self.make_vote(candidate_index, context, working_set)?)
            }

            Self::CallMessage::ClearElection => Ok(self.clear()?),

            Self::CallMessage::FreezeElection => Ok(self.freeze_election(context, working_set)?),
        }
    }

    #[cfg(feature = "native")]
    fn query(
        &self,
        msg: Self::QueryMessage,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> sov_modules_api::QueryResponse {
        match msg {
            Self::QueryMessage::GetResult => {
                let response = serde_json::to_vec(&self.results(working_set)).unwrap();
                sov_modules_api::QueryResponse { response }
            }

            query::QueryMessage::GenNbOfVotes => {
                let response = serde_json::to_vec(&self.number_of_votes(working_set)).unwrap();
                sov_modules_api::QueryResponse { response }
            }
        }
    }
}
