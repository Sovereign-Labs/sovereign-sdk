use sov_modules_macros::rpc_gen;
use sov_state::WorkingSet;

use super::types::Candidate;
use super::Election;

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum GetResultResponse {
    Result(Option<Candidate>),
    Err(String),
}

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum GetNbOfVotesResponse {
    Result(u64),
}

#[rpc_gen(client, server, namespace = "election")]
impl<C: sov_modules_api::Context> Election<C> {
    #[rpc_method(name = "results")]
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

    #[rpc_method(name = "numberOfVotes")]
    pub fn number_of_votes(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> GetNbOfVotesResponse {
        let number_of_votes = self.number_of_votes.get(working_set).unwrap_or_default();
        GetNbOfVotesResponse::Result(number_of_votes)
    }
}
