use super::{types::Candidate, Election};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_state::WorkingSet;

/// Queries supported by the module.
#[derive(BorshDeserialize, BorshSerialize, Debug, PartialEq)]
pub enum QueryMessage {
    GetResult,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum Response {
    Result(Option<Candidate>),
    Err(String),
}

impl<C: sov_modules_api::Context> Election<C> {
    pub fn results(&self, working_set: &mut WorkingSet<C::Storage>) -> Response {
        let is_frozen = self.is_frozen.get(working_set).unwrap_or_default();

        if is_frozen {
            let candidates = self.candidates.get(working_set).unwrap_or(Vec::default());

            // In case of tie, returns the candidate with the higher index in the vec, it is ok for the example.
            let candidate = candidates
                .into_iter()
                .max_by(|c1, c2| c1.count.cmp(&c2.count));

            Response::Result(candidate)
        } else {
            Response::Err("Election is not frozen".to_owned())
        }
    }
}
