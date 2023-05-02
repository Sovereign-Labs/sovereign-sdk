use super::{types::Candidate, Election};
use borsh::{BorshDeserialize, BorshSerialize};
use sov_state::WorkingSet;

/// Queries supported by the module.
#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetResult,
    GenNbOfVotes,
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub enum GetResultResponse {
    Result(Option<Candidate>),
    Err(String),
}

#[cfg_attr(feature = "native", derive(serde::Deserialize, serde::Serialize))]
#[derive(Debug, Eq, PartialEq)]
pub enum GetNbOfVotesResponse {
    Result(u64),
}

impl<C: sov_modules_api::Context> Election<C> {
    pub fn results(&self, working_set: &mut WorkingSet<C::Storage>) -> GetResultResponse {
        let is_frozen = self.is_frozen.get(working_set).unwrap_or_default();

        if is_frozen {
            let candidates = self.candidates.get(working_set).unwrap_or(Vec::default());

            // In case of tie, returns the candidate with the higher index in the vec, it is ok for the example.
            let candidate = candidates
                .into_iter()
                .max_by(|c1, c2| c1.count.cmp(&c2.count));

            GetResultResponse::Result(candidate)
        } else {
            GetResultResponse::Err("Election is not frozen".to_owned())
        }
    }

    pub fn number_of_votes(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> GetNbOfVotesResponse {
        let number_of_votes = self.number_of_votes.get(working_set).unwrap_or_default();
        GetNbOfVotesResponse::Result(number_of_votes)
    }
}
