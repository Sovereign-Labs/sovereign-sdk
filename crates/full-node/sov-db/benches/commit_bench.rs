//! Measures `NomtStorageManager::finalize` (one `DbGroup::commit` per slot) under a
//! production-like per-slot workload.
//!
//! Workload per slot (estimated from the code paths of a bank-transfer-heavy rollup):
//! - user state: ~15k unique writes (85% updates, 10% inserts, 5% deletes);
//!   key sizes 64/56/34 bytes, values 16 bytes for balances and a heavy-tailed size for
//!   uniqueness-like items; NOMT stores `sha256(value) || len` as in production.
//! - kernel state: 12 writes (6 slot-indexed inserts, 6 updates).
//! - ledger: 5k transactions in 2 batches, 1-2 events each.
//! - accessory: 4k items.
//!
//! Prints `commit_ms_p50:` (the campaign metric) and a few informational lines.
//! Env overrides (for calibration only): `BENCH_DIR`, `BENCH_USER_WRITES`, `BENCH_SLOTS`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use nomt::trie::KeyPath;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rockbound::SchemaBatch;
use sha2::{Digest, Sha256};
use sov_db::accessory_db::AccessoryDb;
use sov_db::config::{PrunerConfig, RollupDbConfig};
use sov_db::historical_state::HistoricalStateReader;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::schema::types::slot_key::{SlotKey, SlotValue};
use sov_db::state_db_nomt::NomtSessionBuilder;
use sov_db::storage_manager::{
    InitializableNativeNomtStorage, NomtChangeSet, NomtStorageManager, StateFinishedSession,
    WitnessMode,
};
use sov_mock_da::{MockBlock, MockBlockHeader, MockDaSpec, MockHash};
use sov_rollup_interface::common::{HexHash, SlotNumber};
use sov_rollup_interface::da::Time;
use sov_rollup_interface::stf::{BatchReceipt, StoredEvent, TransactionReceipt, TxEffect};
use sov_rollup_interface::storage::HierarchicalStorageManager;

const SEED: u64 = 0x5eed_1234;
const PREFILL_USER_KEYS: u64 = 2_000_000;
const PREFILL_SLOTS: u64 = 8;
const PREFILL_KERNEL_KEYS: u64 = 20_000;
const USER_WRITES_PER_SLOT: usize = 15_000;
const KERNEL_WRITES_PER_SLOT: u64 = 12;
const TXS_PER_SLOT: usize = 5_000;
const BATCHES_PER_SLOT: usize = 2;
const ACCESSORY_ITEMS_PER_SLOT: usize = 4_000;
const WARMUP_SLOTS: u64 = 10;
const MEASURED_SLOTS: u64 = 60;

struct BenchStorage {
    builder: NomtSessionBuilder<Sha256, MockHash>,
    historical: HistoricalStateReader,
}

impl InitializableNativeNomtStorage<Sha256, MockHash> for BenchStorage {
    fn new(
        state_db: NomtSessionBuilder<Sha256, MockHash>,
        historical_state: HistoricalStateReader,
        _accessory_db: AccessoryDb,
        _strict_mode: bool,
        _witness_mode: WitnessMode,
    ) -> Self {
        Self {
            builder: state_db,
            historical: historical_state,
        }
    }
}

type Manager = NomtStorageManager<MockDaSpec, Sha256, BenchStorage>;

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn header(height: u64) -> MockBlockHeader {
    MockBlockHeader::new(height, Time::from_secs(1_700_000_000 + height as i64))
}

/// Deterministic user key for an index: 2-byte prefix + hash-derived body.
fn user_key(idx: u64) -> Vec<u8> {
    let len = match idx % 20 {
        0..=10 => 64,
        11..=13 => 56,
        _ => 34,
    };
    let digest = Sha256::digest(idx.to_le_bytes());
    let mut key = Vec::with_capacity(len);
    key.push(10);
    key.push((idx % 3) as u8);
    while key.len() < len {
        let take = (len - key.len()).min(32);
        key.extend_from_slice(&digest[..take]);
    }
    key
}

fn user_value(idx: u64, rng: &mut StdRng) -> Vec<u8> {
    let size = if user_key_len(idx) == 34 {
        let roll: u32 = rng.gen_range(0..100);
        if roll < 85 {
            rng.gen_range(8..=48)
        } else if roll < 98 {
            rng.gen_range(100..=2_000)
        } else {
            rng.gen_range(10_000..=54_000)
        }
    } else {
        16
    };
    let mut value = vec![0u8; size];
    rng.fill(&mut value[..]);
    value
}

fn user_key_len(idx: u64) -> usize {
    match idx % 20 {
        0..=10 => 64,
        11..=13 => 56,
        _ => 34,
    }
}

fn kernel_key(idx: u64) -> Vec<u8> {
    let mut key = vec![200, (idx % 6) as u8];
    key.extend_from_slice(&idx.to_be_bytes());
    key
}

