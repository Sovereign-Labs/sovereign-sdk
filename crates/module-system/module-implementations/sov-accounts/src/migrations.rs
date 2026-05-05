//! One-time data migration from the legacy
//! [`Accounts::accounts`](crate::Accounts::accounts) credential→account index
//! to the [`Accounts::account_owners`](crate::Accounts::account_owners)
//! authorization set.
//!
//! Offline only: requires backend prefix iteration via
//! [`NativeStorage::maybe_iter_user_values_with_prefix`], which is supported by
//! NOMT but not by JMT. Run before deploying a binary that has dropped the
//! layer-1 `accounts` reads.
//!
//! Two-phase API:
//! 1. [`collect_legacy_account_entries`] reads all entries from `storage`
//!    while it is still borrowable.
//! 2. [`apply_legacy_account_migration`] writes the entries to
//!    `account_owners` and deletes them from `accounts` via a
//!    [`sov_modules_api::StateCheckpoint`] (which consumes storage on
//!    construction, so collection has to happen first).

use anyhow::Context;
use sov_modules_api::{CredentialId, Spec, StateWriter};
use sov_state::namespaces::User;
use sov_state::NativeStorage;

use crate::{Account, AccountOwnerKey, Accounts};

/// Summary of a single legacy-accounts migration run.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MigrationReport {
    /// Number of `accounts` entries copied to `account_owners` and deleted
    /// from the source map.
    pub entries_migrated: u64,
}

/// Reads every `(credential_id, Account)` pair currently stored in the
/// legacy [`Accounts::accounts`](crate::Accounts::accounts) map.
///
/// # Errors
///
/// - The backend does not support prefix iteration (e.g. JMT). NOMT does.
/// - A stored value cannot be borsh-decoded as [`Account`].
#[allow(deprecated)]
pub fn collect_legacy_account_entries<S, Storage>(
    accounts: &Accounts<S>,
    storage: &Storage,
) -> anyhow::Result<Vec<(CredentialId, Account<S>)>>
where
    S: Spec,
    Storage: NativeStorage,
{
    let raw_entries: Vec<(CredentialId, Vec<u8>)> = accounts
        .accounts
        .iter_raw(storage)
        .context("failed to start prefix iteration over legacy accounts map")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "storage backend does not support prefix iteration; \
                 legacy-accounts migration requires NOMT"
            )
        })?
        .collect::<anyhow::Result<Vec<_>>>()?;

    raw_entries
        .into_iter()
        .map(|(credential_id, value_bytes)| {
            let account: Account<S> = borsh::from_slice(&value_bytes)
                .with_context(|| format!("failed to decode legacy Account for {credential_id}"))?;
            Ok((credential_id, account))
        })
        .collect()
}

