//! Concurrency tests for the live-value cache of [`FlatStateDb`].
//!
//! A commit writes the archival DB, waits in `before_live` for the other stores, and only then
//! takes the cache write lock to write the live DB and update the cache. These tests check that
//! readers racing with a commit never observe a value that does not belong to their version.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rockbound::versioned_db::VersionedDeltaReader;
use sov_db_types::{SlotKey, SlotValue};
use sov_rollup_interface::common::SlotNumber;

use super::{DbCache, FlatStateDb, StateChanges};
use crate::historical_state::HistoricalStateReader;
use crate::namespaces::UserNamespace;
use crate::schema::namespace::NomtStateValues;
use crate::test_utils::{CommitFaultInjectionLocation, SLEEP_ON_COMMIT_MS_ENV_NAME};

const KEYS: u64 = 512;

fn key(idx: u64) -> SlotKey {
    SlotKey::from_slice(format!("key_{idx:04}").as_bytes())
}

/// What version `version` writes for key `idx`: `None` if the key is not touched,
/// `Some(None)` if it is deleted, `Some(Some(value))` if it is set.
fn write_at(idx: u64, version: u64) -> Option<Option<SlotValue>> {
    if version == 0 {
        return Some(Some(value(idx, version)));
    }
    if (idx + version).is_multiple_of(3) {
        return None;
    }
    if (idx * 7 + version).is_multiple_of(11) {
        return Some(None);
    }
    Some(Some(value(idx, version)))
}

fn value(idx: u64, version: u64) -> SlotValue {
    // Variable length, so cache entries have different weights.
    let padding = "x".repeat(((idx * 13 + version) % 40) as usize);
    SlotValue::from(format!("key_{idx}_version_{version}_{padding}").as_str())
}

/// Expected value of every key at every version: `expected[version][idx]`.
fn expected_values(versions: u64) -> Vec<Vec<Option<SlotValue>>> {
    let mut expected: Vec<Vec<Option<SlotValue>>> = Vec::new();
    for version in 0..=versions {
        let row = (0..KEYS)
            .map(|idx| match write_at(idx, version) {
                Some(written) => written,
                None => expected[version as usize - 1][idx as usize].clone(),
            })
            .collect();
        expected.push(row);
    }
    expected
}

fn change_set(version: u64) -> StateChanges {
    let user: Vec<(SlotKey, Option<SlotValue>)> = (0..KEYS)
        .filter_map(|idx| write_at(idx, version).map(|written| (key(idx), written)))
        .collect();
    // Every version must also update the kernel namespace.
    let kernel = vec![(SlotKey::from_slice(b"kernel_key"), Some(value(0, version)))];
    HistoricalStateReader::materialize_values(
        user,
        kernel,
        [version as u8; 64].to_vec(),
        SlotNumber::new(version),
    )
    .unwrap()
}

fn live_version(flat_db: &FlatStateDb) -> u64 {
    flat_db
        .get_user_db()
        .get_committed_version_live_db()
        .unwrap()
        .expect("Live DB must have a committed version")
}

/// A reader bound to `version`, like a storage snapshot whose deltas are already committed.
fn reader_at(
    flat_db: &FlatStateDb,
    version: u64,
) -> VersionedDeltaReader<NomtStateValues<UserNamespace>, DbCache> {
    VersionedDeltaReader::new(flat_db.get_user_db().clone(), Some(version), vec![])
}

fn assert_all_keys_at_version(
    flat_db: &FlatStateDb,
    expected: &[Vec<Option<SlotValue>>],
    version: u64,
    context: &str,
) {
    let reader = reader_at(flat_db, version);
    for idx in 0..KEYS {
        assert_eq!(
            reader.get_latest_borrowed(&key(idx)).unwrap(),
            expected[version as usize][idx as usize],
            "{context}: reader bound to version {version} read a wrong value for key {idx}"
        );
    }
}

fn assert_live_values_at_version(
    flat_db: &FlatStateDb,
    expected: &[Vec<Option<SlotValue>>],
    version: u64,
    context: &str,
) {
    for idx in 0..KEYS {
        assert_eq!(
            flat_db.get_user_db().get_live_value(&key(idx)).unwrap(),
            expected[version as usize][idx as usize],
            "{context}: live value of key {idx} is not the one of version {version}"
        );
    }
}

/// A reader that fills the cache while the commit waits for the other stores must get the
/// previous version, and the commit must overwrite what it cached.
#[test]
fn test_cache_filled_while_waiting_for_other_stores_is_overwritten() {
    let tempdir = tempfile::tempdir().unwrap();
    let flat_db = FlatStateDb::new(tempdir.path().to_path_buf(), 1_000_000).unwrap();
    let expected = expected_values(2);
    flat_db.commit(change_set(0)).unwrap();
    flat_db.commit(change_set(1)).unwrap();

    flat_db
        .commit_with_before_live(change_set(2), || {
            // Archival already holds version 2, the live DB and the cache are still at 1.
            assert_live_values_at_version(&flat_db, &expected, 1, "before live write");
            assert_all_keys_at_version(&flat_db, &expected, 1, "before live write");
            Ok(())
        })
        .unwrap();

    assert_live_values_at_version(&flat_db, &expected, 2, "after commit");
    assert_all_keys_at_version(&flat_db, &expected, 2, "after commit");
    assert_all_keys_at_version(&flat_db, &expected, 1, "after commit");
}