fn random_bytes(rng: &mut StdRng, min: usize, max: usize) -> Vec<u8> {
    let mut value = vec![0u8; rng.gen_range(min..=max)];
    rng.fill(&mut value[..]);
    value
}

type Writes = BTreeMap<Vec<u8>, Option<Vec<u8>>>;

struct SlotInput {
    user: Writes,
    kernel: Writes,
    accessory: Vec<(Vec<u8>, Option<SlotValue>)>,
    txs: usize,
}

/// Tracks which user/kernel keys exist so updates and deletes hit live keys.
struct KeySpace {
    /// Keys `[0, next_user)` were inserted; `[deleted_below, delete_cursor)` were deleted.
    next_user: u64,
    delete_cursor: u64,
    next_kernel: u64,
}

impl KeySpace {
    fn slot(&mut self, rng: &mut StdRng, user_writes: usize, slot: u64) -> SlotInput {
        let mut user = Writes::new();
        let inserts = user_writes / 10;
        let deletes = user_writes / 20;
        for _ in 0..inserts {
            let idx = self.next_user;
            self.next_user += 1;
            user.insert(user_key(idx), Some(user_value(idx, rng)));
        }
        for _ in 0..deletes {
            let idx = self.delete_cursor;
            self.delete_cursor += 1;
            user.insert(user_key(idx), None);
        }
        while user.len() < user_writes {
            let idx = rng.gen_range(self.delete_cursor..self.next_user);
            user.insert(user_key(idx), Some(user_value(idx, rng)));
        }

        let mut kernel = Writes::new();
        for _ in 0..KERNEL_WRITES_PER_SLOT / 2 {
            let idx = self.next_kernel;
            self.next_kernel += 1;
            kernel.insert(kernel_key(idx), Some(random_bytes(rng, 8, 150)));
        }
        while kernel.len() < KERNEL_WRITES_PER_SLOT as usize {
            let idx = rng.gen_range(0..self.next_kernel);
            kernel.insert(kernel_key(idx), Some(random_bytes(rng, 8, 150)));
        }

        let accessory = (0..ACCESSORY_ITEMS_PER_SLOT)
            .map(|i| {
                let mut key = vec![30, (i % 4) as u8];
                key.extend_from_slice(&slot.to_be_bytes());
                key.extend_from_slice(&(i as u32).to_be_bytes());
                (key, Some(SlotValue::from(random_bytes(rng, 16, 600))))
            })
            .collect();

        SlotInput {
            user,
            kernel,
            accessory,
            txs: TXS_PER_SLOT,
        }
    }
}

fn nomt_writes(writes: &Writes) -> Vec<(KeyPath, nomt::KeyReadWrite)> {
    let mut out: Vec<_> = writes
        .iter()
        .map(|(key, value)| {
            let key_path: KeyPath = Sha256::digest(key).into();
            let nomt_value = value.as_ref().map(|v| {
                let hash: [u8; 32] = Sha256::digest(v).into();
                sov_db_types::val_hash_and_size_inner(hash, v.len() as u32)
            });
            (key_path, nomt::KeyReadWrite::Write(nomt_value))
        })
        .collect();
    out.sort_by_key(|(k, _)| *k);
    out
}

fn ledger_commit(
    height: u64,
    txs: usize,
    rng: &mut StdRng,
) -> SlotCommit<MockBlock, (), ()> {
    let block = MockBlock {
        header: header(height),
        ..Default::default()
    };
    let mut commit = SlotCommit::new(block, vec![]);
    let per_batch = txs / BATCHES_PER_SLOT;
    for b in 0..BATCHES_PER_SLOT {
        let tx_receipts = (0..per_batch)
            .map(|i| {
                let tx_hash: [u8; 32] = rng.gen();
                let events = (0..1 + (i % 2))
                    .map(|e| {
                        let key: &[u8] = if e == 0 {
                            b"Bank/TokenTransferred"
                        } else {
                            b"Uniqueness/NonceUpdated"
                        };
                        StoredEvent::new(key, &random_bytes(rng, 80, 150), tx_hash)
                    })
                    .collect();
                TransactionReceipt {
                    tx_hash: HexHash::new(tx_hash),
                    body_to_save: None,
                    events,
                    receipt: TxEffect::Successful(()),
                }
            })
            .collect();
        let mut batch_hash = [0u8; 32];
        batch_hash[..8].copy_from_slice(&height.to_be_bytes());
        batch_hash[8] = b as u8;
        commit.add_batch(BatchReceipt {
            batch_hash,
            tx_receipts,
            ignored_tx_receipts: vec![],
            inner: (),
        });
    }
    commit
}

