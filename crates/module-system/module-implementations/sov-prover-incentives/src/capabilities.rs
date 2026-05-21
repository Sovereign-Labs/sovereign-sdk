use std::cmp::max;

use sov_bank::{config_gas_token_id, Amount, Coins, IntoPayable};
use sov_modules_api::registration_lib::StakeRegistration;
use sov_modules_api::{
    AggregatedProofPublicData, CodeCommitmentFor, CodeCommitmentHash, CodeCommitmentTrait,
    ExecutionContext, Gas, GasSpec, GetGasPrice, InvalidProofError, SerializedAggregatedProof,
    Spec, StateReader, Storage, TxState, VersionReader, ZkVerifier, Zkvm,
};
use sov_rollup_interface::common::SlotNumber;
use sov_state::Kernel;
use thiserror::Error;

use crate::event::SlashingReason;
use crate::{AdminUpgrade, ProverIncentives};

/// Error raised while processing the aggregated proof.
#[derive(Debug, Error)]
pub enum ProcessProofError {
    #[error(
        "Error occurred when rewarding the prover. This module's account may not have enough funds. This is a bug. Error: {0}"
    )]
    TransferFailure(String),

    #[error("Prover slashed: {0}")]
    ProverSlashedNoRevert(String),

    #[error("Prover penalized: {0}")]
    ProverPenalizedNoRevert(String),

    #[error("Prover is not bonded at the time of the transaction")]
    ProverNotBonded,

    #[error("The bond is not high enough")]
    BondNotHighEnough,

    #[error("An error occurred when trying to access the state, error: {0}")]
    StateAccessorError(#[from] anyhow::Error),

    #[error("Prover incentives called with invalid operating mode")]
    InvalidOperatingMode,

    #[error(
        "Proof range [{initial_slot_number}, {final_slot_number}] starts before latest admin upgrade slot {latest_admin_upgrade_slot}"
    )]
    ProofPrecedesLatestAdminUpgrade {
        initial_slot_number: SlotNumber,
        final_slot_number: SlotNumber,
        latest_admin_upgrade_slot: SlotNumber,
    },
}

impl From<ProcessProofError> for InvalidProofError {
    fn from(error: ProcessProofError) -> Self {
        match error {
            ProcessProofError::ProverSlashedNoRevert(e) => {
                InvalidProofError::ProverSlashed(e.to_string())
            }
            ProcessProofError::ProverPenalizedNoRevert(e) => {
                InvalidProofError::ProverPenalized(e.to_string())
            }
            ProcessProofError::ProverNotBonded
            | ProcessProofError::BondNotHighEnough
            | ProcessProofError::InvalidOperatingMode
            | ProcessProofError::ProofPrecedesLatestAdminUpgrade { .. } => {
                InvalidProofError::PreconditionNotMet(error.to_string())
            }
            ProcessProofError::StateAccessorError(e) => {
                InvalidProofError::StateAccess(e.to_string())
            }
            ProcessProofError::TransferFailure(e) => InvalidProofError::RewardFailure(e),
        }
    }
}

enum Paycheck {
    Penalized,
    Rewarded(Amount),
}

/// Pre-resolved values needed by every step of proof processing.
struct ProofContext<S: Spec> {
    /// Outer code commitment to verify the proof against.
    outer_code_commitment: CodeCommitmentFor<S::OuterZkvm>,
    /// Hash of the *currently-authorized* outer commitment (always derived from
    /// the latest admin upgrade or genesis).
    effective_outer_vk_hash: CodeCommitmentHash,
    /// Currently-authorized origin state root.
    expected_origin_state_root: <S::Storage as Storage>::Root,
    /// Hash of the currently-authorized inner commitment.
    expected_inner_vkey_hash: CodeCommitmentHash,
    /// Slot of the most recent admin upgrade, if any.
    latest_admin_upgrade_slot: Option<SlotNumber>,
}

