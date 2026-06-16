//! Test-only hooks for deterministic proof-manager crash recovery tests.

/// Environment variable used by crash-recovery tests to trigger deterministic panics.
pub(crate) const CRASH_ENV_NAME: &str = "SOV_TEST_CRASH_LOCATION";

/// Deterministic crash points used by cross-DB proof-manager recovery tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum CrashLocation {
    /// Crash after staging STF info in ProofManagerDb.
    AfterStagingProofManagerStfInfo,
    /// Crash after finalizing LedgerDb but before committing ProofManagerDb metadata.
    AfterFinalizingLedgerBeforeProofManagerCommit,
    /// Crash after persisting the proof manager receive cursor.
    AfterPersistingProofManagerNextHeight,
}

impl CrashLocation {
    fn as_str(self) -> &'static str {
        match self {
            Self::AfterStagingProofManagerStfInfo => "after_staging_proof_manager_stf_info",
            Self::AfterFinalizingLedgerBeforeProofManagerCommit => {
                "after_finalizing_ledger_before_proof_manager_commit"
            }
            Self::AfterPersistingProofManagerNextHeight => {
                "after_persisting_proof_manager_next_height"
            }
        }
    }

    /// Sets the crash location environment variable for the current process.
    pub(crate) fn set_crash_env(self) {
        std::env::set_var(CRASH_ENV_NAME, self.as_str());
    }

    /// Panics if the crash location environment variable matches this location.
    pub(crate) fn crash_if_env_set(self) {
        if std::env::var(CRASH_ENV_NAME).as_deref() == Ok(self.as_str()) {
            panic!("crashing at {self:?}");
        }
    }
}