/// Builds and saves the change set for `height`; returns nothing, the caller times `finalize`.
fn save_slot(manager: &mut Manager, height: u64, input: SlotInput, rng: &mut StdRng) {
    let hdr = header(height);
    let (storage, ledger_reader) = manager.create_state_for(&hdr).unwrap();
    let version = storage.historical.get_next_version();

    let sessions = storage.builder.begin_both_sessions(false).unwrap();
    let user = sessions.user.finish(nomt_writes(&input.user)).unwrap();
    let kernel = sessions.kernel.finish(nomt_writes(&input.kernel)).unwrap();
    let root: Vec<u8> = [user.root().into_inner(), kernel.root().into_inner()].concat();

    let to_slot = |w: Writes| {
        w.into_iter()
            .map(|(k, v)| (SlotKey::from_slice(&k), v.map(SlotValue::from)))
            .collect::<Vec<_>>()
    };
    let historical = HistoricalStateReader::materialize_values(
        to_slot(input.user),
        to_slot(input.kernel),
        root.clone(),
        version,
    )
    .unwrap();
    let accessory = AccessoryDb::materialize_values(input.accessory, version).unwrap();

    let ledger_db = LedgerDb::with_reader(ledger_reader).unwrap();
    let mut ledger: SchemaBatch = ledger_db
        .materialize_slot(ledger_commit(height, input.txs, rng), &root)
        .unwrap();
    let slot = SlotNumber::new(height);
    ledger.merge(ledger_db.materialize_latest_finalize_slot(slot, slot).unwrap());

    drop(storage);
    manager
        .save_change_set(
            &hdr,
            NomtChangeSet {
                state: StateFinishedSession::new(user, kernel),
                historical_state: historical,
                accessory,
            },
            ledger,
        )
        .unwrap();
}

fn percentile(sorted: &[Duration], p: usize) -> f64 {
    let idx = (sorted.len() * p / 100).min(sorted.len() - 1);
    sorted[idx].as_secs_f64() * 1000.0
}

fn main() {
    let total_start = Instant::now();
    let base_dir = std::env::var("BENCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let tmp = tempfile::tempdir_in(base_dir).unwrap();
    let user_writes = env_or("BENCH_USER_WRITES", USER_WRITES_PER_SLOT);
    let measured_slots = env_or("BENCH_SLOTS", MEASURED_SLOTS);

    let config = RollupDbConfig {
        path: tmp.path().to_path_buf(),
        ledger_db_path: None,
        state_cache_size: None,
        user_commit_concurrency: Some(4),
        user_hashtable_buckets: Some(4_000_000),
        user_preallocate_ht: Some(false),
        user_page_cache_size: None,
        user_leaf_cache_size: None,
        user_page_cache_upper_levels: None,
        kernel_commit_concurrency: Some(2),
        kernel_hashtable_buckets: Some(256_000),
        kernel_preallocate_ht: Some(false),
        kernel_page_cache_size: None,
        kernel_leaf_cache_size: None,
        kernel_page_cache_upper_levels: None,
        pruner: PrunerConfig::Off,
    };
    let mut manager = Manager::new(config, false).unwrap();
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut keys = KeySpace {
        next_user: 0,
        delete_cursor: 0,
        next_kernel: 0,
    };

    // Prefill: large commits that build the initial state.
    let prefill_start = Instant::now();
    let mut height = 1;
    for _ in 0..PREFILL_SLOTS {
        let mut user = Writes::new();
        for _ in 0..PREFILL_USER_KEYS / PREFILL_SLOTS {
            let idx = keys.next_user;
            keys.next_user += 1;
            user.insert(user_key(idx), Some(user_value(idx, &mut rng)));
        }
        let mut kernel = Writes::new();
        for _ in 0..PREFILL_KERNEL_KEYS / PREFILL_SLOTS {
            let idx = keys.next_kernel;
            keys.next_kernel += 1;
            kernel.insert(kernel_key(idx), Some(random_bytes(&mut rng, 8, 150)));
        }
        let input = SlotInput {
            user,
            kernel,
            accessory: vec![],
            txs: 2,
        };
        save_slot(&mut manager, height, input, &mut rng);
        manager.finalize(&header(height)).unwrap();
        height += 1;
    }
    let prefill = prefill_start.elapsed();

    let mut timings = Vec::with_capacity(measured_slots as usize);
    for i in 0..WARMUP_SLOTS + measured_slots {
        let input = keys.slot(&mut rng, user_writes, height);
        save_slot(&mut manager, height, input, &mut rng);
        let start = Instant::now();
        manager.finalize(&header(height)).unwrap();
        let elapsed = start.elapsed();
        if i >= WARMUP_SLOTS {
            timings.push(elapsed);
        }
        height += 1;
    }

    timings.sort();
    let mean = timings.iter().sum::<Duration>().as_secs_f64() * 1000.0 / timings.len() as f64;
    println!("commit_ms_p50: {:.2}", percentile(&timings, 50));
    println!("commit_ms_p90: {:.2}", percentile(&timings, 90));
    println!("commit_ms_mean: {:.2}", mean);
    println!("prefill_s: {:.1}", prefill.as_secs_f64());
    println!("total_s: {:.1}", total_start.elapsed().as_secs_f64());
}
