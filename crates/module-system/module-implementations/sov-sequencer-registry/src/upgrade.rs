//! Native tooling for preparing and verifying the DA rotation upgrade address list.

use std::collections::BTreeSet;

use anyhow::{ensure, Context};
use sov_modules_api::{DaSpec, Spec};
use sov_state::NativeStorage;

use crate::{KnownSequencer, SequencerRegistry};

/// Exports every registered DA address, including pending withdrawals, in canonical order.
///
/// Fails if prefix iteration is unavailable, an entry cannot be decoded, or an owner has
/// multiple registrations. Resolve duplicate ownership before choosing the upgrade snapshot.
pub fn export_addresses<S: Spec>(
    registry: &SequencerRegistry<S>,
    storage: &impl NativeStorage,
) -> anyhow::Result<String> {
    let entries = registry
        .known_sequencers
        .iter_raw(storage)?
        .context("storage backend does not support kernel prefix iteration")?;
    let mut owners = BTreeSet::new();
    let mut addresses = Vec::new();
    for entry in entries {
        let (address, bytes) = entry?;
        let known: KnownSequencer<S> = borsh::from_slice(&bytes)
            .with_context(|| format!("invalid sequencer registration at {address}"))?;
        ensure!(
            owners.insert(known.address),
            "rollup owner {} has multiple registered DA addresses",
            known.address
        );
        addresses.push(address.to_string());
    }
    addresses.sort();
    Ok(addresses.join(" "))
}

/// Verifies that a configured list contains exactly the snapshot's registered DA addresses.
///
/// Order and whitespace are ignored; malformed, noncanonical, duplicate, missing, and extra
/// addresses are rejected, as are duplicate owners in the snapshot.
pub fn verify_addresses<S: Spec>(
    registry: &SequencerRegistry<S>,
    storage: &impl NativeStorage,
    configured: &str,
) -> anyhow::Result<()> {
    let mut addresses = BTreeSet::new();
    for token in configured.split_whitespace() {
        let address: <S::Da as DaSpec>::Address = token
            .parse()
            .map_err(|error| anyhow::anyhow!("invalid DA address {token}: {error}"))?;
        ensure!(
            address.to_string() == token,
            "noncanonical DA address {token}"
        );
        ensure!(addresses.insert(token), "duplicate DA address {token}");
    }
    ensure!(
        addresses.into_iter().collect::<Vec<_>>().join(" ") == export_addresses(registry, storage)?,
        "configured DA addresses do not match the snapshot's complete registry"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use sov_mock_da::MockAddress;
    use sov_modules_api::capabilities::mocks::MockKernel;
    use sov_modules_api::{Address, Amount, StateCheckpoint, VisibleSlotNumber};
    use sov_state::Storage;
    use sov_test_utils::storage::SimpleStorageManager;
    use sov_test_utils::TestSpec as S;

    use super::*;
    use crate::BalanceState;

    fn with_snapshot(
        duplicate_owner: bool,
        check: impl FnOnce(&SequencerRegistry<S>, &<S as Spec>::Storage),
    ) {
        let mut manager = SimpleStorageManager::new();
        let mut state =
            StateCheckpoint::<S>::new(manager.create_storage(), &MockKernel::<S>::default());
        let mut registry = SequencerRegistry::<S>::default();
        registry
            .minimum_bond
            .set(&Amount::ZERO, &mut state)
            .unwrap();
        for index in [2, 1] {
            registry
                .known_sequencers
                .set(
                    &MockAddress::from([index; 32]),
                    &KnownSequencer {
                        address: Address::from([if duplicate_owner { 1 } else { index }; 28]),
                        balance: Amount::new(100),
                        balance_state: if index == 1 {
                            BalanceState::Active
                        } else {
                            BalanceState::PendingWithdrawal {
                                ready_at: VisibleSlotNumber::new_dangerous(10),
                            }
                        },
                    },
                    &mut state,
                )
                .unwrap();
        }
        let (_, update, _, _, storage) =
            state.materialize_update(<<S as Spec>::Storage as Storage>::PRE_GENESIS_ROOT);
        manager.commit(storage.materialize_changes(update));
        check(&registry, &manager.create_storage());
    }

    #[test]
    fn export_and_verify_include_pending_withdrawals() {
        with_snapshot(false, |registry, storage| {
            let first = MockAddress::from([1; 32]).to_string();
            let second = MockAddress::from([2; 32]).to_string();
            let extra = MockAddress::from([3; 32]).to_string();
            assert_eq!(
                export_addresses(registry, storage).unwrap(),
                format!("{first} {second}")
            );
            verify_addresses(registry, storage, &format!("{second}\n{first}")).unwrap();
            assert!(
                verify_addresses(registry, storage, first.trim_start_matches("0x"))
                    .unwrap_err()
                    .to_string()
                    .contains("noncanonical")
            );
            for invalid in [
                first.clone(),
                format!("{first} {second} {extra}"),
                format!("{first} {second} {first}"),
                "invalid-address".to_string(),
            ] {
                assert!(verify_addresses(registry, storage, &invalid).is_err());
            }
        });
    }

    #[test]
    fn export_rejects_duplicate_owners() {
        with_snapshot(true, |registry, storage| {
            assert!(export_addresses(registry, storage)
                .unwrap_err()
                .to_string()
                .contains("multiple registered DA addresses"));
        });
    }
}