/// Writes every entry from `entries` to
/// [`Accounts::account_owners`](crate::Accounts::account_owners) as
/// `(addr, credential_id) -> true` and deletes the corresponding
/// [`Accounts::accounts`](crate::Accounts::accounts) row.
///
/// Idempotent: passing an empty slice (or running a second time after the
/// first run cleared the source) returns `entries_migrated: 0` without error.
///
/// # Errors
///
/// - The `writer` returns an error from `set`/`delete`.
#[allow(deprecated)]
pub fn apply_legacy_account_migration<S, Writer>(
    accounts: &mut Accounts<S>,
    entries: &[(CredentialId, Account<S>)],
    writer: &mut Writer,
) -> anyhow::Result<MigrationReport>
where
    S: Spec,
    Writer: StateWriter<User>,
{
    for (credential_id, account) in entries {
        let owner_key = AccountOwnerKey::new(account.addr, *credential_id);
        accounts.account_owners.set(&owner_key, &true, writer)?;
        accounts.accounts.delete(credential_id, writer)?;
    }

    Ok(MigrationReport {
        entries_migrated: entries.len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use sov_modules_api::Spec;
    use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
    use sov_test_utils::runtime::TestRunner;
    use sov_test_utils::{generate_optimistic_runtime, TestSpec};

    use super::*;
    use crate::Accounts;

    type S = TestSpec;
    generate_optimistic_runtime!(MigrationTestRuntime <=);
    type RT = MigrationTestRuntime<S>;

    fn setup_runner() -> TestRunner<RT, S> {
        let genesis_config = HighLevelOptimisticGenesisConfig::generate();
        let genesis = GenesisConfig::from_minimal_config(genesis_config.into());
        TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
    }

    fn cred(byte: u8) -> CredentialId {
        [byte; 32].into()
    }

    fn addr(byte: u8) -> <S as Spec>::Address {
        let mut bytes = [0u8; 28];
        bytes[0] = byte;
        <S as Spec>::Address::from(bytes)
    }

    /// Migrating a populated `accounts` map writes `account_owners` entries
    /// for every pair and clears the source rows.
    #[test]
    #[allow(deprecated)]
    fn apply_migration_moves_entries_to_owners() {
        let mut runner = setup_runner();
        let cred_1 = cred(1);
        let cred_2 = cred(2);
        let addr_1 = addr(0xAA);
        let addr_2 = addr(0xBB);

        runner.__apply_to_state(|state| {
            let mut accounts = Accounts::<S>::default();

            accounts
                .accounts
                .set(&cred_1, &Account { addr: addr_1 }, state)
                .unwrap();
            accounts
                .accounts
                .set(&cred_2, &Account { addr: addr_2 }, state)
                .unwrap();

            assert_eq!(
                accounts.accounts.get(&cred_1, state).unwrap(),
                Some(Account { addr: addr_1 })
            );
            assert_eq!(
                accounts.accounts.get(&cred_2, state).unwrap(),
                Some(Account { addr: addr_2 })
            );

            let entries = vec![
                (cred_1, Account { addr: addr_1 }),
                (cred_2, Account { addr: addr_2 }),
            ];
            let report = apply_legacy_account_migration(&mut accounts, &entries, state).unwrap();
            assert_eq!(report.entries_migrated, 2);

            assert!(accounts.accounts.get(&cred_1, state).unwrap().is_none());
            assert!(accounts.accounts.get(&cred_2, state).unwrap().is_none());
            assert!(accounts
                .is_explicitly_authorized(&addr_1, &cred_1, state)
                .unwrap());
            assert!(accounts
                .is_explicitly_authorized(&addr_2, &cred_2, state)
                .unwrap());
        });
    }

    /// Empty input is a valid no-op.
    #[test]
    fn apply_migration_empty_input() {
        let mut runner = setup_runner();
        runner.__apply_to_state(|state| {
            let mut accounts = Accounts::<S>::default();
            let report = apply_legacy_account_migration(&mut accounts, &[], state).unwrap();
            assert_eq!(report.entries_migrated, 0);
        });
    }

    /// Re-applying the same entries after they've already been migrated
    /// re-writes the same `account_owners` entries (still `true`) and
    /// no-op-deletes the (already empty) source rows; the post-state is
    /// indistinguishable from a single application.
    #[test]
    #[allow(deprecated)]
    fn apply_migration_is_safe_to_repeat() {
        let mut runner = setup_runner();
        let cred_1 = cred(7);
        let addr_1 = addr(0xCC);
        let entries = vec![(cred_1, Account { addr: addr_1 })];

        runner.__apply_to_state(|state| {
            let mut accounts = Accounts::<S>::default();

            accounts
                .accounts
                .set(&cred_1, &Account { addr: addr_1 }, state)
                .unwrap();

            apply_legacy_account_migration(&mut accounts, &entries, state).unwrap();
            apply_legacy_account_migration(&mut accounts, &entries, state).unwrap();

            assert!(accounts.accounts.get(&cred_1, state).unwrap().is_none());
            assert!(accounts
                .is_explicitly_authorized(&addr_1, &cred_1, state)
                .unwrap());
        });
    }
}
