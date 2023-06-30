use sov_rollup_interface::zk::traits::{ValidityCondition, Zkvm};
use sov_state::WorkingSet;

use crate::ValidityConditions;

impl<Ctx: sov_modules_api::Context, Vm: Zkvm, Cond: ValidityCondition>
    ValidityConditions<Ctx, Vm, Cond>
{
    pub(crate) fn process_proof(
        &self,
        proof: &[u8],
        context: &Ctx,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<sov_modules_api::CallResponse, anyhow::Error> {
        // Get the prover's old balance.
        // Revert if they aren't bonded
        let old_balance = self
            .bonded_provers
            .get_or_err(context.sender(), working_set)?;

        // Check that the prover has enough balance to process the proof.
        let minimum_bond = self.minimum_bond.get_or_err(working_set)?;

        anyhow::ensure!(old_balance >= minimum_bond, "Prover is not bonded");
        let code_commitment = self
            .commitment_of_allowed_verifier_method
            .get_or_err(working_set)?
            .commitment;

        // Lock the prover's bond amount.
        self.bonded_provers
            .set(context.sender(), &(old_balance - minimum_bond), working_set);

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        if let Ok(_public_outputs) =
            Vm::verify(proof, &code_commitment).map_err(|e| anyhow::format_err!("{:?}", e))
        {
            // TODO: decide what the proof output is and do something with it
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/272

            // Unlock the prover's bond
            // TODO: reward the prover with newly minted tokens as appropriate based on gas fees.
            //     https://github.com/Sovereign-Labs/sovereign-sdk/issues/271
            self.bonded_provers
                .set(context.sender(), &old_balance, working_set);

            working_set.add_event(
                "processed_valid_proof",
                &format!("prover: {:?}", context.sender()),
            );
        } else {
            working_set.add_event(
                "processed_invalid_proof",
                &format!("slashed_prover: {:?}", context.sender()),
            );
        }

        Ok(CallResponse::default())
    }
}