/// While the commit holds the cache write lock and the live write is pending, readers bypass the
/// cache and must still get the previous version.
#[test]
fn test_reads_while_live_write_is_pending_return_previous_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let flat_db = FlatStateDb::new(tempdir.path().to_path_buf(), 1_000_000).unwrap();
    let expected = expected_values(2);
    flat_db.commit(change_set(0)).unwrap();
    flat_db.commit(change_set(1)).unwrap();
    // Warm the cache with version 1 values.
    assert_live_values_at_version(&flat_db, &expected, 1, "warm up");

    let gate = CommitFaultInjectionLocation::BeforeCommittingLive;
    gate.arm_wait_gate();
    std::thread::scope(|s| {
        let commit = s.spawn(|| flat_db.commit(change_set(2)));
        assert!(
            gate.wait_until_wait_gate_reached(Duration::from_secs(30)),
            "Commit did not reach the live write"
        );
        assert!(
            flat_db.db_cache.inner.try_read().is_none(),
            "Commit must hold the cache write lock until the live write"
        );

        assert_live_values_at_version(&flat_db, &expected, 1, "live write pending");
        assert_all_keys_at_version(&flat_db, &expected, 1, "live write pending");

        gate.release_wait_gate();
        commit.join().unwrap().unwrap();
    });

    assert_live_values_at_version(&flat_db, &expected, 2, "after commit");
    assert_all_keys_at_version(&flat_db, &expected, 1, "after commit");
}

/// Readers race with a stream of commits on a cache small enough to evict constantly.
/// - A reader bound to version `v` must read exactly the values of `v`.
/// - An unbound read of the live value must return the value of some version between the live
///   version before and after the read.
#[test]
fn test_concurrent_reads_during_commits_are_consistent() {
    const VERSIONS: u64 = 40;
    const READERS: u64 = 8;
    // About 100 of 512 entries fit, so reads keep evicting and refilling the cache.
    const CACHE_SIZE_BYTES: usize = 16 * 1024;

    let tempdir = tempfile::tempdir().unwrap();
    let flat_db = FlatStateDb::new(tempdir.path().to_path_buf(), CACHE_SIZE_BYTES).unwrap();
    let expected = expected_values(VERSIONS);
    flat_db.commit(change_set(0)).unwrap();

    // Hold the cache write lock a little longer, so readers also hit that window.
    CommitFaultInjectionLocation::BeforeCommittingLive.set_sleep_env(1);
    let done = AtomicBool::new(false);
    let bound_reads = AtomicU64::new(0);
    let unbound_reads = AtomicU64::new(0);

    std::thread::scope(|s| {
        for reader_id in 0..READERS {
            let (flat_db, expected, done) = (&flat_db, &expected, &done);
            let (bound_reads, unbound_reads) = (&bound_reads, &unbound_reads);
            s.spawn(move || {
                let mut rng = StdRng::seed_from_u64(reader_id);
                while !done.load(Ordering::Relaxed) {
                    let idx = rng.gen_range(0..KEYS);
                    if rng.gen_bool(0.5) {
                        let version = live_version(flat_db);
                        let reader = reader_at(flat_db, version);
                        assert_eq!(
                            reader.get_latest_borrowed(&key(idx)).unwrap(),
                            expected[version as usize][idx as usize],
                            "Reader bound to version {version} read a wrong value for key {idx}"
                        );
                        bound_reads.fetch_add(1, Ordering::Relaxed);
                    } else {
                        let before = live_version(flat_db);
                        let value = flat_db.get_user_db().get_live_value(&key(idx)).unwrap();
                        let after = live_version(flat_db);
                        assert!(
                            (before..=after)
                                .any(|version| expected[version as usize][idx as usize] == value),
                            "Live value of key {idx} is {value:?}, which no version in {before}..={after} has"
                        );
                        unbound_reads.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }

        let mut rng = StdRng::seed_from_u64(READERS);
        for version in 1..=VERSIONS {
            let wait = Duration::from_micros(rng.gen_range(0..2_000));
            flat_db
                .commit_with_before_live(change_set(version), || {
                    // Simulates waiting for NOMT, ledger and accessory.
                    std::thread::sleep(wait);
                    Ok(())
                })
                .unwrap();
        }
        done.store(true, Ordering::Relaxed);
    });
    std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);

    assert_eq!(live_version(&flat_db), VERSIONS);
    assert_live_values_at_version(&flat_db, &expected, VERSIONS, "after all commits");
    assert!(
        bound_reads.load(Ordering::Relaxed) > 0 && unbound_reads.load(Ordering::Relaxed) > 0,
        "Readers must have run during the commits"
    );
}
