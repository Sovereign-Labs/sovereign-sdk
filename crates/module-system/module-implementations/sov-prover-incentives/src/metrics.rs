use sov_modules_api::ExecutionContext;
use sov_rollup_interface::common::SlotNumber;

use crate::capabilities::ProcessProofError;

#[derive(Debug, Clone, Copy)]
pub(crate) enum AdminUpgradeMetricReason {
    Recorded,
    StaleAdminUpgrade,
    InvalidAdminUpgradeVkeyHash,
    ProverPenalized,
}

#[derive(Debug, Clone, Copy)]
enum AdminUpgradeMetricStatus {
    Success,
    Failed,
}

pub(crate) fn track_latest_verified_proof(
    final_slot_number: SlotNumber,
    execution_context: ExecutionContext,
) {
    implementation::track_latest_verified_proof(final_slot_number, execution_context);
}

pub(crate) fn track_rejected_proof(execution_context: ExecutionContext, error: &ProcessProofError) {
    implementation::track_rejected_proof(execution_context, error);
}

pub(crate) fn track_admin_upgrade_success(slot: SlotNumber) {
    implementation::track_admin_upgrade(
        slot,
        AdminUpgradeMetricStatus::Success,
        AdminUpgradeMetricReason::Recorded,
    );
}

pub(crate) fn track_admin_upgrade_failure(slot: SlotNumber, reason: AdminUpgradeMetricReason) {
    implementation::track_admin_upgrade(slot, AdminUpgradeMetricStatus::Failed, reason);
}

#[cfg(feature = "native")]
mod implementation {
    use std::io::Write;

    use sov_metrics::Metric;
    use sov_modules_api::ExecutionContext;
    use sov_rollup_interface::common::SlotNumber;

    use super::{AdminUpgradeMetricReason, AdminUpgradeMetricStatus};
    use crate::capabilities::ProcessProofError;

    #[derive(Debug)]
    struct LatestVerifiedProofMetric {
        final_slot_number: u64,
        execution_context: &'static str,
    }

    impl Metric for LatestVerifiedProofMetric {
        fn measurement_name(&self) -> &'static str {
            "sov_prover_incentives_latest_verified_proof"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{},context={} final_slot_number={}",
                self.measurement_name(),
                self.execution_context,
                self.final_slot_number
            )
        }
    }

    #[derive(Debug)]
    struct RejectedProofMetric {
        execution_context: &'static str,
        reason: &'static str,
    }

    impl Metric for RejectedProofMetric {
        fn measurement_name(&self) -> &'static str {
            "sov_prover_incentives_rejected_proof"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{},context={},reason={} count=1i",
                self.measurement_name(),
                self.execution_context,
                self.reason,
            )
        }
    }

    #[derive(Debug)]
    struct AdminUpgradeMetric {
        status: AdminUpgradeMetricStatus,
        reason: AdminUpgradeMetricReason,
        slot: u64,
    }

    impl Metric for AdminUpgradeMetric {
        fn measurement_name(&self) -> &'static str {
            "sov_prover_incentives_admin_upgrade"
        }

        fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
            write!(
                buffer,
                "{},status={},reason={} count=1i,slot={}i",
                self.measurement_name(),
                self.status.as_str(),
                self.reason.as_str(),
                self.slot,
            )
        }
    }

    pub(super) fn track_latest_verified_proof(
        final_slot_number: SlotNumber,
        execution_context: ExecutionContext,
    ) {
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(LatestVerifiedProofMetric {
                final_slot_number: final_slot_number.get(),
                execution_context: execution_context.str(),
            });
        });
    }

    pub(super) fn track_rejected_proof(
        execution_context: ExecutionContext,
        error: &ProcessProofError,
    ) {
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(RejectedProofMetric {
                execution_context: execution_context.str(),
                reason: rejected_proof_reason(error),
            });
        });
    }

    pub(super) fn track_admin_upgrade(
        slot: SlotNumber,
        status: AdminUpgradeMetricStatus,
        reason: AdminUpgradeMetricReason,
    ) {
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(AdminUpgradeMetric {
                status,
                reason,
                slot: slot.get(),
            });
        });
    }

    impl AdminUpgradeMetricStatus {
        fn as_str(&self) -> &'static str {
            match self {
                Self::Success => "success",
                Self::Failed => "failed",
            }
        }
    }

    impl AdminUpgradeMetricReason {
        fn as_str(&self) -> &'static str {
            match self {
                Self::Recorded => "recorded",
                Self::StaleAdminUpgrade => "stale_admin_upgrade",
                Self::InvalidAdminUpgradeVkeyHash => "invalid_admin_upgrade_vkey_hash",
                Self::ProverPenalized => "prover_penalized",
            }
        }
    }

    fn rejected_proof_reason(error: &ProcessProofError) -> &'static str {
        match error {
            ProcessProofError::TransferFailure(_) => "transfer_failure",
            ProcessProofError::ProverSlashedNoRevert(_) => "prover_slashed",
            ProcessProofError::ProverPenalizedNoRevert(_) => "prover_penalized",
            ProcessProofError::ProverNotBonded => "prover_not_bonded",
            ProcessProofError::BondNotHighEnough => "bond_not_high_enough",
            ProcessProofError::StateAccessorError(_) => "state_accessor_error",
            ProcessProofError::InvalidOperatingMode => "invalid_operating_mode",
            ProcessProofError::ProofPrecedesLatestAdminUpgrade { .. } => {
                "proof_precedes_latest_admin_upgrade"
            }
        }
    }
}

#[cfg(not(feature = "native"))]
mod implementation {
    use sov_modules_api::ExecutionContext;
    use sov_rollup_interface::common::SlotNumber;

    use super::{AdminUpgradeMetricReason, AdminUpgradeMetricStatus};
    use crate::capabilities::ProcessProofError;

    pub(super) fn track_latest_verified_proof(
        final_slot_number: SlotNumber,
        execution_context: ExecutionContext,
    ) {
        let _ = (final_slot_number, execution_context);
    }

    pub(super) fn track_rejected_proof(
        execution_context: ExecutionContext,
        error: &ProcessProofError,
    ) {
        let _ = (execution_context, error);
    }

    pub(super) fn track_admin_upgrade(
        slot: SlotNumber,
        status: AdminUpgradeMetricStatus,
        reason: AdminUpgradeMetricReason,
    ) {
        let _ = (slot, status, reason);
    }
}