impl<S: Spec> ProofContext<S> {
    /// Returns `true` if `is_admin` is set AND `public_outputs` differ from
    /// the currently-authorized commitment/origin values — i.e. the proof is
    /// requesting an admin upgrade rather than being a normal admin proof.
    /// For non-admin proofs the same divergence is a slashing condition in
    /// `check_proof_outputs`, not an upgrade — hence the `is_admin` gate here.
    fn is_admin_upgrade(
        &self,
        public_outputs: &AggregatedProofPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        is_admin: bool,
    ) -> bool {
        is_admin
            && (self.effective_outer_vk_hash != public_outputs.outer_vk_hash
                || self.expected_inner_vkey_hash != public_outputs.inner_vkey_hash
                || self.expected_origin_state_root != public_outputs.origin_state_root)
    }
}

impl<S: Spec> ProverIncentives<S> {
    /// Try to process a zk proof, if the prover is bonded.
    #[allow(clippy::type_complexity)]
    pub fn process_proof<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: &SerializedAggregatedProof,
        prover_address: &S::Address,
        execution_context: ExecutionContext,
        state: &mut ST,
    ) -> Result<
        AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        ProcessProofError,
    > {
        if !self.should_reward_fees(state) {
            return Err(ProcessProofError::InvalidOperatingMode);
        }

        // Get the prover's old balance.
        // Revert if they aren't bonded
        let old_balance = match self
            .bonded_provers
            .get(prover_address, state)
            .map_err(Into::<anyhow::Error>::into)?
        {
            Some(balance) => balance,
            None => return Err(ProcessProofError::ProverNotBonded),
        };

        // Check that the prover has enough balance to process the proof.
        let minimum_bond = self
            .get_minimum_bond(state)
            .map_err(Into::<anyhow::Error>::into)?;
        let minimum_bond = minimum_bond.expect("The minimum bond should be set at genesis");

        if old_balance < minimum_bond {
            return Err(ProcessProofError::BondNotHighEnough);
        };

        state
            .charge_gas(<S as GasSpec>::fixed_gas_to_charge_per_proof())
            .map_err(Into::<anyhow::Error>::into)?;

        state
            .charge_linear_gas(
                <S as GasSpec>::gas_to_charge_per_proof_byte(),
                proof
                    .raw_aggregated_proof
                    .len()
                    .try_into()
                    .map_err(Into::<anyhow::Error>::into)?,
            )
            .map_err(Into::<anyhow::Error>::into)?;

        let is_admin = self
            .admin
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .as_ref()
            == Some(prover_address);

        // Extract the (untrusted) public outputs.
        let claimed_outputs = Self::claimed_public_outputs(proof);

        // Resolve every admin-state-derived value (outer commitment for
        // verification, effective hashes for slashing/upgrade checks, latest
        // upgrade slot) in one shot so downstream helpers don't re-read state.
        let ctx = self.proof_context(claimed_outputs.as_ref(), is_admin, state)?;

        Self::reject_stale_proof_before_latest_admin_upgrade(claimed_outputs.as_ref(), &ctx)?;

        // Don't return an error for invalid proofs - those are expected and shouldn't cause reverts.
        let verification_result = <<S as Spec>::OuterZkvm as Zkvm>::Verifier::verify_with_proof::<
            AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >(
            &proof.clone().to_serialized_zk_proof(),
            &ctx.outer_code_commitment,
        );

        let public_outputs = match verification_result {
            Ok(public_outputs) => public_outputs,
            Err(e) => {
                tracing::debug!(verification_error = ?e, "Slashing prover for invalid proof");

                self.slash_prover(prover_address, state)?;
                // The state won't be reverted.
                return Err(ProcessProofError::ProverSlashedNoRevert(
                    "Verification failed".to_string(),
                ));
            }
        };

        tracing::debug!(
            %public_outputs.initial_slot_number,
            %public_outputs.final_slot_number,
            "Processing aggregated proof"
        );
        #[cfg(not(feature = "native"))]
        let _ = execution_context;

        // The expected origin state root and inner VK hash were already resolved
        // into `ctx` above; this binds the inner circuit (so a prover cannot
        // verify inner proofs against an arbitrary inner VK) and enforces the
        // origin state root - both stale above when admin upgrades are accepted.
        if let Some(slashing_reason) = self
            .check_proof_outputs(&public_outputs, &ctx, is_admin, state)
            .map_err(Into::<anyhow::Error>::into)?
        {
            tracing::debug!(?slashing_reason, "Slashing prover");

            self.slash_prover(prover_address, state)?;
            // The state won't be reverted.
            return Err(ProcessProofError::ProverSlashedNoRevert(format!(
                "Invalid output {slashing_reason}"
            )));
        }

        let pending_admin_upgrade = self.check_admin_upgrade_or_slash(
            &public_outputs,
            prover_address,
            &ctx,
            is_admin,
            state,
        )?;

        match self.calculate_reward_and_remove(
            public_outputs.initial_slot_number,
            public_outputs.final_slot_number,
            state,
        )? {
            Paycheck::Penalized => {
                self.penalize_prover(old_balance, prover_address, state)?;
                // The state won't be reverted.
                Err(ProcessProofError::ProverPenalizedNoRevert(
                    "Prover penalized".to_string(),
                ))
            }
            Paycheck::Rewarded(total_reward) => {
                self.reward_prover(total_reward, prover_address, state)?;

                // Persist the upgrade only after the proof is fully accepted.
                if let Some(upgrade) = pending_admin_upgrade {
                    self.apply_admin_upgrade(public_outputs.final_slot_number, upgrade, state)?;
                }

                // Only expose proofs that were fully accepted into canonical
                // state; penalized proofs are valid but non-canonical.
                self.latest_proof_succesfully_verified
                    .set(&public_outputs, state)
                    .map_err(Into::<anyhow::Error>::into)?;

                #[cfg(feature = "native")]
                sov_metrics::track_metrics(|tracker| {
                    tracker.submit(crate::metrics::LatestVerifiedProofMetric {
                        final_slot_number: public_outputs.final_slot_number.get(),
                        execution_context: execution_context.str(),
                    });
                });

                Ok(public_outputs)
            }
        }
    }

    /// Extract the proof's claimed public outputs without cryptographic
    /// verification. Callers must treat the result as untrusted metadata.
    #[allow(clippy::type_complexity)]
    fn claimed_public_outputs(
        proof: &SerializedAggregatedProof,
    ) -> Option<AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>> {
        <<S as Spec>::OuterZkvm as Zkvm>::Verifier::extract_public_data::<
            AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >(&proof.clone().to_serialized_zk_proof())
        .ok()
    }

    /// Reject proofs whose claimed range starts before the latest admin upgrade
    /// slot. This is an explicit cutoff: once an upgrade is accepted, pre-upgrade
    /// ranges are stale and should be dropped without slashing.
    ///
    /// The slot range comes from unverified public data, so this helper is only
    /// used for a non-punitive early return. Proofs that fail extraction still
    /// fall through to normal verification and can be slashed there.
    #[allow(clippy::type_complexity)]
    fn reject_stale_proof_before_latest_admin_upgrade(
        claimed_outputs: Option<
            &AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >,
        ctx: &ProofContext<S>,
    ) -> Result<(), ProcessProofError> {
        let Some(latest_admin_upgrade_slot) = ctx.latest_admin_upgrade_slot else {
            return Ok(());
        };

        let Some(claimed_outputs) = claimed_outputs else {
            return Ok(());
        };

        if claimed_outputs.initial_slot_number >= latest_admin_upgrade_slot {
            return Ok(());
        }

        tracing::debug!(
            initial_slot_number = %claimed_outputs.initial_slot_number,
            final_slot_number = %claimed_outputs.final_slot_number,
            %latest_admin_upgrade_slot,
            "Rejecting proof whose claimed range starts before latest admin upgrade slot",
        );

        Err(ProcessProofError::ProofPrecedesLatestAdminUpgrade {
            initial_slot_number: claimed_outputs.initial_slot_number,
            final_slot_number: claimed_outputs.final_slot_number,
            latest_admin_upgrade_slot,
        })
    }

    /// For admin proofs, reconstruct the outer code commitment from the VK hash the
    /// prover claims in the proof's public outputs (extracted without cryptographic
    /// verification). Returns `None` for non-admin proofs, admin proofs whose bytes
    /// are too malformed to extract public outputs, or admin proofs whose claimed
    /// `outer_vk_hash` fails [`CodeCommitmentTrait::try_from_hash`] - the caller
    /// then falls back to the currently-authorized commitment so verification
    /// fails via the usual path rather than panicking.
    #[allow(clippy::type_complexity)]
    fn admin_claimed_outer_commitment(
        claimed_outputs: Option<
            &AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >,
        is_admin: bool,
    ) -> Option<CodeCommitmentFor<S::OuterZkvm>> {
        if !is_admin {
            return None;
        }
        let claimed = claimed_outputs?;

        <<<S as Spec>::OuterZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment::try_from_hash(
            claimed.outer_vk_hash.clone(),
        )
        .ok()
    }

    /// Resolve every admin-state-derived value needed by the rest of
    /// [`Self::process_proof`] in a single state read.
    #[allow(clippy::type_complexity)]
    fn proof_context<ST: TxState<S>>(
        &self,
        claimed_outputs: Option<
            &AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
        >,
        is_admin: bool,
        state: &mut ST,
    ) -> Result<ProofContext<S>, anyhow::Error> {
        let latest_admin_upgrade = self.latest_admin_upgrade(state)?;
        let latest_admin_upgrade_slot = latest_admin_upgrade.as_ref().map(|(slot, _)| *slot);
        let latest_admin_upgrade_value = latest_admin_upgrade.as_ref().map(|(_, upgrade)| upgrade);

        let effective_outer =
            self.effective_outer_code_commitment(latest_admin_upgrade_value, state)?;

        let effective_outer_vk_hash = effective_outer.to_hash();

        let outer_code_commitment = Self::admin_claimed_outer_commitment(claimed_outputs, is_admin)
            .unwrap_or(effective_outer);

        let expected_origin_state_root =
            self.effective_origin_state_root(latest_admin_upgrade_value, state)?;

        let expected_inner_vkey_hash = self
            .effective_inner_code_commitment(latest_admin_upgrade_value, state)?
            .to_hash();

        Ok(ProofContext {
            outer_code_commitment,
            effective_outer_vk_hash,
            expected_origin_state_root,
            expected_inner_vkey_hash,
            latest_admin_upgrade_slot,
        })
    }

    /// Returns the slot and entry of the most recent admin upgrade, if any.
    fn latest_admin_upgrade<ST: TxState<S>>(
        &self,
        state: &mut ST,
    ) -> Result<Option<(SlotNumber, AdminUpgrade<S>)>, anyhow::Error> {
        let Some(slot) = self.latest_admin_upgrade_slot.get(state)? else {
            return Ok(None);
        };
        let upgrade = self
            .admin_upgrades
            .get(&slot, state)?
            .expect("latest_admin_upgrade_slot points to a missing entry");
        Ok(Some((slot, upgrade)))
    }

    /// Returns the outer code commitment to use when verifying proofs. Falls back to
    /// chain_state's genesis value if no admin upgrade has been recorded.
    fn effective_outer_code_commitment<ST: TxState<S>>(
        &self,
        latest_admin_upgrade: Option<&AdminUpgrade<S>>,
        state: &mut ST,
    ) -> Result<CodeCommitmentFor<S::OuterZkvm>, anyhow::Error> {
        if let Some(upgrade) = latest_admin_upgrade {
            return Ok(upgrade.outer_code_commitment.clone());
        }
        Ok(self
            .chain_state
            .outer_code_commitment(state)?
            .expect("The code commitment should be set at genesis"))
    }

    /// Returns the inner code commitment to use when validating proof outputs. Falls
    /// back to chain_state's genesis value if no admin upgrade has been recorded.
    fn effective_inner_code_commitment<ST: TxState<S>>(
        &self,
        latest_admin_upgrade: Option<&AdminUpgrade<S>>,
        state: &mut ST,
    ) -> Result<CodeCommitmentFor<S::InnerZkvm>, anyhow::Error> {
        if let Some(upgrade) = latest_admin_upgrade {
            return Ok(upgrade.inner_code_commitment.clone());
        }
        Ok(self
            .chain_state
            .inner_code_commitment(state)?
            .expect("The inner code commitment should be set at genesis"))
    }

    /// Returns the origin state root to enforce on proofs. Falls back to chain_state's
    /// slot-derived genesis hash if no admin upgrade has been recorded.
    fn effective_origin_state_root<ST: TxState<S>>(
        &self,
        latest_admin_upgrade: Option<&AdminUpgrade<S>>,
        state: &mut ST,
    ) -> Result<<S::Storage as Storage>::Root, anyhow::Error> {
        if let Some(upgrade) = latest_admin_upgrade {
            return Ok(upgrade.origin_state_root.clone());
        }
        Ok(self
            .chain_state
            .get_genesis_hash(state)?
            .expect("The genesis hash should be set at genesis"))
    }

    /// Classifies an admin proof. Returns `Some(upgrade)` with the decoded
    /// commitments ready to persist if the proof requests an upgrade and all
    /// gates pass (target slot strictly newer than the latest recorded upgrade;
    /// hash lengths decode under the verifier). Returns `None` for normal admin
    /// proofs whose outputs already match current effective values. Slashes and
    /// returns `Err` for stale upgrade slots or malformed VK hashes.
    fn check_admin_upgrade_or_slash<ST: TxState<S>>(
        &mut self,
        public_outputs: &AggregatedProofPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        prover_address: &S::Address,
        ctx: &ProofContext<S>,
        is_admin: bool,
        state: &mut ST,
    ) -> Result<Option<AdminUpgrade<S>>, ProcessProofError> {
        if !ctx.is_admin_upgrade(public_outputs, is_admin) {
            return Ok(None);
        }

        let upgrade_slot = public_outputs.final_slot_number;
        if matches!(ctx.latest_admin_upgrade_slot, Some(current) if upgrade_slot <= current) {
            tracing::debug!(
                %upgrade_slot,
                latest = ?ctx.latest_admin_upgrade_slot,
                "Slashing admin: upgrade slot is not newer than latest recorded upgrade"
            );
            self.slash_prover(prover_address, state)?;
            return Err(ProcessProofError::ProverSlashedNoRevert(format!(
                "Invalid output {}",
                SlashingReason::StaleAdminUpgrade
            )));
        }

        // Decode both VK hashes now; a malformed hash is a circuit bug or attack
        // on the admin path, and slashing here keeps the panic-free
        // `try_from_hash` contract end-to-end. Returning the decoded
        // commitments lets `apply_admin_upgrade` avoid re-decoding.
        let outer_code_commitment = match <<<S as Spec>::OuterZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment::try_from_hash(
            public_outputs.outer_vk_hash.clone(),
        ) {
            Ok(commitment) => commitment,
            Err(_) => return self.slash_admin_for_invalid_vkey_hash(public_outputs, prover_address, state),
        };
        let inner_code_commitment = match <<<S as Spec>::InnerZkvm as Zkvm>::Verifier as ZkVerifier>::CodeCommitment::try_from_hash(
            public_outputs.inner_vkey_hash.clone(),
        ) {
            Ok(commitment) => commitment,
            Err(_) => return self.slash_admin_for_invalid_vkey_hash(public_outputs, prover_address, state),
        };

        Ok(Some(AdminUpgrade::<S> {
            outer_code_commitment,
            inner_code_commitment,
            origin_state_root: public_outputs.origin_state_root.clone(),
        }))
    }

    fn slash_admin_for_invalid_vkey_hash<ST: TxState<S>>(
        &mut self,
        public_outputs: &AggregatedProofPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<Option<AdminUpgrade<S>>, ProcessProofError> {
        tracing::debug!(
            outer_vk_hash_len = public_outputs.outer_vk_hash.0.len(),
            inner_vkey_hash_len = public_outputs.inner_vkey_hash.0.len(),
            expected = CodeCommitmentHash::HASH_LEN,
            "Slashing admin: upgrade proof committed to a malformed VK hash"
        );
        self.slash_prover(prover_address, state)?;
        Err(ProcessProofError::ProverSlashedNoRevert(format!(
            "Invalid output {}",
            SlashingReason::InvalidAdminUpgradeVkeyHash
        )))
    }

    /// Record a pre-validated admin upgrade as a new entry in this module's
    /// slot-keyed history. The commitments were decoded by
    /// `check_admin_upgrade_or_slash`. chain_state is left untouched; effective
    /// values are read from this module's `admin_*` maps when
    /// `latest_admin_upgrade_slot` is set.
    fn apply_admin_upgrade<ST: TxState<S>>(
        &mut self,
        slot: SlotNumber,
        upgrade: AdminUpgrade<S>,
        state: &mut ST,
    ) -> Result<(), anyhow::Error> {
        self.admin_upgrades.set(&slot, &upgrade, state)?;
        self.latest_admin_upgrade_slot.set(&slot, state)?;

        tracing::info!(
            %slot,
            outer_vk_hash = ?upgrade.outer_code_commitment.to_hash(),
            inner_vkey_hash = ?upgrade.inner_code_commitment.to_hash(),
            "Admin upgrade recorded"
        );

        Ok(())
    }

    fn slash_prover(
        &mut self,
        prover_address: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<(), anyhow::Error> {
        Ok(self.bonded_provers.delete(prover_address, state)?)
    }

    /// Computes the total reward from the aggregated state transition and rewards the prover with the unclaimed
    /// transition rewards. If all the rewards were already claimed, the prover is fined by a constant amount.
    fn calculate_reward_and_remove(
        &mut self,
        init_slot_num: SlotNumber,
        final_slot_num: SlotNumber,
        state: &mut impl TxState<S>,
    ) -> Result<Paycheck, ProcessProofError> {
        // Let's compute the total reward
        let mut total_reward = Amount::ZERO;

        let first_available_reward = self
            .last_claimed_reward
            .get(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("The last claimed reward should be set at genesis")
            .next();

        // The first reward we can claim is the maximum between the initial rollup height and the first available reward
        let first_claimed_reward = max(init_slot_num, first_available_reward);

        // Here the final rollup height is inclusive
        for slot_num in first_claimed_reward.range_inclusive(final_slot_num) {
            // `slot_at_height` must return `Some`: `check_proof_outputs`
            // verified `final_slot_num` exists, and chain_state slot heights
            // are contiguous, so every slot in this range must exist. A `None`
            // here means that invariant has broken and reward accounting would
            // silently under-pay — fail loud instead of skipping the slot.
            let transition = self
                .chain_state
                .slot_at_height(slot_num, state)
                .map_err(Into::<anyhow::Error>::into)?
                .expect("slot must exist: range bounded by verified final_slot_num");

            // SAFETY: this cannot overflow, because that would require more than the entire token supply to be spent on gas
            // *before* the prover claimed their reward, but gas fees are locked until the prover claims them.
            let curr_reward = transition.gas_used().value(transition.gas_price());
            total_reward = total_reward
                .checked_add(curr_reward)
                .expect("Gas token Overflow");
        }

        if first_claimed_reward > final_slot_num {
            // Nothing in the proof's range was unclaimed, so the cursor must
            // stay put — otherwise a stale proof would silently burn the next
            // honest slot's reward.
            Ok(Paycheck::Penalized)
        } else {
            self.last_claimed_reward
                .set(&final_slot_num, state)
                .map_err(Into::<anyhow::Error>::into)?;
            Ok(Paycheck::Rewarded(total_reward))
        }
    }

    fn penalize_prover<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        old_balance: Amount,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<(), ProcessProofError> {
        // Penalize the prover
        let fine = self
            .proving_penalty_value(state)
            .map_err(Into::<anyhow::Error>::into)?
            .expect("Should be set at genesis");

        let new_balance = old_balance
            .checked_sub(fine)
            .expect("We already checked that the balance is greater than the fine");

        self.bonded_provers
            .set(prover_address, &new_balance, state)
            .map_err(Into::<anyhow::Error>::into)?;

        Ok(())
    }

    fn reward_prover(
        &mut self,
        total_reward: Amount,
        prover_address: &S::Address,
        state: &mut impl TxState<S>,
    ) -> Result<(), ProcessProofError> {
        // We only reward a portion of the total reward - we burn some of it
        // to avoid the provers to collude to prove empty blocks.
        let reward_amount = self.burn_rate().apply(total_reward);

        let coins = Coins {
            token_id: config_gas_token_id(),
            amount: reward_amount,
        };

        self.bank
            .transfer_from(self.id.to_payable(), prover_address, coins, state)
            .map_err(|err| ProcessProofError::TransferFailure(err.to_string()))?;

        Ok(())
    }

    /// Check that the initial and final state values of the proof output are valid against the chain state module
    fn check_proof_outputs<ST: VersionReader + StateReader<Kernel>>(
        &self,
        public_outputs: &AggregatedProofPublicData<
            S::Address,
            S::Da,
            <S::Storage as Storage>::Root,
        >,
        ctx: &ProofContext<S>,
        is_admin: bool,
        state: &mut ST,
    ) -> Result<Option<SlashingReason>, ST::Error> {
        // The admin prover is allowed to submit proofs that do not match the current
        // origin state root or inner VK hash; admin upgrades replace the stored values
        // post-verification.
        if !is_admin && ctx.expected_origin_state_root != public_outputs.origin_state_root {
            return Ok(Some(SlashingReason::IncorrectGenesisHash));
        }

        if !is_admin && ctx.expected_inner_vkey_hash != public_outputs.inner_vkey_hash {
            return Ok(Some(SlashingReason::IncorrectInnerVkeyHash));
        }

        // We start with the initial state values
        let initial_slot_num = public_outputs.initial_slot_number;
        let Some(initial_slot) = self.chain_state.slot_at_height(initial_slot_num, state)? else {
            return Ok(Some(SlashingReason::InitialTransitionDoesNotExist));
        };

        if initial_slot.prev_state_root() != &public_outputs.initial_state_root {
            return Ok(Some(SlashingReason::IncorrectInitialStateRoot));
        }

        let initial_transition_hash = initial_slot.slot_hash();
        if initial_transition_hash != &public_outputs.initial_slot_hash {
            return Ok(Some(SlashingReason::IncorrectInitialSlotHash));
        }

        // Let's move on to the final state values
        let final_slot_num = public_outputs.final_slot_number;
        // Check that the final da block hash is correct
        let expected_final_transition = match self
            .chain_state
            .get_historical_transition_dangerous(final_slot_num, state)?
        {
            Some(expected_final_transition) => expected_final_transition,
            None => {
                tracing::debug!(%final_slot_num, "No historical state transition found for final slot number. Recall that state transitions are not visible until the slot *after* the transition is visible.");
                return Ok(Some(SlashingReason::FinalTransitionDoesNotExist));
            }
        };

        if expected_final_transition.slot().slot_hash() != &public_outputs.final_slot_hash {
            return Ok(Some(SlashingReason::IncorrectFinalSlotHash));
        }

        if expected_final_transition.post_state_root() != &public_outputs.final_state_root {
            return Ok(Some(SlashingReason::IncorrectFinalStateRoot));
        }

        Ok(None)
    }
}
