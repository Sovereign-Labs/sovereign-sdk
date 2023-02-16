use super::{
    types::{Candidate, Voter},
    Election,
};
use anyhow::{anyhow, bail, ensure, Result};
use borsh::BorshDeserialize;

use sov_modules_api::CallResponse;

/// Call actions supported byte the module.
#[derive(BorshDeserialize)]
pub enum CallMessage<C: sov_modules_api::Context> {
    SetCandidates { names: Vec<String> },
    AddVoter(C::PublicKey),
    Vote(usize),
    ClearElection,
    FreezeElection,
}

impl<C: sov_modules_api::Context> Election<C> {
    /// Sets the candidates. Must be called by the Admin.
    pub(crate) fn set_candidates(
        &mut self,
        candidate_names: Vec<String>,
        context: &C,
    ) -> Result<CallResponse> {
        self.exit_if_frozen()?;
        self.exit_if_not_admin(context)?;
        self.exit_if_candidates_already_set()?;

        let candidates = candidate_names.into_iter().map(Candidate::new).collect();
        self.candidates.set(candidates);

        Ok(CallResponse::default())
    }

    /// Adds voter to the allow list. Must be called by the Admin.
    pub(crate) fn add_voter(
        &mut self,
        voter_pub_key: C::PublicKey,
        context: &C,
    ) -> Result<CallResponse> {
        self.exit_if_frozen()?;
        self.exit_if_not_admin(context)?;
        self.exit_if_voter_already_set(&voter_pub_key)?;

        self.allowed_voters.set(&voter_pub_key, Voter::fresh());

        Ok(CallResponse::default())
    }

    /// Votes for a candidate. Must be called by the Voter.
    pub(crate) fn make_vote(
        &mut self,
        // TODO the candidates are stored in `Vec` which allows iteration, but it forces us
        // to use candidate_index instead of candidate_name here. We will change it once
        // we have iterator for `StateMap`.
        candidate_index: usize,
        context: &C,
    ) -> Result<CallResponse> {
        self.exit_if_frozen()?;

        let voter = self
            .allowed_voters
            .get(context.sender())
            .ok_or(anyhow!("Voter missing from the allowed list."))?;

        match voter {
            Voter::Voted => bail!("Voter tried voting a second time!"),
            Voter::Fresh => {
                self.allowed_voters.set(context.sender(), Voter::voted());

                let mut candidates = self.get_candidates()?;

                // Check if a candidate exist.
                let mut candidate = candidates
                    .get(candidate_index)
                    .ok_or(anyhow!("Candidate doesn't exist"))?
                    .clone();

                candidate.count = candidate
                    .count
                    .checked_add(1)
                    .ok_or(anyhow!("Vote count overflow"))?;

                // We already checked that the candidate_index exists.
                candidates[candidate_index] = candidate;
                self.candidates.set(candidates);

                Ok(CallResponse::default())
            }
        }
    }

    /// Freezes the election.
    pub(crate) fn freeze_election(&mut self, context: &C) -> Result<CallResponse> {
        self.exit_if_not_admin(context)?;
        self.is_frozen.set(true);
        Ok(CallResponse::default())
    }

    /// Clears the election.
    pub(crate) fn clear(&mut self) -> Result<CallResponse> {
        // see https://github.com/Sovereign-Labs/sovereign/issues/62
        todo!()
    }

    fn exit_if_not_admin(&self, context: &C) -> Result<()> {
        let admin = self.admin.get().ok_or(anyhow!("Admin is not registered"))?;

        ensure!(
            &admin == context.sender(),
            "Only admin can trigger this action."
        );
        Ok(())
    }

    fn exit_if_frozen(&self) -> Result<()> {
        let is_frozen = self
            .is_frozen
            .get()
            .ok_or(anyhow!("Frozen value is not set."))?;

        if is_frozen {
            bail!("Election is frozen.")
        }

        Ok(())
    }

    fn exit_if_candidates_already_set(&self) -> Result<()> {
        ensure!(self.candidates.get().is_none(), "Candidate already set.");
        Ok(())
    }

    fn exit_if_voter_already_set(&self, voter_pub_key: &C::PublicKey) -> Result<()> {
        ensure!(
            self.allowed_voters.get(voter_pub_key).is_none(),
            "Voter already has the right to vote."
        );
        Ok(())
    }

    fn get_candidates(&self) -> Result<Vec<Candidate>> {
        self.candidates
            .get()
            .ok_or(anyhow!("Candidate not registered."))
    }
}
