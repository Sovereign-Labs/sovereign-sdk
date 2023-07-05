use anyhow::{anyhow, bail, ensure, Result};
use sov_modules_api::{CallResponse, Context};
use sov_state::WorkingSet;

use super::types::{Candidate, Voter};
use super::Election;

/// Call actions supported byte the module.
#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub enum CallMessage<C: Context> {
    SetCandidates { names: Vec<String> },
    AddVoter(C::Address),
    Vote(usize),
    ClearElection,
    FreezeElection,
}

impl<C: sov_modules_api::Context> Election<C> {
    /// Sets the candidates. Must be called by the Admin.
    pub(crate) fn set_candidates(
        &self,
        candidate_names: Vec<String>,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.exit_if_frozen(working_set)?;
        self.exit_if_not_admin(context, working_set)?;
        self.exit_if_candidates_already_set(working_set)?;

        let candidates = candidate_names.into_iter().map(Candidate::new).collect();
        self.candidates.set(&candidates, working_set);
        working_set.add_event("Election: set_candidates", "Candidate was set");

        Ok(CallResponse::default())
    }

    /// Adds voter to the allow list. Must be called by the Admin.
    pub(crate) fn add_voter(
        &self,
        voter_address: C::Address,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.exit_if_frozen(working_set)?;
        self.exit_if_not_admin(context, working_set)?;
        self.exit_if_voter_already_set(&voter_address, working_set)?;

        self.allowed_voters
            .set(&voter_address, &Voter::fresh(), working_set);

        working_set.add_event(
            "Election: add_voter",
            &format!("Voter was added: {voter_address}"),
        );

        Ok(CallResponse::default())
    }

    /// Votes for a candidate. Must be called by the Voter.
    pub(crate) fn make_vote(
        &self,
        // TODO the candidates are stored in `Vec` which allows iteration, but it forces us
        // to use candidate_index instead of candidate_name here. We will change it once
        // we have iterator for `StateMap`.
        candidate_index: usize,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        working_set.add_event(
            "Election: make_vote",
            &format!("Attempt to make a from: {}", context.sender()),
        );

        let new_number_of_votes = self
            .number_of_votes
            .get(working_set)
            .unwrap_or_default()
            .checked_add(1)
            .ok_or(anyhow!("Vote count overflow"))?;

        self.number_of_votes.set(&new_number_of_votes, working_set);
        self.exit_if_frozen(working_set)?;

        let voter = self
            .allowed_voters
            .get_or_err(context.sender(), working_set)?;

        match voter {
            Voter::Voted => bail!("Voter tried voting a second time!"),
            Voter::Fresh => {
                self.allowed_voters
                    .set(context.sender(), &Voter::voted(), working_set);

                let mut candidates = self.candidates.get_or_err(working_set)?;

                // Check if a candidate exist.
                let candidate = candidates
                    .get_mut(candidate_index)
                    .ok_or(anyhow!("Candidate doesn't exist"))?;

                candidate.count = candidate
                    .count
                    .checked_add(1)
                    .ok_or(anyhow!("Vote count overflow"))?;

                self.candidates.set(&candidates, working_set);

                working_set.add_event(
                    "Election: make_vote",
                    &format!("Vote from: {} accepted", context.sender()),
                );
                Ok(CallResponse::default())
            }
        }
    }

    /// Freezes the election.
    pub(crate) fn freeze_election(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        self.exit_if_not_admin(context, working_set)?;
        self.is_frozen.set(&true, working_set);
        working_set.add_event("Election: freeze_election", "Election was frozen");
        Ok(CallResponse::default())
    }

    /// Clears the election.
    pub(crate) fn clear(&self) -> Result<CallResponse> {
        // TODO: https://github.com/Sovereign-Labs/sovereign-sdk/issues/62
        todo!()
    }

    fn exit_if_not_admin(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        let admin = self.admin.get_or_err(working_set)?;

        ensure!(
            &admin == context.sender(),
            "Only admin can trigger this action."
        );
        Ok(())
    }

    fn exit_if_frozen(&self, working_set: &mut WorkingSet<C::Storage>) -> Result<()> {
        let is_frozen = self.is_frozen.get_or_err(working_set)?;

        if is_frozen {
            bail!("Election is frozen.")
        }

        Ok(())
    }

    fn exit_if_candidates_already_set(
        &self,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        ensure!(
            self.candidates.get(working_set).is_none(),
            "Candidate already set."
        );
        Ok(())
    }

    fn exit_if_voter_already_set(
        &self,
        voter_address: &C::Address,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<()> {
        ensure!(
            self.allowed_voters
                .get(voter_address, working_set)
                .is_none(),
            "Voter already has the right to vote."
        );
        Ok(())
    }
}
